import shutil
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from src.manage_doctor import build_report


ROOT = Path(__file__).resolve().parent.parent


class ManageDoctorReportTests(unittest.TestCase):
    def test_report_requires_toml_parser_for_active_toml_targets(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir) / "repo"
            temp_home = Path(temp_dir) / "home"
            shutil.copytree(ROOT, temp_root, ignore=shutil.ignore_patterns("repositories"))
            temp_home.mkdir()

            (temp_root / "config" / "manager.json").write_text(
                '{\n  "agents": {},\n  "tools": {},\n  "skills": {},\n  "mcp": {\n    "augment-context-engine": { "enabled": true }\n  }\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "mcp.json").write_text(
                '{\n  "agent_paths": {\n    "codex": { "path": "%s/.codex/config.toml", "format": "toml" }\n  },\n  "servers": {\n    "augment-context-engine": {\n      "command": "auggie",\n      "args": ["--mcp"],\n      "agents": ["codex"]\n    }\n  }\n}\n'
                % temp_home,
                encoding="utf-8",
            )
            (temp_root / "config" / "skills.json").write_text(
                '{\n  "repositories": {},\n  "agents": {}\n}\n',
                encoding="utf-8",
            )
            (temp_root / "config" / "rules-targets.list").write_text("", encoding="utf-8")

            with mock.patch("src.manage_doctor.load_tomllib", side_effect=ModuleNotFoundError("tomli missing")):
                report = build_report(
                    root_dir=temp_root,
                    config_file=temp_root / "config" / "manager.json",
                    python_cmd="python3",
                )

            self.assertEqual(report["status"], "error")
            self.assertEqual(report["summary"]["errors"], 1)
            runtime_names = {entry["name"]: entry for entry in report["sections"]["runtime"]}
            self.assertIn("TOML parser", runtime_names)
            self.assertEqual(runtime_names["TOML parser"]["status"], "error")
            self.assertIn("tomli", runtime_names["TOML parser"]["message"])
