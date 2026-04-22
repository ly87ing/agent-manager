#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import os
import re
import shlex
import subprocess
import sys
import tarfile
import time
from datetime import datetime, timezone
from pathlib import Path
from urllib import error, request


SCRIPT_PATH = Path(__file__).resolve()
DEFAULT_REPO_ROOT = SCRIPT_PATH.parents[2]
if str(DEFAULT_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(DEFAULT_REPO_ROOT))

from src.template_utils import load_env_file, render_template


DEFAULT_RELEASE_API_URL = "https://api.github.com/repos/MamoWorks/cc-bridge/releases/latest"
DEFAULT_IMAGE_REPO = "ghcr.io/mamoworks/cc-bridge"
DEFAULT_MANAGED_IMAGE_REPO = "agent-manager/cc-bridge-us"
DEFAULT_RUNTIME_DIR = Path.home() / "services" / "cc-bridge"
CONTAINER_NAME = "cc-bridge"
REQUIRED_REGIONAL_ENV = {
    "TZ": "America/Los_Angeles",
    "LANG": "en_US.UTF-8",
    "LANGUAGE": "en_US:en",
    "LC_ALL": "en_US.UTF-8",
}
COMPOSE_TEMPLATE = """services:
  cc-bridge:
    image: "${CC_BRIDGE_IMAGE_REF}"
    container_name: cc-bridge
    env_file:
      - "${CC_BRIDGE_RUNTIME_DIR}/.env.rendered"
    ports:
      - "${SERVER_PORT}:5674"
    volumes:
      - "${CC_BRIDGE_RUNTIME_DIR}/data:/app/data"
    restart: unless-stopped
"""


def manager_root() -> Path:
    override = os.environ.get("AGENT_MANAGER_ROOT")
    if override:
        return Path(override).expanduser().resolve()
    return DEFAULT_REPO_ROOT


def cc_bridge_image_build_context() -> Path:
    return SCRIPT_PATH.parent / "image"


def parse_latest_release_tag(payload: dict) -> str:
    if payload.get("draft"):
        raise ValueError("latest release payload unexpectedly points to a draft release")
    if payload.get("prerelease"):
        raise ValueError("latest release payload unexpectedly points to a prerelease")
    tag_name = str(payload.get("tag_name", "")).strip()
    if not tag_name:
        raise ValueError("latest release payload is missing tag_name")
    return tag_name


def fetch_latest_release_tag(
    api_url: str | None = None,
    urlopen_func=request.urlopen,
) -> str:
    target_url = api_url or os.environ.get("CC_BRIDGE_RELEASE_API_URL", DEFAULT_RELEASE_API_URL)
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "agent-manager-cc-bridge",
    }
    github_token = os.environ.get("GITHUB_TOKEN")
    if github_token:
        headers["Authorization"] = f"Bearer {github_token}"
    with urlopen_func(request.Request(target_url, headers=headers), timeout=20) as response:
        payload = json.load(response)
    return parse_latest_release_tag(payload)


def validate_port(port_text: str) -> str:
    try:
        port = int(str(port_text).strip())
    except ValueError as exc:
        raise ValueError(f"SERVER_PORT must be an integer, got: {port_text!r}") from exc
    if port <= 0 or port > 65535:
        raise ValueError(f"SERVER_PORT must be between 1 and 65535, got: {port}")
    return str(port)


def load_runtime_config(root_dir: Path) -> dict:
    env_path = root_dir / "secrets" / "cc-bridge.env"
    if not env_path.exists():
        raise FileNotFoundError(f"missing required secrets file: {env_path}")

    raw_env = load_env_file(env_path)
    admin_password = raw_env.get("ADMIN_PASSWORD", "").strip()
    if not admin_password:
        raise ValueError("ADMIN_PASSWORD is required in secrets/cc-bridge.env")

    database_driver = raw_env.get("DATABASE_DRIVER", "sqlite").strip().lower()
    if database_driver != "sqlite":
        raise ValueError("DATABASE_DRIVER must be sqlite for the managed cc-bridge runtime")

    for key, required_value in REQUIRED_REGIONAL_ENV.items():
        configured_value = raw_env.get(key, "").strip()
        if configured_value and configured_value != required_value:
            raise ValueError(f"{key} must be {required_value} for the managed cc-bridge runtime")

    runtime_dir = Path(raw_env.get("CC_BRIDGE_RUNTIME_DIR", str(DEFAULT_RUNTIME_DIR))).expanduser()
    if not runtime_dir.is_absolute():
        runtime_dir = (root_dir / runtime_dir).resolve()
    image_repo = raw_env.get("CC_BRIDGE_IMAGE_REPO", DEFAULT_IMAGE_REPO).strip() or DEFAULT_IMAGE_REPO
    server_port = validate_port(raw_env.get("SERVER_PORT", "5674"))

    app_env = dict(raw_env)
    app_env.setdefault("SERVER_HOST", "0.0.0.0")
    app_env["SERVER_PORT"] = server_port
    app_env["DATABASE_DRIVER"] = "sqlite"
    app_env.setdefault("LOG_LEVEL", "info")
    for key, required_value in REQUIRED_REGIONAL_ENV.items():
        app_env[key] = required_value
    for key in list(app_env):
        if key.startswith("CC_BRIDGE_"):
            del app_env[key]

    return {
        "root_dir": root_dir,
        "runtime_dir": runtime_dir,
        "image_repo": image_repo,
        "server_port": server_port,
        "admin_password": admin_password,
        "app_env": app_env,
        "secrets_env_path": env_path,
    }


def normalize_managed_image_tag(tag: str) -> str:
    normalized = re.sub(r"[^0-9A-Za-z_.-]+", "-", str(tag).strip())
    normalized = normalized.strip(".-")
    if not normalized:
        raise ValueError(f"release tag cannot be converted into a Docker image tag: {tag!r}")
    return normalized


def build_upstream_image_ref(settings: dict, tag: str) -> str:
    return f"{settings['image_repo']}:{tag}"


def build_managed_image_ref(tag: str) -> str:
    return f"{DEFAULT_MANAGED_IMAGE_REPO}:{normalize_managed_image_tag(tag)}"


def managed_image_state_name(tag: str) -> str:
    return f"managed-image-{normalize_managed_image_tag(tag)}.fingerprint"


def render_compose_yaml(runtime_dir: Path, image_ref: str, server_port: str) -> str:
    return render_template(
        COMPOSE_TEMPLATE,
        {
            "CC_BRIDGE_RUNTIME_DIR": str(runtime_dir),
            "CC_BRIDGE_IMAGE_REF": image_ref,
            "SERVER_PORT": str(server_port),
        },
    )


def render_env_file(app_env: dict[str, str]) -> str:
    preferred_order = ["SERVER_HOST", "SERVER_PORT", "DATABASE_DRIVER", "LOG_LEVEL", "ADMIN_PASSWORD"]
    keys = [key for key in preferred_order if key in app_env]
    keys.extend(sorted(key for key in app_env if key not in preferred_order))
    lines = [f"{key}={json.dumps(str(app_env[key]), ensure_ascii=False)}" for key in keys]
    return "\n".join(lines) + "\n"


def ensure_runtime_dirs(runtime_dir: Path):
    runtime_dir.mkdir(parents=True, exist_ok=True)
    (runtime_dir / "data").mkdir(exist_ok=True)
    (runtime_dir / "backups").mkdir(exist_ok=True)
    (runtime_dir / "state").mkdir(exist_ok=True)


def write_text(path: Path, content: str):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def read_state_text(runtime_dir: Path, name: str) -> str:
    path = runtime_dir / "state" / name
    if not path.exists():
        return ""
    return path.read_text(encoding="utf-8").strip()


def write_state(runtime_dir: Path, current_tag: str, previous_tag: str, latest_tag: str, timestamp: str):
    state_dir = runtime_dir / "state"
    write_text(state_dir / "current-tag", f"{current_tag}\n")
    write_text(state_dir / "previous-tag", f"{previous_tag}\n")
    write_text(state_dir / "latest-known-tag", f"{latest_tag}\n")
    write_text(state_dir / "last-updated-at", f"{timestamp}\n")


def write_latest_known_tag(runtime_dir: Path, latest_tag: str):
    if latest_tag:
        write_text(runtime_dir / "state" / "latest-known-tag", f"{latest_tag}\n")


def render_runtime_files(settings: dict, tag: str):
    runtime_dir = settings["runtime_dir"]
    ensure_runtime_dirs(runtime_dir)
    image_ref = build_managed_image_ref(tag)
    write_text(runtime_dir / "compose.yaml", render_compose_yaml(runtime_dir, image_ref, settings["server_port"]))
    write_text(runtime_dir / ".env.rendered", render_env_file(settings["app_env"]))


def timestamp_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")


def backup_data_dir(runtime_dir: Path, timestamp: str) -> Path | None:
    data_dir = runtime_dir / "data"
    if not data_dir.exists():
        return None
    backups_dir = runtime_dir / "backups"
    backups_dir.mkdir(parents=True, exist_ok=True)
    backup_path = backups_dir / f"cc-bridge-data-{timestamp}.tar.gz"
    with tarfile.open(backup_path, "w:gz") as archive:
        archive.add(data_dir, arcname="data")
    return backup_path


def default_run_command(argv: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        argv,
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )


def _coerce_run_result(result) -> tuple[int, str]:
    if isinstance(result, int):
        return result, ""
    if isinstance(result, subprocess.CompletedProcess):
        output = (result.stdout or "") + (result.stderr or "")
        return result.returncode, output
    raise TypeError(f"unsupported command result type: {type(result)!r}")


def run_checked(argv: list[str], cwd: Path, run_command_func=default_run_command) -> str:
    result = run_command_func(argv, cwd)
    code, output = _coerce_run_result(result)
    if code != 0:
        raise RuntimeError(f"command failed ({code}): {' '.join(argv)}\n{output}".rstrip())
    return output.strip()


def docker_compose(runtime_dir: Path, args: list[str], run_command_func=default_run_command) -> str:
    docker_bin = os.environ.get("CC_BRIDGE_DOCKER_BIN", "docker")
    return run_checked([docker_bin, "compose", "-f", "compose.yaml", *args], runtime_dir, run_command_func)


def compute_managed_image_fingerprint(settings: dict, tag: str) -> str:
    context_dir = cc_bridge_image_build_context()
    dockerfile_path = context_dir / "Dockerfile"
    if not dockerfile_path.exists():
        raise FileNotFoundError(f"missing managed cc-bridge Dockerfile: {dockerfile_path}")
    hasher = hashlib.sha256()
    hasher.update(build_upstream_image_ref(settings, tag).encode("utf-8"))
    for path in sorted(path for path in context_dir.rglob("*") if path.is_file()):
        hasher.update(str(path.relative_to(context_dir)).encode("utf-8"))
        hasher.update(b"\0")
        hasher.update(path.read_bytes())
        hasher.update(b"\0")
    return hasher.hexdigest()


def docker_image_exists(image_ref: str, root_dir: Path, run_command_func=default_run_command) -> bool:
    docker_bin = os.environ.get("CC_BRIDGE_DOCKER_BIN", "docker")
    result = run_command_func([docker_bin, "image", "inspect", image_ref], root_dir)
    code, _output = _coerce_run_result(result)
    return code == 0


def prepare_runtime_image(settings: dict, tag: str, run_command_func=default_run_command) -> dict[str, object]:
    docker_bin = os.environ.get("CC_BRIDGE_DOCKER_BIN", "docker")
    root_dir = settings["root_dir"]
    runtime_dir = settings["runtime_dir"]
    context_dir = cc_bridge_image_build_context()
    dockerfile_path = context_dir / "Dockerfile"
    if not dockerfile_path.exists():
        raise FileNotFoundError(f"missing managed cc-bridge Dockerfile: {dockerfile_path}")
    upstream_image_ref = build_upstream_image_ref(settings, tag)
    managed_image_ref = build_managed_image_ref(tag)
    ensure_runtime_dirs(runtime_dir)
    fingerprint = compute_managed_image_fingerprint(settings, tag)
    fingerprint_path = runtime_dir / "state" / managed_image_state_name(tag)
    cached_fingerprint = fingerprint_path.read_text(encoding="utf-8").strip() if fingerprint_path.exists() else ""
    if cached_fingerprint == fingerprint and docker_image_exists(managed_image_ref, root_dir, run_command_func):
        return {"image_ref": managed_image_ref, "rebuilt": False}
    run_checked([docker_bin, "pull", upstream_image_ref], root_dir, run_command_func)
    run_checked(
        [
            docker_bin,
            "build",
            "--pull",
            "--build-arg",
            f"CC_BRIDGE_UPSTREAM_IMAGE={upstream_image_ref}",
            "-t",
            managed_image_ref,
            "-f",
            str(dockerfile_path),
            str(context_dir),
        ],
        root_dir,
        run_command_func,
    )
    write_text(fingerprint_path, f"{fingerprint}\n")
    return {"image_ref": managed_image_ref, "rebuilt": True}


def docker_inspect_container(runtime_dir: Path, run_command_func=default_run_command) -> dict:
    docker_bin = os.environ.get("CC_BRIDGE_DOCKER_BIN", "docker")
    try:
        raw = run_checked([docker_bin, "inspect", CONTAINER_NAME], runtime_dir, run_command_func)
    except Exception:
        return {"container_status": "not_created", "pid": 0}

    try:
        payload = json.loads(raw)
    except json.JSONDecodeError:
        return {"container_status": "unknown", "pid": 0}

    if not isinstance(payload, list) or not payload or not isinstance(payload[0], dict):
        return {"container_status": "unknown", "pid": 0}

    state = payload[0].get("State", {})
    status = str(state.get("Status", "unknown")).strip() or "unknown"
    pid_raw = state.get("Pid", 0)
    try:
        pid = int(pid_raw)
    except (TypeError, ValueError):
        pid = 0
    return {"container_status": status, "pid": pid}


def sync_secrets_repo(root_dir: Path, run_command_func=default_run_command):
    secrets_dir = root_dir / "secrets"
    env_path = secrets_dir / "cc-bridge.env"
    if (secrets_dir / ".git").exists():
        result = run_command_func(["git", "-C", str(secrets_dir), "pull", "--ff-only"], root_dir)
        code, output = _coerce_run_result(result)
        if code != 0 and env_path.exists():
            print(f"warning: failed to refresh secrets repo, using local secrets\n{output}".rstrip(), file=sys.stderr)
        elif code != 0:
            raise RuntimeError(f"failed to refresh secrets repo:\n{output}".rstrip())
        return
    if env_path.exists():
        return
    init_script = root_dir / "scripts" / "init-secrets.sh"
    if not init_script.exists():
        raise FileNotFoundError(f"secrets are missing and init script was not found: {init_script}")
    run_checked(["bash", str(init_script)], root_dir, run_command_func)
    if not env_path.exists():
        raise FileNotFoundError(f"secrets initialization finished without creating {env_path}")


def http_request_json(
    method: str,
    url: str,
    *,
    payload: dict | None = None,
    bearer_token: str | None = None,
    timeout: int = 10,
) -> dict | list:
    headers = {"Accept": "application/json"}
    body = None
    if bearer_token:
        headers["Authorization"] = f"Bearer {bearer_token}"
    if payload is not None:
        body = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request_obj = request.Request(url, method=method, headers=headers, data=body)
    try:
        with request.urlopen(request_obj, timeout=timeout) as response:
            raw = response.read().decode("utf-8")
    except error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"http {exc.code} for {method} {url}: {detail}".rstrip()) from exc
    except error.URLError as exc:
        raise RuntimeError(f"request failed for {method} {url}: {exc.reason}") from exc
    if not raw:
        return {}
    return json.loads(raw)


def http_request_ok(
    method: str,
    url: str,
    *,
    bearer_token: str | None = None,
    timeout: int = 10,
):
    headers = {}
    if bearer_token:
        headers["Authorization"] = f"Bearer {bearer_token}"
    request_obj = request.Request(url, method=method, headers=headers)
    try:
        with request.urlopen(request_obj, timeout=timeout):
            return
    except error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"http {exc.code} for {method} {url}: {detail}".rstrip()) from exc
    except error.URLError as exc:
        raise RuntimeError(f"request failed for {method} {url}: {exc.reason}") from exc


def perform_healthcheck(
    settings: dict,
    *,
    attempts: int = 10,
    delay_seconds: float = 1.0,
    sleep_func=time.sleep,
    request_ok_func=http_request_ok,
    request_json_func=http_request_json,
):
    base_url = f"http://127.0.0.1:{settings['server_port']}"
    last_error = None
    for attempt_index in range(max(attempts, 1)):
        try:
            request_ok_func("GET", f"{base_url}/login", timeout=10)
            request_json_func(
                "GET",
                f"{base_url}/admin/dashboard",
                bearer_token=settings["admin_password"],
                timeout=10,
            )
            return
        except (RuntimeError, http.client.HTTPException, OSError) as exc:
            last_error = exc
            if attempt_index + 1 >= max(attempts, 1):
                raise
            sleep_func(delay_seconds)
    if last_error is not None:
        raise last_error


def build_runtime_regional_validation_script() -> str:
    checks = []
    for key, required_value in REQUIRED_REGIONAL_ENV.items():
        expected = shlex.quote(required_value)
        checks.append(
            f'test "${{{key}:-}}" = {expected} || {{ echo "{key} must be {required_value}, got: ${{{key}:-<empty>}}" >&2; exit 1; }}'
        )
    checks.extend(
        [
            'test -f /usr/share/zoneinfo/America/Los_Angeles || { echo "missing zoneinfo file for America/Los_Angeles" >&2; exit 1; }',
            'tz_name="$(date +%Z 2>/dev/null || true)"',
            'case "$tz_name" in PST|PDT) ;; *) echo "timezone check failed: expected PST or PDT, got: ${tz_name:-<empty>}" >&2; exit 1 ;; esac',
            'tz_offset="$(date +%z 2>/dev/null || true)"',
            'case "$tz_offset" in -0700|-0800) ;; *) echo "timezone offset check failed: expected -0700 or -0800, got: ${tz_offset:-<empty>}" >&2; exit 1 ;; esac',
            'command -v locale >/dev/null 2>&1 || { echo "locale command is missing in the container" >&2; exit 1; }',
            'locale -a >/tmp/cc-bridge-locale-a.out 2>/tmp/cc-bridge-locale-a.err || { cat /tmp/cc-bridge-locale-a.err >&2; exit 1; }',
            'test ! -s /tmp/cc-bridge-locale-a.err || { cat /tmp/cc-bridge-locale-a.err >&2; exit 1; }',
            "tr 'A-Z' 'a-z' </tmp/cc-bridge-locale-a.out | grep -Eq '^en_us\\.utf-?8$' || { echo \"en_US.UTF-8 locale is not installed in the container\" >&2; exit 1; }",
            'locale >/tmp/cc-bridge-locale.out 2>/tmp/cc-bridge-locale.err || { cat /tmp/cc-bridge-locale.err >&2; exit 1; }',
            'test ! -s /tmp/cc-bridge-locale.err || { cat /tmp/cc-bridge-locale.err >&2; exit 1; }',
            "grep -Eq '^LANG=en_US\\.UTF-8$' /tmp/cc-bridge-locale.out || { cat /tmp/cc-bridge-locale.out >&2; exit 1; }",
            "grep -Eq '^LANGUAGE=en_US:en$' /tmp/cc-bridge-locale.out || { cat /tmp/cc-bridge-locale.out >&2; exit 1; }",
            "grep -Eq '^LC_ALL=en_US\\.UTF-8$' /tmp/cc-bridge-locale.out || { cat /tmp/cc-bridge-locale.out >&2; exit 1; }",
            'locale charmap >/tmp/cc-bridge-charmap.out 2>/tmp/cc-bridge-charmap.err || { cat /tmp/cc-bridge-charmap.err >&2; exit 1; }',
            'test ! -s /tmp/cc-bridge-charmap.err || { cat /tmp/cc-bridge-charmap.err >&2; exit 1; }',
            "grep -Eq '^UTF-8$' /tmp/cc-bridge-charmap.out || { cat /tmp/cc-bridge-charmap.out >&2; exit 1; }",
        ]
    )
    return "set -eu\n" + "\n".join(checks)


def verify_runtime_regional_settings(settings: dict, run_command_func=default_run_command):
    runtime_dir = settings["runtime_dir"]
    docker_bin = os.environ.get("CC_BRIDGE_DOCKER_BIN", "docker")
    validation_script = build_runtime_regional_validation_script()
    run_checked([docker_bin, "exec", CONTAINER_NAME, "sh", "-lc", validation_script], runtime_dir, run_command_func)


def _deploy_tag(
    settings: dict,
    *,
    target_tag: str,
    latest_tag: str,
    previous_tag: str,
    run_command_func=default_run_command,
    healthcheck_func=perform_healthcheck,
    runtime_validation_func=verify_runtime_regional_settings,
    now_func=timestamp_now,
    take_backup: bool = True,
):
    runtime_dir = settings["runtime_dir"]
    timestamp = now_func()
    render_runtime_files(settings, target_tag)
    docker_compose(runtime_dir, ["down"], run_command_func)
    backup_path = backup_data_dir(runtime_dir, timestamp) if take_backup else None
    docker_compose(runtime_dir, ["up", "-d"], run_command_func)
    healthcheck_func(settings)
    runtime_validation_func(settings, run_command_func=run_command_func)
    write_state(runtime_dir, target_tag, previous_tag, latest_tag, timestamp)
    return backup_path


def update_to_latest_release(
    *,
    root_dir: Path,
    fetch_tag_func=fetch_latest_release_tag,
    run_command_func=default_run_command,
    healthcheck_func=perform_healthcheck,
    runtime_validation_func=verify_runtime_regional_settings,
    now_func=timestamp_now,
    sync_secrets_func=sync_secrets_repo,
):
    sync_secrets_func(root_dir)
    settings = load_runtime_config(root_dir)
    runtime_dir = settings["runtime_dir"]
    current_tag = read_state_text(runtime_dir, "current-tag")
    previous_tag_before_update = read_state_text(runtime_dir, "previous-tag")
    latest_tag = fetch_tag_func()
    prepare_runtime_image(settings, latest_tag, run_command_func=run_command_func)
    try:
        backup_path = _deploy_tag(
            settings,
            target_tag=latest_tag,
            latest_tag=latest_tag,
            previous_tag=current_tag,
            run_command_func=run_command_func,
            healthcheck_func=healthcheck_func,
            runtime_validation_func=runtime_validation_func,
            now_func=now_func,
        )
    except Exception:
        if current_tag and current_tag != latest_tag:
            print(f"update failed, attempting automatic rollback to {current_tag}", file=sys.stderr)
            prepare_runtime_image(settings, current_tag, run_command_func=run_command_func)
            _deploy_tag(
                settings,
                target_tag=current_tag,
                latest_tag=latest_tag,
                previous_tag=previous_tag_before_update,
                run_command_func=run_command_func,
                healthcheck_func=healthcheck_func,
                runtime_validation_func=runtime_validation_func,
                now_func=now_func,
                take_backup=False,
            )
        raise
    return {
        "current_tag": latest_tag,
        "previous_tag": current_tag,
        "latest_tag": latest_tag,
        "backup_path": None if backup_path is None else str(backup_path),
        "runtime_dir": str(runtime_dir),
    }


def install_runtime(
    *,
    root_dir: Path,
    fetch_tag_func=fetch_latest_release_tag,
    run_command_func=default_run_command,
    healthcheck_func=perform_healthcheck,
    runtime_validation_func=verify_runtime_regional_settings,
    now_func=timestamp_now,
    sync_secrets_func=sync_secrets_repo,
):
    return update_to_latest_release(
        root_dir=root_dir,
        fetch_tag_func=fetch_tag_func,
        run_command_func=run_command_func,
        healthcheck_func=healthcheck_func,
        runtime_validation_func=runtime_validation_func,
        now_func=now_func,
        sync_secrets_func=sync_secrets_func,
    )


def start_runtime(
    *,
    root_dir: Path,
    fetch_tag_func=fetch_latest_release_tag,
    run_command_func=default_run_command,
    healthcheck_func=perform_healthcheck,
    runtime_validation_func=verify_runtime_regional_settings,
    now_func=timestamp_now,
    sync_secrets_func=sync_secrets_repo,
):
    sync_secrets_func(root_dir)
    settings = load_runtime_config(root_dir)
    runtime_dir = settings["runtime_dir"]
    current_tag = read_state_text(runtime_dir, "current-tag")
    previous_tag = read_state_text(runtime_dir, "previous-tag")
    latest_tag = fetch_tag_func()

    if not current_tag or not (runtime_dir / "compose.yaml").exists():
        prepare_runtime_image(settings, latest_tag, run_command_func=run_command_func)
        backup_path = _deploy_tag(
            settings,
            target_tag=latest_tag,
            latest_tag=latest_tag,
            previous_tag=current_tag,
            run_command_func=run_command_func,
            healthcheck_func=healthcheck_func,
            runtime_validation_func=runtime_validation_func,
            now_func=now_func,
        )
        return {
            "current_tag": latest_tag,
            "previous_tag": current_tag,
            "latest_tag": latest_tag,
            "backup_path": None if backup_path is None else str(backup_path),
            "runtime_dir": str(runtime_dir),
        }

    if current_tag != latest_tag:
        return update_to_latest_release(
            root_dir=root_dir,
            fetch_tag_func=lambda: latest_tag,
            run_command_func=run_command_func,
            healthcheck_func=healthcheck_func,
            runtime_validation_func=runtime_validation_func,
            now_func=now_func,
            sync_secrets_func=lambda root_dir: None,
        )

    image_prepare_result = prepare_runtime_image(settings, current_tag, run_command_func=run_command_func)
    render_runtime_files(settings, current_tag)
    write_latest_known_tag(runtime_dir, latest_tag)
    compose_args = ["up", "-d"]
    if image_prepare_result["rebuilt"]:
        compose_args.append("--force-recreate")
    docker_compose(runtime_dir, compose_args, run_command_func)
    healthcheck_func(settings)
    runtime_validation_func(settings, run_command_func=run_command_func)
    return {
        "current_tag": current_tag,
        "previous_tag": previous_tag,
        "latest_tag": latest_tag,
        "runtime_dir": str(runtime_dir),
    }


def stop_runtime(
    *,
    root_dir: Path,
    run_command_func=default_run_command,
    sync_secrets_func=sync_secrets_repo,
):
    sync_secrets_func(root_dir)
    settings = load_runtime_config(root_dir)
    runtime_dir = settings["runtime_dir"]
    if not (runtime_dir / "compose.yaml").exists():
        return {
            "runtime_dir": str(runtime_dir),
            "stopped": False,
            "reason": "compose.yaml not rendered yet",
        }
    docker_compose(runtime_dir, ["stop"], run_command_func)
    return {
        "runtime_dir": str(runtime_dir),
        "stopped": True,
    }


def rollback_to_previous_release(
    *,
    root_dir: Path,
    run_command_func=default_run_command,
    healthcheck_func=perform_healthcheck,
    runtime_validation_func=verify_runtime_regional_settings,
    now_func=timestamp_now,
    sync_secrets_func=sync_secrets_repo,
):
    sync_secrets_func(root_dir)
    settings = load_runtime_config(root_dir)
    runtime_dir = settings["runtime_dir"]
    current_tag = read_state_text(runtime_dir, "current-tag")
    previous_tag = read_state_text(runtime_dir, "previous-tag")
    latest_tag = read_state_text(runtime_dir, "latest-known-tag") or current_tag or previous_tag
    if not previous_tag:
        raise ValueError("no previous-tag state is available for rollback")
    prepare_runtime_image(settings, previous_tag, run_command_func=run_command_func)
    _deploy_tag(
        settings,
        target_tag=previous_tag,
        latest_tag=latest_tag,
        previous_tag=current_tag,
        run_command_func=run_command_func,
        healthcheck_func=healthcheck_func,
        runtime_validation_func=runtime_validation_func,
        now_func=now_func,
    )
    return {
        "current_tag": previous_tag,
        "previous_tag": current_tag,
        "latest_tag": latest_tag,
        "runtime_dir": str(runtime_dir),
    }


def read_seed_entries(path: Path) -> list[dict]:
    if not path.exists():
        return []
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, list):
        raise ValueError(f"{path} must contain a JSON array")
    for index, item in enumerate(payload):
        if not isinstance(item, dict):
            raise ValueError(f"{path} entry {index} must be a JSON object")
        if not str(item.get("name", "")).strip():
            raise ValueError(f"{path} entry {index} is missing a non-empty name")
    return payload


def seed_runtime(root_dir: Path):
    sync_secrets_repo(root_dir)
    settings = load_runtime_config(root_dir)
    perform_healthcheck(settings)
    runtime_dir = settings["runtime_dir"]
    base_url = f"http://127.0.0.1:{settings['server_port']}"
    accounts_seed = read_seed_entries(root_dir / "secrets" / "cc-bridge.accounts.json")
    tokens_seed = read_seed_entries(root_dir / "secrets" / "cc-bridge.api-tokens.json")

    existing_accounts = http_request_json(
        "GET",
        f"{base_url}/admin/accounts?page=1&page_size=1000",
        bearer_token=settings["admin_password"],
    )
    existing_account_names = {
        item.get("name", "")
        for item in existing_accounts.get("data", [])
        if isinstance(item, dict)
    }

    created_accounts = 0
    for account in accounts_seed:
        if account["name"] in existing_account_names:
            continue
        http_request_json(
            "POST",
            f"{base_url}/admin/accounts",
            payload=account,
            bearer_token=settings["admin_password"],
        )
        created_accounts += 1

    existing_tokens = http_request_json(
        "GET",
        f"{base_url}/admin/tokens?page=1&page_size=1000",
        bearer_token=settings["admin_password"],
    )
    existing_token_names = {
        item.get("name", "")
        for item in existing_tokens.get("data", [])
        if isinstance(item, dict)
    }

    created_tokens = 0
    for token in tokens_seed:
        if token["name"] in existing_token_names:
            continue
        http_request_json(
            "POST",
            f"{base_url}/admin/tokens",
            payload=token,
            bearer_token=settings["admin_password"],
        )
        created_tokens += 1

    return {
        "runtime_dir": str(runtime_dir),
        "created_accounts": created_accounts,
        "created_tokens": created_tokens,
    }


def build_claude_code_config(base_url: str) -> dict:
    placeholder_token = "<cc_bridge_api_token>"
    return {
        "settings_path": "~/.claude/settings.json",
        "base_url_env": "ANTHROPIC_BASE_URL",
        "auth_token_env": "ANTHROPIC_AUTH_TOKEN",
        "auth_token_placeholder": placeholder_token,
        "token_note": "Use a cc-bridge API token for Claude Code; the admin password only works for admin APIs.",
        "shell_exports": {
            "ANTHROPIC_BASE_URL": base_url,
            "ANTHROPIC_AUTH_TOKEN": placeholder_token,
        },
        "settings_json": {
            "env": {
                "ANTHROPIC_BASE_URL": base_url,
                "ANTHROPIC_AUTH_TOKEN": placeholder_token,
            }
        },
    }


def build_status_report(
    runtime_dir: Path,
    latest_tag: str,
    docker_ps_output: str,
    *,
    server_port: str,
    container_status: str,
    pid: int,
) -> dict:
    current_tag = read_state_text(runtime_dir, "current-tag")
    previous_tag = read_state_text(runtime_dir, "previous-tag")
    cleaned_status = docker_ps_output.strip()
    base_url = f"http://127.0.0.1:{server_port}"
    return {
        "current_tag": current_tag,
        "previous_tag": previous_tag,
        "latest_tag": latest_tag,
        "update_available": bool(current_tag and latest_tag and current_tag != latest_tag),
        "runtime_dir": str(runtime_dir),
        "container_name": CONTAINER_NAME,
        "container_status": container_status,
        "pid": pid,
        "port": str(server_port),
        "base_url": base_url,
        "login_url": f"{base_url}/login",
        "admin_url": f"{base_url}/admin/dashboard",
        "claude_code": build_claude_code_config(base_url),
        "docker_status": cleaned_status,
    }


def fetch_latest_tag_for_status(runtime_dir: Path) -> str:
    cached = read_state_text(runtime_dir, "latest-known-tag")
    try:
        return fetch_latest_release_tag()
    except Exception:
        return cached or "unknown"


def collect_status(
    root_dir: Path,
    run_command_func=default_run_command,
    fetch_latest_tag_func=fetch_latest_tag_for_status,
) -> dict:
    sync_secrets_repo(root_dir, run_command_func)
    settings = load_runtime_config(root_dir)
    runtime_dir = settings["runtime_dir"]
    container_state = {"container_status": "not_installed", "pid": 0}
    try:
        if not (runtime_dir / "compose.yaml").exists():
            docker_ps_output = "compose.yaml not rendered yet"
        else:
            docker_ps_output = docker_compose(runtime_dir, ["ps"], run_command_func)
            container_state = docker_inspect_container(runtime_dir, run_command_func)
    except Exception as exc:
        docker_ps_output = str(exc)
    latest_tag = fetch_latest_tag_func(runtime_dir)
    return build_status_report(
        runtime_dir,
        latest_tag,
        docker_ps_output,
        server_port=settings["server_port"],
        container_status=container_state["container_status"],
        pid=container_state["pid"],
    )


def print_status(report: dict, as_json: bool):
    if as_json:
        print(json.dumps(report, indent=2, ensure_ascii=False))
        return
    claude_code = report["claude_code"]
    print(f"current_tag: {report['current_tag'] or 'unknown'}")
    print(f"previous_tag: {report['previous_tag'] or 'unknown'}")
    print(f"latest_tag: {report['latest_tag'] or 'unknown'}")
    print(f"update_available: {'yes' if report['update_available'] else 'no'}")
    print(f"container_status: {report['container_status']}")
    print(f"pid: {report['pid'] or 'unknown'}")
    print(f"runtime_dir: {report['runtime_dir']}")
    print(f"port: {report['port']}")
    print(f"base_url: {report['base_url']}")
    print(f"login_url: {report['login_url']}")
    print(f"admin_url: {report['admin_url']}")
    print(f"claude_code_settings_path: {claude_code['settings_path']}")
    print(f"claude_code_note: {claude_code['token_note']}")
    print("claude_code_shell_exports:")
    print(f"export {claude_code['base_url_env']}={claude_code['shell_exports'][claude_code['base_url_env']]}")
    print(f"export {claude_code['auth_token_env']}={claude_code['shell_exports'][claude_code['auth_token_env']]}")
    print("claude_code_settings_json:")
    print(json.dumps(claude_code["settings_json"], indent=2, ensure_ascii=False))
    print("docker_status:")
    if report["docker_status"]:
        print(report["docker_status"])
    else:
        print("(empty)")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Manage a host-mounted cc-bridge runtime via Docker.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser("install", help="Install the latest stable release and start the runtime.")
    subparsers.add_parser("start", help="Start the runtime; upgrade first when a newer stable release exists.")
    subparsers.add_parser("stop", help="Stop the runtime without deleting data.")
    subparsers.add_parser("deploy", help="Fetch the latest stable release tag and deploy it.")
    subparsers.add_parser("update", help="Fetch the latest stable release tag and update the runtime.")
    subparsers.add_parser("seed", help="Seed accounts and gateway tokens from secrets files.")
    subparsers.add_parser("rollback", help="Rollback to the previously recorded release tag.")
    status_parser = subparsers.add_parser("status", help="Show current and latest release state.")
    status_parser.add_argument("--json", action="store_true", help="Emit machine-readable JSON.")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    root_dir = manager_root()
    try:
        if args.command in {"deploy", "install"}:
            result = install_runtime(root_dir=root_dir)
            print(json.dumps(result, indent=2, ensure_ascii=False))
            return 0
        if args.command == "start":
            result = start_runtime(root_dir=root_dir)
            print(json.dumps(result, indent=2, ensure_ascii=False))
            return 0
        if args.command == "stop":
            result = stop_runtime(root_dir=root_dir)
            print(json.dumps(result, indent=2, ensure_ascii=False))
            return 0
        if args.command == "update":
            result = update_to_latest_release(root_dir=root_dir)
            print(json.dumps(result, indent=2, ensure_ascii=False))
            return 0
        if args.command == "seed":
            result = seed_runtime(root_dir)
            print(json.dumps(result, indent=2, ensure_ascii=False))
            return 0
        if args.command == "rollback":
            result = rollback_to_previous_release(root_dir=root_dir)
            print(json.dumps(result, indent=2, ensure_ascii=False))
            return 0
        if args.command == "status":
            print_status(collect_status(root_dir), getattr(args, "json", False))
            return 0
        raise ValueError(f"unsupported command: {args.command}")
    except Exception as exc:
        print(str(exc), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
