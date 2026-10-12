# Current Architecture

The native Rust library reads and writes `native-manager.db`. CLI and the Tauri bridge call the same library; the React UI displays results and submits requests through the bridge.

## Source of truth

Actual assistant files and directories are authoritative for active configuration (see "Data model: the actual configuration is authoritative" in `GUI-REDESIGN.md`). `native-manager.db` stores app preferences, assistant recognition definitions, ownership records, and inactive alternatives; settings are in `app_settings`. There is no second writable JSON settings file or shared profile layer. Reads do not create a missing database; writes initialize schemas through SQLite transactions and record operation logs.

Built-in policy consists of read-only defaults compiled into Rust. `policy:get` reports only builtin and sqlite ownership. Writes use `--layer local`, while the persisted layer is named `sqlite`.

## Call boundaries

```text
CLI ───────┐
           ├── engine-rs (native / store / envelope) ── native-manager.db
Tauri ─────┘
React UI ── Tauri commands
```

The entry point is `execute(Context, Request) -> Envelope`. Tauri validates the command, arguments, and run id, then calls the shared library on a blocking thread.

The desktop is a foreground configuration tool. Closing the window exits; it has
no tray webview, quota/session collector, notification plugin, automatic updater
check, or OS-login enabling. Page queries and explicit actions drive refresh.
Account selectors use `agents:status --metadata-only`; Sources uses
`skills:status --collections-only` to defer per-skill hashing. Resource and buffer
limits are documented in [resource bounds](subsystems/resource-bounds.md).

## Saving and execution

Saving definitions updates only SQLite; it neither links resources nor performs external actions. `status` reads state without writes. Resource execution triggers real `apply`, including preflight/write validation, exact scope, an exclusive lock, external-change detection, CAS/conditional rollback, auditing, and an operation log.

Skills, MCP, and other resources and their per-assistant assignments support add, edit/update, enable, disable, and remove. The assistant UI manages accounts (separate configuration directory + launch command). Assistant installation, upgrades, uninstallation, and full definition editing belong to the original installation channel. The Programs page retains explicit dependency installation and upgrades. Legacy status-line configuration can be removed explicitly from a main account after ownership and baseline checks; the app cannot install that collector.

Public `*:plan` and `--dry-run` entry points are retired and rejected.

Legacy rule migration recognizes the original shared default by its content digest. Unmodified defaults create no saved template. When only an assistant-specific source was customized, migration combines it with the sanitized shared fallback. Custom shared sources and actual assistant files remain unchanged.

## Native MCP domain

MCP uses actual entries in assistant configuration files: top-level `mcpServers` in Claude `~/.claude.json`, `[mcp_servers.*]` in Codex `config.toml`, and other assistants/instances with `mcp_config`. Read-only `mcp:status` directly lists stdio and remote entries. Env/header references are preserved, literals masked, and unmodeled official fields preserved; every entry has a write baseline `base`.

`mcp:apply` performs `add`, `edit`, `enable`, `disable`, `remove`, `copy`, or `forget`, changing only one entry for one assistant per action. It re-reads and checks `--base` under the lock, refuses external changes, validates before writing, replaces atomically, reads back, and conditionally rolls back. Audit records only action and scope. To disable, Codex/OpenCode write official `enabled = false`; Claude Code removes the entry and stores a disabled snapshot (`mcp_saved`). Literals resembling credentials move to the user-selected external `.env` file, leaving only `${VAR}` in the snapshot. Variables resolve from that file, falling back to the process environment. SQLite stores only the external-file path and saved but inactive items; status, diagnostics, and logs neither expose nor store actual values. Old service-library definitions and assignment tables migrate on the first MCP write without changing configuration files. See the [MCP configuration guide](MCP-CONFIG-GUIDE.md).

## Default channels and status

Claude Code and Codex default to official `native` commands. Brew is shown only when detected as the actual source and does not alter defaults.

`system:audit` is connected to the Rust dispatcher and aggregates agents/skills/mcp/rules/aliases/secrets and runtime/storage read-only. `secrets:status` reports variable names, sources, and resolvability only. `secrets:source` validates and saves an external `.env` path: a regular file, Unix mode `600`, valid `KEY=VALUE`, outside the app data directory. `secrets:set` writes only that file; moving credential-like Claude MCP literals on disable does too, rejecting a same-name variable with a different value. Explicit `secrets:migrate-legacy` writes the external file first and removes the old `secret_values` table only after every write succeeds. Settings Configuration Tools opens client diagnostics, Aliases, and Secrets; native `aliases:status/apply` is connected. The 2026-09-12 decision retired Tools, snapshots (`snapshots:*`), and `system:converge`, removing their pages, Tauri allowlist entries, and contract schemas. `system:refresh-upstream` is connected: probes are cached in SQLite `upstream_probes` and agents/mcp/skills status returns freshness.

The following acceptance records describe earlier milestones; later platform evidence is in [platform status](GUI-PLATFORM-STATUS.md). SQLite core, the Tauri bridge, Settings, main resource layouts, native Agent/Skills/MCP/Rules/Aliases (actual-configuration model; `sources:get/set` retired), Runs SQLite `operation_logs`, and Python removal are complete. Agent and Skills apply use cross-process locks and structured audits. The built-in skill repository is only a recommendation until added as a user collection. Collections and Skill×Agent entries support update, enable/disable, and removal through snapshots and safe symlinks; status performs zero writes. SQLite `operation_logs` records history and prunes expired records when adding a record according to `log_retention_days` (0 disables pruning); `runs:clear` requires confirmation. The 2026-09-19 decision retired JSONL diagnostics and `logs:clear`; envelopes no longer carry `log_file`, while errors still pass through redaction. Since 2026-09-22, connection configuration, MCP, Rules, Hooks, Aliases, and instance-generated files use actual configuration: status returns actual content and `base`, and apply requires `--base`. External changes are rejected with refresh guidance; adoption no longer means overwriting files from the database (see the data-model section in [GUI redesign](GUI-REDESIGN.md)). Old DMG/GUI E2E results do not prove this round's direct-apply interaction. Source, browser, isolated CLI, and uncommitted dirty-DMG acceptance were completed; the final clean DMG was rebuilt after commit `a01d0f5` and independently verified by the lead reviewer ([delivery receipts](goal-rust-gui.md)). Signing, notarization, real arm64 execution, Linux, Windows, and update releases were not verified at that milestone. See the [contract](../contract/README.md) and [platform status](GUI-PLATFORM-STATUS.md).
