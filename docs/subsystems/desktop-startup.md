# Desktop Startup

## Responsibilities and sources of truth

The current executable's location determines whether macOS needs an installation warning. [`startup.rs`](../../app/src-tauri/src/startup.rs) reads the filesystem's read-only flag and checks for an `AppTranslocation` path component. A writable external volume alone does not trigger a warning. This is runtime information, never a saved preference.

[`autostart.rs`](../../app/src-tauri/src/autostart.rs) owns the OS login entry. Its state comes from that entry, not SQLite or local storage. macOS uses `~/Library/LaunchAgents/<identifier>.plist`; Linux uses `$XDG_CONFIG_HOME/autostart/<identifier>.desktop` (defaulting to `~/.config`); Windows uses the application's named value under current-user `Run` and reads `StartupApproved/Run` for Task Manager's override.

## Runtime path

[`settings_data`](../../app/src-tauri/src/settings.rs) includes an optional `runtimeWarning`. [`App`](../../app/src/App.tsx) displays it above the current page. The [Settings page](../../app/src/pages/SettingsPage.tsx) suppresses manual update checks and installation while the warning is present. There is no automatic update check. Closing the main window uses Tauri's normal close/exit flow; no tray or second webview is created.

Settings reads `autostart_get` on opening. Enabling is retired in the UI and
backend. The explicit Remove old login entry action calls `autostart_set(false)`;
the backend reads back the result and the frontend refreshes after failures too.
The removal action remains available for an OS-disabled Windows entry, since a
false enabled state does not prove the Run value is absent. Existing entry formats
are recognized only for ownership-checked removal.

The native exit event stops the engine's active temporary command groups before
Tauri terminates the process. Independent account terminal sessions continue;
see [External commands](external-commands.md) for ownership and race handling.

### Share links and authentication commands

[`import_links.rs`](../../app/src-tauri/src/import_links.rs) keeps the deep-link plugin's initial URL batch and assigns each opening a delivery ID. The runtime listener is installed before seeding from `get_current`; a newer opening wins over that setup snapshot. [`subscribeImportLinks`](../../app/src/lib/shareLink.ts) registers its frontend listener before reading `import_links_current`, deduplicating the two delivery paths by monotonically increasing ID and ignoring stale snapshots after a newer runtime opening. Another click on the same URL gets a new ID. Disposing a subscription prevents callbacks from a delayed initial read and removes even a listener whose registration completes after disposal. Links only prefill the existing import dialog; they never apply configuration by themselves.

The Accounts page explicitly chooses the shell for copied authentication commands and new-account previews. Windows defaults to PowerShell, other platforms to Bash/Zsh; Windows Terminal is not treated as a shell. Users in cmd are told to enter PowerShell first. [`authTerminalCommand`](../../app/src/lib/profileDefinition.ts) quotes paths for the selected shell. PowerShell saves and restores the account-directory environment variable in a `finally` block, including failed command lookup/login; POSIX uses a command-local environment assignment. Copying does not execute authentication.

## Constraints and failure behavior

- Detection only reads the executable path and mount flags; it does not move the app or write configuration.
- The message asks the user to quit, drag the app into Applications and reopen it.
- Other platforms omit the macOS-specific warning.
- Enabling autostart is rejected before creating directories or registry values, regardless of launch location.
- Reading login state creates no directory or file. File entries must be regular files with exactly the expected contents; registry values must point to this executable. External changes and symlinks are refused, including before deletion. Errors remain visible with a reread action.
- macOS/Linux registration uses the current executable (the original AppImage on Linux), and no software or assistant resource is installed. Moving the app can leave an old entry that must be reviewed manually.
- Linux refuses executable paths with `%`, `=`, control characters or invalid UTF-8. GIO validates the executable before expanding `%%`, and the specification forbids `=` in executable names; do not report registration success for an unusable path. See the [Desktop Entry specification](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html) and [GIO loader](https://github.com/GNOME/glib/blob/main/gio/gdesktopappinfo.c).
- OS policy may still prevent a login launch. A registered entry does not prove a physical reboot/login succeeded.

## Verification

```sh
cargo test --manifest-path app/src-tauri/Cargo.toml import_links::tests
cargo test --manifest-path app/src-tauri/Cargo.toml startup::tests
cargo test --manifest-path app/src-tauri/Cargo.toml autostart::tests
cargo check --manifest-path app/src-tauri/Cargo.toml --target x86_64-pc-windows-gnu --all-targets
cd app
pnpm test:unit run src/test/SettingsPage.test.tsx src/test/UpdateBadge.test.tsx --maxWorkers=2
pnpm test:unit run src/test/importLinkSubscription.test.ts src/test/authTerminalCommand.test.ts src/test/AccountsPage.test.tsx src/test/AppNavigation.test.tsx --maxWorkers=2
```

Share-link tests cover cold and warm openings, either snapshot/event race ordering, repeated URLs, and unmount during both asynchronous steps. Authentication tests execute disposable assistant fixtures under POSIX on Unix and Windows PowerShell on Windows in the existing frontend CI matrix; UI tests verify the selected shell reaches preview and copy actions. These tests do not replace native OS link activation or an actual official login.

Tests distinguish real read-only system and writable development mounts, translocated paths, and writable external-volume paths. Frontend tests cover the banner and both update-check paths. Browser layout acceptance uses mock settings at 1000 and 760 px; a controlled native executable path exercises the Tauri warning response without creating an installer.

The Windows registry test is ignored in ordinary runs because it requires an isolated user hive. Run it explicitly with `AM_ISOLATED_WINDOWS_HIVE=1` in a disposable Wine container or test Windows user. Source-module harnesses exercised that test in Wine and file registration plus actual `gio launch` on Linux; these do not prove Windows/Linux Tauri IPC or desktop login. Native macOS acceptance uses a separate identifier and isolated HOME/data, inspects the plist after clicks, and preserves an external edit. A disposable launchd probe confirms the child survives its parent exit.
