#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_CASES_PATH = SCRIPT_DIR / "fixtures" / "rule_effectiveness_cases.json"
DEFAULT_BASELINE_PATH = SCRIPT_DIR / "fixtures" / "rule_effectiveness_baseline.json"


def load_json_object(path: Path, label: str) -> dict:
    try:
        raw_text = path.read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        raise ValueError(f"{label} missing: {path}") from exc

    try:
        data = json.loads(raw_text)
    except json.JSONDecodeError as exc:
        raise ValueError(f"{label} invalid JSON: {path}") from exc

    if not isinstance(data, dict):
        raise ValueError(f"{label} must be a JSON object: {path}")
    return data


def require_string(mapping: dict, key: str, label: str, index: int) -> str:
    value = mapping.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{label}[{index}].{key} must be a non-empty string")
    return value


def require_bool(mapping: dict, key: str, label: str, index: int) -> bool:
    value = mapping.get(key)
    if not isinstance(value, bool):
        raise ValueError(f"{label}[{index}].{key} must be a boolean")
    return value


def require_number(mapping: dict, key: str, label: str, index: int) -> float:
    value = mapping.get(key)
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise ValueError(f"{label}[{index}].{key} must be a number")
    value = float(value)
    if not math.isfinite(value):
        raise ValueError(f"{label}[{index}].{key} must be finite")
    return value


def require_non_negative_int(mapping: dict, key: str, label: str, index: int) -> int:
    value = mapping.get(key)
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{label}[{index}].{key} must be a non-negative integer")
    if value < 0:
        raise ValueError(f"{label}[{index}].{key} must be a non-negative integer")
    return value


def require_non_negative_number(mapping: dict, key: str, label: str, index: int) -> float:
    value = require_number(mapping, key, label, index)
    if value < 0:
        raise ValueError(f"{label}[{index}].{key} must be non-negative")
    return value


def load_cases(path: Path) -> list[dict]:
    payload = load_json_object(path, "cases")
    cases = payload.get("cases")
    if not isinstance(cases, list):
        raise ValueError("cases must contain a list named 'cases'")

    normalized = []
    seen_case_ids = set()
    for index, case in enumerate(cases):
        if not isinstance(case, dict):
            raise ValueError(f"cases[{index}] must be a JSON object")
        case_id = require_string(case, "id", "cases", index)
        require_string(case, "title", "cases", index)
        require_string(case, "prompt", "cases", index)
        if case_id in seen_case_ids:
            raise ValueError(f"duplicate case_id: {case_id}")
        seen_case_ids.add(case_id)
        normalized.append({"id": case_id, **case})
    return normalized


def load_runs(path: Path) -> list[dict]:
    payload = load_json_object(path, "baseline")
    runs = payload.get("runs")
    if not isinstance(runs, list):
        raise ValueError("baseline must contain a list named 'runs'")

    normalized = []
    for index, run in enumerate(runs):
        if not isinstance(run, dict):
            raise ValueError(f"runs[{index}] must be a JSON object")
        case_id = require_string(run, "case_id", "runs", index)
        success = require_bool(run, "success", "runs", index)
        verified = require_bool(run, "verified", "runs", index)
        prompt_tokens = require_non_negative_int(run, "prompt_tokens", "runs", index)
        duration_s = require_non_negative_number(run, "duration_s", "runs", index)
        file_reads = require_non_negative_int(run, "file_reads", "runs", index)
        tool_calls = require_non_negative_int(run, "tool_calls", "runs", index)
        normalized.append(
            {
                "case_id": case_id,
                "success": success,
                "verified": verified,
                "prompt_tokens": prompt_tokens,
                "duration_s": duration_s,
                "file_reads": file_reads,
                "tool_calls": tool_calls,
            }
        )
    return normalized


def _index_runs(runs: list[dict]) -> dict[str, dict]:
    indexed = {}
    for run in runs:
        case_id = run["case_id"]
        if case_id in indexed:
            raise ValueError(f"duplicate run for case_id: {case_id}")
        indexed[case_id] = run
    return indexed


def grade_rule_effectiveness(cases: list[dict], runs: list[dict]) -> dict:
    runs_by_case_id = _index_runs(runs)

    case_ids = {case["id"] for case in cases}
    run_ids = set(runs_by_case_id)
    missing_runs = sorted(case_ids - run_ids)
    unexpected_runs = sorted(run_ids - case_ids)
    if missing_runs:
        raise ValueError(f"missing runs for cases: {', '.join(missing_runs)}")
    if unexpected_runs:
        raise ValueError(f"unexpected runs for cases: {', '.join(unexpected_runs)}")

    graded_cases = []
    total_prompt_tokens = 0.0
    total_duration_s = 0.0
    total_file_reads = 0.0
    total_tool_calls = 0.0
    success_count = 0
    verification_count = 0

    for case in sorted(cases, key=lambda item: item["id"]):
        run = runs_by_case_id[case["id"]]
        success_count += int(run["success"])
        verification_count += int(run["verified"])
        total_prompt_tokens += run["prompt_tokens"]
        total_duration_s += run["duration_s"]
        total_file_reads += run["file_reads"]
        total_tool_calls += run["tool_calls"]
        graded_cases.append(
            {
                "case_id": case["id"],
                "title": case["title"],
                "success": run["success"],
                "verified": run["verified"],
                "prompt_tokens": run["prompt_tokens"],
                "duration_s": run["duration_s"],
                "file_reads": run["file_reads"],
                "tool_calls": run["tool_calls"],
            }
        )

    case_count = len(cases)
    if case_count == 0:
        raise ValueError("cases list must not be empty")

    return {
        "case_count": case_count,
        "success_rate": success_count / case_count,
        "verification_rate": verification_count / case_count,
        "avg_prompt_tokens": total_prompt_tokens / case_count,
        "avg_duration_s": total_duration_s / case_count,
        "avg_file_reads": total_file_reads / case_count,
        "avg_tool_calls": total_tool_calls / case_count,
        "cases": graded_cases,
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run the offline rule effectiveness eval")
    parser.add_argument("--cases", type=Path, default=DEFAULT_CASES_PATH, help="Path to the rule effectiveness cases fixture")
    parser.add_argument(
        "--baseline",
        type=Path,
        default=DEFAULT_BASELINE_PATH,
        help="Path to the run artifact baseline fixture",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    try:
        cases = load_cases(args.cases)
        runs = load_runs(args.baseline)
        summary = grade_rule_effectiveness(cases, runs)
    except ValueError as exc:
        print(f"rule effectiveness eval: {exc}", file=sys.stderr)
        return 1

    json.dump(summary, sys.stdout, ensure_ascii=False, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
