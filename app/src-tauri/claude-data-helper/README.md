# Read-only Claude data inspector

The Tauri build hook copies the native `agent-manager-claude-data-helper` here
for macOS, Windows and Linux. It runs as the current user and accepts only the
restricted Orca search-inspection operation. It never initializes Manager's
database, loads credentials or elevates privileges. Source-file handles belong
to this process, so their closure cannot release the parent's POSIX locks.
The protocol returns bounded counts, never session IDs or content. Missing or
unsupported helpers leave inspection unavailable; there is no in-process fallback.

For direct desktop development, build it with
`node scripts/build-claude-data-helper.mjs --debug` from the repository root.
The standalone CLI uses the adjacent helper executable. See
[Claude local data](../../../docs/subsystems/claude-local-data.md).
