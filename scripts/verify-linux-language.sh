#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Linux && "${CI:-}" == true && "${RUNNER_ENVIRONMENT:-}" == github-hosted && "$(id -u)" != 0 ]]
language_root="$(mktemp -d "$RUNNER_TEMP/gnome-language.XXXXXX")"
trap 'rm -rf -- "$language_root"' EXIT
chmod 700 "$language_root"
env ROOTDIR="$language_root" HOME="$language_root/missing-home" \
  XDG_CONFIG_HOME="$language_root/settings" AGENT_MANAGER_DATA_DIR="$language_root/manager-data" \
  GSETTINGS_BACKEND=keyfile XDG_CURRENT_DESKTOP=GNOME AM_ISOLATED_LINUX_LANGUAGE=1 \
  dbus-run-session -- bash -c 'export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"; exec node scripts/verify-linux-language.mjs'
