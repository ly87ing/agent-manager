# Quota and retired background presentation

The [lightweight goal](../goal-lightweight.md) retires quota monitoring. The first
slice removes tray residency, reminders, and closed-palette polling. The Accounts
quota cells and engine readers still exist until the following retirement slice.

## Responsibilities and sources of truth

Account status currently reads local quota records through
[agent_usage.rs](../../engine-rs/src/agent_usage.rs). Accounts still displays
those readings through [QuotaCells.tsx](../../app/src/components/QuotaCells.tsx).
There is no background quota publisher or notification sender in the active app.

[CommandPalette.tsx](../../app/src/components/CommandPalette.tsx) queries account
metadata only while open; resource search queries run only in full-search mode.
Navigation does not mount the global Inbox status collector. Skills and MCP derive
their attention items from their own already-requested status results.

## Runtime path and constraints

- One main webview is created. Closing it uses Tauri's normal exit behavior.
- No tray setup, exit interception, or notification plugin is registered.
- No automatic application update check runs; Settings retains manual updates.
- Actual configuration, sessions, credentials, and old database rows are preserved.
- Unreachable tray and reminder files are temporarily retained for a separate
  cleanup commit; their presence does not activate them.

## Verification

```sh
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app
pnpm test:unit run src/test/AppNavigation.test.tsx src/test/CommandPalette.test.tsx
```

Native acceptance must close a separately identified app using isolated HOME/data
and verify its process exits. Browser fixtures alone do not prove this lifecycle.
