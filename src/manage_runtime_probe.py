from __future__ import annotations

import os
import re
import shutil
import subprocess


def load_json(path):
    if not os.path.exists(path):
        return {}
    try:
        with open(path, "r", encoding="utf-8") as handle:
            import json

            return json.load(handle)
    except Exception:
        return {}


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

        brew_pkg = get_pkg_name(config.get("providers", {}).get("brew", {}).get("install", ""))
        if brew_pkg:
            if brew_pkg in self.get_brew_formulae():
                found.append("brew")
            elif brew_pkg in self.get_brew_casks():
                found.append("brew-cask")

        if bin_path and (
            "/node/" in bin_path
            or "/npm/" in bin_path
            or "/node_modules/" in bin_path
            or bin_path.endswith("node")
            or bin_path.endswith("npm")
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
