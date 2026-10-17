#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Linux && "${CI:-}" == true && "${RUNNER_ENVIRONMENT:-}" == github-hosted && "$(id -u)" != 0 ]]
language_service_user="$(systemctl show polkit.service --property=User --value)"
[[ -n "$language_service_user" && "$language_service_user" != root ]]
language_service_uid="$(id -u "$language_service_user")"
language_service_gid="$(id -g "$language_service_user")"
[[ "$language_service_uid" != 0 && "$language_service_gid" != 0 ]]
language_root="$(mktemp -d "$RUNNER_TEMP/gnome-language.XXXXXX")"
trap 'rm -rf -- "$language_root"' EXIT
chmod 700 "$language_root"
cat > "$language_root/bus.conf" <<EOF
<busconfig>
  <type>session</type>
  <listen>unix:abstract=am-$(basename "$language_root")</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <deny user="*"/>
    <allow user="$(id -u)"/>
    <allow user="0"/>
    <allow user="$language_service_uid"/>
    <allow own="*"/>
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
  </policy>
</busconfig>
EOF
env ROOTDIR="$language_root" HOME="$language_root/missing-home" \
  AM_LANGUAGE_SERVICE_UID="$language_service_uid" AM_LANGUAGE_SERVICE_GID="$language_service_gid" \
  XDG_CONFIG_HOME="$language_root/settings" AGENT_MANAGER_DATA_DIR="$language_root/manager-data" \
  GSETTINGS_BACKEND=keyfile XDG_CURRENT_DESKTOP=GNOME AM_ISOLATED_LINUX_LANGUAGE=1 \
  dbus-run-session --config-file="$language_root/bus.conf" -- bash -c 'export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"; exec node scripts/verify-linux-language.mjs'
