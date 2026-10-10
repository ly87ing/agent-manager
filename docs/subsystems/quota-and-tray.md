# Retired quota and background presentation

The [lightweight goal](../goal-lightweight.md) retires quota, cost, recent-session
collection, tray residency, and notifications. Accounts remains a configuration
and login dashboard; it does not collect or display quota.

## Responsibilities and sources of truth

[Native dispatch](../../engine-rs/src/native.rs) rejects every usage-domain command
before refreshing secrets, opening a database, or reading assistant files.
Account status returns a null legacy usage field. Existing databases, session
files, credentials, and deployed scripts are preserved.

The runtime writer rejects installation of the built-in quota status line. Its
existing explicit cleanup removes only a main-account statusLine pointing to this
tool's data-directory script, with the normal file baseline check. A custom
command is preserved. Seed and shared-setting copy refuse this retired script;
custom Claude and Codex status-line settings can still be copied explicitly.

[CommandPalette](../../app/src/components/CommandPalette.tsx) queries account
metadata only while open; resource queries run only in full-search mode. Skills
and MCP derive attention items from their own already-requested status results.

## Runtime path and constraints

- One main webview is created. Closing it uses normal Tauri exit behavior.
- No tray setup, notification plugin, or automatic app-update check runs.
- Settings retains manual updates and explicit cleanup of an owned old login entry.
- Retired collectors and presentation files are temporarily retained for a separate
  cleanup commit. Their presence does not activate them.
- No database migration or deletion of installed scripts is performed.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test usage --test agent_usage --test agent_settings
cargo test --manifest-path app/src-tauri/Cargo.toml
cd app
pnpm test:unit run src/test/AccountsPage.test.tsx src/test/SettingsPage.test.tsx src/test/AppNavigation.test.tsx
```

Native acceptance uses a separately identified app with isolated HOME/data.
Browser fixtures prove layout only.
