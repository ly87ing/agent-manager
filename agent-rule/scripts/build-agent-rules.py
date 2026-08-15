#!/usr/bin/env python3

from __future__ import annotations

import shutil
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SRC_ROOT = ROOT / "rules" / "src"
DIST_ROOT = ROOT / "rules" / "dist"
TARGETS = ("codex", "claude")
TARGET_SPECS = {
    "codex": {"adapter": "codex", "filename": "AGENTS.md"},
    "claude": {"adapter": "claude", "filename": "CLAUDE.md"},
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


def render_target_content(core_lines: list[str], adapter_lines: list[str]) -> str:
    return "\n".join([*core_lines, *adapter_lines]) + "\n"


def build_target_file(target: str) -> None:
    target_dir = DIST_ROOT / target
    target_dir.mkdir(parents=True, exist_ok=True)
    core_lines = compact_lines(read_text(SRC_ROOT / "shared" / "core.md"))
    spec = TARGET_SPECS[target]
    adapter_lines = compact_lines(read_text(SRC_ROOT / "agents" / spec["adapter"] / "adapter.md"))
    content = render_target_content(core_lines, adapter_lines)
    filename = spec["filename"]
    output_path = target_dir / filename
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(content, encoding="utf-8")


def main() -> int:
    reset_directory(DIST_ROOT)
    write_dist_gitkeep()
    for target in TARGETS:
        build_target_file(target)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
