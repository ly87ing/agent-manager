import hashlib
import importlib.util
import io
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("build_standalone_resources", ROOT / "scripts/build-standalone-resources.py")
BUILD = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(BUILD)


class StandaloneResourceTests(unittest.TestCase):
    def test_seed_directory_filters_hidden_backup_and_non_source_files(self):
        with tempfile.TemporaryDirectory() as directory:
            old_root = BUILD.ROOT
            BUILD.ROOT = Path(directory)
            try:
                source = BUILD.ROOT / "src"
                source.mkdir()
                (source / "ok.py").write_text("pass")
                (source / "ignore.txt").write_text("private")
                (source / ".hidden.py").write_text("private")
                (source / "old.py.bak").write_text("private")
                self.assertEqual([p.name for p in BUILD._iter_seed_files("src", {".py"})], ["ok.py"])
            finally:
                BUILD.ROOT = old_root

    def test_seed_symlink_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            old_root = BUILD.ROOT
            BUILD.ROOT = Path(directory)
            try:
                (BUILD.ROOT / "real.py").write_text("pass")
                (BUILD.ROOT / "link.py").symlink_to("real.py")
                with self.assertRaisesRegex(RuntimeError, "contains symlink"):
                    list(BUILD._iter_seed_files("link.py", None))
            finally:
                BUILD.ROOT = old_root

    def test_runtime_keeps_internal_relative_link_and_rejects_external_link(self):
        with tempfile.TemporaryDirectory() as directory:
            runtime = Path(directory) / "runtime"
            (runtime / "bin").mkdir(parents=True)
            (runtime / "bin/python3.13").write_bytes(b"python")
            (runtime / "bin/python3").symlink_to("python3.13")
            BUILD._validate_runtime(runtime)
            outside = Path(directory) / "outside"
            outside.write_bytes(b"outside")
            (runtime / "bin/escape").symlink_to(outside)
            with self.assertRaisesRegex(RuntimeError, "escapes package"):
                BUILD._validate_runtime(runtime)

    def test_runtime_cache_isolated_by_archive_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            old_cache = BUILD.CACHE
            BUILD.CACHE = Path(directory) / "cache"
            metadata = dict(BUILD.ARCHIVES["x86_64-apple-darwin"])
            archive_bytes = io.BytesIO()
            with tarfile.open(fileobj=archive_bytes, mode="w:gz") as archive:
                info = tarfile.TarInfo("python/bin/python3")
                info.size = 6
                archive.addfile(info, io.BytesIO(b"python"))
            data = archive_bytes.getvalue()
            metadata["archive_sha256"] = hashlib.sha256(data).hexdigest()
            old_archives = BUILD.ARCHIVES
            BUILD.ARCHIVES = {"x86_64-apple-darwin": metadata}
            try:
                with patch.object(BUILD.urllib.request, "urlretrieve", side_effect=lambda _url, path: Path(path).write_bytes(data)):
                    runtime, _ = BUILD._download_runtime("x86_64-apple-darwin")
                self.assertEqual(runtime, BUILD.CACHE / "python" / "x86_64-apple-darwin" / metadata["archive_sha256"] / "extracted" / "python")
                self.assertTrue(runtime.joinpath("bin/python3").is_file())
            finally:
                BUILD.CACHE = old_cache
                BUILD.ARCHIVES = old_archives

    def test_wrong_archive_hash_is_rejected_and_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            old_cache = BUILD.CACHE
            BUILD.CACHE = Path(directory) / "cache"
            metadata = dict(BUILD.ARCHIVES["x86_64-apple-darwin"])
            metadata["archive_sha256"] = "0" * 64
            old_archives = BUILD.ARCHIVES
            BUILD.ARCHIVES = {"x86_64-apple-darwin": metadata}
            try:
                with patch.object(BUILD.urllib.request, "urlretrieve", side_effect=lambda _url, path: Path(path).write_bytes(b"wrong")):
                    with self.assertRaisesRegex(RuntimeError, "SHA-256 mismatch"):
                        BUILD._download_runtime("x86_64-apple-darwin")
                self.assertFalse((BUILD.CACHE / "python" / "x86_64-apple-darwin" / metadata["archive_sha256"] / metadata["archive"]).exists())
            finally:
                BUILD.CACHE = old_cache
                BUILD.ARCHIVES = old_archives

    def test_engine_build_target_dir_matches_binary_lookup(self):
        with tempfile.TemporaryDirectory() as directory:
            old_root = BUILD.ROOT
            BUILD.ROOT = Path(directory)
            target = "x86_64-apple-darwin"
            binary = BUILD.ROOT / "engine-rs" / "target" / target / "release" / "agent-manager-engine"

            def run(command, **kwargs):
                if command == ["rustc", "-vV"]:
                    return type("Result", (), {"stdout": f"host: {target}\n"})()
                binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_bytes(b"engine")
                self.assertEqual(command[-2:], ["--target-dir", str(BUILD.ROOT / "engine-rs" / "target")])
                self.assertIn("--remap-path-prefix=", kwargs["env"]["RUSTFLAGS"])
                return type("Result", (), {})()

            try:
                with patch.object(BUILD.subprocess, "run", side_effect=run):
                    self.assertEqual(BUILD._build_engine(), binary)
            finally:
                BUILD.ROOT = old_root

    def test_clean_bytecode_removes_pyc_and_pycache(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pycache = root / "pkg" / "__pycache__"
            pycache.mkdir(parents=True)
            (pycache / "mod.cpython-313.pyc").write_bytes(b"pyc")
            (root / "pkg" / "loose.pyc").write_bytes(b"loose")
            (root / "pkg" / "loose.pyo").write_bytes(b"loose")
            source = root / "pkg" / "mod.py"
            source.write_text("print(1)")

            BUILD._clean_bytecode(root)

            self.assertTrue(source.is_file())
            self.assertFalse(pycache.exists())
            self.assertFalse((root / "pkg" / "loose.pyc").exists())
            self.assertFalse((root / "pkg" / "loose.pyo").exists())
            self.assertEqual(list(root.rglob("*.py[co]")), [])

    def test_validate_runtime_completeness_rejects_bytecode_and_missing_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            runtime = Path(directory) / "runtime"
            (runtime / "bin").mkdir(parents=True)
            (runtime / "bin/python3.13").write_bytes(b"python")
            (runtime / "bin/python3").symlink_to("python3.13")
            stdlib = runtime / "lib" / "python3.13"
            stdlib.mkdir(parents=True)
            for mod in ("json", "ssl.py", "tomllib", "subprocess.py"):
                (stdlib / mod).write_bytes(b"dummy")
            (runtime / "lib" / "libpython3.13.dylib").write_bytes(b"dynlib")
            (stdlib / "LICENSE.txt").write_text("license")

            # Complete runtime passes
            BUILD._validate_runtime_completeness(runtime)

            # Presence of pyc is rejected
            pyc = stdlib / "test.pyc"
            pyc.write_bytes(b"pyc")
            with self.assertRaisesRegex(RuntimeError, "contains bytecode files"):
                BUILD._validate_runtime_completeness(runtime)
            pyc.unlink()

            # Missing license is rejected
            (stdlib / "LICENSE.txt").unlink()
            with self.assertRaisesRegex(RuntimeError, "license file missing"):
                BUILD._validate_runtime_completeness(runtime)
            (stdlib / "LICENSE.txt").write_text("license")

            # Missing dynamic lib is rejected
            (runtime / "lib" / "libpython3.13.dylib").unlink()
            with self.assertRaisesRegex(RuntimeError, "dynamic library missing"):
                BUILD._validate_runtime_completeness(runtime)

    def test_packaged_resources_cleanliness_and_completeness(self):
        if not BUILD.OUT.exists():
            self.skipTest("packaged resources not built")
        pyc_files = list(BUILD.OUT.rglob("*.py[co]")) + [p for p in BUILD.OUT.rglob("__pycache__") if p.is_dir()]
        rel_pyc = [p.relative_to(BUILD.OUT).as_posix() for p in pyc_files]
        self.assertEqual(rel_pyc, [], f"found bytecode in package: {rel_pyc}")
        env_files = list(BUILD.OUT.rglob("*.env*"))
        rel_env = [p.relative_to(BUILD.OUT).as_posix() for p in env_files]
        self.assertEqual(rel_env, [], f"found .env in package: {rel_env}")
        BUILD._validate_runtime_completeness(BUILD.OUT / "runtime" / "python")


if __name__ == "__main__":
    unittest.main()
