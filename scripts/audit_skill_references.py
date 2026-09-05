#!/usr/bin/env python3
"""Detect content-level dead skill references in agent-manager-managed files.

Structural cleanup (dead symlinks, orphan skill repos) is already handled by
the skills link step during `make update`. This audit covers the *content* layer that
sync does not touch: a rule or SKILL.md that names a skill which no longer
exists (e.g. a rule routing to a removed/renamed skill).

Scope and limits, on purpose:
- Only the well-defined reference form ```<name>` skill`` is checked. Bare
  names in prose or table cells have no consistent syntax and are left to
  per-skill evals / human review, not a fragile catch-all regex.
- The "known skill universe" is the discovery result of each declared collection
  (``skill_paths`` is the in-repo discovery scope; a skill's identity is the
  ``SKILL.md`` frontmatter ``name``), falling back to the declared ``skill_paths``
  basenames when the local repo cache is absent (cold CI checkout), plus every
  skill actually installed under the agent skill dirs. That lets a valid
  cross-source reference (e.g. an installed ``xiaoyu-bug-loop``) pass while a
  reference to a retired skill fails.
- Detection only. It never rewrites content: the correct replacement for a dead
  reference is a human decision, so the audit reports and exits non-zero instead.

Exit code: 0 clean, 1 dead references found, 2 on config error. Callers choose
severity: the `make update` pipeline treats a non-zero exit as a non-fatal warning;
the test suite treats it as fail-close.
"""
import argparse
import json
import os
import re
import sys
from pathlib import Path

# `<kebab-name>` immediately followed by the word "skill" — the routing form used
# by shared rules and SKILL.md guidance.
REFERENCE_RE = re.compile(r"`([a-z0-9][a-z0-9-]*)`\s+skill\b")

CONTENT_GLOBS = (
    "agent-rule/rules/src/**/*.md",
    "agent-rule/rules/dist/**/*.md",
    "agent-skill/repositories/agent-skills/*/SKILL.md",
    "agent-skill/repositories/agent-skills/*/references/*.md",
)


def managed_content_files(root: Path):
    seen = set()
    for pattern in CONTENT_GLOBS:
        for path in sorted(root.glob(pattern)):
            if path.is_file() and path not in seen:
                seen.add(path)
                yield path


def _resolve_agent_dir(target: str) -> Path:
    # skills.json agent targets look like "~/.claude/skills"; expand the leading ~.
    return Path(target).expanduser()


EXCLUDED_DISCOVERY_DIRS = {".git", "node_modules", "__pycache__", "venv", ".venv"}
SKILL_FILE = "SKILL.md"


def _read_frontmatter_name(skill_md: Path) -> str | None:
    try:
        lines = skill_md.read_text(encoding="utf-8").splitlines()
    except OSError:
        return None
    if not lines or lines[0].strip() != "---":
        return None
    for line in lines[1:]:
        stripped = line.strip()
        if stripped == "---":
            break
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        if key.strip() == "name":
            name = value.strip().strip("'\"")
            return name or None
    return None


def _discover_repo_skill_names(root: Path, repo_name: str, repo: dict) -> set[str] | None:
    """按发现语义枚举 collection 的 skill 名（frontmatter name，解析失败回退目录名）。

    ``skill_paths`` 是仓库内的发现范围；本地缓存缺失（如 CI 冷启动）时返回 None，
    由调用方回退到声明的 basename —— 旧格式下两者等价。
    """
    cache = (root / "agent-skill" / "repositories" / repo_name).resolve()
    if not cache.is_dir():
        return None
    raw_paths = repo.get("skill_paths")
    if raw_paths is None:
        search_roots = [cache]
    else:
        search_roots = []
        for raw_path in raw_paths:
            candidate = (cache / raw_path).resolve(strict=False)
            if candidate != cache and cache not in candidate.parents:
                continue
            search_roots.append(candidate)
    names: set[str] = set()
    for search_root in search_roots:
        if not search_root.is_dir():
            continue
        for dirpath, dirnames, filenames in os.walk(search_root):
            dirnames[:] = sorted(d for d in dirnames if d not in EXCLUDED_DISCOVERY_DIRS)
            if SKILL_FILE not in filenames:
                continue
            skill_dir = Path(dirpath)
            names.add(_read_frontmatter_name(skill_dir / SKILL_FILE) or skill_dir.name)
    return names


def build_universe(root: Path) -> set[str]:
    skills_cfg = json.loads((root / "config" / "skills.json").read_text(encoding="utf-8"))
    universe: set[str] = set()
    for repo_name, repo in skills_cfg.get("repositories", {}).items():
        discovered = _discover_repo_skill_names(root, repo_name, repo)
        if discovered is not None:
            universe.update(discovered)
            continue
        for skill_path in repo.get("skill_paths", []):
            universe.add(Path(skill_path).name)
    for target in skills_cfg.get("agents", {}).values():
        if not isinstance(target, str):
            continue
        agent_dir = _resolve_agent_dir(target)
        if agent_dir.is_dir():
            for entry in agent_dir.iterdir():
                universe.add(entry.name)
    return universe


def load_retired_names(root: Path) -> list[str]:
    # Retired/renamed skill names that must not appear as references in managed content.
    # Catches bare names (prose/table cells) that the `<name>` skill form misses.
    path = root / "config" / "retired-skills.json"
    if not path.is_file():
        return []
    data = json.loads(path.read_text(encoding="utf-8"))
    return [str(name) for name in data.get("retired", [])]


def audit(root: Path):
    universe = build_universe(root)
    retired = load_retired_names(root)
    retired_patterns = [
        (name, re.compile(rf"(?<![\w-]){re.escape(name)}(?![\w-])")) for name in retired
    ]
    # keyed by (rel, lineno, name) so a retired name in `<name>` skill form is not double-reported;
    # "retired" wins over "unknown" as the more specific, actionable reason.
    found: dict[tuple, str] = {}
    for path in managed_content_files(root):
        rel = path.relative_to(root)
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            for match in REFERENCE_RE.finditer(line):
                name = match.group(1)
                if name not in universe:
                    found.setdefault((rel, lineno, name), "unknown")
            for name, pattern in retired_patterns:
                if pattern.search(line):
                    found[(rel, lineno, name)] = "retired"
    findings = [(rel, lineno, name, kind) for (rel, lineno, name), kind in found.items()]
    findings.sort(key=lambda item: (str(item[0]), item[1], item[2]))
    return findings, universe, retired


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root-dir", default=str(Path(__file__).resolve().parent.parent))
    args = parser.parse_args()
    root = Path(args.root_dir)

    try:
        findings, universe, retired = audit(root)
    except (OSError, ValueError) as exc:
        print(f"skill-reference audit: 配置读取失败: {exc}", file=sys.stderr)
        return 2

    if not findings:
        print(
            f"skill-reference audit: OK(无失效引用;已知 skill 全集 {len(universe)} 个,"
            f"退役黑名单 {len(retired)} 个)"
        )
        return 0

    print("skill-reference audit: 发现失效 skill 引用(内容点名了不存在的 skill):")
    for rel, lineno, name, kind in findings:
        reason = (
            "已退役/改名(见 config/retired-skills.json)"
            if kind == "retired"
            else "不在已知 skill 全集内(未安装/未声明)"
        )
        print(f"  {rel}:{lineno}: {name} — {reason}")
    print("修复:把引用改指向真实 skill,或安装被引用的 skill;本审计不改写内容。")
    return 1


if __name__ == "__main__":
    sys.exit(main())
