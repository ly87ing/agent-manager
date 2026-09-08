"""不可变 Skill 版本存储。

给定一个含根 SKILL.md 的 skill 目录，把所有普通文件（排除 .git）快照到
store/<sha256>/content，版本 ID 由排序相对路径、文件内容摘要与执行位计算。
同内容重复导入复用原版本；源目录后续修改产生新版本，绝不原地修改旧版本。

仅做纯本地文件操作：无外部命令、网络、git clone、安装或自动清理。
本模块是独立存储切片，尚未接入 link_skills / engine。
"""

import hashlib
import json
import os
import re
import shutil
import stat
import tempfile
from pathlib import Path

SKILL_FILE = "SKILL.md"
MANIFEST_FILE = "manifest.json"
CONTENT_DIRNAME = "content"
EXCLUDED_DIRS = {".git"}
MANIFEST_FORMAT = 1

_VERSION_ID_RE = re.compile(r"^[0-9a-f]{64}$")


class SkillVersionError(ValueError):
    """skill 版本存储的所有校验/完整性错误。"""


def _is_executable(mode):
    # 只看用户执行位，不依据组/其他位推断。
    return bool(mode & stat.S_IXUSR)


def _file_mode(executable):
    if executable:
        # 保留用户执行位，不扩展到组/其他用户。
        return 0o500
    return 0o400


def _collect_source_files(source_dir):
    """枚举源目录中的普通文件，返回 {相对posix路径: (绝对路径, 是否可执行)}。"""
    entries = {}
    for root, dirs, files in os.walk(source_dir, followlinks=False):
        root_path = Path(root)
        kept_dirs = []
        for dirname in sorted(dirs):
            dir_path = root_path / dirname
            if dirname in EXCLUDED_DIRS:
                continue
            if dir_path.is_symlink():
                raise SkillVersionError(f"拒绝符号链接目录: {dir_path}")
            kept_dirs.append(dirname)
        dirs[:] = kept_dirs
        for filename in sorted(files):
            file_path = root_path / filename
            if file_path.is_symlink():
                raise SkillVersionError(f"拒绝符号链接文件: {file_path}")
            if not file_path.is_file():
                raise SkillVersionError(f"拒绝非常规文件: {file_path}")
            rel = file_path.relative_to(source_dir).as_posix()
            entries[rel] = (file_path, _is_executable(file_path.stat().st_mode))
    return entries


def _hash_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def compute_version_id(entries):
    """由排序相对路径、内容摘要、执行位计算 sha256 版本 ID。"""
    manifest_entries = []
    for rel in sorted(entries):
        file_path, executable = entries[rel]
        manifest_entries.append(
            {"path": rel, "sha256": _hash_file(file_path), "executable": bool(executable)}
        )
    canonical = json.dumps(manifest_entries, separators=(",", ":"), sort_keys=True).encode("utf-8")
    return hashlib.sha256(canonical).hexdigest(), manifest_entries


def _validate_version_id(version_id):
    if not isinstance(version_id, str) or not _VERSION_ID_RE.match(version_id):
        raise SkillVersionError(f"非法版本 ID: {version_id!r}")


def _validate_store_not_nested(source_dir, store_dir):
    source_resolved = source_dir.resolve()
    store_resolved = store_dir.resolve(strict=False)
    if store_resolved == source_resolved or source_resolved in store_resolved.parents:
        raise SkillVersionError("store 目录不能位于 source 目录内部（避免递归导入）")


def _build_version_tree(source_dir, entries, manifest_entries, staging_dir):
    content_dir = staging_dir / CONTENT_DIRNAME
    content_dir.mkdir(parents=True)
    for rel in sorted(entries):
        src_path, executable = entries[rel]
        dest_path = content_dir / rel
        # 相对路径来自 os.walk，不允许出现逃逸成分。
        if dest_path.resolve(strict=False) != content_dir.resolve() / rel:
            raise SkillVersionError(f"非法相对路径: {rel}")
        dest_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src_path, dest_path)
        os.chmod(dest_path, _file_mode(executable))
    manifest = {"format": MANIFEST_FORMAT, "files": manifest_entries}
    manifest_path = staging_dir / MANIFEST_FILE
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.chmod(manifest_path, 0o400)
    # copy 期间源可能被改动：rename 前必须复核 staging 内容与 manifest 一致，
    # 不一致则中止发布（不产出与版本 ID 不符的内容）。
    _verify_content(staging_dir, manifest_entries)
    # 版本目录整体写保护（自内向外）。
    for root, dirs, _files in os.walk(staging_dir, topdown=False):
        for dirname in dirs:
            os.chmod(Path(root) / dirname, 0o500)
    # 顶层目录留待 rename 后再写保护（BSD 上 rename 不可写目录会失败）。


def _restore_writable(path):
    path = Path(path)
    if not path.exists():
        return
    for root, dirs, files in os.walk(path, topdown=False):
        for name in files:
            try:
                os.chmod(Path(root) / name, 0o644)
            except OSError:
                pass
        for name in dirs:
            try:
                os.chmod(Path(root) / name, 0o755)
            except OSError:
                pass
    try:
        os.chmod(path, 0o755)
    except OSError:
        pass


def _read_manifest(version_dir):
    manifest_path = Path(version_dir) / MANIFEST_FILE
    if manifest_path.is_symlink():
        raise SkillVersionError(f"manifest 是符号链接，拒绝读取: {manifest_path}")
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise SkillVersionError(f"manifest 损坏: {manifest_path}: {exc}") from exc
    if not isinstance(manifest, dict) or manifest.get("format") != MANIFEST_FORMAT:
        raise SkillVersionError(f"manifest 格式不受支持: {manifest_path}")
    files = manifest.get("files")
    if not isinstance(files, list):
        raise SkillVersionError(f"manifest 缺少 files 列表: {manifest_path}")
    for entry in files:
        if (
            not isinstance(entry, dict)
            or not isinstance(entry.get("path"), str)
            or not isinstance(entry.get("sha256"), str)
            or not _VERSION_ID_RE.match(entry["sha256"])
            or not isinstance(entry.get("executable"), bool)
        ):
            raise SkillVersionError(f"manifest 条目非法: {manifest_path}: {entry!r}")
    return files


def _verify_content(version_dir, files):
    content_dir = Path(version_dir) / CONTENT_DIRNAME
    if content_dir.is_symlink() or not content_dir.is_dir():
        raise SkillVersionError(f"版本缺少 content 目录: {version_dir}")
    # 先完整遍历拒绝符号链接与未知文件，再按 manifest 读取哈希，
    # 避免在报错前顺着 symlink 父目录读取外部文件。
    actual_paths = set()
    for root, dirs, filenames in os.walk(content_dir, followlinks=False):
        for dirname in dirs:
            if (Path(root) / dirname).is_symlink():
                raise SkillVersionError(f"版本含符号链接目录: {Path(root) / dirname}")
        for filename in filenames:
            file_path = Path(root) / filename
            if file_path.is_symlink():
                raise SkillVersionError(f"版本含符号链接文件: {file_path}")
            actual_paths.add(file_path.relative_to(content_dir).as_posix())
    expected_paths = set()
    for entry in files:
        rel = entry["path"]
        if rel.startswith("/") or ".." in rel.split("/"):
            raise SkillVersionError(f"manifest 含路径逃逸: {rel!r}")
        expected_paths.add(rel)
        file_path = content_dir / rel
        if not file_path.is_file() or file_path.is_symlink():
            raise SkillVersionError(f"版本内容缺失或非法: {file_path}")
        if _hash_file(file_path) != entry["sha256"]:
            raise SkillVersionError(f"版本内容被篡改: {file_path}")
        if _is_executable(file_path.stat().st_mode) != entry["executable"]:
            raise SkillVersionError(f"版本执行位被改动: {file_path}")
    extra = actual_paths - expected_paths
    if extra:
        raise SkillVersionError(f"版本含未知文件: {sorted(extra)}")


def snapshot_skill(source_dir, store_dir):
    """把 skill 源目录快照进 store，返回 {"version_id", "content_path"}。

    同内容重复导入复用原版本；已存在版本先做完整性校验，损坏则报错而不是覆盖。
    """
    source_dir = Path(source_dir)
    store_dir = Path(store_dir)
    if source_dir.is_symlink():
        raise SkillVersionError(f"source 目录不能是符号链接: {source_dir}")
    if not source_dir.is_dir():
        raise SkillVersionError(f"source 目录不存在: {source_dir}")
    if not (source_dir / SKILL_FILE).is_file():
        raise SkillVersionError(f"source 缺少根 {SKILL_FILE}: {source_dir}")
    if store_dir.is_symlink():
        raise SkillVersionError(f"store 目录不能是符号链接: {store_dir}")
    _validate_store_not_nested(source_dir, store_dir)

    entries = _collect_source_files(source_dir)
    version_id, manifest_entries = compute_version_id(entries)
    version_dir = store_dir / version_id

    if version_dir.exists():
        # 已发布版本：只校验、不覆盖。
        verify_skill_version(store_dir, version_id)
        return {"version_id": version_id, "content_path": version_dir / CONTENT_DIRNAME}

    store_dir.mkdir(parents=True, exist_ok=True)
    # 同盘临时目录 + 原子 rename，失败不留半成品。
    staging_dir = Path(tempfile.mkdtemp(prefix=f".{version_id}.tmp-", dir=store_dir))
    try:
        _build_version_tree(source_dir, entries, manifest_entries, staging_dir)
        try:
            os.rename(staging_dir, version_dir)
        except OSError:
            # 并发发布同内容版本：目标已存在则校验复用，不当作竞争错误。
            if not version_dir.exists():
                raise
            verify_skill_version(store_dir, version_id)
            _restore_writable(staging_dir)
            shutil.rmtree(staging_dir, ignore_errors=True)
            return {"version_id": version_id, "content_path": version_dir / CONTENT_DIRNAME}
        os.chmod(version_dir, 0o500)
    except BaseException:
        _restore_writable(staging_dir)
        shutil.rmtree(staging_dir, ignore_errors=True)
        raise
    return {"version_id": version_id, "content_path": version_dir / CONTENT_DIRNAME}


def verify_skill_version(store_dir, version_id):
    """校验已发布版本的 manifest 与内容摘要，返回 content 路径。"""
    _validate_version_id(version_id)
    store_dir = Path(store_dir)
    if store_dir.is_symlink():
        raise SkillVersionError(f"store 目录不能是符号链接: {store_dir}")
    version_dir = store_dir / version_id
    if version_dir.is_symlink():
        raise SkillVersionError(f"版本目录是符号链接，拒绝: {version_dir}")
    if not version_dir.is_dir():
        raise SkillVersionError(f"版本不存在: {version_id}")
    files = _read_manifest(version_dir)
    _verify_content(version_dir, files)
    # manifest 声明的摘要集合必须与版本 ID 一致。
    canonical = json.dumps(files, separators=(",", ":"), sort_keys=True).encode("utf-8")
    if hashlib.sha256(canonical).hexdigest() != version_id:
        raise SkillVersionError(f"manifest 与版本 ID 不匹配: {version_id}")
    return version_dir / CONTENT_DIRNAME
