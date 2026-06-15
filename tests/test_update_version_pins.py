import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "scripts" / "update_version_pins.py"
SPEC = importlib.util.spec_from_file_location("update_version_pins", MODULE_PATH)
update_version_pins = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(update_version_pins)


class UpdateVersionPinsTests(unittest.TestCase):
    def test_updates_npm_and_git_ref_version_locks(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            config_dir = Path(temp_dir)
            (config_dir / "agents.json").write_text(
                json.dumps(
                    {
                        "codex": {
                            "providers": {
                                "npm": {
                                    "install": "npm install -g @openai/codex@0.1.0",
                                    "upgrade": "npm install -g @openai/codex@0.1.0",
                                    "version_lock": {
                                        "type": "npm",
                                        "package": "@openai/codex",
                                        "fields": {
                                            "install": "npm install -g @openai/codex@{version}",
                                            "upgrade": "npm install -g @openai/codex@{version}",
                                        },
                                    },
                                }
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            (config_dir / "tools.json").write_text('{"tools": {}}\n', encoding="utf-8")
            (config_dir / "mcp.json").write_text(
                json.dumps(
                    {
                        "servers": {
                            "demo": {
                                "args": [
                                    "--from",
                                    "git+https://example.invalid/repo@oldref",
                                    "demo",
                                ],
                                "version_lock": {
                                    "type": "git-ref",
                                    "remote": "https://example.invalid/repo",
                                    "ref": "main",
                                    "fields": {
                                        "args.1": "git+https://example.invalid/repo@{version}",
                                    },
                                },
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )

            with mock.patch.object(update_version_pins, "fetch_npm_latest_version", return_value="1.2.3"):
                with mock.patch.object(
                    update_version_pins,
                    "fetch_git_ref_commit",
                    return_value="0123456789abcdef0123456789abcdef01234567",
                ):
                    result = update_version_pins.main_with_args_for_test(config_dir)

            self.assertEqual(result, 0)
            agents = json.loads((config_dir / "agents.json").read_text(encoding="utf-8"))
            npm_provider = agents["codex"]["providers"]["npm"]
            self.assertEqual(npm_provider["install"], "npm install -g @openai/codex@1.2.3")
            self.assertEqual(npm_provider["upgrade"], "npm install -g @openai/codex@1.2.3")

            mcp = json.loads((config_dir / "mcp.json").read_text(encoding="utf-8"))
            self.assertEqual(
                mcp["servers"]["demo"]["args"][1],
                "git+https://example.invalid/repo@0123456789abcdef0123456789abcdef01234567",
            )

    def test_dry_run_does_not_resolve_or_write(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            config_dir = Path(temp_dir)
            original = {
                "tools": {
                    "demo": {
                        "install": "npm install -g demo@0.1.0",
                        "version_lock": {
                            "type": "npm",
                            "package": "demo",
                            "fields": {"install": "npm install -g demo@{version}"},
                        },
                    }
                }
            }
            (config_dir / "agents.json").write_text("{}\n", encoding="utf-8")
            (config_dir / "tools.json").write_text(json.dumps(original), encoding="utf-8")
            (config_dir / "mcp.json").write_text('{"servers": {}}\n', encoding="utf-8")

            with mock.patch.object(update_version_pins, "fetch_npm_latest_version") as fetch_version:
                result = update_version_pins.main_with_args_for_test(config_dir, dry_run=True)

            self.assertEqual(result, 0)
            fetch_version.assert_not_called()
            self.assertEqual(json.loads((config_dir / "tools.json").read_text(encoding="utf-8")), original)


if __name__ == "__main__":
    unittest.main()
