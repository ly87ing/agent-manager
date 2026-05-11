import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "evals" / "run_rule_effectiveness_eval.py"


class SkillUsageMetricsTests(unittest.TestCase):
    def _run_eval(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(SCRIPT), *args],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def _expected_instruction_tokens_by_agent(self) -> dict[str, int]:
        path_map = {
            "codex": [ROOT / "agent-rule" / "rules" / "dist" / "codex" / "AGENTS.md", ROOT / "AGENTS.md"],
            "claude-code": [ROOT / "agent-rule" / "rules" / "dist" / "claude" / "CLAUDE.md", ROOT / "CLAUDE.md"],
            "gemini-cli": [ROOT / "agent-rule" / "rules" / "dist" / "gemini" / "GEMINI.md"],
            "cursor": [
                ROOT / "agent-rule" / "rules" / "dist" / "cursor" / "agent-manager.mdc",
                ROOT / ".cursor" / "rules" / "agent-manager.mdc",
            ],
            "kimi-cli": [
                ROOT / "agent-rule" / "rules" / "dist" / "kimi" / "AGENTS.md",
                ROOT / ".kimi" / "AGENTS.md",
            ],
            "qodercli": [
                ROOT / "agent-rule" / "rules" / "dist" / "qoder" / "QODER.md",
                ROOT / ".qoder" / "QODER.md",
            ],
        }
        return {
            agent: sum(round(len(path.read_text(encoding="utf-8")) / 4) for path in paths)
            for agent, paths in path_map.items()
        }

    def test_eval_reports_skill_exposure_and_instruction_budget_metrics(self):
        result = self._run_eval()
        combined = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, combined)

        summary = json.loads(result.stdout)
        self.assertEqual(
            summary["available_skill_count_by_agent"],
            {
                "claude-code": 26,
                "codex": 26,
                "cursor": 16,
                "gemini-cli": 15,
                "kimi-cli": 27,
                "opencode": 16,
                "qodercli": 27,
            },
        )
        self.assertEqual(summary["avg_triggered_skill_count"], 13 / 11)
        self.assertEqual(summary["always_on_instruction_tokens_by_agent"], self._expected_instruction_tokens_by_agent())
        self.assertEqual(
            [(case["case_id"], case["agent"], case["triggered_skill_count"]) for case in summary["cases"]],
            [
                ("add-make-target", "codex", 1),
                ("browser-tool-intent-boundary", "cursor", 1),
                ("doctor-flags-unmaterialized-rules", "gemini-cli", 1),
                ("manifest-targets-dist-artifacts", "kimi-cli", 1),
                ("prevent-dist-edit-drift", "cursor", 1),
                ("qodercli-case", "qodercli", 1),
                ("rule-budget-regression-guard", "claude-code", 2),
                ("scoped-memory-contract", "claude-code", 1),
                ("simple-task-preamble-boundary", "codex", 1),
                ("sync-codex-hooks", "codex", 1),
                ("validate-fixture-schema", "claude-code", 2),
            ],
        )

    def test_eval_rejects_invalid_triggered_skills_schema(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            cases_path = temp_root / "cases.json"
            baseline_path = temp_root / "baseline.json"

            cases_path.write_text(
                json.dumps(
                    {
                        "requirements": {
                            "min_case_count": 1,
                            "required_agents": ["codex"],
                            "min_success_rate": 1.0,
                            "min_verification_rate": 1.0,
                        },
                        "cases": [
                            {
                                "id": "single-case",
                                "title": "Valid case",
                                "prompt": "Valid prompt",
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )
            baseline_path.write_text(
                json.dumps(
                    {
                        "runs": [
                            {
                                "case_id": "single-case",
                                "agent": "codex",
                                "triggered_skills": "frontend-skill",
                                "success": True,
                                "verified": True,
                                "prompt_tokens": 1,
                                "duration_s": 1.0,
                                "file_reads": 1,
                                "tool_calls": 1,
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )

            result = self._run_eval("--cases", str(cases_path), "--baseline", str(baseline_path))
            combined = result.stdout + result.stderr

            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("runs[0].triggered_skills", combined)

    def test_eval_rejects_unknown_agent_in_skill_scope(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            skills_path = temp_root / "skills.json"
            skills_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "repo": {
                                "skill_count": 2,
                                "agents": ["unknown-agent"],
                            }
                        },
                        "agents": {
                            "codex": "~/.codex/skills",
                        },
                    }
                ),
                encoding="utf-8",
            )

            result = self._run_eval("--skills-config", str(skills_path))
            combined = result.stdout + result.stderr

            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("unknown agent", combined)

    def test_eval_rejects_skill_count_mismatch_when_repo_inventory_is_available(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            config_dir = temp_root / "config"
            config_dir.mkdir()
            repo_root = temp_root / "agent-skill" / "repositories" / "repo"
            (repo_root / "skills" / "alpha").mkdir(parents=True)
            (repo_root / "skills" / "beta").mkdir(parents=True)
            (repo_root / "skills" / "alpha" / "SKILL.md").write_text("# Alpha\n", encoding="utf-8")
            (repo_root / "skills" / "beta" / "SKILL.md").write_text("# Beta\n", encoding="utf-8")
            skills_path = config_dir / "skills.json"
            skills_path.write_text(
                json.dumps(
                    {
                        "repositories": {
                            "repo": {
                                "skill_paths": ["skills"],
                                "skill_count": 1,
                                "agents": ["codex"],
                            }
                        },
                        "agents": {
                            "codex": "~/.codex/skills",
                        },
                    }
                ),
                encoding="utf-8",
            )

            result = self._run_eval("--skills-config", str(skills_path))
            combined = result.stdout + result.stderr

            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("skill_count mismatch", combined)
