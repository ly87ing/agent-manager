# Desktop Startup

## Responsibilities and sources of truth

The current executable's location determines whether macOS needs an installation warning. [`startup.rs`](../../app/src-tauri/src/startup.rs) reads the filesystem's read-only flag and checks for an `AppTranslocation` path component. A writable external volume alone does not trigger a warning. This is runtime information, never a saved preference.

[`autostart.rs`](../../app/src-tauri/src/autostart.rs) owns the OS login entry. Its state comes from that entry, not SQLite or local storage. macOS uses `~/Library/LaunchAgents/<identifier>.plist`; Linux uses `$XDG_CONFIG_HOME/autostart/<identifier>.desktop` (defaulting to `~/.config`); Windows uses the application's named value under current-user `Run` and reads `StartupApproved/Run` for Task Manager's override.

## Runtime path

[`settings_data`](../../app/src-tauri/src/settings.rs) includes an optional `runtimeWarning`. [`App`](../../app/src/App.tsx) displays it above the current page. The [automatic update hook](../../app/src/lib/update-check.ts) and [Settings page](../../app/src/pages/SettingsPage.tsx) suppress update checks and installation while the warning is present.

Settings reads `autostart_get` on opening. Only an explicit switch click calls `autostart_set`; the backend reads back the result before returning, and the [frontend query](../../app/src/lib/autostart.ts) refreshes even after a failed write. macOS uses a plist serializer and `AbandonProcessGroup` so an updater's replacement process can survive its launchd parent's exit. Windows quotes the executable path; Linux uses Desktop Entry quoting rather than a shell command.

## Constraints and failure behavior

- Detection only reads the executable path and mount flags; it does not move the app or write configuration.
- The message asks the user to quit, drag the app into Applications and reopen it.
- Other platforms omit the macOS-specific warning.
- A launch warning blocks enabling autostart in both the UI and backend; disabling remains possible.
- Reading login state creates no directory or file. File entries must be regular files with exactly the expected contents; registry values must point to this executable. External changes and symlinks are refused, including before deletion. Errors remain visible with a reread action.
- macOS/Linux registration uses the current executable (the original AppImage on Linux), and no software or assistant resource is installed. Moving the app can leave an old entry that must be reviewed manually.
- Linux refuses executable paths with `%`, `=`, control characters or invalid UTF-8. GIO validates the executable before expanding `%%`, and the specification forbids `=` in executable names; do not report registration success for an unusable path. See the [Desktop Entry specification](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html) and [GIO loader](https://github.com/GNOME/glib/blob/main/gio/gdesktopappinfo.c).
- OS policy may still prevent a login launch. A registered entry does not prove a physical reboot/login succeeded.

## Verification

```sh
cargo test --manifest-path app/src-tauri/Cargo.toml startup::tests
cargo test --manifest-path app/src-tauri/Cargo.toml autostart::tests
cargo check --manifest-path app/src-tauri/Cargo.toml --target x86_64-pc-windows-gnu --all-targets
cd app
pnpm test:unit run src/test/SettingsPage.test.tsx src/test/UpdateBadge.test.tsx --maxWorkers=2
```

Tests distinguish real read-only system and writable development mounts, translocated paths, and writable external-volume paths. Frontend tests cover the banner and both update-check paths. Browser layout acceptance uses mock settings at 1000 and 760 px; a controlled native executable path exercises the Tauri warning response without creating an installer.

The Windows registry test is ignored in ordinary runs because it requires an isolated user hive. Run it explicitly with `AM_ISOLATED_WINDOWS_HIVE=1` in a disposable Wine container or test Windows user. Source-module harnesses exercised that test in Wine and file registration plus actual `gio launch` on Linux; these do not prove Windows/Linux Tauri IPC or desktop login. Native macOS acceptance uses a separate identifier and isolated HOME/data, inspects the plist after clicks, and preserves an external edit. A disposable launchd probe confirms the child survives its parent exit.
