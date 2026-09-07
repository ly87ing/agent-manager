#!/usr/bin/env python3
"""Personalized config sync engine (docs/GUI-ARCHITECTURE-PROPOSAL.md section 7.2).

Three-layer policy stack: baseline (config/manager.json, committed to the main
repo) + synced (config/manager.synced.json, owned by this engine, never
hand-edited) + local (config/manager.local.json, never leaves the machine).
This module is the writer of the synced layer across machines and keeps a base
snapshot (config/manager.synced.base.json, gitignored) holding the remote
content as of the last successful pull/push — the third leg of every
three-way merge.

Pluggable backends: the engine only talks to the SyncBackend interface
(status/fetch/publish). v1 implements GitBackend (any git remote, same
clone/pull pattern as scripts/init-secrets.sh); gist / s3 / webdav plug in
later by implementing the same three methods and registering in
build_backend() — no command-flow changes needed.

Git backend sync-repo layout:

    profile.json        # single source of truth for the shared layer
    meta.json           # {"schema_version": 1, "machines": {<machine-id>: {"last_sync": ...}}}
    machines/<id>.json  # (P2 远期) per-machine split — layout documented, not implemented

Hard rules (proposal section 8 red lines):
- The backend address never enters the main repo: env PROFILE_SYNC_REPO_URL
  or manager.local.json key profile_sync.repo_url only.
- push runs the suspected-plaintext scan (src/secret_scan.py patterns, same
  set as scripts/check_no_secrets.py) as a hard gate: any hit blocks the push
  with zero remote writes.
- pull/push are network-grade actions: refused under --offline.
- pull/push snapshot the local layer + base before writing (reuses
  src/engine_apply.py's snapshot mechanism — the same one '<domain>:apply'
  uses, so snapshots:list/restore can see and roll back these runs).
- stdout carries exactly one envelope JSON (contract/engine-v1/envelope.schema.json);
  constructors are reused from src/engine_v1.py. Exit codes: 0 ok
  (warnings allowed), 1 errors non-empty, 2 usage/contract error.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from getpass import getuser
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src import engine_apply
from src.engine_v1 import diagnostic, emit_envelope, envelope, generate_run_id
from src.json_config import atomic_write_json_file, load_json_object_file
from src.policy_layers import layer_paths
from src.secret_scan import scan_text_for_secrets

PROFILE_SCHEMA_VERSION = 1
PROFILE_JSON = "profile.json"
META_JSON = "meta.json"

REPO_URL_ENV = "PROFILE_SYNC_REPO_URL"
CACHE_DIR_ENV = "AGENT_MANAGER_PROFILE_CACHE"
MACHINE_ID_ENV = "AGENT_MANAGER_PROFILE_MACHINE_ID"

GIT_TIMEOUT_SEC = 120

VERBS = ("status", "pull", "push")

# Three-way merge sentinel for "key absent on this side" (JSON values never
# compare equal to it).
_ABSENT = object()


class BackendError(Exception):
    """Backend operation failure (clone/fetch/push); exit 1 PROFILE_BACKEND_ERROR."""


class SecretGateError(Exception):
    """Suspected plaintext in the content to publish; zero remote writes."""


class DivergedError(Exception):
    """Remote moved since the last sync; push would clobber it — pull first."""


def _now_iso():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def machine_id():
    """Hostname-derived stable machine id (only ever written into meta.json's
    machine registry). AGENT_MANAGER_PROFILE_MACHINE_ID overrides it (tests,
    containers with unstable hostnames)."""
    raw = os.environ.get(MACHINE_ID_ENV, "").strip() or socket.gethostname()
    normalized = re.sub(r"[^a-z0-9]+", "-", raw.lower()).strip("-")
    return normalized or "machine"


def mask_repo_url(url):
    """Redact credentials from the backend URL before it enters any payload."""
    if not url:
        return None
    if "://" in url:
        match = re.match(r"^([a-zA-Z][a-zA-Z0-9+.-]*://)[^@/]*@(.*)$", url)
        if match:
            return "%s***@%s" % (match.group(1), match.group(2))
        return url
    match = re.match(r"^[^@/]+@([^:]+:.*)$", url)  # scp-like ssh: user@host:path
    if match:
        return "***@%s" % match.group(1)
    return url


def derive_base_path(synced_path):
    """config/manager.synced.json -> config/manager.synced.base.json (alongside
    whatever AGENT_MANAGER_SYNCED_CONFIG points at)."""
    synced_path = Path(synced_path)
    name = synced_path.name
    stem = name[: -len(".json")] if name.endswith(".json") else name
    return synced_path.with_name(stem + ".base.json")


def _cache_root():
    configured = os.environ.get(CACHE_DIR_ENV, "").strip()
    if configured:
        return Path(configured).expanduser()
    return Path(tempfile.gettempdir()) / ("agent_manager_profile_%s" % getuser())


# ---------------------------------------------------------------------------
# backend interface (section 7.2 pluggable backends)
# ---------------------------------------------------------------------------


class SyncBackend:
    """Backend interface the engine works against.

    Extension point for gist / s3 / webdav: implement the three methods and
    register the new class in build_backend(); the command flow (merge,
    conflicts, gates, envelopes) is backend-agnostic.

    - status():  zero-network self-description for payloads (type, masked url).
    - fetch():   network; returns {"initialized", "profile", "meta", "ref"} —
      profile/meta are None when the remote holds no sync content yet.
    - publish(snapshot, profile, meta): network; makes the remote hold exactly
      the given profile/meta content (snapshot must come from the fetch that
      guarded this publish).
    """

    def status(self):
        raise NotImplementedError

    def fetch(self):
        raise NotImplementedError

    def publish(self, snapshot, profile, meta):
        raise NotImplementedError


class GitBackend(SyncBackend):
    """git remote backend: clones into a per-user tmp cache, fetch/push via git."""

    def __init__(self, url, cache_root):
        self.url = url
        self.repo_dir = Path(cache_root) / "repo"

    def status(self):
        return {"type": "git", "url": mask_repo_url(self.url), "configured": True}

    def _git(self, args, cwd=None, allow_fail=False):
        env = os.environ.copy()
        env["GIT_TERMINAL_PROMPT"] = "0"  # never hang on credential prompts
        workdir = str(cwd) if cwd is not None else (str(self.repo_dir) if self.repo_dir.is_dir() else None)
        result = subprocess.run(
            ["git"] + args,
            cwd=workdir,
            capture_output=True,
            text=True,
            env=env,
            timeout=GIT_TIMEOUT_SEC,
            check=False,
        )
        if result.returncode != 0 and not allow_fail:
            detail = (result.stderr or result.stdout or "").strip().splitlines()
            raise BackendError(
                "git %s 失败: %s" % (args[0], detail[-1] if detail else "exit %d" % result.returncode)
            )
        return result

    def _ensure_clone(self):
        if (self.repo_dir / ".git").is_dir():
            return
        if self.repo_dir.exists():
            shutil.rmtree(self.repo_dir)  # partial clone residue
        self.repo_dir.parent.mkdir(parents=True, exist_ok=True)
        result = self._git(["clone", self.url, str(self.repo_dir)], cwd=self.repo_dir.parent, allow_fail=True)
        if result.returncode != 0:
            detail = (result.stderr or result.stdout or "").strip().splitlines()
            raise BackendError("clone 失败: %s" % (detail[-1] if detail else "unknown"))

    def _remote_ref(self):
        """Default-branch remote ref ('refs/remotes/origin/main'), or None when
        the remote has no commits yet (uninitialized)."""
        result = self._git(["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"], allow_fail=True)
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout.strip()
        for candidate in ("refs/remotes/origin/main", "refs/remotes/origin/master"):
            if self._git(["show-ref", "--verify", candidate], allow_fail=True).returncode == 0:
                return candidate
        result = self._git(["for-each-ref", "--format=%(refname)", "refs/remotes/origin/"], allow_fail=True)
        refs = [line.strip() for line in result.stdout.splitlines() if line.strip()]
        return refs[0] if refs else None

    def _read_remote_json(self, ref, name):
        result = self._git(["show", "%s:%s" % (ref, name)], allow_fail=True)
        if result.returncode != 0:
            return None
        try:
            data = json.loads(result.stdout)
        except ValueError as exc:
            raise BackendError("远端 %s 不是合法 JSON: %s" % (name, exc))
        if not isinstance(data, dict):
            raise BackendError("远端 %s 顶层必须是对象" % name)
        return data

    def fetch(self):
        self._ensure_clone()
        self._git(["fetch", "origin"])
        ref = self._remote_ref()
        if ref is None:
            return {"initialized": False, "profile": None, "meta": None, "ref": None}
        profile = self._read_remote_json(ref, PROFILE_JSON)
        meta = self._read_remote_json(ref, META_JSON)
        return {
            "initialized": True,
            "profile": profile if profile is not None else {},
            "meta": meta if meta is not None else {},
            "ref": ref,
        }

    def publish(self, snapshot, profile, meta):
        self._ensure_clone()
        ref = snapshot["ref"]
        if ref is not None:
            # refs/remotes/origin/main -> main（不能按 / 切两段：会留下 "origin/main"）
            branch = ref[len("refs/remotes/origin/"):] if ref.startswith("refs/remotes/origin/") else ref.rsplit("/", 1)[-1]
            self._git(["checkout", "-B", branch, ref])
        else:
            # Empty-remote clone: HEAD already points at the unborn default
            # branch (remote HEAD symref when advertised, else local default).
            result = self._git(["symbolic-ref", "--short", "HEAD"], allow_fail=True)
            branch = result.stdout.strip() if result.returncode == 0 and result.stdout.strip() else "main"
        atomic_write_json_file(str(self.repo_dir / PROFILE_JSON), profile, chmod=0o600)
        atomic_write_json_file(str(self.repo_dir / META_JSON), meta, chmod=0o600)
        self._git(["add", PROFILE_JSON, META_JSON])
        staged = self._git(["diff", "--cached", "--quiet"], allow_fail=True)
        if staged.returncode == 0:
            return {"committed": False, "branch": branch}
        if staged.returncode != 1:
            raise BackendError("git diff --cached 异常: exit %d" % staged.returncode)
        self._git(
            [
                "-c", "user.name=agent-manager-profile-sync",
                "-c", "user.email=agent-manager-profile-sync@localhost",
                "-c", "commit.gpgsign=false",
                "commit", "-m", "profile sync: publish shared layer (%s)" % machine_id(),
            ]
        )
        pushed = self._git(["push", "origin", "HEAD:refs/heads/%s" % branch], allow_fail=True)
        if pushed.returncode != 0:
            detail = (pushed.stderr or pushed.stdout or "").strip().splitlines()
            raise BackendError(
                "git push 被拒绝: %s（远端可能已移动，先 profile:pull 合并）"
                % (detail[-1] if detail else "unknown")
            )
        return {"committed": True, "branch": branch}


def build_backend(local_config_path):
    """Resolve the backend address (env first, then the local-only layer) and
    construct the v1 git backend. Returns None when unconfigured."""
    url = os.environ.get(REPO_URL_ENV, "").strip()
    if not url and Path(local_config_path).is_file():
        local_cfg = load_json_object_file(str(local_config_path), strict=False)
        section = local_cfg.get("profile_sync")
        if isinstance(section, dict):
            url = str(section.get("repo_url") or "").strip()
    if not url:
        return None
    return GitBackend(url, _cache_root())


# ---------------------------------------------------------------------------
# three-way merge (section 7.2 conflict semantics)
# ---------------------------------------------------------------------------


def _json_type_name(value):
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, (int, float)):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    return "object"


def _value_summary(value, present=True):
    """Digest-only summary of one side of a conflict — values (which might be
    sensitive) never enter the payload."""
    if not present:
        return {"type": "absent", "digest": None}
    canonical = json.dumps(value, sort_keys=True, ensure_ascii=False)
    return {
        "type": _json_type_name(value),
        "digest": "sha256:" + hashlib.sha256(canonical.encode("utf-8")).hexdigest(),
    }


def three_way_merge(base, local, remote, _path=""):
    """Per-key three-way merge; returns (merged, conflicts).

    - only one side changed vs base -> take that side (including deletions);
    - both changed and differ: dicts recurse (same recursion rule as the
      deep_merge used for local overrides; arrays are whole values), anything
      else is a conflict — the local value is kept and the key is reported;
    - neither changed -> keep the value as-is.
    """
    merged = {}
    conflicts = []
    for key in sorted(set(base) | set(local) | set(remote)):
        b = base.get(key, _ABSENT)
        l = local.get(key, _ABSENT)
        r = remote.get(key, _ABSENT)
        key_path = key if not _path else "%s.%s" % (_path, key)
        if l is r or l == r:
            if l is not _ABSENT:
                merged[key] = l
            continue
        if l is b or l == b:
            if r is not _ABSENT:
                merged[key] = r
            continue
        if r is b or r == b:
            if l is not _ABSENT:
                merged[key] = l
            continue
        if isinstance(l, dict) and isinstance(r, dict):
            sub, sub_conflicts = three_way_merge(b if isinstance(b, dict) else {}, l, r, key_path)
            merged[key] = sub
            conflicts.extend(sub_conflicts)
            continue
        conflicts.append(
            {
                "key": key_path,
                "base": _value_summary(b, b is not _ABSENT),
                "local": _value_summary(l, l is not _ABSENT),
                "remote": _value_summary(r, r is not _ABSENT),
            }
        )
        if l is not _ABSENT:
            merged[key] = l
    return merged, conflicts


def _changed_top_keys(base, other):
    return sorted(k for k in set(base) | set(other) if base.get(k, _ABSENT) != other.get(k, _ABSENT))


def classify_state(base, local, initialized, remote):
    """(state, ahead_count, behind_count): top-level key divergence vs base."""
    if not initialized:
        return "uninitialized", None, None
    if local == remote:
        return "clean", 0, 0
    ahead = _changed_top_keys(base, local)
    behind = _changed_top_keys(base, remote)
    if ahead and behind:
        return "diverged", len(ahead), len(behind)
    if ahead:
        return "ahead", len(ahead), 0
    return "behind", 0, len(behind)


def _short_digest(value, present=True):
    """sha256 前 12 位（值永不进 payload）；absent → (null, 'absent')。"""
    if not present:
        return None, "absent"
    canonical = json.dumps(value, sort_keys=True, ensure_ascii=False)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()[:12], _json_type_name(value)


def compute_diff(base, local, remote):
    """profile:status 的 diff 数组：per 顶层键的变更归属与双侧 digest。

    change 分类（相对三向合并 base 快照）：只本地改 → local；只远端改 →
    remote；两侧都改且不同 → both；都没改 → none。digest 为 sha256 前 12 位，
    值永不进 payload；某侧无此键 → digest=null + type='absent'。
    """
    diff = []
    for key in sorted(set(base) | set(local) | set(remote)):
        b = base.get(key, _ABSENT)
        l = local.get(key, _ABSENT)
        r = remote.get(key, _ABSENT)
        local_changed = l != b
        remote_changed = r != b
        if local_changed and remote_changed and l != r:
            change = "both"
        elif local_changed:
            change = "local"
        elif remote_changed:
            change = "remote"
        else:
            change = "none"
        local_digest, local_type = _short_digest(l, l is not _ABSENT)
        remote_digest, remote_type = _short_digest(r, r is not _ABSENT)
        diff.append(
            {
                "key": key,
                "change": change,
                "local_type": local_type,
                "remote_type": remote_type,
                "local_digest": local_digest,
                "remote_digest": remote_digest,
            }
        )
    return diff


# ---------------------------------------------------------------------------
# command kernels
# ---------------------------------------------------------------------------


def _load_layer(path):
    if not Path(path).is_file():
        return False, {}
    return True, load_json_object_file(str(path), strict=True, missing_ok=False)


def _machine_last_sync(meta, machine):
    machines = meta.get("machines") if isinstance(meta, dict) else None
    entry = machines.get(machine) if isinstance(machines, dict) else None
    if isinstance(entry, dict):
        return entry.get("last_sync")
    return None


def _base_data(verb, backend, machine, synced_path, local_exists, local_content, base_exists):
    return {
        "verb": verb,
        "state": None,
        "backend": backend.status(),
        "machine_id": machine,
        "local": {
            "path": str(synced_path),
            "exists": local_exists,
            "key_count": len(local_content),
        },
        "base": {"exists": base_exists},
        "ahead": None,
        "behind": None,
        "last_sync": None,
        "conflicts": [],
        "snapshot": None,
        "published": None,
        "gate": None,
        "diff": [],
    }


def _snapshot_layers(run_id, command, synced_path, base_path):
    """Pre-write snapshot via the engine's run-snapshot mechanism
    (src/engine_apply.py) — listed by snapshots:list, restorable by
    snapshots:restore like any '<domain>:apply' run."""
    engine_apply.create_snapshot(run_id, command, [str(synced_path), str(base_path)], [])
    return {"run_id": run_id, "path": str(engine_apply.snapshot_root() / run_id)}


def run_status(paths, backend, mode, machine, warnings):
    synced_path, base_path = paths["synced"], paths["base"]
    local_exists, local = _load_layer(synced_path)
    base_exists = base_path.is_file()
    data = _base_data("status", backend, machine, synced_path, local_exists, local, base_exists)
    if mode["offline"]:
        data["state"] = "local-only"
        warnings.append(
            diagnostic(
                "PROFILE_OFFLINE_LOCAL_ONLY",
                None,
                "offline 模式：仅报告本机共享层状态，未做远端比对",
            )
        )
        return data
    snapshot = backend.fetch()
    if not snapshot["initialized"]:
        data["state"] = "uninitialized"
        return data
    _, base = _load_layer(base_path)
    state, ahead, behind = classify_state(base, local, True, snapshot["profile"])
    data.update(
        state=state,
        ahead=ahead,
        behind=behind,
        last_sync=_machine_last_sync(snapshot["meta"], machine),
    )
    # diff 需要三方齐全：远端已初始化 + 本地有 base 快照（上次成功同步的远端内容）。
    if base_exists:
        data["diff"] = compute_diff(base, local, snapshot["profile"])
    return data


def run_pull(paths, backend, mode, machine, run_id, warnings, accept_remote=False, only=None):
    synced_path, base_path = paths["synced"], paths["base"]
    local_exists, local = _load_layer(synced_path)
    base_exists, base = _load_layer(base_path)
    snapshot = backend.fetch()
    if not snapshot["initialized"]:
        data = _base_data("pull", backend, machine, synced_path, local_exists, local, base_exists)
        data["state"] = "uninitialized"
        warnings.append(
            diagnostic(
                "PROFILE_REMOTE_UNINITIALIZED",
                None,
                "远端同步仓库尚未初始化，无可拉取内容（profile:push 可初始化）",
            )
        )
        return data
    remote = snapshot["profile"]
    merged, conflicts = three_way_merge(base, local, remote)
    if accept_remote and only is None:
        # 远端完全覆盖本地（含 base 推进）；本地键消失、远端键落地。
        merged = dict(remote)
        conflicts = []
    elif only is not None:
        # 键级「以远端为准」：指定顶层键取远端（远端无此键 = 本地删除），
        # 其余键仍走三向合并。
        if only in remote:
            merged[only] = remote[only]
        else:
            merged.pop(only, None)
        conflicts = [entry for entry in conflicts if entry["key"] != only and not entry["key"].startswith(only + ".")]
    if conflicts:
        warnings.append(
            diagnostic(
                "PROFILE_CONFLICTS",
                None,
                "%d 个键两侧都改且不同，已保留本地值: %s"
                % (len(conflicts), ", ".join(entry["key"] for entry in conflicts)),
            )
        )
    # 合并完成后 base 推进到远端内容；若冲突保留了本地值，本地相对远端处于
    # ahead（待 profile:push 发布解决结果）。
    state, ahead, behind = classify_state(remote, merged, True, remote)
    snap_info = None
    if not mode["dry_run"]:
        snap_info = _snapshot_layers(run_id, "profile:pull", synced_path, base_path)
        atomic_write_json_file(str(synced_path), merged, chmod=0o600)
        atomic_write_json_file(str(base_path), remote, chmod=0o600)
    if mode["dry_run"]:
        data = _base_data("pull", backend, machine, synced_path, local_exists, local, base_exists)
    else:
        data = _base_data("pull", backend, machine, synced_path, True, merged, True)
    data.update(
        state=state,
        ahead=ahead,
        behind=behind,
        last_sync=_machine_last_sync(snapshot["meta"], machine),
        conflicts=conflicts,
        snapshot=snap_info,
    )
    return data


def run_push(paths, backend, mode, machine, run_id, warnings, force=False):
    synced_path, base_path = paths["synced"], paths["base"]
    local_exists, local = _load_layer(synced_path)
    base_exists, base = _load_layer(base_path)

    # 硬门禁：疑似明文扫描命中即零推送（同步仓库一旦误设为公开，代价必须可控）。
    rendered = json.dumps(local, ensure_ascii=False, sort_keys=True)
    findings = scan_text_for_secrets(rendered, file_label=Path(synced_path).name)
    if findings:
        kinds = ", ".join(sorted({finding.kind for finding in findings}))
        raise SecretGateError("疑似明文敏感信息（%s），已阻断推送，远端零变更" % kinds)
    gate = {"scanned": True, "findings": 0}

    snapshot = backend.fetch()
    if snapshot["initialized"] and snapshot["profile"] != base and not force:
        raise DivergedError(
            "远端内容相对上次同步已变化（behind/diverged），直接 push 会丢失远端键；"
            "请先 profile:pull 合并后再 push"
        )
    state, ahead, behind = classify_state(
        base, local, snapshot["initialized"], snapshot["profile"] if snapshot["initialized"] else None
    )

    if mode["dry_run"]:
        data = _base_data("push", backend, machine, synced_path, local_exists, local, base_exists)
        data.update(state=state, ahead=ahead, behind=behind, published=False, gate=gate)
        return data

    snap_info = _snapshot_layers(run_id, "profile:push", synced_path, base_path)
    now = _now_iso()
    meta = snapshot["meta"] if isinstance(snapshot["meta"], dict) else {}
    machines = meta.get("machines") if isinstance(meta.get("machines"), dict) else {}
    machines = dict(machines)
    machines[machine] = {"last_sync": now}
    meta = {"schema_version": PROFILE_SCHEMA_VERSION, "machines": machines}
    backend.publish(snapshot, local, meta)
    atomic_write_json_file(str(base_path), local, chmod=0o600)

    data = _base_data("push", backend, machine, synced_path, local_exists, local, True)
    data.update(
        state="clean",
        ahead=0,
        behind=0,
        last_sync=now,
        snapshot=snap_info,
        published=True,
        gate=gate,
    )
    return data


# ---------------------------------------------------------------------------
# CLI (envelope contract: stdout = exactly one envelope JSON)
# ---------------------------------------------------------------------------


def _build_parser():
    parser = argparse.ArgumentParser(
        prog="sync_profile.py",
        description="Profile sync engine (contract v1): profile:status/pull/push with envelope JSON on stdout.",
    )
    parser.add_argument("command", nargs="?", help="profile:status | profile:pull | profile:push")
    parser.add_argument("--synced-config", help="共享层文件（默认 config/manager.synced.json 或 AGENT_MANAGER_SYNCED_CONFIG）")
    parser.add_argument("--local-config", help="仅本机层文件（后端地址 profile_sync.repo_url 的读取处）")
    parser.add_argument("--json", action="store_true", help="required: selects the envelope machine interface")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--accept-remote", action="store_true", help="profile:pull 远端完全覆盖本地共享层")
    parser.add_argument("--only", help="profile:pull 只让指定顶层键以远端为准（键级覆盖）")
    parser.add_argument("--force", action="store_true", help="profile:push 分叉时强制以本地覆盖远端")
    return parser


def main(argv=None):
    run_id = os.environ.get("AGENT_MANAGER_RUN_ID") or generate_run_id()
    log_file = os.environ.get("AGENT_MANAGER_LOG_FILE") or None
    lock_held = os.environ.get("AGENT_MANAGER_LOCK_HELD") == "true"
    lock_pid_raw = os.environ.get("AGENT_MANAGER_LOCK_PID") or ""
    if lock_held:
        lock = {"held": True, "pid": int(lock_pid_raw) if lock_pid_raw.isdigit() else os.getpid()}
    else:
        lock = {"held": False, "pid": None}
    mode = {"dry_run": False, "offline": False}

    parser = _build_parser()
    try:
        args, leftover = parser.parse_known_args(argv)
    except SystemExit:
        payload = envelope(
            command="unknown",
            mode=mode,
            lock=lock,
            run_id=run_id,
            log_file=log_file,
            data={},
            warnings=[],
            errors=[diagnostic("USAGE_ERROR", None, "参数解析失败；期望 profile:<verb> --json")],
        )
        emit_envelope(payload)
        return 2

    mode["dry_run"] = bool(args.dry_run)
    mode["offline"] = bool(args.offline)
    raw_command = (args.command or "").strip()
    dotted = raw_command.replace(":", ".", 1)

    def usage_error(code, message, hint):
        payload = envelope(
            command=dotted or "unknown",
            mode=mode,
            lock=lock,
            run_id=run_id,
            log_file=log_file,
            data={},
            warnings=[],
            errors=[diagnostic(code, None, message)],
        )
        emit_envelope(payload)
        print(hint, file=sys.stderr)
        return 2

    def runtime_error(code, message):
        payload = envelope(
            command=dotted,
            mode=mode,
            lock=lock,
            run_id=run_id,
            log_file=log_file,
            data={},
            warnings=[],
            errors=[diagnostic(code, None, message)],
        )
        emit_envelope(payload)
        print("错误: %s" % message, file=sys.stderr)
        return 1

    if not raw_command:
        return usage_error(
            "USAGE_ERROR",
            "缺少命令；期望 profile:status / profile:pull / profile:push",
            "用法: sync_profile.py profile:<verb> --json [--dry-run] [--offline]",
        )
    if not args.json:
        return usage_error(
            "JSON_REQUIRED",
            "命令 '%s' 是机器接口，必须显式追加 --json" % dotted,
            "错误: profile 命令只输出 envelope JSON；请追加 --json",
        )
    if leftover:
        return usage_error(
            "USAGE_ERROR",
            "无法识别的参数: %s" % " ".join(leftover),
            "错误: 无法识别的参数 '%s'" % " ".join(leftover),
        )
    domain, _, verb = dotted.partition(".")
    if domain != "profile" or verb not in VERBS:
        return usage_error(
            "USAGE_ERROR",
            "命令 '%s' 不是 profile:status / profile:pull / profile:push 之一" % raw_command,
            "错误: 未知命令 '%s'" % raw_command,
        )

    defaults = layer_paths(REPO_ROOT)
    synced_path = Path(args.synced_config).expanduser() if args.synced_config else defaults["synced"]
    local_config_path = Path(args.local_config).expanduser() if args.local_config else defaults["local"]
    paths = {"synced": synced_path, "base": derive_base_path(synced_path)}

    backend = build_backend(local_config_path)
    if backend is None:
        return usage_error(
            "PROFILE_BACKEND_UNCONFIGURED",
            "未配置同步后端地址（env %s 或仅本机层 %s 的 profile_sync.repo_url 键）"
            % (REPO_URL_ENV, local_config_path.name),
            "错误: 未配置同步后端；示例: %s=git@YOUR_HOST:YOUR_PROFILE_REPO.git ./manage.sh profile:status --json"
            % REPO_URL_ENV,
        )

    if verb in ("pull", "push") and mode["offline"]:
        return runtime_error(
            "OFFLINE_NETWORK_DENIED",
            "profile:%s 是 network 级动作，offline 模式下拒绝执行" % verb,
        )

    if (args.accept_remote or args.only) and verb != "pull":
        return usage_error(
            "USAGE_ERROR",
            "--accept-remote/--only 仅适用于 profile:pull",
            "错误: --accept-remote/--only 仅适用于 profile:pull",
        )
    if args.force and verb != "push":
        return usage_error(
            "USAGE_ERROR",
            "--force 仅适用于 profile:push",
            "错误: --force 仅适用于 profile:push",
        )
    if args.only is not None:
        from src.policy_layers import _KEY_SEGMENT_RE

        if not _KEY_SEGMENT_RE.match(args.only):
            return usage_error(
                "USAGE_ERROR",
                "--only 只接受顶层键名（字母/数字/_/-）: %s" % args.only,
                "错误: --only 只接受顶层键名（字母/数字/_/-）",
            )

    machine = machine_id()
    warnings = []
    try:
        if verb == "status":
            data = run_status(paths, backend, mode, machine, warnings)
        elif verb == "pull":
            data = run_pull(
                paths, backend, mode, machine, run_id, warnings,
                accept_remote=bool(args.accept_remote), only=args.only,
            )
        else:
            data = run_push(paths, backend, mode, machine, run_id, warnings, force=bool(args.force))
    except SecretGateError as exc:
        return runtime_error("SECRET_GATE_BLOCKED", str(exc))
    except DivergedError as exc:
        return runtime_error("PROFILE_DIVERGED", str(exc))
    except BackendError as exc:
        return runtime_error("PROFILE_BACKEND_ERROR", str(exc))
    except ValueError as exc:
        return runtime_error("CONFIG_INVALID", str(exc))

    payload = envelope(
        command=dotted,
        mode=mode,
        lock=lock,
        run_id=run_id,
        log_file=log_file,
        data=data,
        warnings=warnings,
        errors=[],
    )
    emit_envelope(payload)
    for warning in warnings:
        print("警告: [%s] %s" % (warning["code"], warning["message"]), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
