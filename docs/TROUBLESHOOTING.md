# Troubleshooting

This guide covers the current Rust/Tauri implementation. Real assistant files are authoritative for active configuration. Agent Manager preferences, ownership records, and inactive alternatives are in `native-manager.db`; settings are in `app_settings`. Python, `manage.sh`, three-layer JSON, and profile synchronization entry points are retired and are not troubleshooting steps.

## Check the development environment first

From the repository root:

```sh
cargo --version
node --version
pnpm --version
cargo check --manifest-path engine-rs/Cargo.toml --all-targets
cargo check --manifest-path app/src-tauri/Cargo.toml --all-targets
```

For frontend issues, run `pnpm typecheck`, `pnpm test:unit run`, or `pnpm build` in `app/`.

## Database missing or reads empty

Check in an isolated directory without overriding HOME:

```sh
export AGENT_MANAGER_DATA_DIR="$(mktemp -d)"
cargo run --manifest-path engine-rs/Cargo.toml -- db:status --json
cargo run --manifest-path engine-rs/Cargo.toml -- policy:get --json
```

With no database, `db:status` should report absence and `policy:get` should return Rust built-in defaults without creating a database. Saving initializes `native-manager.db`.

## Settings not saved

Save through the Tauri Settings page; the Rust bridge writes `app_settings`. Check the data directory and database write permissions, then reopen Settings or restart the app to read back. Queries and status perform zero writes.

## Unexpected policy source

Policy UI shows only `builtin` and `sqlite`. Edit requests use `--layer local`; persisted results show `sqlite`. The old shared/synchronization layers and JSON settings no longer participate in effective-value calculation.

## Action failed

Saving a definition does not install, uninstall, link, or synchronize. Investigate the engine action shown on the page:

1. Check the observed state returned by `status`.
2. Execute triggers real `apply` immediately. Internal preflight checks applicability, unmanaged content, and external changes, failing closed. Envelope errors identify the rejection category.
3. Read page error details and the operation log; do not reconstruct old commands manually.

Only destructive actions such as removing definitions or clearing logs require confirmation. Ordinary synchronization, enable/disable, and updates have no second confirmation.

Native Agent, Skills, MCP, Rules, Aliases, and instance domains are connected; Hooks was removed on 2026-09-29. Active configuration uses actual files. Read-only status returns actual content and write baseline `base`; writes require `--base`. If a file changed externally after opening, the app reports an external modification. Refresh and retry; adoption no longer means overwriting from the database. The engine neither offers nor executes actions when the actual installation source cannot be managed, the Agent is disabled, offline blocks the action, or it is inapplicable. Skills collections/assignments use snapshots and safe symlinks and refuse to overwrite unmanaged external content or conflicts. MCP supports local stdio and remote HTTP; `mcp:status` directly lists file entries, and `mcp:apply` edits, toggles, copies, and removes individual entries. Disabled Claude entries are saved snapshots with credential-like values moved externally and only `${VAR}` in storage; Codex/OpenCode use official `enabled = false`. Runs uses SQLite `operation_logs` for history and confirmed clearing; `log_retention_days` controls retention.

Public `*:plan` and `--dry-run` entry points are retired and rejected. Check `<domain>:status` before executing.

Historical acceptance: the candidate at `5a110be3` proves only the older plan→confirm interaction, not direct apply. Source, browser, isolated CLI, and uncommitted dirty-DMG acceptance for direct apply were completed. A final clean DMG was rebuilt after `a01d0f5` and independently verified by the lead reviewer; see [delivery receipts](goal-rust-gui.md) for SHA256 and results. The 2026-09-09 Darwin x86_64 DMG passed read-only mounting, copying outside the repository, Python independence, path redaction, and five-second isolated startup from `/`. It predates current Rules/MCP/Hooks/Secrets and read-only `system:audit` and cannot prove those GUI workflows. Signing, notarization, arm64, Linux, Windows, and update releases were still unverified at that milestone; see [platform status](GUI-PLATFORM-STATUS.md) for later evidence.

`system:audit` aggregates agents/skills/mcp/rules/aliases/secrets and runtime/storage read-only through the Rust dispatcher. Secrets resolve first from the external `.env` selected on Secrets, then the process environment; the database stores only the path. If the file is unavailable, check existence, regular-file status, UTF-8 `KEY=VALUE`, and Unix mode `600`. The app does not automatically create files, change permissions, or reveal values. Legacy database plaintext requires explicit migration with the confirmation word; failures retain the old table.

## Claude Code quota stuck several days in the past

Assistant cards read local files without network requests. Claude uses the newer of two sources:

| Source | Update time |
| --- | --- |
| `<data-directory>/usage/<encoded-config-directory>.json` | Every status-line render, event-driven with 300ms debounce |
| `cachedUsageUtilization` in `~/.claude.json` | Only when Claude Code actively checks quota; **ordinary conversation does not trigger it** |

With only the latter, numbers remain at the last check, even after days of use. This is expected, not a read error. The card reports how many days ago it updated and hides percentages after a window's reset time.

There are two ways to capture live values matching the Claude Code status line:

**Use this tool's status line:** select it in the assistant edit dialog; saving deploys it. It shows model, directory, branch, context, cost, and 5h/7d quota while recording quota. If you want to preserve your own status line, do not select this option: it redirects `statusLine` to this tool's script, and the first write to a populated field requires explicit adoption.

**Keep your own status line:** add quota recording to your script. Claude Code supplies `rate_limits` on stdin each render but writes no file for another process to read. Add this after parsing usage:

```bash
# $five / $week are used percentages; $five_reset / $week_reset are Unix reset seconds.
# $now is the current time; adjust variable names to match your own script.
# Write to Agent Manager's data directory (macOS default ~/Library/Application Support/agent-manager;
# Linux ${XDG_DATA_HOME:-~/.local/share}/agent-manager; AGENT_MANAGER_DATA_DIR overrides these).
# Encode the configuration-directory filename: % -> %25, then / -> %2F.
# Do not write into the assistant's configuration directory.
if [[ -n $five || -n $week ]]; then
  am_dir=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
  am_pct='%'
  am_key=${am_dir//$am_pct/%25}
  am_key=${am_key//\//%2F}
  am_usage="$HOME/Library/Application Support/agent-manager/usage"
  [[ -d $am_usage ]] || mkdir -p "$am_usage" 2>/dev/null
  am_file=$am_usage/$am_key.json
  am_tmp=$am_file.$$
  {
    printf '{"at":%s' "${now:-0}"
    [[ -n $five ]] && printf ',"five_hour":{"used_percentage":%s,"resets_at":%s}' "$five" "${five_reset:-null}"
    [[ -n $week ]] && printf ',"seven_day":{"used_percentage":%s,"resets_at":%s}' "$week" "${week_reset:-null}"
    printf '}\n'
  } > "$am_tmp" 2>/dev/null && mv -f "$am_tmp" "$am_file" 2>/dev/null || rm -f "$am_tmp" 2>/dev/null
fi
```

A temporary file followed by `mv` avoids partial reads across concurrent windows. Invalid content falls back to the cached source. The path follows `CLAUDE_CONFIG_DIR`, keeping instance records separate. `rate_limits` appears only for Pro/Max subscriptions, after the session's first API response.

Codex needs no setup: the server writes quota to session records on every response, so use refreshes it.

## Installation channels

Claude Code and Codex default to official `native` commands. Brew appears only as an actual detected source. If observed state differs from expectations, inspect local PATH and the actual installation source.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app && pnpm test:unit run && pnpm typecheck && pnpm build
./scripts/ci-smoke.sh
```
