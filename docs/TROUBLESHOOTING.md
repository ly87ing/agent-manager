# Troubleshooting

Use the current extension-only interface for MCP, skills, rules, dependencies,
saved launch configurations and environment profiles. Actual assistant files
remain authoritative. Agent Manager stores its preferences, ownership records,
inactive alternatives and operation history in `native-manager.db`.

## Database missing or reads empty

Open Settings and check **Data directory**. The desktop and CLI share database
selection: `AGENT_MANAGER_DATA_DIR` wins when nonempty; otherwise new installations
use the application-data root plus `io.github.ly87ing.agent-manager`. A lone legacy
`agent-manager/native-manager.db` remains in use. When both databases exist, the
current app directory wins and neither database is merged or deleted. See
[desktop startup](subsystems/desktop-startup.md).

From the repository root, read the selected directory without creating state:

```sh
cargo run --manifest-path engine-rs/Cargo.toml -- db:status --json
```

To inspect a different existing database, explicitly select its directory:

```sh
AGENT_MANAGER_DATA_DIR="YOUR_DATA_DIRECTORY" cargo run --manifest-path engine-rs/Cargo.toml -- db:status --json
```

An absent database is normal before the first save. Reads return defaults or
empty saved lists; saving initializes the database. Do not copy or merge live
SQLite files to resolve a display difference.

## Settings not saved

Save preferences on Settings, then reopen the page. Check its displayed data
directory and write permissions if the save fails. Language and log retention
live in `app_settings`; themes use the interface's existing local preference.
Login-entry state comes from the operating system and has its own reread action.

## Actual files changed outside Agent Manager

Use **Read again** on Skills, MCP, Rules or Sources and check the displayed read
time. This reads local state without applying configuration or checking upstream
versions. Close and reopen an editor after an external-change rejection, review
the fresh file contents and re-enter only the intended changes.

Writes compare the file baseline again under the write lock. Conflicting or
unmanaged content is preserved when a write is refused. Saving an inactive
alternative does not activate it. Read the failure details and Settings →
Activity log before retrying a partially completed operation.

## MCP connection test failed

Read the reported stage, command or address, working directory and original
sanitized diagnostic. A project-dependent server may need an explicit working
directory. Configure only referenced MCP variables in Settings → MCP secrets,
using the selected external private file. Check that the file is readable and,
on Unix, has mode `600`; the app does not create it or change its permissions.

The test runs a separate protocol session. A passing test does not establish
that an assistant has loaded the service or inherited the same environment.
See [MCP connection checks](subsystems/mcp-check.md).

## Environment checks or repair unavailable

System inspection and actions currently support macOS only. Other platforms can
still manage saved environment profiles. A capability/profile read failure keeps
inspection disabled until **Reload collections** succeeds.

Comparison is read-only, including legacy profiles with automatic-correction
flags. Choose **Repair automatically** for an explicit change preview, or **Apply
profile** for individual actions. Repair may change system time settings,
current-user language/region and future-process session variables after
confirmation and any required macOS authorization. Existing processes retain
their environment; verify the target application after relaunching it yourself.

Partial failure retains completed changes and stops later changes. Review the
individual results before retrying or restoring previous values. Unknown evidence
remains unknown. See [environment checks](subsystems/environment-checks.md).

## Installation and application updates

Use an installer from a successful [GUI CI run](https://github.com/ly87ing/agent-manager/actions/workflows/gui-ci.yml)
whose `build-info.json` matches the intended commit, and verify `SHA256SUMS.txt`.
The published October 8 preview predates the current extension-only behavior.
See [platform status](GUI-PLATFORM-STATUS.md) for installed-app and update-chain
acceptance. An absent update control means that build has no configured updater.

Account creation, authentication, provider/runtime management, assistant
installation/upgrades, shell-file alias management and quota collection are
retired. Do not use old account dialogs, status-line helpers or plaintext-secret
migration instructions as recovery steps. Prepare assistants with their own tools.

## Development verification

Prerequisites are Rust, Node.js and pnpm. From the repository root:

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
pnpm --dir app test:unit run
pnpm --dir app build
./scripts/ci-smoke.sh
```

The smoke script builds the CLI, then executes writes and readback only under
isolated HOME/data. Source and CI checks do not replace installed desktop,
privileged system-write or signed update acceptance.
