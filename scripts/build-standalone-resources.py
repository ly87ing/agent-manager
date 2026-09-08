#!/usr/bin/env python3
"""Build the explicit, self-contained engine resources shipped by Tauri."""
from __future__ import annotations

import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "app" / "src-tauri" / "resources" / "engine"
CACHE = ROOT / "app" / ".standalone-build"
PYTHON_METADATA = json.loads((Path(__file__).with_name("python-build-standalone.json")).read_text(encoding="utf-8"))
RELEASE_TAG = PYTHON_METADATA["release_tag"]
RELEASE_DATE = PYTHON_METADATA["release_date"]
RELEASE_URL = PYTHON_METADATA["source_release_url"]
PYTHON_VERSION = PYTHON_METADATA["python_version"]
ARCHIVES = {entry["target"]: entry for entry in PYTHON_METADATA["archives"]}

SEED_ENTRIES = (
    ("manage.sh", None), ("Makefile", None), ("src", {".py"}),
    ("config/agents.json", None), ("config/claude-env.json", None),
    ("config/claude-hooks.json", None), ("config/codex-hooks.json", None),
    ("config/manager.json", None), ("config/mcp.json", None),
    ("config/retired-skills.json", None), ("config/rules-targets.list", None),
    ("config/skills.json", None), ("config/shell-aliases.json", None),
    ("config/tools.json", None), ("contract/engine-v1", {".json"}),
    ("agent-mcp/sync_mcp.py", None), ("agent-lifecycle/sync_agents.py", None),
    ("agent-profile/sync_profile.py", None), ("agent-skill/link_skills.py", None),
    ("agent-tool/sync_tools.py", None), ("agent-rule/AGENTS.md", None),
    ("agent-rule/CLAUDE.md", None), ("agent-rule/hooks", {".sh"}),
    ("agent-rule/scripts", {".py", ".sh"}), ("agent-rule/rules/src", {".md"}),
    ("agent-rule/rules/dist", {".md"}), ("scripts/claude", {".sh"}),
    ("scripts/browser_us", {".py"}), ("scripts/audit_skill_references.py", None),
    ("scripts/check_no_secrets.py", None), ("scripts/sync_browser_google_us.py", None),
    ("scripts/init-secrets.sh", None),
)
SKIP_DIRS = {".git", "__pycache__", "node_modules", "repositories", "playwright-report", "test-results"}
SKIP_SUFFIXES = (".bak", ".env", ".pyc", ".pyo", ".log")


def _reject_symlink(path: Path) -> None:
    if path.is_symlink():
        raise RuntimeError(f"resource whitelist contains symlink: {path.relative_to(ROOT)}")


def _iter_seed_files(relative: str, suffixes: set[str] | None):
    source = ROOT / relative
    if not source.exists() and not source.is_symlink():
        raise RuntimeError(f"resource whitelist entry is missing: {relative}")
    _reject_symlink(source)
    if source.is_file():
        yield source
        return
    if not source.is_dir():
        raise RuntimeError(f"resource whitelist entry is not a file or directory: {relative}")
    for path in sorted(source.rglob("*")):
        if any(part in SKIP_DIRS or part.startswith(".") for part in path.relative_to(ROOT).parts):
            continue
        if path.name.endswith(SKIP_SUFFIXES) or ".bak." in path.name:
            continue
        _reject_symlink(path)
        if path.is_file() and (suffixes is None or any(path.name.endswith(s) for s in suffixes)):
            yield path


def _clean_bytecode(directory: Path) -> None:
    if not directory.exists():
        return
    for path in list(directory.rglob("*.py[co]")):
        if path.is_file() or path.is_symlink():
            path.unlink(missing_ok=True)
    for path in list(directory.rglob("__pycache__")):
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path, ignore_errors=True)


def _host_target() -> str:
    system, machine = platform.system(), platform.machine().lower()
    if system == "Darwin" and machine in {"x86_64", "amd64"}: return "x86_64-apple-darwin"
    if system == "Darwin" and machine in {"arm64", "aarch64"}: return "aarch64-apple-darwin"
    if system == "Linux" and machine in {"x86_64", "amd64"}: return "x86_64-unknown-linux-gnu"
    if system == "Linux" and machine in {"arm64", "aarch64"}: return "aarch64-unknown-linux-gnu"
    raise RuntimeError(f"unsupported host for bundled Python: {system}/{machine}")


def _download_runtime(target: str) -> tuple[Path, dict[str, str]]:
    metadata = ARCHIVES[target]
    cache = CACHE / "python" / target / metadata["archive_sha256"]
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / metadata["archive"]
    if not archive.exists():
        urllib.request.urlretrieve(metadata["archive_url"], archive)
    actual = hashlib.sha256(archive.read_bytes()).hexdigest()
    if actual != metadata["archive_sha256"]:
        archive.unlink(missing_ok=True)
        raise RuntimeError(f"Python archive SHA-256 mismatch for {target}: {actual}")
    extracted, python_root = cache / "extracted", cache / "extracted" / "python"
    if not python_root.exists():
        extracted.mkdir(parents=True, exist_ok=True)
        subprocess.run(["tar", "-xzf", str(archive), "-C", str(extracted)], check=True)
    _clean_bytecode(python_root)
    return python_root, metadata


def _validate_runtime(runtime: Path) -> None:
    root = runtime.resolve()
    if not runtime.is_dir():
        raise RuntimeError(f"standalone Python runtime is missing: {runtime}")
    for path in runtime.rglob("*"):
        if not path.is_symlink():
            continue
        target = path.resolve(strict=False)
        try:
            target.relative_to(root)
        except ValueError as exc:
            raise RuntimeError(f"runtime symlink escapes package: {path} -> {target}") from exc
        if not target.exists():
            raise RuntimeError(f"runtime symlink target is missing: {path} -> {target}")


def _validate_runtime_completeness(runtime: Path) -> None:
    _validate_runtime(runtime)
    pyc_files = list(runtime.rglob("*.py[co]")) + [p for p in runtime.rglob("__pycache__") if p.is_dir()]
    if pyc_files:
        rel_pyc = [p.relative_to(runtime).as_posix() for p in pyc_files[:5]]
        raise RuntimeError(f"runtime contains bytecode files ({len(pyc_files)} item(s)): {rel_pyc}")
    lib = runtime / "lib"
    if not lib.is_dir():
        raise RuntimeError("runtime lib directory missing")
    stdlib_dirs = [d for d in lib.iterdir() if d.is_dir() and d.name.startswith("python3.")]
    if not stdlib_dirs:
        raise RuntimeError("runtime standard library directory missing in lib/")
    stdlib = stdlib_dirs[0]
    for required_mod in ("json", "ssl.py", "tomllib", "subprocess.py"):
        if not (stdlib / required_mod).exists():
            raise RuntimeError(f"runtime standard library missing {required_mod}")
    licenses = list(runtime.rglob("LICENSE*"))
    if not licenses:
        raise RuntimeError("runtime license file missing")
    dynlibs = list(lib.glob("libpython3.*"))
    if not dynlibs:
        raise RuntimeError("runtime dynamic library missing")


def _rust_host() -> str:
    result = subprocess.run(["rustc", "-vV"], check=True, capture_output=True, text=True)
    for line in result.stdout.splitlines():
        if line.startswith("host:"): return line.split(":", 1)[1].strip()
    raise RuntimeError("rustc -vV did not report a host target")


def _remap_env() -> dict[str, str]:
    env = dict(os.environ)
    home = str(Path.home())
    root = str(ROOT)
    remap = f"--remap-path-prefix={home}=/YOUR_BUILD_HOME --remap-path-prefix={root}=/YOUR_SOURCE_ROOT"
    current = env.get("RUSTFLAGS", "").strip()
    if remap not in current:
        env["RUSTFLAGS"] = f"{current} {remap}".strip() if current else remap
    return env


def _build_engine() -> Path:
    target = _rust_host()
    target_dir = ROOT / "engine-rs" / "target"
    subprocess.run(
        ["cargo", "build", "--manifest-path", str(ROOT / "engine-rs/Cargo.toml"), "--release", "--target", target, "--target-dir", str(target_dir)],
        check=True,
        cwd=ROOT,
        env=_remap_env(),
    )
    binary = target_dir / target / "release" / "agent-manager-engine"
    if not binary.is_file() or binary.is_symlink(): raise RuntimeError(f"release engine binary missing: {binary}")
    return binary


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""): digest.update(block)
    return digest.hexdigest()


def _managed_fingerprint(managed: dict[str, str]) -> str:
    payload = "".join(f"{key}\0{managed[key]}\n" for key in sorted(managed)).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def main() -> None:
    target = _host_target()
    runtime, runtime_metadata = _download_runtime(target)
    _validate_runtime_completeness(runtime)
    python = runtime / "bin" / "python3"
    if not python.exists(): python = runtime / "bin" / "python3.13"
    if not python.is_file(): raise RuntimeError(f"standalone Python runtime is missing: {runtime}")
    check_env = dict(os.environ)
    check_env["PYTHONDONTWRITEBYTECODE"] = "1"
    subprocess.run([str(python), "-B", "-c", "import json, ssl, tomllib, subprocess"], check=True, env=check_env)
    engine_binary = _build_engine()
    if OUT.exists(): shutil.rmtree(OUT)
    OUT.mkdir(parents=True)
    managed = {}
    for relative, suffixes in SEED_ENTRIES:
        for source in _iter_seed_files(relative, suffixes):
            rel, destination = source.relative_to(ROOT).as_posix(), OUT / source.relative_to(ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
            managed[rel] = _sha256(destination)
    shutil.copytree(runtime, OUT / "runtime" / "python", symlinks=True, ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "*.pyo"))
    binary_target = OUT / "bin" / "agent-manager-engine"
    binary_target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(engine_binary, binary_target)

    home_bytes = str(Path.home()).encode("utf-8")
    embed_count = binary_target.read_bytes().count(home_bytes)
    if embed_count != 0:
        raise RuntimeError(f"engine binary embeds build machine home path ({embed_count} occurrence(s) detected)")
    manifest = {
        "schema": 1, "managed_files": sorted(managed),
        "managed_file_sha256": {key: managed[key] for key in sorted(managed)},
        "managed_content_sha256": _managed_fingerprint(managed),
        "engine_binary": {"path": "bin/agent-manager-engine", "sha256": _sha256(binary_target)},
        "runtime": {"python_version": PYTHON_VERSION, "release_tag": RELEASE_TAG,
                     "release_date": RELEASE_DATE, "source_release_url": RELEASE_URL,
                     **runtime_metadata, "available_archives": [ARCHIVES[key] for key in sorted(ARCHIVES)]},
    }
    (OUT / "standalone-manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    # Assert package cleanliness and completeness
    pyc_in_out = list(OUT.rglob("*.py[co]")) + [p for p in OUT.rglob("__pycache__") if p.is_dir()]
    if pyc_in_out:
        rel_paths = [p.relative_to(OUT).as_posix() for p in pyc_in_out]
        raise RuntimeError(f"packaged resources contain bytecode files ({len(pyc_in_out)} item(s)): {rel_paths}")
    env_in_out = list(OUT.rglob("*.env"))
    if env_in_out:
        rel_paths = [p.relative_to(OUT).as_posix() for p in env_in_out]
        raise RuntimeError(f"packaged resources contain .env files ({len(env_in_out)} item(s)): {rel_paths}")
    _validate_runtime_completeness(OUT / "runtime" / "python")

    print(f"standalone resources: {len(managed)} managed files, target={target} -> {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    try: main()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as exc:
        print(f"build-standalone-resources: {exc}", file=sys.stderr)
        raise SystemExit(1)
