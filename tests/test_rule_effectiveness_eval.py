import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "evals" / "run_rule_effectiveness_eval.py"
CASES = ROOT / "scripts" / "evals" / "fixtures" / "rule_effectiveness_cases.json"
BASELINE = ROOT / "scripts" / "evals" / "fixtures" / "rule_effectiveness_baseline.json"


class RuleEffectivenessEvalTests(unittest.TestCase):
    def _run_eval(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(SCRIPT), *args],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_makefile_exposes_eval_rules_target(self):
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn(".PHONY: all bootstrap sync update plan self-update smoke ui help doctor detect fetch-models status cleanup unlock backups-list backups\\:list restore backups\\:restore logs rules-build rules\\:build rules-sync rules\\:sync skills-link skills\\:link skills\\:upgrade mcp-sync mcp\\:sync mcp-prepare mcp\\:prepare tools-sync tools\\:sync terminal-sync terminal\\:sync dry-run clean-logs offline eval-rules", makefile)
        self.assertIn("\neval-rules:\n\t@python3 ./scripts/evals/run_rule_effectiveness_eval.py\n", makefile)

    def test_rule_effectiveness_eval_produces_expected_summary(self):
        result = self._run_eval()
        combined = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, combined)

        summary = json.loads(result.stdout)
        self.assertEqual(summary["case_count"], 2)
        self.assertEqual(summary["success_rate"], 0.5)
        self.assertEqual(summary["verification_rate"], 0.5)
        self.assertEqual(summary["avg_prompt_tokens"], 100.0)
        self.assertEqual(summary["avg_duration_s"], 3.5)
        self.assertEqual(summary["avg_file_reads"], 4.0)
        self.assertEqual(summary["avg_tool_calls"], 6.0)
        self.assertEqual(
            [case["case_id"] for case in summary["cases"]],
            ["add-make-target", "validate-fixture-schema"],
        )

    def test_rule_effectiveness_eval_rejects_invalid_case_schema(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            cases_path = temp_root / "cases.json"
            baseline_path = temp_root / "baseline.json"

            cases_path.write_text(
                json.dumps(
                    {
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

    def test_rule_effectiveness_eval_rejects_duplicate_case_ids(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            cases_path = temp_root / "cases.json"
            baseline_path = temp_root / "baseline.json"

            cases_path.write_text(
                json.dumps(
                    {
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
