import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "agent-skill" / "skill_toggle.py"
DEF_FILE = ROOT / "config" / "skills.json"


def _run(*args, local):
    return subprocess.run(
        ["python3", str(SCRIPT), *args, "--def-file", str(DEF_FILE), "--local", str(local)],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )


class SkillToggleTests(unittest.TestCase):
    def test_disable_adds_entry_and_preserves_other_keys(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            local = Path(temp_dir) / "local.json"
            local.write_text('{"browser_us_sync": {"enabled": false}}', encoding="utf-8")
            result = _run("disable", "safe-merge-review", local=local)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(local.read_text(encoding="utf-8"))
            self.assertEqual(
                data["skills"]["agent-skills"]["disabled"], ["safe-merge-review"]
            )
            # 不误删本机其它覆盖键
            self.assertEqual(data["browser_us_sync"], {"enabled": False})

    def test_enable_removes_entry_and_prunes_empty_skills_key(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            local = Path(temp_dir) / "local.json"
            local.write_text(
                json.dumps(
                    {
                        "browser_us_sync": {"enabled": False},
                        "skills": {"agent-skills": {"disabled": ["safe-merge-review"]}},
                    }
                ),
                encoding="utf-8",
            )
            result = _run("enable", "safe-merge-review", local=local)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(local.read_text(encoding="utf-8"))
            self.assertNotIn("skills", data)  # 清空后剪掉空壳
            self.assertEqual(data["browser_us_sync"], {"enabled": False})

    def test_unknown_skill_exits_nonzero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            local = Path(temp_dir) / "local.json"
            local.write_text("{}", encoding="utf-8")
            result = _run("disable", "bogus-skill", local=local)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("未知 skill", result.stdout + result.stderr)

    def test_list_reports_enabled_and_disabled_state(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            local = Path(temp_dir) / "local.json"
            local.write_text(
                json.dumps({"skills": {"agent-skills": {"disabled": ["safe-merge-review"]}}}),
                encoding="utf-8",
            )
            result = _run("list", local=local)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("[已禁用] safe-merge-review", result.stdout)
            self.assertIn("[已启用] change-discipline", result.stdout)


if __name__ == "__main__":
    unittest.main()
