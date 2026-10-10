# Desktop Startup

## Responsibilities and sources of truth

The current executable's location determines whether macOS needs an installation warning. [`startup.rs`](../../app/src-tauri/src/startup.rs) reads the filesystem's read-only flag and checks for an `AppTranslocation` path component. A writable external volume alone does not trigger a warning. This is runtime information, never a saved preference.

## Runtime path

[`settings_data`](../../app/src-tauri/src/settings.rs) includes an optional `runtimeWarning`. [`App`](../../app/src/App.tsx) displays it above the current page. The [automatic update hook](../../app/src/lib/update-check.ts) and [Settings page](../../app/src/pages/SettingsPage.tsx) suppress update checks and installation while the warning is present.

## Constraints and failure behavior

- Detection only reads the executable path and mount flags; it does not move the app or write configuration.
- The message asks the user to quit, drag the app into Applications and reopen it.
- Other platforms omit the macOS-specific warning.

## Verification

```sh
cargo test --manifest-path app/src-tauri/Cargo.toml startup::tests
cd app
pnpm test:unit run src/test/SettingsPage.test.tsx src/test/UpdateBadge.test.tsx --maxWorkers=2
```

Tests distinguish real read-only system and writable development mounts, translocated paths, and writable external-volume paths. Frontend tests cover the banner and both update-check paths. Browser layout acceptance uses mock settings at 1000 and 760 px; a controlled native executable path exercises the Tauri warning response without creating an installer.
