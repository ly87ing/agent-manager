#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import math
import os
import sys
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent
DEFAULT_CASES_PATH = SCRIPT_DIR / "fixtures" / "rule_effectiveness_cases.json"
DEFAULT_BASELINE_PATH = SCRIPT_DIR / "fixtures" / "rule_effectiveness_baseline.json"
DEFAULT_SKILLS_PATH = REPO_ROOT / "config" / "skills.json"


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


def require_optional_string(mapping: dict, key: str, label: str, index: int) -> str | None:
    value = mapping.get(key)
    if value is None:
        return None
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{label}[{index}].{key} must be a non-empty string")
    return value


def require_optional_string_list(mapping: dict, key: str, label: str, index: int) -> list[str]:
    value = mapping.get(key)
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"{label}[{index}].{key} must be a list of non-empty strings")
    normalized = []
    for item_index, item in enumerate(value):
        if not isinstance(item, str) or not item.strip():
            raise ValueError(f"{label}[{index}].{key}[{item_index}] must be a non-empty string")
        normalized.append(item)
    return normalized


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


def load_case_suite(path: Path) -> tuple[dict, list[dict]]:
    payload = load_json_object(path, "cases")
    requirements = payload.get("requirements")
    if not isinstance(requirements, dict):
        raise ValueError("cases.requirements must be an object")

    min_case_count = requirements.get("min_case_count")
    if not isinstance(min_case_count, int) or isinstance(min_case_count, bool) or min_case_count <= 0:
        raise ValueError("cases.requirements.min_case_count must be a positive integer")

    required_agents = requirements.get("required_agents")
    if not isinstance(required_agents, list) or not required_agents:
        raise ValueError("cases.requirements.required_agents must be a non-empty list")
    normalized_agents = []
    for index, agent_name in enumerate(required_agents):
        if not isinstance(agent_name, str) or not agent_name.strip():
            raise ValueError(f"cases.requirements.required_agents[{index}] must be a non-empty string")
        normalized_agents.append(agent_name)

    min_success_rate = requirements.get("min_success_rate")
    if not isinstance(min_success_rate, (int, float)) or isinstance(min_success_rate, bool):
        raise ValueError("cases.requirements.min_success_rate must be a number")
    min_success_rate = float(min_success_rate)
    if not 0.0 <= min_success_rate <= 1.0:
        raise ValueError("cases.requirements.min_success_rate must be between 0 and 1")

    min_verification_rate = requirements.get("min_verification_rate")
    if not isinstance(min_verification_rate, (int, float)) or isinstance(min_verification_rate, bool):
        raise ValueError("cases.requirements.min_verification_rate must be a number")
    min_verification_rate = float(min_verification_rate)
    if not 0.0 <= min_verification_rate <= 1.0:
        raise ValueError("cases.requirements.min_verification_rate must be between 0 and 1")

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

    return (
        {
            "min_case_count": min_case_count,
            "required_agents": normalized_agents,
            "min_success_rate": min_success_rate,
            "min_verification_rate": min_verification_rate,
        },
        normalized,
    )


def load_cases(path: Path) -> list[dict]:
    _, cases = load_case_suite(path)
    return cases


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
        agent = require_optional_string(run, "agent", "runs", index)
        triggered_skills = require_optional_string_list(run, "triggered_skills", "runs", index)
        success = require_bool(run, "success", "runs", index)
        verified = require_bool(run, "verified", "runs", index)
        prompt_tokens = require_non_negative_int(run, "prompt_tokens", "runs", index)
        duration_s = require_non_negative_number(run, "duration_s", "runs", index)
        file_reads = require_non_negative_int(run, "file_reads", "runs", index)
        tool_calls = require_non_negative_int(run, "tool_calls", "runs", index)
        normalized.append(
            {
                "case_id": case_id,
                "agent": agent,
                "triggered_skills": triggered_skills,
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


def load_available_skill_count_by_agent(path: Path) -> dict[str, int]:
    payload = load_json_object(path, "skills")
    repositories = payload.get("repositories")
    agents = payload.get("agents")
    if not isinstance(repositories, dict):
        raise ValueError("skills must contain an object named 'repositories'")
    if not isinstance(agents, dict):
        raise ValueError("skills must contain an object named 'agents'")

    counts = {agent_name: 0 for agent_name in sorted(agents)}
    repo_root = path.resolve().parent.parent
    skills_repo_root = repo_root / "agent-skill" / "repositories"
    for repo_name, repo in repositories.items():
        if not isinstance(repo, dict):
            raise ValueError(f"skills.repositories.{repo_name} must be an object")
        skill_count = repo.get("skill_count")
        if not isinstance(skill_count, int) or isinstance(skill_count, bool) or skill_count <= 0:
            raise ValueError(f"skills.repositories.{repo_name}.skill_count must be a positive integer")
        actual_skill_count = measure_repo_skill_count(repo_name, repo, skills_repo_root)
        if actual_skill_count is not None and actual_skill_count != skill_count:
            raise ValueError(
                f"skills.repositories.{repo_name}.skill_count mismatch: config={skill_count} actual={actual_skill_count}"
            )
        allowed_agents = repo.get("agents")
        if allowed_agents is None:
            target_agents = list(counts)
        else:
            if not isinstance(allowed_agents, list) or not allowed_agents:
                raise ValueError(f"skills.repositories.{repo_name}.agents must be a non-empty list when present")
            target_agents = []
            for index, agent_name in enumerate(allowed_agents):
                if not isinstance(agent_name, str) or not agent_name.strip():
                    raise ValueError(
                        f"skills.repositories.{repo_name}.agents[{index}] must be a non-empty string"
                    )
                if agent_name not in counts:
                    raise ValueError(
                        f"skills.repositories.{repo_name}.agents[{index}] references unknown agent: {agent_name}"
                    )
                target_agents.append(agent_name)
        for agent_name in target_agents:
            counts[agent_name] += skill_count
    return counts


def measure_repo_skill_count(repo_name: str, repo: dict, skills_repo_root: Path) -> int | None:
    repo_path = skills_repo_root / repo_name
    if not repo_path.exists():
        return None
    raw_paths = repo.get("skill_paths")
    search_paths = [repo_path]
    if raw_paths is not None:
        if not isinstance(raw_paths, list) or not raw_paths:
            raise ValueError(f"skills.repositories.{repo_name}.skill_paths must be a non-empty list when present")
        search_paths = []
        for index, raw_path in enumerate(raw_paths):
            if not isinstance(raw_path, str) or not raw_path.strip():
                raise ValueError(
                    f"skills.repositories.{repo_name}.skill_paths[{index}] must be a non-empty string"
                )
            search_path = (repo_path / raw_path).resolve(strict=False)
            if search_path != repo_path and repo_path not in search_path.parents:
                raise ValueError(f"skills.repositories.{repo_name}.skill_paths[{index}] escapes the repository")
            if not search_path.exists() or not search_path.is_dir():
                raise ValueError(
                    f"skills.repositories.{repo_name}.skill_paths[{index}] must point to an existing directory"
                )
            search_paths.append(search_path)

    skill_count = 0
    for search_path in search_paths:
        for root, dirs, files in os.walk(search_path):
            dirs[:] = sorted(d for d in dirs if d not in {".git", "node_modules", "__pycache__", "venv", ".venv"})
            if "SKILL.md" in files:
                skill_count += 1
    return skill_count


def measure_always_on_instruction_tokens_by_agent() -> dict[str, int]:
    path_map = {
        "codex": [REPO_ROOT / "agent-rule" / "rules" / "dist" / "codex" / "AGENTS.md", REPO_ROOT / "AGENTS.md"],
        "claude-code": [
            REPO_ROOT / "agent-rule" / "rules" / "dist" / "claude" / "main.md",
            REPO_ROOT / "CLAUDE.md",
        ],
        "gemini-cli": [REPO_ROOT / "agent-rule" / "rules" / "dist" / "gemini" / "main.md"],
        "cursor": [REPO_ROOT / "agent-rule" / "rules" / "dist" / "ide" / "main.md"],
        "github-copilot": [REPO_ROOT / "agent-rule" / "rules" / "dist" / "ide" / "main.md"],
        "opencode": [REPO_ROOT / "agent-rule" / "rules" / "dist" / "ide" / "main.md"],
    }
    return {
        agent_name: sum(round(len(path.read_text(encoding="utf-8")) / 4) for path in paths)
        for agent_name, paths in path_map.items()
    }


def grade_rule_effectiveness(
    requirements: dict,
    cases: list[dict],
    runs: list[dict],
    available_skill_count_by_agent: dict[str, int],
    always_on_instruction_tokens_by_agent: dict[str, int],
) -> dict:
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
    total_triggered_skill_count = 0.0
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
        total_triggered_skill_count += len(run["triggered_skills"])
        graded_cases.append(
            {
                "case_id": case["id"],
                "title": case["title"],
                "agent": run["agent"],
                "triggered_skill_count": len(run["triggered_skills"]),
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

    if case_count < requirements["min_case_count"]:
        raise ValueError(
            f"case_count {case_count} is below cases.requirements.min_case_count {requirements['min_case_count']}"
        )

    evaluated_agents = sorted({run["agent"] for run in runs if run["agent"]})
    missing_agents = sorted(set(requirements["required_agents"]) - set(evaluated_agents))
    if missing_agents:
        raise ValueError(
            f"required_agents coverage missing: {', '.join(missing_agents)}"
        )

    success_rate = success_count / case_count
    verification_rate = verification_count / case_count
    if success_rate < requirements["min_success_rate"]:
        raise ValueError(
            f"success_rate {success_rate} is below cases.requirements.min_success_rate {requirements['min_success_rate']}"
        )
    if verification_rate < requirements["min_verification_rate"]:
        raise ValueError(
            "verification_rate "
            f"{verification_rate} is below cases.requirements.min_verification_rate {requirements['min_verification_rate']}"
        )

    return {
        "case_count": case_count,
        "success_rate": success_rate,
        "verification_rate": verification_rate,
        "avg_prompt_tokens": total_prompt_tokens / case_count,
        "avg_duration_s": total_duration_s / case_count,
        "avg_file_reads": total_file_reads / case_count,
        "avg_tool_calls": total_tool_calls / case_count,
        "avg_triggered_skill_count": total_triggered_skill_count / case_count,
        "evaluated_agents": evaluated_agents,
        "available_skill_count_by_agent": available_skill_count_by_agent,
        "always_on_instruction_tokens_by_agent": always_on_instruction_tokens_by_agent,
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
    parser.add_argument(
        "--skills-config",
        type=Path,
        default=DEFAULT_SKILLS_PATH,
        help="Path to the skills configuration used for skill exposure metrics",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    try:
        requirements, cases = load_case_suite(args.cases)
        runs = load_runs(args.baseline)
        available_skill_count_by_agent = load_available_skill_count_by_agent(args.skills_config)
        always_on_instruction_tokens_by_agent = measure_always_on_instruction_tokens_by_agent()
        summary = grade_rule_effectiveness(
            requirements,
            cases,
            runs,
            available_skill_count_by_agent,
            always_on_instruction_tokens_by_agent,
        )
    except ValueError as exc:
        print(f"rule effectiveness eval: {exc}", file=sys.stderr)
        return 1

    json.dump(summary, sys.stdout, ensure_ascii=False, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
