#!/usr/bin/env python3
"""本机 per-skill 开关（免手编 JSON）。

把某个 skill 名加入/移出 `manager.local.json` 的 `skills.<repo>.disabled`。
- 只改本机覆盖文件，不动 `skills.json` / `manager.json`。
- 不做 reconcile：改完由 `skills:apply` 重新合并配置并清/建链使其生效。

用法: skill_toggle.py {disable|enable|list} [SKILL] [--def-file ...] [--local ...]
"""
import argparse
import json
import sys
from pathlib import Path


def _load_json(path, default):
    p = Path(path)
    if not p.exists():
        return default
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except (ValueError, OSError) as exc:
        sys.exit(f"读取 {path} 失败: {exc}")


def build_skill_repo_map(def_file):
    """skill 名 -> 所属 repo 名。名字取 skill_paths 的 basename（当前各仓 basename 即 SKILL.md name）。"""
    defs = _load_json(def_file, {})
    repositories = defs.get("repositories", {})
    if not isinstance(repositories, dict):
        sys.exit(f"{def_file}: repositories 必须是对象")
    mapping = {}
    for repo_name, repo in repositories.items():
        for skill_path in (repo.get("skill_paths") or []):
            mapping.setdefault(Path(skill_path).name, repo_name)
    return mapping


def _disabled_list(local, repo):
    raw = local.get("skills", {}).get(repo, {}).get("disabled")
    return list(raw) if isinstance(raw, list) else []


def cmd_list(repo_map, local):
    if not repo_map:
        print("（skills.json 未声明任何 skill）")
        return 0
    for skill in sorted(repo_map):
        repo = repo_map[skill]
        state = "已禁用" if skill in _disabled_list(local, repo) else "已启用"
        print(f"  [{state}] {skill}  (repo: {repo})")
    return 0


def cmd_toggle(action, skill, repo_map, local, local_path):
    repo = repo_map.get(skill)
    if repo is None:
        sys.exit(f"未知 skill: {skill}\n可选: {', '.join(sorted(repo_map))}")

    skills = local.setdefault("skills", {})
    if not isinstance(skills, dict):
        sys.exit(f"{local_path}: skills 必须是对象")
    repo_cfg = skills.setdefault(repo, {})
    if not isinstance(repo_cfg, dict):
        sys.exit(f"{local_path}: skills.{repo} 必须是对象")
    disabled = _disabled_list(local, repo)

    changed = False
    if action == "disable":
        if skill in disabled:
            print(f"{skill} 已处于禁用状态 (repo: {repo})")
        else:
            repo_cfg["disabled"] = sorted(set(disabled) | {skill})
            changed = True
            print(f"已禁用 {skill} (repo: {repo})")
    else:  # enable
        if skill not in disabled:
            print(f"{skill} 未被禁用 (repo: {repo})")
        else:
            remaining = sorted(set(disabled) - {skill})
            if remaining:
                repo_cfg["disabled"] = remaining
            else:
                repo_cfg.pop("disabled", None)
            changed = True
            print(f"已启用 {skill} (repo: {repo})")

    if not changed:
        return 0

    # 修剪空壳，保持覆盖文件干净（不误删其它键，如 browser_us_sync）
    if not repo_cfg:
        skills.pop(repo, None)
    if not skills:
        local.pop("skills", None)

    Path(local_path).write_text(
        json.dumps(local, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    return 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=["disable", "enable", "list"])
    parser.add_argument("skill", nargs="?")
    parser.add_argument("--def-file", default="config/skills.json")
    parser.add_argument("--local", default="config/manager.local.json")
    args = parser.parse_args()

    repo_map = build_skill_repo_map(args.def_file)
    local = _load_json(args.local, {})
    if not isinstance(local, dict):
        sys.exit(f"{args.local}: 顶层必须是 JSON 对象")

    if args.action == "list":
        return cmd_list(repo_map, local)

    skill = (args.skill or "").strip()
    if not skill:
        sys.exit(f"用法: skill_toggle.py {args.action} <SKILL>")
    return cmd_toggle(args.action, skill, repo_map, local, args.local)


if __name__ == "__main__":
    sys.exit(main())
