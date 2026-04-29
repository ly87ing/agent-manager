#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import shlex
import shutil
import subprocess
import sys
from pathlib import Path
from typing import NamedTuple


ROOT = Path(__file__).resolve().parent.parent


class GraphifyTarget(NamedTuple):
    agent_id: str
    kind: str
    path: Path
    skill_file: str = ""


SKILL_FILE_BY_AGENT = {
    "claude-code": "skill.md",
    "codex": "skill-codex.md",
    "gemini-cli": "skill.md",
    "opencode": "skill-opencode.md",
    "kimi-cli": "skill.md",
}

CURSOR_RULE = """\
---
description: graphify knowledge graph context
alwaysApply: true
---

When the current project has a graphify knowledge graph at graphify-out/:

- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- After meaningful code changes, run `graphify update .` when keeping the graph current is part of the task
"""

COPY_SKILLS_CODE = r"""
import json
import shutil
import sys
from importlib.metadata import version
from pathlib import Path

import graphify

payload = json.loads(sys.stdin.read())
package_dir = Path(graphify.__file__).parent
package_version = version("graphifyy")

for target in payload["targets"]:
    source = package_dir / target["skill_file"]
    if not source.exists():
        raise SystemExit(f"missing graphify skill file: {source}")
    destination = Path(target["path"]).expanduser()
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy(source, destination)
    (destination.parent / ".graphify_version").write_text(package_version, encoding="utf-8")

print(package_version)
"""


def read_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def target_base_path(raw_target: object) -> Path | None:
    if isinstance(raw_target, str) and raw_target.strip():
        return Path(raw_target.strip())
    if isinstance(raw_target, dict):
        raw_path = raw_target.get("path")
        if isinstance(raw_path, str) and raw_path.strip():
            return Path(raw_path.strip())
    return None


def add_unique_target(targets: list[GraphifyTarget], target: GraphifyTarget) -> None:
    identity = (target.agent_id, target.kind, target.path.as_posix(), target.skill_file)
    if any((item.agent_id, item.kind, item.path.as_posix(), item.skill_file) == identity for item in targets):
        return
    targets.append(target)


def build_graphify_targets(skill_cfg: dict) -> list[GraphifyTarget]:
    targets: list[GraphifyTarget] = []
    agent_targets = skill_cfg.get("agents", {})
    if not isinstance(agent_targets, dict):
        raise ValueError("skills.json.agents must be an object")

    for agent_id, raw_target in agent_targets.items():
        target_base = target_base_path(raw_target)
        if target_base is None:
            continue
        if agent_id == "cursor":
            add_unique_target(
                targets,
                GraphifyTarget(
                    agent_id=agent_id,
                    kind="cursor-rule",
                    path=target_base / "graphify.mdc",
                ),
            )
            continue
        skill_file = SKILL_FILE_BY_AGENT.get(agent_id)
        if not skill_file:
            continue
        add_unique_target(
            targets,
            GraphifyTarget(
                agent_id=agent_id,
                kind="skill",
                path=target_base / "graphify" / "SKILL.md",
                skill_file=skill_file,
            ),
        )

    if "codex" in agent_targets:
        add_unique_target(
            targets,
            GraphifyTarget(
                agent_id="codex",
                kind="skill",
                path=Path("~/.agents/skills/graphify/SKILL.md"),
                skill_file="skill-codex.md",
            ),
        )

    return targets


def resolve_graphify_python() -> str:
    graphify_bin = shutil.which("graphify")
    if not graphify_bin:
        raise RuntimeError("graphify binary not found on PATH")

    try:
        first_line = Path(graphify_bin).read_text(encoding="utf-8", errors="ignore").splitlines()[0]
    except (OSError, IndexError):
        return sys.executable
    if not first_line.startswith("#!"):
        return sys.executable

    try:
        parts = shlex.split(first_line[2:].strip())
    except ValueError:
        return sys.executable
    if not parts:
        return sys.executable
    if Path(parts[0]).name == "env" and len(parts) >= 2:
        resolved = shutil.which(parts[-1])
        return resolved or parts[-1]
    if Path(parts[0]).exists():
        return parts[0]
    return sys.executable


def install_cursor_rules(targets: list[GraphifyTarget]) -> None:
    for target in targets:
        if target.kind != "cursor-rule":
            continue
        destination = target.path.expanduser()
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(CURSOR_RULE, encoding="utf-8")


def install_skill_files(targets: list[GraphifyTarget], graphify_python: str | None = None) -> str:
    skill_targets = [target for target in targets if target.kind == "skill"]
    if not skill_targets:
        return "unknown"
    payload = {
        "targets": [
            {
                "agent_id": target.agent_id,
                "path": target.path.as_posix(),
                "skill_file": target.skill_file,
            }
            for target in skill_targets
        ]
    }
    result = subprocess.run(
        [graphify_python or resolve_graphify_python(), "-c", COPY_SKILLS_CODE],
        input=json.dumps(payload),
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or "graphify skill install failed")
    return result.stdout.strip().splitlines()[-1] if result.stdout.strip() else "unknown"


def write_marker(marker_dir: Path, targets: list[GraphifyTarget], version: str) -> None:
    marker_dir.mkdir(parents=True, exist_ok=True)
    payload = {
        "version": version,
        "targets": [
            {
                "agent_id": target.agent_id,
                "kind": target.kind,
                "path": target.path.as_posix(),
                "skill_file": target.skill_file,
            }
            for target in targets
        ],
    }
    (marker_dir / "managed-targets.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Install graphify skill files for managed agents")
    parser.add_argument("--config", default=str(ROOT / "config" / "skills.json"), help="skills.json path")
    parser.add_argument("--marker-dir", default="graphify", help="managed marker directory")
    args = parser.parse_args(argv)

    skill_cfg = read_json(Path(args.config))
    targets = build_graphify_targets(skill_cfg)
    install_cursor_rules(targets)
    version = install_skill_files(targets)
    write_marker(Path(args.marker_dir), targets, version)
    print(f"graphify installed for {len(targets)} managed agent target(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
