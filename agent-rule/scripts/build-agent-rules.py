#!/usr/bin/env python3

from __future__ import annotations

import shutil
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SRC_ROOT = ROOT / "rules" / "src"
DIST_ROOT = ROOT / "rules" / "dist"
MODULES_SRC = SRC_ROOT / "shared" / "modules"
MODULES_DIST = DIST_ROOT / "modules"
TARGETS = ("codex", "claude", "gemini", "cursor")
TARGET_SPECS = {
    "codex": {"adapter": "codex", "filename": "AGENTS.md"},
    "claude": {"adapter": "claude", "filename": "CLAUDE.md"},
    "gemini": {"adapter": "gemini", "filename": "GEMINI.md"},
    "cursor": {
        "adapter": "ide",
        "filename": "agent-manager.mdc",
        "prefix_lines": ["---", "alwaysApply: true", "---"],
    },
}


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def compact_lines(text: str) -> list[str]:
    return [line.rstrip() for line in text.splitlines() if line.strip()]


def reset_directory(path: Path) -> None:
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True, exist_ok=True)


def write_dist_gitkeep() -> None:
    (DIST_ROOT / ".gitkeep").write_text("", encoding="utf-8")


def build_target_file(target: str) -> None:
    target_dir = DIST_ROOT / target
    target_dir.mkdir(parents=True, exist_ok=True)
    core_lines = compact_lines(read_text(SRC_ROOT / "shared" / "core.md"))
    spec = TARGET_SPECS[target]
    adapter_lines = compact_lines(read_text(SRC_ROOT / "agents" / spec["adapter"] / "adapter.md"))
    content = "\n".join([*spec.get("prefix_lines", []), *core_lines, *adapter_lines]) + "\n"
    filename = spec["filename"]
    (target_dir / filename).write_text(content, encoding="utf-8")


def copy_modules() -> None:
    reset_directory(MODULES_DIST)
    for src in sorted(MODULES_SRC.glob("*.md")):
        shutil.copy2(src, MODULES_DIST / src.name)


def main() -> int:
    reset_directory(DIST_ROOT)
    write_dist_gitkeep()
    copy_modules()
    for target in TARGETS:
        build_target_file(target)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
