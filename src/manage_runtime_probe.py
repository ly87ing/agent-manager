from __future__ import annotations

import os
import re
import shutil
import subprocess

from src.json_config import load_json_file


def load_json(path, *, strict=False, missing_ok=True):
    return load_json_file(path, strict=strict, missing_ok=missing_ok)


def default_run_cmd(cmd, timeout=10):
    try:
        result = subprocess.run(
            cmd,
            check=False,
            capture_output=True,
            text=True,
            shell=False,
            timeout=timeout,
        )
        return result.returncode == 0, result.stdout
    except Exception:
        return False, ""


def get_pkg_name(cmd):
    if not cmd:
        return None
    match = re.search(r"(?:install|uninstall)(?:\s+--cask)?\s+([^\s@]+)", cmd)
    return match.group(1) if match else None


def normalize_path_for_match(path):
    if not path:
        return ""
    return path.replace("\\", "/")


def path_looks_brew_managed(path):
    normalized = normalize_path_for_match(path)
    return "/Cellar/" in normalized or "/Caskroom/" in normalized


def path_indicates_npm(path):
    normalized = normalize_path_for_match(path)
    return bool(
        normalized
        and (
            "/node/" in normalized
            or "/npm/" in normalized
            or "/node_modules/" in normalized
            or normalized.endswith("node")
            or normalized.endswith("npm")
            or normalized.endswith("npm.cmd")
        )
    )


def detect_npm_from_paths(bin_path, resolved_bin_path="", brew_detected=False):
    if path_indicates_npm(bin_path):
        return True
    if not resolved_bin_path:
        return False
    if brew_detected and path_looks_brew_managed(resolved_bin_path):
        return False
    return path_indicates_npm(resolved_bin_path)


def detect_tool_readiness(name, definition, repo_root):
    managed_repo_dir = os.path.join(repo_root, "agent-tool", name)
    if definition.get("url"):
        return os.path.isdir(managed_repo_dir) and os.path.isdir(os.path.join(managed_repo_dir, ".git"))
    return shutil.which(name) is not None


class ProviderInventory:
    def __init__(
        self,
        run_cmd=None,
        which=None,
        path_exists=None,
        realpath=None,
    ):
        self._run_cmd = run_cmd or default_run_cmd
        self._which = which or shutil.which
        self._path_exists = path_exists or os.path.exists
        self._realpath = realpath or os.path.realpath
        self._brew_formulae = None
        self._brew_casks = None
        self._gh_extensions = None

    def get_brew_formulae(self):
        if self._brew_formulae is None:
            self._brew_formulae = set()
            if self._which("brew"):
                success, out = self._run_cmd(["brew", "list", "--formula", "-1"], timeout=10)
                if success and out.strip():
                    self._brew_formulae = set(out.strip().split("\n"))
        return self._brew_formulae

    def get_brew_casks(self):
        if self._brew_casks is None:
            self._brew_casks = set()
            if self._which("brew"):
                success, out = self._run_cmd(["brew", "list", "--cask", "-1"], timeout=10)
                if success and out.strip():
                    self._brew_casks = set(out.strip().split("\n"))
        return self._brew_casks

    def get_gh_extensions(self):
        if self._gh_extensions is None:
            self._gh_extensions = ""
            if self._which("gh"):
                success, out = self._run_cmd(["gh", "extension", "list"], timeout=10)
                if success:
                    self._gh_extensions = out
        return self._gh_extensions

    def detect_all_providers(self, bin_name, config=None):
        found = []
        config = config or {}

        bin_path = self._which(bin_name)
        resolved_bin_path = self._realpath(bin_path) if bin_path else ""

        brew_pkg = get_pkg_name(config.get("providers", {}).get("brew", {}).get("install", ""))
        if brew_pkg:
            if brew_pkg in self.get_brew_formulae():
                found.append("brew")
            elif brew_pkg in self.get_brew_casks():
                found.append("brew-cask")

        if detect_npm_from_paths(
            bin_path,
            resolved_bin_path,
            brew_detected=any(provider.startswith("brew") for provider in found),
        ):
            found.append("npm")

        ext_name = config.get("ext_name")
        if ext_name and any(line.strip().startswith(ext_name) for line in self.get_gh_extensions().splitlines()):
            found.append("gh")

        app_path = config.get("app_path")
        if (app_path and self._path_exists(app_path)) or bin_path:
            if not found:
                found.append("system")

        return found


def detect_all_providers(bin_name, config=None, inventory=None):
    active_inventory = inventory or ProviderInventory()
    return active_inventory.detect_all_providers(bin_name, config)
