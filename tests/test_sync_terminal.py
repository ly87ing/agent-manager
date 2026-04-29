import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MODULE = ROOT / "agent-terminal" / "sync_terminal.py"


def _run(args, extra_env=None):
    env = os.environ.copy()
    if extra_env:
        env.update(extra_env)
    return subprocess.run(
        ["python3", str(MODULE)] + args,
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
        env=env,
    )


class SyncTerminalTests(unittest.TestCase):
    def test_missing_config_exits_nonzero(self):
        with tempfile.TemporaryDirectory() as d:
            result = _run(["--config", str(Path(d) / "nonexistent.json")])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("不存在", result.stdout + result.stderr)

    def test_invalid_config_json_exits_nonzero(self):
        with tempfile.TemporaryDirectory() as d:
            cfg = Path(d) / "terminal.json"
            cfg.write_text("{ invalid", encoding="utf-8")
            result = _run(["--config", str(cfg)])
            self.assertNotEqual(result.returncode, 0)

    def test_dry_run_does_not_write_file(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            tmpl = d / "tmpl" / "test.tmpl"
            tmpl.parent.mkdir()
            tmpl.write_text("hello world", encoding="utf-8")
            dest = d / "out" / "result.txt"
            cfg = d / "terminal.json"
            cfg.write_text(
                json.dumps(
                    {
                        "targets": {
                            "test-target": {
                                "template": str(tmpl),
                                "destination": str(dest),
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            result = _run(["--config", str(cfg), "--dry-run"])
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(dest.exists(), "dry-run 不应写入文件")

    def test_renders_template_and_writes_destination(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            tmpl = d / "test.tmpl"
            tmpl.write_text("user=${MY_USER}", encoding="utf-8")
            dest = d / "output" / "result.txt"
            secrets = d / "secrets.env"
            secrets.write_text("MY_USER=alice", encoding="utf-8")
            cfg = d / "terminal.json"
            cfg.write_text(
                json.dumps(
                    {
                        "secrets_file": str(secrets),
                        "targets": {
                            "test-target": {
                                "template": str(tmpl),
                                "destination": str(dest),
                            }
                        },
                    }
                ),
                encoding="utf-8",
            )
            result = _run(["--config", str(cfg)])
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(dest.exists())
            self.assertEqual(dest.read_text(encoding="utf-8"), "user=alice")

    def test_unknown_variable_preserved_passthrough(self):
        """模板中未在 secrets 定义的 ${VAR} 应原样保留（不报错）。"""
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            tmpl = d / "test.tmpl"
            tmpl.write_text("export PATH=${UNKNOWN_VAR}:$PATH", encoding="utf-8")
            dest = d / "result.txt"
            cfg = d / "terminal.json"
            cfg.write_text(
                json.dumps(
                    {
                        "targets": {
                            "test-target": {
                                "template": str(tmpl),
                                "destination": str(dest),
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            result = _run(["--config", str(cfg)])
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("${UNKNOWN_VAR}", dest.read_text(encoding="utf-8"))

    def test_chmod_sets_file_permissions(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            tmpl = d / "test.tmpl"
            tmpl.write_text("secret", encoding="utf-8")
            dest = d / "result.txt"
            cfg = d / "terminal.json"
            cfg.write_text(
                json.dumps(
                    {
                        "targets": {
                            "test-target": {
                                "template": str(tmpl),
                                "destination": str(dest),
                                "chmod": "0600",
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            result = _run(["--config", str(cfg)])
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(dest.exists())
            mode = oct(os.stat(dest).st_mode & 0o777)
            self.assertEqual(mode, "0o600")

    def test_missing_template_file_exits_nonzero(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            dest = d / "result.txt"
            cfg = d / "terminal.json"
            cfg.write_text(
                json.dumps(
                    {
                        "targets": {
                            "test-target": {
                                "template": str(d / "nonexistent.tmpl"),
                                "destination": str(dest),
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            result = _run(["--config", str(cfg)])
            self.assertNotEqual(result.returncode, 0)

    def test_target_skipped_when_requires_unknown_tool(self):
        """requires 中列出的工具未在 tools.json 中定义时，视为未就绪并跳过（不失败）。"""
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            tmpl = d / "test.tmpl"
            tmpl.write_text("hello", encoding="utf-8")
            dest = d / "result.txt"
            # 工具名不在项目 config/tools.json 中 → tool_defs 查无此项 → ready_check 为 None → 跳过
            cfg = d / "terminal.json"
            cfg.write_text(
                json.dumps(
                    {
                        "targets": {
                            "test-target": {
                                "template": str(tmpl),
                                "destination": str(dest),
                                "requires": ["definitely-not-installed-tool-xyz"],
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            result = _run(["--config", str(cfg)])
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(dest.exists(), "requires 未满足时不应写入文件")

    def test_disabled_target_is_skipped_without_writing(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            tmpl = d / "test.tmpl"
            tmpl.write_text("hello", encoding="utf-8")
            dest = d / "result.txt"
            cfg = d / "terminal.json"
            cfg.write_text(
                json.dumps(
                    {
                        "targets": {
                            "test-target": {
                                "enabled": False,
                                "template": str(tmpl),
                                "destination": str(dest),
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            result = _run(["--config", str(cfg)])
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(dest.exists(), "disabled 目标不应写入文件")

    def test_zshrc_codex_s_alias_uses_full_auto(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            dest = d / "zshrc"
            secrets = d / "empty.env"
            secrets.write_text("", encoding="utf-8")
            cfg = d / "terminal.json"
            cfg.write_text(
                json.dumps(
                    {
                        "secrets_file": str(secrets),
                        "targets": {
                            "zshrc": {
                                "template": "agent-terminal/templates/zshrc.tmpl",
                                "destination": str(dest),
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )

            result = _run(["--config", str(cfg)])
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

            rendered = dest.read_text(encoding="utf-8")
            self.assertIn("alias codex-s='codex --full-auto'", rendered)
            self.assertNotIn("alias codex-s='codex -s danger-full-access'", rendered)

    def test_non_dict_targets_exits_nonzero(self):
        with tempfile.TemporaryDirectory() as d:
            cfg = Path(d) / "terminal.json"
            cfg.write_text(json.dumps({"targets": ["bad"]}), encoding="utf-8")
            result = _run(["--config", str(cfg)])
            self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
