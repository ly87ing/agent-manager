#!/usr/bin/env python3
import os
import argparse
import logging
import subprocess
import shutil
import json
import shlex
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.json_config import get_typed_mapping_section, load_json_object_file
from src.npm_flags import OFFICIAL_NPM_REGISTRY, inject_npm_flags
from src.version_pins import apply_local_version_pins
from src.command_runner import run_command
from src.git_remote_probe import probe_git_remote_state as probe_git_remote_repo_state
from src.manage_runtime_probe import (
    TOOL_BROKEN_MARKER,
    TOOL_READY_MARKER,
    check_tool_ready,
    detect_tool_readiness,
    repo_tool_artifacts_ready,
)
from src.runtime_logging import configure_logging

configure_logging()

OWNER_MARKER = ".agent-manager-owned"


def run_cmd(cmd, cwd=None, timeout=600):
    """Run a shell command and return the result.

    Bounded by default, unlike the previous timeout=None: git clone/pull, install,
    upgrade, uninstall and any setup without its own setup_timeout_sec all ran
    unbounded here, while sync_mcp and sync_agents have always defaulted to 300.
    600 is manage.sh's own supervisor ceiling for tools:apply, so nothing that
    currently succeeds starts failing — a hang that the supervisor would have
    killed anyway now ends as a clean per-command timeout instead.
    """
    cmd = inject_npm_flags(cmd)
    try:
        result = run_command(cmd, cwd=cwd, check=True, capture_output=True, text=True, timeout=timeout)
        return True, result.stdout
    except subprocess.TimeoutExpired:
        return False, f"TIMEOUT after {timeout} seconds: {cmd}"
    except subprocess.CalledProcessError as e:
        return False, f"Command '{cmd}' failed with exit code {e.returncode}: {e.stderr}"
    except FileNotFoundError as e:
        return False, f"Directory or executable not found: {e}"
    except Exception as e:
        return False, f"Unexpected error executing '{cmd}': {e}"


class GitStatusCache:
    """Cache git repository state to avoid repeating slow network requests."""
    _instance = None
    
    def __new__(cls):
        if cls._instance is None:
            cls._instance = super().__new__(cls)
            cls._instance._cache = {}
        return cls._instance
    
    def probe_git_remote_state(self, cwd, timeout=5):
        """Check git remote state (with caching and timeout)."""
        cwd_str = str(Path(cwd).resolve())
        if cwd_str in self._cache:
            return self._cache[cwd_str]

        result = probe_git_remote_repo_state(run_cmd, cwd, timeout=timeout)
        self._cache[cwd_str] = result
        return result

    def is_git_up_to_date(self, cwd, timeout=5):
        """Compatibility shim for legacy callers; returns only the boolean up-to-date state."""
        return self.probe_git_remote_state(cwd, timeout=timeout)["status"] == "latest"

# Instantiate the cache
git_cache = GitStatusCache()


def probe_git_remote_state(cwd, timeout=5):
    return git_cache.probe_git_remote_state(cwd, timeout=timeout)


def _extract_npm_package_name(command):
    if not command:
        return None
    try:
        tokens = shlex.split(command)
    except ValueError:
        tokens = command.split()
    if len(tokens) < 3 or tokens[0] != "npm" or tokens[1] != "install":
        return None
    for token in tokens[2:]:
        if token.startswith("-"):
            continue
        # A scoped name starts with "@", so the leading character is part of the
        # name, not a version separator. Splitting on the first "@" returns ""
        # for an unversioned scoped package, and "" is falsy — the npm probe
        # branch is then skipped, the state comes back "unknown", and the tool
        # silently stops updating. Only a second "@" delimits a version.
        if token.startswith("@"):
            return token.rsplit("@", 1)[0] if "@" in token[1:] else token
        if "@" in token:
            return token.split("@", 1)[0]
        return token
    return None


def _extract_uv_tool_name(command):
    if not command:
        return None
    try:
        tokens = shlex.split(command)
    except ValueError:
        tokens = command.split()
    if len(tokens) < 4 or tokens[:2] != ["uv", "tool"] or tokens[2] not in {"install", "upgrade"}:
        return None

    flags_with_value = {
        "-p",
        "--python",
        "--with",
        "--with-requirements",
        "-c",
        "--constraints",
        "--overrides",
        "-b",
        "--build-constraints",
        "-P",
        "--upgrade-package",
        "--resolution",
        "--prerelease",
        "--fork-strategy",
        "--exclude-newer",
        "--index",
        "--default-index",
        "-i",
        "--index-url",
        "--extra-index-url",
        "-f",
        "--find-links",
        "--index-strategy",
        "--keyring-provider",
        "--reinstall-package",
        "--link-mode",
        "-C",
        "--config-setting",
        "--no-build-isolation-package",
        "--no-build-package",
        "--no-binary-package",
        "--cache-dir",
    }
    index = 3
    while index < len(tokens):
        token = tokens[index]
        if token in flags_with_value:
            index += 2
            continue
        if token.startswith("-"):
            index += 1
            continue
        return token.split("[", 1)[0].split("=", 1)[0].strip()
    return None


def _parse_uv_tool_version(output, package_name):
    prefix = f"{package_name} "
    for line in output.splitlines():
        stripped = line.strip()
        if not stripped.startswith(prefix):
            continue
        parts = stripped.split()
        if len(parts) < 2:
            return None
        version = parts[1]
        return version[1:] if version.startswith("v") else version
    return None


def fetch_pypi_latest_version(package_name):
    request = urllib.request.Request(
        f"https://pypi.org/pypi/{package_name}/json",
        headers={"User-Agent": "agent-manager"},
    )
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            payload = json.loads(response.read().decode("utf-8"))
    except (urllib.error.URLError, TimeoutError, ValueError, KeyError):
        return None
    version = payload.get("info", {}).get("version")
    if not isinstance(version, str) or not version.strip():
        return None
    return version.strip()


def _build_brew_outdated_probe(command):
    if not command:
        return None
    try:
        tokens = shlex.split(command)
    except ValueError:
        tokens = command.split()
    if len(tokens) < 3 or tokens[0] != "brew" or tokens[1] not in {"install", "upgrade"}:
        return None
    cask = False
    for token in tokens[2:]:
        if token == "--cask":
            cask = True
            continue
        if token.startswith("-"):
            continue
        if cask:
            return ["brew", "outdated", "--cask", token]
        return ["brew", "outdated", token]
    return None


def _extract_git_pull_target(command):
    if not command:
        return None
    try:
        tokens = shlex.split(command)
    except ValueError:
        tokens = command.split()
    if len(tokens) < 4 or tokens[0] != "git":
        return None
    if tokens[1] != "-C":
        return None
    target_dir = tokens[2]
    action = tokens[3]
    if action != "pull":
        return None
    return Path(target_dir).expanduser()


def probe_package_update_state(name, definition):
    upgrade_cmd = definition.get("upgrade") or definition.get("install")
    npm_package = _extract_npm_package_name(upgrade_cmd)
    if npm_package:
        ok, out = run_cmd(["npm", "list", "-g", npm_package, "--depth=0", "--json"], timeout=60)
        if not ok or not out.strip():
            return "unknown"
        try:
            parsed = json.loads(out)
            local_version = parsed["dependencies"][npm_package]["version"]
        except (KeyError, TypeError, ValueError):
            return "unknown"
        ok, out = run_cmd(
            ["npm", "view", npm_package, "version", "--registry", OFFICIAL_NPM_REGISTRY],
            timeout=60,
        )
        if not ok:
            return "unknown"
        remote_version = out.strip()
        if not remote_version:
            return "unknown"
        return "latest" if local_version == remote_version else "stale"

    uv_tool = _extract_uv_tool_name(upgrade_cmd) or _extract_uv_tool_name(definition.get("install"))
    if uv_tool:
        ok, out = run_cmd(["uv", "tool", "list"], timeout=60)
        if not ok or not out.strip():
            return "unknown"
        local_version = _parse_uv_tool_version(out, uv_tool)
        if not local_version:
            return "unknown"
        remote_version = fetch_pypi_latest_version(uv_tool)
        if not remote_version:
            return "unknown"
        return "latest" if local_version == remote_version else "stale"

    brew_probe = _build_brew_outdated_probe(upgrade_cmd)
    if brew_probe:
        ok, out = run_cmd(brew_probe, timeout=60)
        if not ok:
            return "unknown"
        return "latest" if not out.strip() else "stale"

    git_pull_target = _extract_git_pull_target(upgrade_cmd)
    if git_pull_target:
        return probe_git_remote_state(git_pull_target, timeout=5)["status"]

    logging.info(f"[TOOL: {name}] 未识别的升级探测类型，回退为 unknown。")
    return "unknown"


def ready_marker_path(target_dir: Path) -> Path:
    return target_dir / TOOL_READY_MARKER


def broken_marker_path(target_dir: Path) -> Path:
    return target_dir / TOOL_BROKEN_MARKER


def mark_tool_ready(target_dir: Path):
    ready_marker_path(target_dir).write_text("ready\n", encoding="utf-8")
    broken_marker_path(target_dir).unlink(missing_ok=True)


def mark_tool_broken(target_dir: Path):
    broken_marker_path(target_dir).write_text("broken\n", encoding="utf-8")
    ready_marker_path(target_dir).unlink(missing_ok=True)


def clear_tool_markers(target_dir: Path):
    ready_marker_path(target_dir).unlink(missing_ok=True)
    broken_marker_path(target_dir).unlink(missing_ok=True)


def owner_marker_path(target_dir: Path) -> Path:
    return target_dir / OWNER_MARKER


def write_owner_marker(target_dir: Path, tool_name: str):
    owner_marker_path(target_dir).write_text(f"{tool_name}\n", encoding="utf-8")


def is_manager_owned_tool_dir(target_dir: Path) -> bool:
    return owner_marker_path(target_dir).is_file()


def apply_overrides(name, target_dir, dry_run=False):
    """Apply local patches to the target tool directory."""
    override_dir = Path(__file__).parent / "overrides" / name
    if not override_dir.exists(): return
    
    logging.info(f"应用 {name} 本地补丁 (Overrides)...")
    for root, _, files in os.walk(override_dir):
        for f in files:
            src = Path(root) / f
            rel_path = src.relative_to(override_dir)
            dst = target_dir / rel_path
            if dry_run:
                logging.info(f"  (预览: 计划覆盖 {rel_path})")
                continue
            dst.parent.mkdir(parents=True, exist_ok=True)
            if dst.is_symlink() or dst.is_file():
                dst.unlink()
            elif dst.is_dir():
                shutil.rmtree(dst)
            shutil.copy2(src, dst)
            logging.info(f"  已覆盖: {rel_path}")

def prune_orphan_tools(base_dir, managed_names, dry_run=False, force_yes=False):
    """Clean up tool directories that are no longer managed."""
    base_path = Path(base_dir).resolve()
    excluded = {"overrides", ".gitignore", "sync_tools.py", "__pycache__"}
    if not base_path.exists(): return

    for item in base_path.iterdir():
        if item.is_dir() and item.name not in managed_names and item.name not in excluded:
            if not is_manager_owned_tool_dir(item):
                logging.info(f"跳过非托管工具目录: {item.name}")
                continue
            logging.warning(f"检测到孤儿工具目录: {item.name}")
            if dry_run: continue
            
            should_delete = force_yes
            if not should_delete and sys.stdin.isatty():
                try:
                    confirm = input(f"  是否删除 {item.name} 以释放磁盘空间? (y/n): ")
                    should_delete = (confirm.lower() == 'y')
                except EOFError: should_delete = False
            
            if should_delete:
                # An orphan directory can be a full git clone with its own upstream
                # source, not just a marker file, and this runs after the tools were
                # already synced. An unhandled permission or lock error here would
                # abort the pass with a traceback and lose that work; report it and
                # keep going instead.
                try:
                    shutil.rmtree(item)
                except OSError as exc:
                    logging.error(f"  清理失败，保留目录: {item.name}: {exc}")
                    continue
                logging.info(f"  已成功清理: {item.name}")

def _check_ready(name: str, ready_check: str | None) -> bool:
    """Determine whether a tool is ready based on the ready_check field. Delegates to manage_runtime_probe.check_tool_ready."""
    return check_tool_ready(name, ready_check)


def _validate_setup_timeout(name: str, definition) -> int | None:
    timeout = definition.get("setup_timeout_sec")
    if timeout is None:
        return None
    if isinstance(timeout, bool) or not isinstance(timeout, int) or timeout <= 0:
        logging.error(f"[TOOL: {name}] setup_timeout_sec 必须是正整数秒。")
        return None
    return timeout


def _setup_timeout_label(setup_timeout_sec: int | None) -> str:
    if setup_timeout_sec is None:
        return ""
    return f" (超时 {setup_timeout_sec}s)"


def sync_tool(name, definition, policy, base_dir, dry_run=False, force=False, offline=False, force_refresh=False):
    # Merge policy
    enabled = policy.get("enabled", True)
    auto_update = policy.get("auto_update", True)
    
    if not enabled:
        uninstall_cmd = definition.get("uninstall")
        ready_check = definition.get("ready_check")
        if uninstall_cmd and _check_ready(name, ready_check):
            if dry_run:
                logging.info(f"[TOOL: {name}] (预览: 策略已禁用，计划卸载)")
            else:
                logging.info(f"[TOOL: {name}] 策略已禁用，执行卸载...")
                ok, out = run_cmd(uninstall_cmd)
                if not ok:
                    logging.error(f"[TOOL: {name}] 卸载失败: {out}")
                    return False
                logging.info(f"[TOOL: {name}] 已卸载")
        else:
            logging.info(f"[TOOL: {name}] (策略已禁用，跳过)")
        return True

    url = definition.get("url")
    install_cmd = definition.get("install")
    setup_cmd = definition.get("setup")
    setup_timeout_sec = _validate_setup_timeout(name, definition)
    if definition.get("setup_timeout_sec") is not None and setup_timeout_sec is None:
        return False
    target_dir = Path(base_dir) / name
    needs_setup = force
    setup_failed = False
    success_build = True
    should_apply_overrides = False
    repo_state_changed = False

    if url:
        if target_dir.exists() and (target_dir / ".git").exists():
            if not auto_update:
                if detect_tool_readiness(name, definition, base_dir):
                    logging.info(f"[TOOL: {name}] 已就绪，auto_update 关闭。")
                    should_apply_overrides = True
                else:
                    logging.warning(f"[TOOL: {name}] 检测到现有仓库未就绪，auto_update 关闭，保留当前状态。")
                    success_build = False
            else:
                if dry_run: logging.info(f"[TOOL: {name}] (预览: 计划检查更新)")
                else:
                    status_ok, status_out = run_cmd(["git", "status", "--porcelain"], cwd=target_dir, timeout=5)
                    if status_ok and status_out.strip():
                        logging.warning(f"[TOOL: {name}] 检测到本地未提交更改，跳过更新以保护工作区。")
                    else:
                        probe_state = probe_git_remote_state(target_dir, timeout=5)
                        if probe_state["status"] == "latest":
                            logging.info(f"[TOOL: {name}] (内容已最新)")
                            should_apply_overrides = True
                        else:
                            if probe_state["status"] == "unknown" and not force_refresh:
                                logging.warning(f"[TOOL: {name}] 更新探测失败，默认跳过拉取；如需强制刷新请使用 --force-refresh。")
                                should_apply_overrides = True
                            else:
                                if probe_state["status"] == "unknown":
                                    logging.warning(f"[TOOL: {name}] 更新探测失败，但已显式开启强制刷新，继续执行拉取。")
                                success, out = run_cmd(["git", "pull", "--ff-only"], cwd=target_dir)
                                if success:
                                    logging.info(f"[TOOL: {name}] (已更新)")
                                    needs_setup = True
                                    should_apply_overrides = True
                                    repo_state_changed = True
                                else: logging.error(f"[TOOL: {name}] (更新失败: {out})"); success_build = False
        else:
            if offline:
                if target_dir.exists():
                    logging.error(f"[TOOL: {name}] 离线模式且本地仓库缺失/损坏，无法重装。")
                else:
                    logging.error(f"[TOOL: {name}] 离线模式且本地仓库缺失，无法下载。")
                return False
            if target_dir.exists():
                logging.warning(f"[TOOL: {name}] 目录异常，正在重装...")
                if not dry_run: shutil.rmtree(target_dir)
            logging.info(f"正在安装 {name}...")
            if not dry_run:
                success, out = run_cmd(["git", "clone", url, str(target_dir)])
                if success:
                    logging.info(f"[TOOL: {name}] (安装成功)")
                    needs_setup = True
                    should_apply_overrides = True
                    repo_state_changed = True
                else: logging.error(f"[TOOL: {name}] (下载失败: {out})"); success_build = False
        
        if not dry_run and should_apply_overrides: apply_overrides(name, target_dir, dry_run)
        if setup_cmd and needs_setup and not dry_run:
            logging.info(f"执行 {name} 初始化{_setup_timeout_label(setup_timeout_sec)}...")
            ok, out = run_cmd(setup_cmd, cwd=target_dir, timeout=setup_timeout_sec)
            if not ok:
                logging.error(f"[TOOL: {name}] 构建失败: {out}")
                setup_failed = True
                success_build = False

        if not dry_run and target_dir.exists() and (target_dir / ".git").exists():
            write_owner_marker(target_dir, name)
            repo_ready = repo_tool_artifacts_ready(definition, target_dir)
            if setup_failed:
                mark_tool_broken(target_dir)
            elif success_build and repo_ready and (
                repo_state_changed or force or not broken_marker_path(target_dir).exists()
            ):
                mark_tool_ready(target_dir)
            elif (repo_state_changed or force) and not repo_ready:
                mark_tool_broken(target_dir)
            if not detect_tool_readiness(name, definition, base_dir):
                success_build = False

    elif install_cmd:
        ready_check = definition.get("ready_check")
        upgrade_cmd = definition.get("upgrade")
        setup_cmd = definition.get("setup")
        runtime_ready_checks = definition.get("runtime_ready_checks_any") or []
        runtime_ready_globs = definition.get("runtime_ready_globs_any") or []
        ready_paths = definition.get("ready_paths") or []
        binary_ready = _check_ready(name, ready_check)
        tool_ready = detect_tool_readiness(name, definition, base_dir)
        if offline:
            if dry_run:
                if not binary_ready:
                    logging.info(f"[TOOL: {name}] (预览: 离线模式会阻止安装)")
                elif setup_cmd and not tool_ready:
                    logging.info(f"[TOOL: {name}] (预览: 离线模式会阻止初始化)")
                else:
                    logging.info(f"[TOOL: {name}] (预览: 离线模式下保留当前状态)")
                return True
            if not binary_ready:
                logging.error(f"[TOOL: {name}] 离线模式且工具未安装，禁止安装。")
                return False
            if setup_cmd and not tool_ready:
                logging.error(f"[TOOL: {name}] 离线模式且运行时未就绪，禁止初始化。")
                return False
            logging.info(f"[TOOL: {name}] 离线模式，保留当前就绪状态。")
            return True
        needs_setup = force or (bool(setup_cmd) and not tool_ready)
        setup_failed = False

        def setup_required_after_binary_change():
            if not setup_cmd:
                return False
            if force:
                return True
            if runtime_ready_checks or runtime_ready_globs or ready_paths:
                return not detect_tool_readiness(name, definition, base_dir)
            return True

        if binary_ready:
            if auto_update and upgrade_cmd:
                if dry_run:
                    logging.info(f"[TOOL: {name}] (预览: 计划执行升级)")
                else:
                    package_state = "stale" if force else probe_package_update_state(name, definition)
                    if package_state == "latest":
                        logging.info(f"[TOOL: {name}] 已是最新版本，跳过升级。")
                    else:
                        if package_state == "unknown" and not force_refresh:
                            logging.warning(f"[TOOL: {name}] 更新探测失败，默认跳过升级；如需强制刷新请使用 --force-refresh。")
                        else:
                            if package_state == "unknown":
                                logging.warning(f"[TOOL: {name}] 更新探测失败，但已显式开启强制刷新，继续执行升级。")
                            logging.info(f"[TOOL: {name}] 已安装，执行升级...")
                            ok, out = run_cmd(upgrade_cmd)
                            if not ok:
                                logging.warning(f"[TOOL: {name}] 升级失败（保留现有版本）: {out}")
                            else:
                                binary_ready = _check_ready(name, ready_check)
                                needs_setup = setup_required_after_binary_change()
            elif tool_ready:
                logging.info(f"[TOOL: {name}] 已就绪，跳过安装。")
            else:
                logging.info(f"[TOOL: {name}] CLI 已安装但运行时未就绪，准备执行初始化。")
        else:
            if dry_run:
                logging.info(f"[TOOL: {name}] (预览: 计划执行安装)")
            else:
                logging.info(f"执行 {name} 全局安装...")
                ok, out = run_cmd(install_cmd)
                if not ok:
                    logging.error(f"[TOOL: {name}] 安装失败: {out}")
                    success_build = False
                else:
                    binary_ready = _check_ready(name, ready_check)
                    needs_setup = setup_required_after_binary_change()

        if setup_cmd and needs_setup:
            if dry_run:
                logging.info(f"[TOOL: {name}] (预览: 计划执行初始化)")
            elif success_build:
                target_dir.mkdir(parents=True, exist_ok=True)
                write_owner_marker(target_dir, name)
                clear_tool_markers(target_dir)
                logging.info(f"执行 {name} 初始化{_setup_timeout_label(setup_timeout_sec)}...")
                ok, out = run_cmd(setup_cmd, cwd=REPO_ROOT, timeout=setup_timeout_sec)
                if not ok:
                    logging.error(f"[TOOL: {name}] 初始化失败: {out}")
                    setup_failed = True
                    success_build = False
                    mark_tool_broken(target_dir)
                else:
                    mark_tool_ready(target_dir)

        if not dry_run and setup_cmd and (target_dir.exists() or setup_failed):
            target_dir.mkdir(parents=True, exist_ok=True)
            write_owner_marker(target_dir, name)
            if setup_failed:
                mark_tool_broken(target_dir)
            elif success_build and not detect_tool_readiness(name, definition, base_dir):
                mark_tool_broken(target_dir)
                success_build = False

    return success_build

def main():
    parser = argparse.ArgumentParser(description="Agent Tool 管理工具 (策略驱动)")
    parser.add_argument("--base-dir", default=str(Path(__file__).resolve().parent), help="基础目录")
    parser.add_argument("--config", default=str(REPO_ROOT / "config" / "manager.json"), help="策略路径")
    parser.add_argument("--def-file", default=str(REPO_ROOT / "config" / "tools.json"), help="定义路径")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    parser.add_argument("--force", action="store_true", help="强制初始化")
    parser.add_argument("-f", "--force-refresh", action="store_true", help="更新探测失败时仍强制执行远端刷新")
    parser.add_argument("--yes", action="store_true", help="自动清理")
    parser.add_argument("--offline", action="store_true", help="离线模式 (跳过网络操作)")
    args = parser.parse_args()

    try:
        policy_root = load_json_object_file(args.config, strict=True, missing_ok=False)
        def_root = load_json_object_file(args.def_file, strict=True, missing_ok=False)
        def_root = apply_local_version_pins(def_root, args.def_file)
        policy_cfg = get_typed_mapping_section(policy_root, "tools", Path(args.config).name, (dict,), "对象")
        def_cfg = get_typed_mapping_section(def_root, "tools", Path(args.def_file).name, (dict,), "对象")
    except ValueError as exc:
        logging.error(str(exc))
        sys.exit(1)

    base_dir = Path(args.base_dir).resolve()
    if not base_dir.exists() and not args.dry_run:
        base_dir.mkdir(parents=True, exist_ok=True)

    if args.offline:
        logging.info("离线模式：跳过 git 更新检查")
    
    logging.info("同步全局工具 (策略驱动)...")
    all_ok = True
    for name, definition in def_cfg.items():
        if name not in policy_cfg:
            logging.info(f"[TOOL: {name}] 未声明策略，跳过。")
            continue
        # Check whether prerequisite dependencies are ready
        requires = definition.get("requires", [])
        unmet = [
            req for req in requires
            if not _check_ready(req, (def_cfg.get(req) or {}).get("ready_check"))
        ]
        if unmet:
            logging.info(f"[TOOL: {name}] 前置依赖未就绪 {unmet}，跳过。")
            continue
        policy = policy_cfg.get(name, {})
        # Disable auto_update in offline mode
        if args.offline:
            policy = {**policy, "auto_update": False}
        if not sync_tool(
            name,
            definition,
            policy,
            base_dir,
            args.dry_run,
            args.force,
            args.offline,
            args.force_refresh,
        ):
            all_ok = False
    
    if not args.dry_run: prune_orphan_tools(base_dir, def_cfg.keys(), args.dry_run, args.yes)
    if not all_ok: sys.exit(1)
    logging.info("工具同步任务完成！")

if __name__ == "__main__":
    main()
