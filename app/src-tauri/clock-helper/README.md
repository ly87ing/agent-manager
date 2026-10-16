# Native clock helper

The Tauri build hook copies `agent-manager-clock-helper` (or `.exe`) here for
Windows and Linux bundles. Run `node scripts/build-clock-helper.mjs --debug`
from the repository root before directly running the desktop binary in development.
macOS uses its existing system SNTP authorization path.

This resource has no graphical dependency. It accepts only the restricted clock
operation, rejects malformed or expired requests before accessing state, and
measures the selected fixed NTP peer again after elevation. It never loads the
application database or assistant configuration. See
[Environment checks](../../../docs/subsystems/environment-checks.md).
