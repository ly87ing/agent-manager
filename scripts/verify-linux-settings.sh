#!/usr/bin/env bash
set -euo pipefail

[[ "$(uname -s)" == Linux && "${CI:-}" == true && "${RUNNER_ENVIRONMENT:-}" == github-hosted && "$(id -u)" != 0 ]]
settings_root="$(mktemp -d "$RUNNER_TEMP/kde-settings.XXXXXX")"
trap 'rm -rf -- "$settings_root"' EXIT
mkdir -p "$settings_root/home" "$settings_root/config" "$settings_root/cache" "$settings_root/data" "$settings_root/runtime"
chmod 700 "$settings_root" "$settings_root/runtime"

# Set activation inputs before creating the private bus, and remove its HOME
# only after dbus-run-session and xvfb-run have finished their own cleanup.
env HOME="$settings_root/home" USERPROFILE="$settings_root/home" \
  XDG_CONFIG_HOME="$settings_root/config" XDG_CACHE_HOME="$settings_root/cache" \
  XDG_DATA_HOME="$settings_root/data" XDG_RUNTIME_DIR="$settings_root/runtime" \
  AGENT_MANAGER_DATA_DIR="$settings_root/manager-data" XDG_CURRENT_DESKTOP=KDE \
  AM_ISOLATED_LINUX_SETTINGS=1 AM_LINUX_SETTINGS_ROOT="$settings_root" \
  LANG=C.UTF-8 LC_ALL=C.UTF-8 QT_QUICK_BACKEND=software QSG_RENDER_LOOP=basic \
  xvfb-run -a dbus-run-session -- node scripts/verify-linux-settings.mjs "${1:-engine-rs/target/debug/agent-manager-engine}"
