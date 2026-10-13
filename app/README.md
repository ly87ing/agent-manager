# Agent Manager Desktop App

This directory contains the Tauri 2 app. The React UI calls `engine-rs` through the bridge; CLI and desktop share one native Rust library. Runtime does not depend on Python, `manage.sh`, or source-tree resources.

Current scope: [the extension boundary](../docs/EXTENSIONS-ONLY.md). Skills, MCP,
Rules, Sources, dependency programs and read-only assistant targets remain. The
independent [Launch configurations](../docs/subsystems/launch-configurations.md)
tool is available under Settings for copying commands, confirmed shortcut writes,
and explicit graphical application launches. The app exits on
window close, loads resource queries on demand, and has no quota/session collector,
tray webview, notifications, automatic update check, or OS-login enabling.

The following records describe earlier implementation milestones. For the current product model, active configuration is authoritative as specified in [the multi-account design](../docs/MULTI-ACCOUNT-DESIGN.md) and [GUI redesign](../docs/GUI-REDESIGN.md).

`native-manager.db` is the source of truth for this tool's definitions, assignments, and preferences; settings are in `app_settings`. Queries and status perform zero writes; saving definitions writes only SQLite.

The built-in default channel for Claude Code and Codex is the official `native` command. Homebrew is shown only when detected as the actual source, never as the default.

Saving a definition neither installs nor links resources. Execute triggers real `apply`, which runs preflight validation before writing, then applies exact scope, an exclusive lock, external-change detection, CAS/conditional rollback, and auditing. Ordinary synchronization, enable/disable, and updates need no second confirmation. Removing a managed definition and uninstalling software are separate actions; destructive operations retain independent confirmation.

Agent, MCP, and Skills pages use the target interaction, and their lifecycles are connected to native Rust domains. Skills supports updates, enable/disable, and removal for collections and individual skills, using content snapshots and safe symlinks; status is read-only. SQLite core, the Tauri bridge, Settings, Runs history and cleanup (`log_retention_days` controls retention; Runs requires confirmation to clear), and Python removal are complete. Errors are redacted before leaving storage. JSONL diagnostic logs were retired by the 2026-09-19 decision. Native Rules status/apply and rule-specific `sources:get/set` were connected for the fixed Codex/Claude/OpenCode targets, with fail-closed external-change checks. Native Hooks status/apply and hook-specific `sources:get/set` (`hooks.claude/hooks.codex`) were connected for Claude `~/.claude/settings.json` signature-set merging and Codex `~/.codex/hooks.json` whole-file writes, also fail closed. The candidate at acceptance baseline `5a110be3` proves only the older plan→confirm interaction. Source, browser, isolated CLI, and uncommitted dirty-DMG acceptance for direct apply have been completed. The final clean DMG was rebuilt after final commit `a01d0f5` and independently verified by the lead reviewer; see [delivery receipts](../docs/goal-rust-gui.md).

Public `*:plan` and `--dry-run` entry points are retired and rejected.

The Rust dispatcher implemented read-only `mcp:status` and real `mcp:apply` for local stdio and remote HTTP MCP. In that milestone, definitions and assignments were SQLite tables in `native-manager.db`, supporting Claude JSON, Codex TOML, and OpenCode JSON; OpenCode requires an `mcp_config` target in its Agent definition. Per-assistant actions were `add`, `update`, `enable`, `disable`, and `remove`; library actions were `enable`, `disable`, and `remove`. Status performs zero writes. Apply has exclusive locking, audit, CAS, and rollback. SQLite definition/binding `env` values persist only literal `${VAR}` references; status, diagnostics, and logs never expose or store resolved values. Apply resolves them from the process environment and uses the target codec to write client configuration; file permissions and client security boundaries protect target-file plaintext. Remote definitions use `url` (only `https://`, or loopback `http://localhost` / `http://127.0.0.1`) and optional `headers` whose values must be `${VAR}` placeholders, mutually exclusive with stdio fields. Headers are written as references (Claude `${VAR}`, Codex `env_http_headers`, OpenCode `{env:VAR}`), never plaintext.

`system:audit` is connected to the Rust dispatcher and Tauri offline bridge, aggregating agents/skills/mcp/rules/hooks/aliases/secrets and runtime/storage read-only. `secrets:status` checks valid `${VAR}` references in enabled MCP definitions, resolves from the explicitly selected external `.env` file before falling back to the process environment, and returns only variable names, sources, and booleans. `secrets:source` stores only the external path; `secrets:set` writes only that file. `secrets:migrate-legacy` removes the old `secret_values` table only after all external writes succeed. Neither secret values nor external-file paths enter configuration backups; status, diagnostics, and operation records omit values. Malformed definitions fail with fixed, redacted messages. Advanced Settings exposes Aliases, the update center, and other pages; native `aliases:status/apply` is connected. The 2026-09-12 decision retired Tools, snapshots (`snapshots:*`), and `system:converge`, removing their pages, Tauri allowlist entries, and contract schemas. `system:refresh-upstream` is connected: probes are cached in SQLite `upstream_probes`, and freshness is returned by status for the three domains.

## Local development

```sh
pnpm install --frozen-lockfile
pnpm test:unit run
pnpm typecheck
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml --all-targets
```

Run CLI queries from the repository root. Test databases use a temporary `AGENT_MANAGER_DATA_DIR`, without overriding HOME.

### macOS packaging and standalone acceptance (Darwin x86_64 only)

```sh
pnpm run build:macos
# Outputs are in src-tauri/target/release/bundle/dmg/; use the verification script or npm script:
pnpm run verify:macos
# Or provide the DMG path explicitly:
../scripts/verify-macos-dmg.sh src-tauri/target/release/bundle/dmg/Agent\ Manager_0.1.0_x64.dmg
```

The build script sets `--remap-path-prefix` to strip the local build HOME and source-root paths. Verification mounts read-only, copies outside the repository, checks static and dynamic Python independence and redaction, then checks five-second survival, zero source references, and exit from `/` with a minimal PATH and isolated directories. A DMG was built and passed standalone acceptance on a real Darwin x86_64 machine on 2026-09-09. That package predates the current Rules/MCP/Hooks/Secrets and read-only `system:audit` implementation and cannot prove their GUI workflows. See the root README for newer package status (baseline `5a110be3` is evidence for the old interaction). Apple signing, notarization, real arm64 execution, Linux, Windows, and update releases were still unverified at this milestone; later status is recorded in [platform status](../docs/GUI-PLATFORM-STATUS.md).
