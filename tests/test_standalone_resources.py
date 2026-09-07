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
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b"engine")
                self.assertEqual(command[-2:], ["--target-dir", str(BUILD.ROOT / "engine-rs" / "target")])
                return type("Result", (), {})()

            try:
                with patch.object(BUILD.subprocess, "run", side_effect=run):
                    self.assertEqual(BUILD._build_engine(), binary)
            finally:
                BUILD.ROOT = old_root


if __name__ == "__main__":
    unittest.main()
