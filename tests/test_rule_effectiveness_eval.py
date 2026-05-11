import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "evals" / "run_rule_effectiveness_eval.py"
CASES = ROOT / "scripts" / "evals" / "fixtures" / "rule_effectiveness_cases.json"
BASELINE = ROOT / "scripts" / "evals" / "fixtures" / "rule_effectiveness_baseline.json"
RULE_TARGETS = ROOT / "config" / "rules-targets.list"


RULE_TARGET_AGENT_IDS = {
    "claude": "claude-code",
    "codex": "codex",
    "cursor": "cursor",
    "gemini": "gemini-cli",
    "kimi": "kimi-cli",
    "qoder": "qodercli",
}


class RuleEffectivenessEvalTests(unittest.TestCase):
    def _run_eval(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(SCRIPT), *args],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def _managed_rule_agents(self) -> set[str]:
        managed_rule_agents = set()
        for raw_line in RULE_TARGETS.read_text(encoding="utf-8").splitlines():
            line = raw_line.strip()
            if not line or line.startswith("#"):
                continue
            target_name, _group, source, _target = line.split("|", 3)
            if not target_name.endswith("_global"):
                continue
            source_parts = Path(source).parts
            self.assertGreaterEqual(len(source_parts), 4, source)
            rule_target = source_parts[2]
            self.assertIn(rule_target, RULE_TARGET_AGENT_IDS, source)
            managed_rule_agents.add(RULE_TARGET_AGENT_IDS[rule_target])
        return managed_rule_agents

    def test_makefile_exposes_eval_rules_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn(
            ".PHONY: all bootstrap sync update plan self-update smoke ui help doctor detect fetch-models status cleanup unlock backups-list backups\\:list restore backups\\:restore logs rules-build rules\\:build rules-sync rules\\:sync skills-link skills\\:link skills\\:upgrade mcp-sync mcp\\:sync mcp-prepare mcp\\:prepare tools-sync tools\\:sync terminal-sync terminal\\:sync claude-hooks-sync claude\\:hooks\\:sync codex-hooks-sync codex\\:hooks\\:sync dry-run clean-logs offline eval-rules secrets-check secrets\\:check instructions-check instructions\\:check cc-bridge-install cc-bridge-start cc-bridge-stop cc-bridge-deploy cc-bridge-update cc-bridge-seed cc-bridge-rollback cc-bridge-status",
            makefile,
        )
        self.assertIn("\neval-rules:\n\t@python3 ./scripts/evals/run_rule_effectiveness_eval.py\n", makefile)

    def test_rule_effectiveness_eval_produces_expected_summary(self):
        result = self._run_eval()
        combined = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, combined)

        summary = json.loads(result.stdout)
        self.assertEqual(summary["case_count"], 9)
        self.assertEqual(summary["success_rate"], 1.0)
        self.assertEqual(summary["verification_rate"], 1.0)
        self.assertEqual(summary["avg_prompt_tokens"], 96.0)
        self.assertEqual(summary["avg_duration_s"], 31.0 / 9)
        self.assertEqual(summary["avg_file_reads"], 38.0 / 9)
        self.assertEqual(summary["avg_tool_calls"], 56.0 / 9)
        self.assertEqual(summary["avg_triggered_skill_count"], 11 / 9)
        self.assertEqual(summary["evaluated_agents"], ["claude-code", "codex", "cursor", "gemini-cli", "kimi-cli", "qodercli"])
        self.assertEqual(
            [case["case_id"] for case in summary["cases"]],
            [
                "add-make-target",
                "doctor-flags-unmaterialized-rules",
                "manifest-targets-dist-artifacts",
                "prevent-dist-edit-drift",
                "qodercli-case",
                "rule-budget-regression-guard",
                "scoped-memory-contract",
                "sync-codex-hooks",
                "validate-fixture-schema",
            ],
        )

    def test_rule_effectiveness_cases_do_not_use_placeholder_prompts(self):
        cases = json.loads(CASES.read_text(encoding="utf-8"))["cases"]

        for case in cases:
            prompt = case["prompt"].lower()
            title = case["title"].lower()
            self.assertNotIn("placeholder", prompt, case["id"])
            self.assertNotIn("placeholder", title, case["id"])

    def test_default_required_agents_cover_managed_rule_targets(self):
        requirements = json.loads(CASES.read_text(encoding="utf-8"))["requirements"]
        required_agents = set(requirements["required_agents"])

        self.assertEqual(required_agents, self._managed_rule_agents())

    def test_instruction_token_metrics_cover_managed_rule_targets(self):
        result = self._run_eval()
        combined = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, combined)

        summary = json.loads(result.stdout)
        measured_agents = set(summary["always_on_instruction_tokens_by_agent"])

        self.assertEqual(measured_agents, self._managed_rule_agents())

    def test_rule_effectiveness_eval_rejects_invalid_case_schema(self):
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
                                "id": "broken-case",
                                "title": "Broken fixture",
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )
            baseline_path.write_text(json.dumps({"runs": []}), encoding="utf-8")

            result = self._run_eval("--cases", str(cases_path), "--baseline", str(baseline_path))
            combined = result.stdout + result.stderr

            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("cases[0].prompt", combined)

    def test_rule_effectiveness_eval_rejects_missing_suite_requirements(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            cases_path = temp_root / "cases.json"
            baseline_path = temp_root / "baseline.json"

            cases_path.write_text(
                json.dumps(
                    {
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
            self.assertIn("cases.requirements", combined)

    def test_rule_effectiveness_eval_rejects_duplicate_case_ids(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            cases_path = temp_root / "cases.json"
            baseline_path = temp_root / "baseline.json"

            cases_path.write_text(
                json.dumps(
                    {
                        "requirements": {
                            "min_case_count": 2,
                            "required_agents": ["codex"],
                            "min_success_rate": 1.0,
                            "min_verification_rate": 1.0,
                        },
                        "cases": [
                            {
                                "id": "duplicate-case",
                                "title": "First copy",
                                "prompt": "First prompt",
                            },
                            {
                                "id": "duplicate-case",
                                "title": "Second copy",
                                "prompt": "Second prompt",
                            },
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
                                "case_id": "duplicate-case",
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
            self.assertIn("duplicate case_id", combined)

    def test_rule_effectiveness_eval_rejects_negative_prompt_tokens(self):
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
                                "success": True,
                                "verified": True,
                                "prompt_tokens": -1,
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
            self.assertIn("runs[0].prompt_tokens", combined)

    def test_rule_effectiveness_eval_rejects_fractional_count_metrics(self):
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
                                "success": True,
                                "verified": True,
                                "prompt_tokens": 1,
                                "duration_s": 1.0,
                                "file_reads": 1.5,
                                "tool_calls": 2.25,
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )

            result = self._run_eval("--cases", str(cases_path), "--baseline", str(baseline_path))
            combined = result.stdout + result.stderr

            self.assertNotEqual(result.returncode, 0, combined)
            self.assertIn("runs[0].file_reads", combined)

    def test_rule_effectiveness_eval_rejects_non_finite_baseline_metrics(self):
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
                                "success": True,
                                "verified": True,
                                "prompt_tokens": 1,
                                "duration_s": float("inf"),
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
            self.assertIn("runs[0].duration_s", combined)

    def test_rule_effectiveness_eval_rejects_threshold_regression(self):
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
                        ],
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
                                "success": False,
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
            self.assertIn("success_rate", combined)

    def test_rule_effectiveness_eval_rejects_missing_required_agent_coverage(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            cases_path = temp_root / "cases.json"
            baseline_path = temp_root / "baseline.json"

            cases_path.write_text(
                json.dumps(
                    {
                        "requirements": {
                            "min_case_count": 1,
                            "required_agents": ["codex", "claude-code"],
                            "min_success_rate": 1.0,
                            "min_verification_rate": 1.0,
                        },
                        "cases": [
                            {
                                "id": "single-case",
                                "title": "Valid case",
                                "prompt": "Valid prompt",
                            }
                        ],
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
            self.assertIn("required_agents", combined)
