import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.skill_versions import (
    CONTENT_DIRNAME,
    MANIFEST_FILE,
    SkillVersionError,
    snapshot_skill,
    verify_skill_version,
)


def _make_skill(root, files):
    for rel, content in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(content, tuple):
            text, mode = content
            path.write_text(text, encoding="utf-8")
            os.chmod(path, mode)
        else:
            path.write_text(content, encoding="utf-8")


def _restore_writable(path):
    for root, dirs, files in os.walk(path, topdown=False):
        for name in files:
            os.chmod(Path(root) / name, 0o644)
        for name in dirs:
            os.chmod(Path(root) / name, 0o755)
    os.chmod(path, 0o755)


class SkillVersionsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="skill-versions-test-"))
        self.source = self.tmp / "source"
        self.store = self.tmp / "store"
        self.source.mkdir()

    def tearDown(self):
        if self.store.exists():
            for child in self.store.iterdir():
                if child.is_dir():
                    _restore_writable(child)
        import shutil

        shutil.rmtree(self.tmp, ignore_errors=True)

    def _snapshot(self):
        return snapshot_skill(self.source, self.store)

    def test_snapshot_basic_and_idempotent(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        first = self._snapshot()
        second = self._snapshot()
        self.assertEqual(first["version_id"], second["version_id"])
        self.assertEqual(first["content_path"], second["content_path"])
        self.assertTrue((first["content_path"] / "SKILL.md").is_file())
        self.assertEqual(
            (first["content_path"] / "SKILL.md").read_text(encoding="utf-8"),
            "---\nname: demo\n---\n",
        )
        # store 中没有遗留临时目录。
        leftovers = [p.name for p in self.store.iterdir() if ".tmp-" in p.name]
        self.assertEqual(leftovers, [])

    def test_modified_source_creates_new_version_old_bytes_unchanged(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\nv1\n"})
        first = self._snapshot()
        old_bytes = (first["content_path"] / "SKILL.md").read_bytes()

        (self.source / "SKILL.md").write_text("---\nname: demo\n---\nv2\n", encoding="utf-8")
        second = self._snapshot()
        self.assertNotEqual(first["version_id"], second["version_id"])
        self.assertTrue(second["content_path"].is_dir())
        # 旧版本字节不变。
        self.assertEqual((first["content_path"] / "SKILL.md").read_bytes(), old_bytes)

    def test_exec_bit_distinguishes_versions(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n", "run.sh": "echo hi\n"})
        first = self._snapshot()
        os.chmod(self.source / "run.sh", 0o755)
        second = self._snapshot()
        self.assertNotEqual(first["version_id"], second["version_id"])
        exec_mode = (second["content_path"] / "run.sh").stat().st_mode
        self.assertTrue(exec_mode & stat.S_IXUSR)
        plain_mode = (first["content_path"] / "run.sh").stat().st_mode
        self.assertFalse(plain_mode & stat.S_IXUSR)

    def test_nested_files_and_git_excluded(self):
        _make_skill(
            self.source,
            {
                "SKILL.md": "---\nname: demo\n---\n",
                "a/b/c.txt": "nested\n",
                ".git/HEAD": "ref: refs/heads/main\n",
            },
        )
        result = self._snapshot()
        self.assertEqual(
            (result["content_path"] / "a" / "b" / "c.txt").read_text(encoding="utf-8"),
            "nested\n",
        )
        self.assertFalse((result["content_path"] / ".git").exists())

    def test_missing_skill_md_rejected(self):
        _make_skill(self.source, {"readme.md": "hi\n"})
        with self.assertRaises(SkillVersionError):
            self._snapshot()
        self.assertFalse(self.store.exists())

    def test_symlink_rejected(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        os.symlink(self.source / "SKILL.md", self.source / "link.md")
        with self.assertRaises(SkillVersionError):
            self._snapshot()
        self.assertFalse(self.store.exists())

    def test_store_inside_source_rejected(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        with self.assertRaises(SkillVersionError):
            snapshot_skill(self.source, self.source / "store")

    def test_invalid_version_id_rejected(self):
        for bad in ("../x", "not-a-hash", "A" * 64, "", None):
            with self.assertRaises(SkillVersionError):
                verify_skill_version(self.store, bad)

    def test_verify_missing_version_rejected(self):
        with self.assertRaises(SkillVersionError):
            verify_skill_version(self.store, "0" * 64)

    def test_tampered_content_rejected(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        result = self._snapshot()
        target = result["content_path"] / "SKILL.md"
        os.chmod(target, 0o644)
        target.write_text("tampered\n", encoding="utf-8")
        with self.assertRaises(SkillVersionError):
            verify_skill_version(self.store, result["version_id"])

    def test_bad_manifest_rejected(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        result = self._snapshot()
        manifest_path = self.store / result["version_id"] / MANIFEST_FILE
        os.chmod(manifest_path, 0o644)
        manifest_path.write_text("not json", encoding="utf-8")
        with self.assertRaises(SkillVersionError):
            verify_skill_version(self.store, result["version_id"])

    def test_extra_file_rejected(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        result = self._snapshot()
        content_dir = self.store / result["version_id"] / CONTENT_DIRNAME
        os.chmod(content_dir, 0o755)
        (content_dir / "extra.txt").write_text("x", encoding="utf-8")
        with self.assertRaises(SkillVersionError):
            verify_skill_version(self.store, result["version_id"])

    def test_interrupted_publish_leaves_no_partial_version(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n", "big.txt": "data\n"})
        import src.skill_versions as sv

        real_build = sv._build_version_tree

        def boom(*args, **kwargs):
            real_build(*args, **kwargs)
            raise RuntimeError("simulated failure")

        sv._build_version_tree = boom
        try:
            with self.assertRaises(RuntimeError):
                self._snapshot()
        finally:
            sv._build_version_tree = real_build
        # 无版本目录、无临时目录残留；重试可成功。
        self.assertEqual(list(self.store.iterdir()), [])
        result = self._snapshot()
        self.assertTrue(result["content_path"].is_dir())

    def test_corrupt_existing_version_not_overwritten(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        result = self._snapshot()
        target = result["content_path"] / "SKILL.md"
        os.chmod(target, 0o644)
        target.write_text("corrupted\n", encoding="utf-8")
        # 同内容重复导入必须报错，而不是静默覆盖损坏版本。
        with self.assertRaises(SkillVersionError):
            self._snapshot()

    def test_source_dir_not_modified(self):
        _make_skill(
            self.source,
            {"SKILL.md": "---\nname: demo\n---\n", "run.sh": ("echo hi\n", 0o755)},
        )
        before = {
            p.relative_to(self.source).as_posix(): (p.read_bytes(), p.stat().st_mode)
            for p in self.source.rglob("*")
            if p.is_file()
        }
        self._snapshot()
        after = {
            p.relative_to(self.source).as_posix(): (p.read_bytes(), p.stat().st_mode)
            for p in self.source.rglob("*")
            if p.is_file()
        }
        self.assertEqual(before, after)

    def test_version_files_read_only(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        result = self._snapshot()
        skill_md = result["content_path"] / "SKILL.md"
        self.assertFalse(skill_md.stat().st_mode & stat.S_IWUSR)
        version_dir = self.store / result["version_id"]
        self.assertFalse(version_dir.stat().st_mode & stat.S_IWUSR)

    def test_exec_bit_not_widened_to_group_other(self):
        _make_skill(
            self.source,
            {"SKILL.md": "---\nname: demo\n---\n", "run.sh": ("echo hi\n", 0o700)},
        )
        result = self._snapshot()
        stored_mode = (result["content_path"] / "run.sh").stat().st_mode
        self.assertTrue(stored_mode & stat.S_IXUSR)
        self.assertEqual(stored_mode & 0o077, 0)

    def test_source_mutation_during_copy_aborts_publish(self):
        _make_skill(
            self.source,
            {"SKILL.md": "---\nname: demo\n---\n", "a.txt": "before\n", "b.txt": "data\n"},
        )
        import src.skill_versions as sv

        real_copyfile = sv.shutil.copyfile

        def mutating_copyfile(src, dst):
            if Path(src).name == "a.txt":
                # 模拟 hash 之后、copy 之前源被改动：staging 内容将与 manifest 不一致。
                (self.source / "a.txt").write_text("after\n", encoding="utf-8")
            return real_copyfile(src, dst)

        sv.shutil.copyfile = mutating_copyfile
        try:
            with self.assertRaises(SkillVersionError):
                self._snapshot()
        finally:
            sv.shutil.copyfile = real_copyfile
        # 未发布损坏版本，无半成品残留。
        self.assertEqual(list(self.store.iterdir()), [])

    def test_source_root_symlink_rejected(self):
        real_source = self.tmp / "real-source"
        real_source.mkdir()
        _make_skill(real_source, {"SKILL.md": "---\nname: demo\n---\n"})
        link_source = self.tmp / "link-source"
        os.symlink(real_source, link_source)
        with self.assertRaises(SkillVersionError):
            snapshot_skill(link_source, self.store)

    def test_store_symlink_rejected(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        real_store = self.tmp / "real-store"
        real_store.mkdir()
        link_store = self.tmp / "link-store"
        os.symlink(real_store, link_store)
        with self.assertRaises(SkillVersionError):
            snapshot_skill(self.source, link_store)
        with self.assertRaises(SkillVersionError):
            result = snapshot_skill(self.source, self.store)
            verify_skill_version(link_store, result["version_id"])

    def _make_writable(self, path):
        _restore_writable(path)

    def test_version_dir_symlink_rejected(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        result = self._snapshot()
        version_dir = self.store / result["version_id"]
        decoy = self.tmp / "decoy"
        decoy.mkdir()
        self._make_writable(self.store)
        version_dir.rename(self.tmp / "saved-version")
        os.symlink(decoy, version_dir)
        with self.assertRaises(SkillVersionError):
            verify_skill_version(self.store, result["version_id"])
        # 恢复以便 tearDown 清理。
        version_dir.unlink()
        (self.tmp / "saved-version").rename(version_dir)

    def test_content_symlink_rejected(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        result = self._snapshot()
        version_dir = self.store / result["version_id"]
        self._make_writable(version_dir)
        content_dir = version_dir / CONTENT_DIRNAME
        content_dir.rename(self.tmp / "saved-content")
        os.symlink(self.tmp, content_dir)
        with self.assertRaises(SkillVersionError):
            verify_skill_version(self.store, result["version_id"])

    def test_manifest_symlink_rejected(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        result = self._snapshot()
        version_dir = self.store / result["version_id"]
        self._make_writable(version_dir)
        manifest_path = version_dir / MANIFEST_FILE
        manifest_path.rename(self.tmp / "saved-manifest")
        os.symlink(self.tmp / "saved-manifest", manifest_path)
        with self.assertRaises(SkillVersionError):
            verify_skill_version(self.store, result["version_id"])

    def test_symlink_subdir_in_content_rejected(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        result = self._snapshot()
        content_dir = self.store / result["version_id"] / CONTENT_DIRNAME
        self._make_writable(self.store / result["version_id"])
        os.symlink(self.tmp, content_dir / "linked-dir")
        with self.assertRaises(SkillVersionError):
            verify_skill_version(self.store, result["version_id"])

    def test_concurrent_same_version_rename_reuses_existing(self):
        _make_skill(self.source, {"SKILL.md": "---\nname: demo\n---\n"})
        import src.skill_versions as sv

        real_rename = os.rename
        state = {"raced": False}

        def racy_rename(src, dst):
            if not state["raced"]:
                state["raced"] = True
                # 并发者抢先完成同内容发布，本进程 rename 失败。
                real_rename(src, dst)
                raise FileExistsError("simulated rename race")
            return real_rename(src, dst)

        sv.os.rename = racy_rename
        try:
            result = self._snapshot()
        finally:
            sv.os.rename = real_rename
        self.assertTrue(state["raced"])
        self.assertTrue(result["content_path"].is_dir())
        # 复用后的版本可通过完整性校验。
        verify_skill_version(self.store, result["version_id"])
        leftovers = [p.name for p in self.store.iterdir() if ".tmp-" in p.name]
        self.assertEqual(leftovers, [])


if __name__ == "__main__":
    unittest.main()
