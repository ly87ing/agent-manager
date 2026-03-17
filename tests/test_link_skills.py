import importlib.util
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "agent-skill" / "link_skills.py"
SPEC = importlib.util.spec_from_file_location("link_skills", MODULE_PATH)
link_skills = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(link_skills)


class LinkSkillsTests(unittest.TestCase):
    def test_clone_failure_exits_non_zero(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_path = temp_root / "config.json"
            def_path = temp_root / "defs.json"
            config_path.write_text(
                json.dumps({"skills": {"bad-repo": {"enabled": True}}}),
                encoding="utf-8",
            )
            def_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "bad-repo": {
                                "url": "https://invalid.example.invalid/nope.git",
                                "agents": [],
                            }
                        },
                        "agents": {},
                    }
                ),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    "--config",
                    str(config_path),
                    "--def-file",
                    str(def_path),
                    "--base-dir",
                    str(temp_root / "skills"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_scrub_dead_links_removes_broken_relative_link(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            managed_dir = temp_root / "managed"
            agent_dir = temp_root / "agent" / "skills"
            managed_dir.mkdir()
            agent_dir.mkdir(parents=True)

            target = managed_dir / "repo" / "skill"
            target.parent.mkdir(parents=True)
            target.mkdir()
            link = agent_dir / "skill"
            link.symlink_to(os.path.relpath(target, agent_dir))

            target.rmdir()
            target.parent.rmdir()

            link_skills.scrub_dead_links({"TestAgent": str(agent_dir)}, managed_dir)

            self.assertFalse(link.is_symlink())
