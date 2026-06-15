#!/usr/bin/env python3
import argparse
import copy
import json
import logging
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from src.json_config import atomic_write_json_file, get_typed_mapping_section, load_json_object_file
from src.runtime_logging import configure_logging

configure_logging()

GITHUB_REPO_PATTERNS = (
    re.compile(r"^https://github\.com/(?P<owner>[^/]+)/(?P<repo>[^/]+?)(?:\.git)?/?$"),
    re.compile(r"^git@github\.com:(?P<owner>[^/]+)/(?P<repo>[^/]+?)(?:\.git)?$"),
    re.compile(r"^ssh://git@github\.com/(?P<owner>[^/]+)/(?P<repo>[^/]+?)(?:\.git)?/?$"),
)


class SkillUpgradeError(ValueError):
    pass


def run_cmd(cmd, cwd=None, timeout=300):
    try:
        result = subprocess.run(
            cmd,
            cwd=cwd,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
        )
        return True, result.stdout
    except subprocess.TimeoutExpired:
        return False, f"TIMEOUT after {timeout} seconds: {cmd}"
    except subprocess.CalledProcessError as exc:
        return False, exc.stderr or exc.stdout
    except FileNotFoundError as exc:
        return False, f"Directory or executable not found: {exc}"
    except Exception as exc:
        return False, f"Unexpected error executing cmd: {exc}"


def is_local_repository_url(url):
    return Path(url).expanduser().exists()


def parse_github_repo(url):
    for pattern in GITHUB_REPO_PATTERNS:
        match = pattern.match(url)
        if match:
            return match.group("owner"), match.group("repo")
    return None


def resolve_remote_revision_to_commit(url, revision, offline=False):
    local_repo_path = Path(url).expanduser()
    if local_repo_path.exists():
        ok, out = run_cmd(
            ["git", "rev-parse", "--verify", f"{revision}^{{commit}}"],
            cwd=local_repo_path,
        )
        if ok:
            return out.strip()
        raise SkillUpgradeError(f"无法解析 revision {revision}: {out}")

    if offline:
        raise SkillUpgradeError("离线模式下无法解析远程 revision。")

    refs_to_query = [f"{revision}^{{}}", revision]
    ok, out = run_cmd(["git", "ls-remote", url, *refs_to_query])
    if not ok:
        raise SkillUpgradeError(f"解析远程 revision {revision} 失败: {out}")

    preferred_commit = None
    fallback_commit = None
    for line in out.splitlines():
        parts = line.split("\t", 1)
        if len(parts) != 2:
            continue
        commit, ref_name = parts
        if ref_name.endswith("^{}"):
            preferred_commit = commit
        else:
            fallback_commit = commit
    if preferred_commit:
        return preferred_commit
    if fallback_commit:
        return fallback_commit
    raise SkillUpgradeError(f"远程 revision 不存在: {revision}")


def list_remote_tags(url, offline=False):
    if offline and not is_local_repository_url(url):
        raise SkillUpgradeError("离线模式下无法查询远程 tag。")

    ok, out = run_cmd(["git", "ls-remote", "--tags", "--refs", "--sort=-version:refname", url])
    if not ok:
        raise SkillUpgradeError(f"查询远程 tag 失败: {out}")

    tags = []
    for line in out.splitlines():
        parts = line.split("\t", 1)
        if len(parts) != 2:
            continue
        ref_name = parts[1]
        if ref_name.startswith("refs/tags/"):
            tags.append(ref_name.removeprefix("refs/tags/"))
    return tags


def fetch_latest_github_release_tag(url, offline=False):
    if offline:
        raise SkillUpgradeError("离线模式下无法查询 GitHub release。")

    repo_info = parse_github_repo(url)
    if repo_info is None:
        raise SkillUpgradeError("latest-release 仅支持 GitHub 仓库。")

    owner, repo = repo_info
    api_url = f"https://api.github.com/repos/{owner}/{repo}/releases/latest"
    request = urllib.request.Request(
        api_url,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "agent-manager",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            payload = json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            raise SkillUpgradeError("未发现可用 release。") from exc
        raise SkillUpgradeError(f"查询 GitHub release 失败: HTTP {exc.code}") from exc
    except urllib.error.URLError as exc:
        raise SkillUpgradeError(f"查询 GitHub release 失败: {exc.reason}") from exc

    tag_name = payload.get("tag_name")
    if not isinstance(tag_name, str) or not tag_name.strip():
        raise SkillUpgradeError("GitHub release 缺少 tag_name。")
    return tag_name.strip()


def resolve_latest_commit_for_repository(repo_config, offline=False):
    url = str(repo_config.get("url", "")).strip()
    if not url:
        raise SkillUpgradeError("缺少仓库 url。")

    strategy = repo_config.get("upgrade_strategy")
    if strategy == "latest-branch":
        upgrade_ref = str(repo_config.get("upgrade_ref", "HEAD")).strip()
        if not upgrade_ref:
            raise SkillUpgradeError("latest-branch 缺少 upgrade_ref。")
        if upgrade_ref == "HEAD" or upgrade_ref.startswith("refs/"):
            revision = upgrade_ref
        else:
            revision = f"refs/heads/{upgrade_ref}"
        return resolve_remote_revision_to_commit(url, revision, offline=offline)
    if strategy == "latest-tag":
        tags = list_remote_tags(url, offline=offline)
        if not tags:
            raise SkillUpgradeError("未发现可用 tag。")
        return resolve_remote_revision_to_commit(url, f"refs/tags/{tags[0]}", offline=offline)
    if strategy == "latest-release":
        tag_name = fetch_latest_github_release_tag(url, offline=offline)
        return resolve_remote_revision_to_commit(url, f"refs/tags/{tag_name}", offline=offline)
    raise SkillUpgradeError(f"不支持的 upgrade_strategy: {strategy}")


def collect_target_repositories(policy_cfg, repositories, requested_repos):
    if requested_repos:
        missing = [name for name in requested_repos if name not in repositories]
        if missing:
            raise SkillUpgradeError(f"未定义的 skill 仓库: {', '.join(missing)}")
        return requested_repos, []

    targets = []
    skipped_missing_strategy = []
    for repo_name, repo_config in repositories.items():
        policy = policy_cfg.get(repo_name)
        if policy is None or not policy.get("enabled", True):
            continue
        if repo_config.get("upgrade_strategy"):
            targets.append(repo_name)
            continue
        skipped_missing_strategy.append(repo_name)
    return targets, skipped_missing_strategy


def upgrade_repositories(config_path, def_path, requested_repos=None, dry_run=False, offline=False):
    policy_root = load_json_object_file(config_path, strict=True, missing_ok=False)
    defs_root = load_json_object_file(def_path, strict=True, missing_ok=False)
    policy_cfg = get_typed_mapping_section(policy_root, "skills", Path(config_path).name, (dict,), "对象")
    repositories = get_typed_mapping_section(defs_root, "repositories", Path(def_path).name, (dict,), "对象")

    targets, skipped_missing_strategy = collect_target_repositories(policy_cfg, repositories, requested_repos or [])
    if skipped_missing_strategy:
        logging.info(
            "以下已启用 skill 仓库未配置 upgrade_strategy，跳过显式升级: "
            + ", ".join(skipped_missing_strategy)
        )
        for repo_name in skipped_missing_strategy:
            logging.debug(f"[{repo_name}] 已启用，但未配置 upgrade_strategy。")
    if not targets:
        if skipped_missing_strategy:
            logging.info("当前策略下没有可升级 skill 仓库；已启用仓库均未配置 upgrade_strategy。")
        else:
            logging.info("当前策略未配置任何可升级 skill 仓库。")
        return 0

    updated_defs = copy.deepcopy(defs_root)
    changed = False
    errors = []

    for repo_name in targets:
        repo_config = updated_defs["repositories"][repo_name]
        if not repo_config.get("upgrade_strategy"):
            errors.append(f"[{repo_name}] 缺少 upgrade_strategy。")
            continue
        try:
            latest_commit = resolve_latest_commit_for_repository(repo_config, offline=offline)
        except SkillUpgradeError as exc:
            errors.append(f"[{repo_name}] {exc}")
            continue

        current_commit = str(repo_config.get("commit", "")).strip()
        if current_commit == latest_commit:
            logging.info(f"[{repo_name}] 已是最新固定版本，无需更新。")
            continue

        repo_config["commit"] = latest_commit
        repo_config.pop("ref", None)
        changed = True
        if dry_run:
            logging.info(f"[{repo_name}] (预览: 计划更新 commit -> {latest_commit})")
        else:
            logging.info(f"[{repo_name}] 已解析最新版本 commit -> {latest_commit}")

    if errors:
        for message in errors:
            logging.error(message)
        logging.error("存在升级失败项，配置未写回。")
        return 1

    if not changed:
        logging.info("没有需要写回的 skill 版本变更。")
        return 0

    if dry_run:
        logging.info("预览模式：已跳过 skills.json 写回。")
        return 0

    atomic_write_json_file(def_path, updated_defs)
    logging.info("skills.json 已原子更新。")
    return 0


def main():
    parser = argparse.ArgumentParser(description="Agent Skill 版本升级工具")
    parser.add_argument("--config", default=str(REPO_ROOT / "config" / "manager.json"), help="策略配置")
    parser.add_argument("--def-file", default=str(REPO_ROOT / "config" / "skills.json"), help="定义文件")
    parser.add_argument("--repo", action="append", dest="repos", help="只升级指定仓库，可重复")
    parser.add_argument("--dry-run", action="store_true", help="预览")
    parser.add_argument("--offline", action="store_true", help="离线模式")
    args = parser.parse_args()

    try:
        sys.exit(
            upgrade_repositories(
                args.config,
                args.def_file,
                requested_repos=args.repos,
                dry_run=args.dry_run,
                offline=args.offline,
            )
        )
    except ValueError as exc:
        logging.error(str(exc))
        sys.exit(1)


if __name__ == "__main__":
    main()
