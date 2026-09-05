"""Engine write-path plumbing: run snapshots/rollback and syncer step execution.

Used by the '<domain>:apply' / 'system:converge' kernels in src/engine_v1.py
(docs/GUI-ARCHITECTURE-PROPOSAL.md sections 2.4 and 4.10). This module owns no
convergence logic — apply always delegates to the existing syncer scripts with
the same argument assembly as manage.sh's run_*_task functions.

Snapshot layout (proposal section 4.10, organized by run_id):

    $AGENT_MANAGER_SNAPSHOT_DIR/            (default $TMPDIR/agent_manager_snapshots_<user>/)
      <run_id>/
        manifest.json     # {run_id, command, created_at, files[], links[]}
        files/<absolute target path mirrored as a relative subtree>

Symlink entries (skills domain) are recorded in the manifest's links list as
{path, target} instead of being copied. restore_snapshot() first re-snapshots
the current state of the same paths (so a restore is itself undoable), then
atomically writes files back and recreates managed symlinks. Real directories
at a link path are never clobbered — they are reported in 'skipped'.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
from datetime import datetime, timezone
from getpass import getuser
from pathlib import Path

SNAPSHOT_DIR_ENV = "AGENT_MANAGER_SNAPSHOT_DIR"
MANIFEST_NAME = "manifest.json"


def snapshot_root():
    configured = os.environ.get(SNAPSHOT_DIR_ENV)
    if configured:
        return Path(configured).expanduser()
    return Path(tempfile.gettempdir()) / ("agent_manager_snapshots_%s" % getuser())


def _now_iso():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _file_slot(snap_dir, abs_path):
    return snap_dir / "files" / str(abs_path).lstrip("/")


def create_snapshot(run_id, command, files, links):
    """Snapshot the given target paths. ``files`` is the full target list; each
    entry is recorded as {"path", "existed"} so restore can also delete files a
    run created. ``links`` is a list of {path, target} symlink records.
    Returns the manifest."""
    snap_dir = snapshot_root() / run_id
    snap_dir.mkdir(parents=True, exist_ok=False)
    file_entries = []
    for abs_path in files:
        path = Path(abs_path)
        entry = {"path": str(path), "existed": path.is_file() and not path.is_symlink()}
        if entry["existed"]:
            slot = _file_slot(snap_dir, abs_path)
            slot.parent.mkdir(parents=True, exist_ok=True)
            fd_path = slot.with_name(slot.name + ".tmp")
            shutil.copyfile(path, fd_path)
            os.chmod(fd_path, 0o600)
            os.replace(fd_path, slot)
        file_entries.append(entry)
    link_entries = [
        {"path": str(link["path"]), "target": str(link["target"])} for link in links
    ]
    manifest = {
        "run_id": run_id,
        "command": command,
        "created_at": _now_iso(),
        "files": file_entries,
        "links": link_entries,
    }
    manifest_path = snap_dir / MANIFEST_NAME
    tmp_path = snap_dir / (MANIFEST_NAME + ".tmp")
    tmp_path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.chmod(tmp_path, 0o600)
    os.replace(tmp_path, manifest_path)
    return manifest


def read_manifest(snap_dir):
    try:
        manifest = json.loads((snap_dir / MANIFEST_NAME).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(manifest, dict) or not isinstance(manifest.get("files"), list):
        return None
    normalized_files = []
    for entry in manifest["files"]:
        if isinstance(entry, str):
            normalized_files.append({"path": entry, "existed": True})
        elif isinstance(entry, dict) and isinstance(entry.get("path"), str):
            normalized_files.append({"path": entry["path"], "existed": bool(entry.get("existed", True))})
    manifest["files"] = normalized_files
    manifest.setdefault("links", [])
    return manifest


def list_snapshots():
    """All snapshots, newest first; entries carry counts and the manifest echo."""
    root = snapshot_root()
    entries = []
    if not root.is_dir():
        return entries
    for child in sorted(root.iterdir(), key=lambda p: p.name, reverse=True):
        if not child.is_dir():
            continue
        manifest = read_manifest(child)
        if manifest is None:
            continue
        entries.append(
            {
                "run_id": manifest.get("run_id", child.name),
                "path": str(child),
                "created_at": manifest.get("created_at"),
                "command": manifest.get("command"),
                "file_count": len(manifest["files"]),
                "link_count": len(manifest["links"]),
            }
        )
    return entries


def _write_bytes_atomic(path, content):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp_path = path.with_name(path.name + ".tmp")
    tmp_path.write_bytes(content)
    os.chmod(tmp_path, 0o600)
    os.replace(tmp_path, path)


def restore_snapshot(run_id, *, resnapshot_run_id):
    """Restore a snapshot; the current state is snapshotted first under
    ``resnapshot_run_id`` so the restore itself stays undoable."""
    snap_dir = snapshot_root() / run_id
    manifest = read_manifest(snap_dir)
    if manifest is None:
        raise ValueError("快照不存在或 manifest 损坏: %s" % run_id)

    current_files = [entry["path"] for entry in manifest["files"]]
    current_links = []
    for link in manifest["links"]:
        path = Path(link["path"])
        if path.is_symlink():
            try:
                current_links.append({"path": link["path"], "target": os.readlink(path)})
            except OSError:
                pass
    create_snapshot(resnapshot_run_id, "snapshots:restore(pre-%s)" % run_id, current_files, current_links)

    files_restored = 0
    files_deleted = 0
    for entry in manifest["files"]:
        path = Path(entry["path"])
        slot = _file_slot(snap_dir, entry["path"])
        if not entry["existed"]:
            # 快照时不存在的文件 = 该次运行新建的产物；回滚即删除。
            if path.is_symlink() or path.is_file():
                path.unlink()
                files_deleted += 1
            continue
        if not slot.is_file():
            continue
        _write_bytes_atomic(path, slot.read_bytes())
        files_restored += 1
    links_restored = 0
    skipped = []
    for link in manifest["links"]:
        path = Path(link["path"])
        if path.exists() and not path.is_symlink():
            skipped.append(link["path"])
            continue
        if path.is_symlink():
            path.unlink()
        path.parent.mkdir(parents=True, exist_ok=True)
        os.symlink(link["target"], path)
        links_restored += 1
    return {
        "restored_run": run_id,
        "files_restored": files_restored,
        "files_deleted": files_deleted,
        "links_restored": links_restored,
        "skipped": skipped,
        "resnapshot_run_id": resnapshot_run_id,
    }


def run_step(name, cmd, cwd, log_file, timeout=600):
    """Run one syncer step; output is appended to the per-run log file.

    Returns {"name", "ok", "exit_code"}. Never raises on step failure — the
    caller aggregates failures into envelope errors (manage.sh TASK_FAILURES
    precedent).
    """
    try:
        result = subprocess.run(
            cmd, cwd=str(cwd), capture_output=True, text=True, check=False, timeout=timeout
        )
        exit_code = result.returncode
        output = (result.stdout or "") + (result.stderr or "")
    except subprocess.TimeoutExpired:
        exit_code = 124
        output = "TIMEOUT after %ds: %s\n" % (timeout, cmd)
    except OSError as exc:
        exit_code = 127
        output = "无法执行步骤: %s\n" % exc
    if log_file:
        log_path = Path(log_file)
        log_path.parent.mkdir(parents=True, exist_ok=True)
        with open(log_path, "a", encoding="utf-8") as handle:
            handle.write("=== engine step: %s ===\n" % name)
            handle.write(output)
            if output and not output.endswith("\n"):
                handle.write("\n")
    return {"name": name, "ok": exit_code == 0, "exit_code": exit_code}
