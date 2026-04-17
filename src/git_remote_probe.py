from __future__ import annotations

from pathlib import Path


GIT_REMOTE_STATE_LATEST = "latest"
GIT_REMOTE_STATE_STALE = "stale"
GIT_REMOTE_STATE_UNKNOWN = "unknown"


def _resolve_git_head(run_cmd, cwd, revision, timeout=5):
    ok, out = run_cmd(["git", "rev-parse", revision], cwd=cwd, timeout=timeout)
    if not ok:
        return None
    lines = out.strip().splitlines()
    if not lines:
        return None
    head = lines[0].strip()
    return head or None


def probe_git_remote_state(run_cmd, cwd, timeout=5):
    cwd_path = Path(cwd).resolve()
    result = {
        "status": GIT_REMOTE_STATE_UNKNOWN,
        "local_head": None,
        "remote_head": None,
        "local_remote_head": None,
    }

    local_head = _resolve_git_head(run_cmd, cwd_path, "HEAD", timeout=timeout)
    result["local_head"] = local_head
    if local_head is None:
        return result

    ok, out = run_cmd(["git", "ls-remote", "origin", "HEAD"], cwd=cwd_path, timeout=timeout)
    if not ok:
        return result
    lines = out.strip().splitlines()
    if not lines:
        return result
    fields = lines[0].split()
    if not fields:
        return result
    remote_head = fields[0].strip()
    if not remote_head:
        return result

    result["remote_head"] = remote_head
    result["local_remote_head"] = _resolve_git_head(
        run_cmd,
        cwd_path,
        "refs/remotes/origin/HEAD",
        timeout=timeout,
    )
    result["status"] = (
        GIT_REMOTE_STATE_LATEST if local_head == remote_head else GIT_REMOTE_STATE_STALE
    )
    return result
