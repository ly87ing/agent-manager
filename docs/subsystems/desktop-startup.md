# Desktop Startup

## Responsibilities and sources of truth

The current executable's location determines whether macOS needs an installation warning. [`startup.rs`](../../app/src-tauri/src/startup.rs) reads the filesystem's read-only flag and checks for an `AppTranslocation` path component. A writable external volume alone does not trigger a warning. This is runtime information, never a saved preference.

[`autostart.rs`](../../app/src-tauri/src/autostart.rs) owns the OS login entry. Its state comes from that entry, not SQLite or local storage. macOS uses `~/Library/LaunchAgents/<identifier>.plist`; Linux uses `$XDG_CONFIG_HOME/autostart/<identifier>.desktop` (defaulting to `~/.config`); Windows uses the application's named value under current-user `Run` and reads `StartupApproved/Run` for Task Manager's override.

## Runtime path

The CLI, desktop engine and Settings use the shared
[`data_dir`](../../engine-rs/src/data_dir.rs) selector. A nonempty
`AGENT_MANAGER_DATA_DIR` takes precedence. New installations use the platform's
application-data root plus `io.github.ly87ing.agent-manager`. When only the sibling
legacy `agent-manager/native-manager.db` exists, both entry points keep using it.
When both databases exist, the current application directory wins; neither file
is moved, merged or deleted. To inspect the other database, explicitly select its
directory. Separate application identifiers do not adopt the main app's legacy
database. Reads never create a directory or database.

[`settings_data`](../../app/src-tauri/src/settings.rs) includes an optional `runtimeWarning`. [`App`](../../app/src/App.tsx) displays it above the current page. The [Settings page](../../app/src/pages/SettingsPage.tsx) suppresses manual update checks and installation while the warning is present. There is no automatic update check. Closing the main window uses Tauri's normal close/exit flow; no tray or second webview is created.

Settings reads `autostart_get` on opening and presents explicit Enable and Disable
actions. A click calls `autostart_set`; the backend reads the OS entry back before
reporting success, and the frontend refreshes after failures too. Merely opening
Agent Manager or Settings never registers a login entry. Enabling is unavailable
from a macOS translocated or read-only launch location; disabling remains available.

The committed first-launch window is 1000 × 650. Later geometry remains owned by
the window-state plugin. Native UI previews use a fresh identifier and isolated
HOME/data while inheriting the committed window settings; overriding their height
would not verify the default proportions. Check empty and populated pages at the
normal height, following [UI rules](../UI-RULES.md).

The native exit event stops the engine's active temporary command groups before
Tauri terminates the process. Independent account terminal sessions continue;
see [External commands](external-commands.md) for ownership and race handling.

### Share links

[`import_links.rs`](../../app/src-tauri/src/import_links.rs) keeps the deep-link plugin's initial URL batch and assigns each opening a delivery ID. The runtime listener is installed before seeding from `get_current`; a newer opening wins over that setup snapshot. [`subscribeImportLinks`](../../app/src/lib/shareLink.ts) registers its frontend listener before reading `import_links_current`, deduplicating the two delivery paths by monotonically increasing ID and ignoring stale snapshots after a newer runtime opening. Another click on the same URL gets a new ID. Disposing a subscription prevents callbacks from a delayed initial read and removes even a listener whose registration completes after disposal. Links only prefill the existing import dialog; they never apply configuration by themselves.

Account authentication command generation and its shell selector have been removed.
The [read-only Assistants page](account-context.md) only lists immutable extension
target metadata; generic tips do not generate or execute account commands.

Linux AppImages expose an explicit Settings action to associate share links.
[`desktop_links`](../../engine-rs/src/desktop_links.rs) reads the actual default
handler and previews the original AppImage path and current-user desktop entry.
Confirmation rechecks that fingerprint, creates only an exact owned entry under
`$XDG_DATA_HOME/applications` (default `~/.local/share/applications`), and asks
`xdg-mime` to associate `x-scheme-handler/agentmanager`. It then reads the actual
default through `gio mime` under a fixed English UTF-8 message locale. Older generic
`xdg-mime query` implementations split `Exec` without parsing desktop quoting
and reject valid quoted AppImage paths; GIO uses the launch parser instead.
Merely opening Settings, previewing or canceling never registers.
Existing handler files and other MIME associations are preserved. Changed,
non-regular or symlinked owned entries are refused; moving the AppImage requires
manual review of an old entry rather than silently overwriting it. A failed
association may leave the exact owned entry, allowing a safe fresh retry.
The asynchronous Tauri bridge bounds each desktop utility call to eight seconds.
This action is omitted for macOS, Windows and installed Linux DEBs, whose scheme
declarations belong to the package. See [xdg-mime](https://portland.freedesktop.org/doc/xdg-mime.html).

## Constraints and failure behavior

- Detection only reads the executable path and mount flags; it does not move the app or write configuration.
- The message asks the user to quit, drag the app into Applications and reopen it.
- Other platforms omit the macOS-specific warning.
- Enabling autostart from a macOS translocated or read-only launch location is rejected before creating an entry.
- Reading login state creates no directory or file. File entries must be regular files with exactly the expected contents; registry values must point to this executable. External changes and symlinks are refused, including before deletion. Errors remain visible with a reread action.
- Enabling creates only the parent login-entry directory and an owned entry. Repeated enables are idempotent. Disabling deletes only the exact owned entry.
- macOS/Linux registration uses the current executable (the original AppImage on Linux), and no software or assistant resource is installed. Moving the app can leave an old entry that must be reviewed manually.
- Linux refuses executable paths with `%`, `=`, control characters or invalid UTF-8. GIO validates the executable before expanding `%%`, and the specification forbids `=` in executable names; do not report registration success for an unusable path. See the [Desktop Entry specification](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html) and [GIO loader](https://github.com/GNOME/glib/blob/main/gio/gdesktopappinfo.c).
- OS policy may still prevent a login launch. A registered entry does not prove a physical reboot/login succeeded.

## Verification

GUI CI starts the extracted Linux AppImage and deb executables with private
HOME/data/runtime directories, Xvfb and a dedicated D-Bus session. Document
portal activation may create a FUSE mount below that private runtime directory.
After session exit, teardown uses the read-only
[`wait-linux-installer-unmount.mjs`](../../scripts/wait-linux-installer-unmount.mjs)
barrier to poll for all visible mounts below the fixture root to disappear,
with at most fifty 100 ms waits. It parses the kernel's
[mountinfo format](https://docs.kernel.org/filesystems/proc.html),
including escaped paths and same-device bind mounts. Unknown evidence or a
remaining mount fails teardown and retains the fixture. Only then does cleanup
remove the owned directory with `--one-file-system` and verify absence.
The helper does not unmount resources or change system preferences.

Run its parser and wait/refusal cases with
`node --test scripts/wait-linux-installer-unmount.test.mjs`. Mock tables establish
the barrier logic; actual portal retirement requires native Linux CI evidence.

```sh
cargo test --manifest-path app/src-tauri/Cargo.toml import_links::tests
cargo test --manifest-path app/src-tauri/Cargo.toml startup::tests
cargo test --manifest-path app/src-tauri/Cargo.toml autostart::tests
cargo test --manifest-path engine-rs/Cargo.toml --lib desktop_links
cargo check --manifest-path app/src-tauri/Cargo.toml --target x86_64-pc-windows-gnu --all-targets
cd app
pnpm test:unit run src/test/SettingsPage.test.tsx src/test/UpdateBadge.test.tsx --maxWorkers=2
pnpm test:unit run src/test/ShareLinkRegistration.test.tsx --maxWorkers=2
pnpm test:unit run src/test/importLinkSubscription.test.ts src/test/AgentsPage.test.tsx src/test/AppNavigation.test.tsx --maxWorkers=2
```

Share-link tests cover cold and warm openings, either snapshot/event race ordering,
repeated URLs, and unmount during both asynchronous steps. Assistants tests preserve
metadata-only navigation and failure handling. These checks do not replace native
OS link activation.

Native Linux registration tests use isolated HOME/XDG directories, preserve a
foreign handler and unrelated MIME type, reject stale/changed/symlinked entries,
and launch the generated entry through real GIO to read back the exact URI.
This proves desktop-entry argument delivery, not an installed AppImage's full
Tauri import-dialog acceptance.

Tests distinguish real read-only system and writable development mounts, translocated paths, and writable external-volume paths. Frontend tests cover the banner and both update-check paths. Browser layout acceptance uses mock settings at 1000 and 760 px; a controlled native executable path exercises the Tauri warning response without creating an installer.

The Windows registry test is ignored in ordinary runs. GUI CI explicitly runs
it on its disposable Windows runner with `AM_ISOLATED_WINDOWS_HIVE=1`, requiring
the native Windows/GitHub CI markers before opening registry keys. HOME does not
isolate HKCU; never use a personal user hive as this fixture's target. Unique
fixture names are checked absent first, cleaned during failure unwinding, and independently
checked absent afterwards. The fixture verifies quoted registration, enable/disable,
idempotency, Task Manager's stored override and external Run-value protection;
unrelated raw registry values must match their original snapshot after cleanup.
This is native registry behavior, not Windows Tauri IPC or a real desktop login.
Earlier source-module harnesses used Wine, while Linux exercised file registration
and actual `gio launch`; those remain separate evidence. Native macOS acceptance
uses a separate identifier and isolated HOME/data, inspects the plist after clicks,
and preserves an external edit. A disposable launchd probe confirms the child
survives its parent exit.
