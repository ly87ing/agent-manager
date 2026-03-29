from __future__ import annotations

import os
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path


EXCLUDED_DIR_NAMES = {
    ".git",
    ".worktrees",
    ".venv",
    ".pytest_cache",
    ".mypy_cache",
    "__pycache__",
    "env",
    "ENV",
    "venv",
    "node_modules",
    "secrets",
}

EXCLUDED_PATH_PREFIXES = {
    "agent-skill/repositories",
}

SECRET_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("openai_like_key", re.compile(r"\bsk-(?:proj-|ant-)?[A-Za-z0-9_-]{20,}\b")),
    ("github_token", re.compile(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b")),
    ("google_api_key", re.compile(r"\bAIza[0-9A-Za-z_-]{20,}\b")),
    ("aws_access_key", re.compile(r"\bAKIA[0-9A-Z]{16}\b")),
    ("private_key_block", re.compile(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----")),
    ("augment_session_token", re.compile(r'"accessToken"\s*:\s*"ace_[A-Za-z0-9_-]{12,}"')),
)


@dataclass(frozen=True)
class SecretFinding:
    path: str
    line: int
    kind: str


def scan_text_for_secrets(text: str, *, file_label: str = "<text>") -> list[SecretFinding]:
    findings: list[SecretFinding] = []
    seen: set[tuple[str, int, str]] = set()
    for kind, pattern in SECRET_PATTERNS:
        for match in pattern.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            key = (file_label, line, kind)
            if key in seen:
                continue
            seen.add(key)
            findings.append(SecretFinding(path=file_label, line=line, kind=kind))
    return findings


def scan_repo_for_secrets(root_dir: Path | str) -> list[SecretFinding]:
    root_dir = Path(root_dir)
    findings: list[SecretFinding] = []
    for path in iter_repo_scan_paths(root_dir):
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        rel_path = str(path.relative_to(root_dir)).replace(os.sep, "/")
        findings.extend(scan_text_for_secrets(text, file_label=rel_path))
    return sorted(findings, key=lambda item: (item.path, item.line, item.kind))


def iter_repo_scan_paths(root_dir: Path) -> list[Path]:
    tracked_paths = git_repo_paths(root_dir)
    if tracked_paths is not None:
        return [path for path in tracked_paths if not is_excluded_repo_path(path.relative_to(root_dir))]

    paths: list[Path] = []
    for dirpath, dirnames, filenames in os.walk(root_dir):
        current = Path(dirpath)
        rel_dir = current.relative_to(root_dir)
        dirnames[:] = [
            name for name in sorted(dirnames) if not is_excluded_repo_path(rel_dir / name, directory_only=True)
        ]
        for filename in sorted(filenames):
            rel_path = rel_dir / filename
            if is_excluded_repo_path(rel_path):
                continue
            paths.append(current / filename)
    return paths


def git_repo_paths(root_dir: Path) -> list[Path] | None:
    if not (root_dir / ".git").exists():
        return None
    result = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=root_dir,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        return None
    paths: list[Path] = []
    for raw in result.stdout.decode("utf-8", errors="ignore").split("\0"):
        if not raw:
            continue
        paths.append(root_dir / raw)
    return paths


def is_excluded_repo_path(rel_path: Path, *, directory_only: bool = False) -> bool:
    normalized = str(rel_path).replace(os.sep, "/").lstrip("./")
    if not normalized:
        return False
    if any(normalized == prefix or normalized.startswith(f"{prefix}/") for prefix in EXCLUDED_PATH_PREFIXES):
        return True

    parts = rel_path.parts
    if any(part in EXCLUDED_DIR_NAMES for part in parts[:-1]):
        return True

    name = rel_path.name
    if directory_only:
        return name in EXCLUDED_DIR_NAMES

    if name in EXCLUDED_DIR_NAMES:
        return True
    if ".bak." in name or name.startswith(".tmp_") or name.endswith(".pyc"):
        return True
    return False
