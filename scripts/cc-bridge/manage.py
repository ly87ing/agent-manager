#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tarfile
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
DEFAULT_RUNTIME_DIR = Path.home() / "services" / "cc-bridge"
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


def render_runtime_files(settings: dict, tag: str):
    runtime_dir = settings["runtime_dir"]
    ensure_runtime_dirs(runtime_dir)
    image_ref = f"{settings['image_repo']}:{tag}"
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


def perform_healthcheck(settings: dict):
    base_url = f"http://127.0.0.1:{settings['server_port']}"
    http_request_ok("GET", f"{base_url}/login", timeout=10)
    http_request_json(
        "GET",
        f"{base_url}/admin/dashboard",
        bearer_token=settings["admin_password"],
        timeout=10,
    )


def _deploy_tag(
    settings: dict,
    *,
    target_tag: str,
    latest_tag: str,
    previous_tag: str,
    run_command_func=default_run_command,
    healthcheck_func=perform_healthcheck,
    now_func=timestamp_now,
    take_backup: bool = True,
):
    runtime_dir = settings["runtime_dir"]
    timestamp = now_func()
    render_runtime_files(settings, target_tag)
    docker_compose(runtime_dir, ["down"], run_command_func)
    backup_path = backup_data_dir(runtime_dir, timestamp) if take_backup else None
    docker_compose(runtime_dir, ["pull"], run_command_func)
    docker_compose(runtime_dir, ["up", "-d"], run_command_func)
    healthcheck_func(settings)
    write_state(runtime_dir, target_tag, previous_tag, latest_tag, timestamp)
    return backup_path


def update_to_latest_release(
    *,
    root_dir: Path,
    fetch_tag_func=fetch_latest_release_tag,
    run_command_func=default_run_command,
    healthcheck_func=perform_healthcheck,
    now_func=timestamp_now,
    sync_secrets_func=sync_secrets_repo,
):
    sync_secrets_func(root_dir)
    settings = load_runtime_config(root_dir)
    runtime_dir = settings["runtime_dir"]
    current_tag = read_state_text(runtime_dir, "current-tag")
    previous_tag_before_update = read_state_text(runtime_dir, "previous-tag")
    latest_tag = fetch_tag_func()
    try:
        backup_path = _deploy_tag(
            settings,
            target_tag=latest_tag,
            latest_tag=latest_tag,
            previous_tag=current_tag,
            run_command_func=run_command_func,
            healthcheck_func=healthcheck_func,
            now_func=now_func,
        )
    except Exception:
        if current_tag and current_tag != latest_tag:
            print(f"update failed, attempting automatic rollback to {current_tag}", file=sys.stderr)
            _deploy_tag(
                settings,
                target_tag=current_tag,
                latest_tag=latest_tag,
                previous_tag=previous_tag_before_update,
                run_command_func=run_command_func,
                healthcheck_func=healthcheck_func,
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


def rollback_to_previous_release(
    *,
    root_dir: Path,
    run_command_func=default_run_command,
    healthcheck_func=perform_healthcheck,
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
    _deploy_tag(
        settings,
        target_tag=previous_tag,
        latest_tag=latest_tag,
        previous_tag=current_tag,
        run_command_func=run_command_func,
        healthcheck_func=healthcheck_func,
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


def build_status_report(runtime_dir: Path, latest_tag: str, docker_ps_output: str) -> dict:
    current_tag = read_state_text(runtime_dir, "current-tag")
    previous_tag = read_state_text(runtime_dir, "previous-tag")
    cleaned_status = docker_ps_output.strip()
    return {
        "current_tag": current_tag,
        "previous_tag": previous_tag,
        "latest_tag": latest_tag,
        "update_available": bool(current_tag and latest_tag and current_tag != latest_tag),
        "docker_status": cleaned_status,
    }


def fetch_latest_tag_for_status(runtime_dir: Path) -> str:
    cached = read_state_text(runtime_dir, "latest-known-tag")
    try:
        return fetch_latest_release_tag()
    except Exception:
        return cached or "unknown"


def collect_status(root_dir: Path, run_command_func=default_run_command) -> dict:
    sync_secrets_repo(root_dir, run_command_func)
    settings = load_runtime_config(root_dir)
    runtime_dir = settings["runtime_dir"]
    try:
        if not (runtime_dir / "compose.yaml").exists():
            docker_ps_output = "compose.yaml not rendered yet"
        else:
            docker_ps_output = docker_compose(runtime_dir, ["ps"], run_command_func)
    except Exception as exc:
        docker_ps_output = str(exc)
    latest_tag = fetch_latest_tag_for_status(runtime_dir)
    return build_status_report(runtime_dir, latest_tag, docker_ps_output)


def print_status(report: dict, as_json: bool):
    if as_json:
        print(json.dumps(report, indent=2, ensure_ascii=False))
        return
    print(f"current_tag: {report['current_tag'] or 'unknown'}")
    print(f"previous_tag: {report['previous_tag'] or 'unknown'}")
    print(f"latest_tag: {report['latest_tag'] or 'unknown'}")
    print(f"update_available: {'yes' if report['update_available'] else 'no'}")
    print("docker_status:")
    if report["docker_status"]:
        print(report["docker_status"])
    else:
        print("(empty)")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Manage a host-mounted cc-bridge runtime via Docker.")
    subparsers = parser.add_subparsers(dest="command", required=True)

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
        if args.command in {"deploy", "update"}:
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
