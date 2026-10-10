# Program Lifecycle

Programs required by skills and MCP services can remain in a global installation
or be explicitly adopted into the data directory. Assistant executables are not
managed by this subsystem. Product scope is in [Program Lifecycle](../PROGRAM-LIFECYCLE.md).

## Responsibilities and sources of truth

- [`locate.rs`](../../engine-rs/src/native_programs/locate.rs) resolves commands
  against the user's PATH, including Windows PATHEXT, and reads package metadata.
- [`status.rs`](../../engine-rs/src/native_programs/status.rs) reads completed
  version directories and resolves `current` to a managed version identifier.
  Both relative symlinks and absolute Windows junctions are supported.
- [`apply.rs`](../../engine-rs/src/native_programs/apply.rs) builds plans, backs up
  external packages and their launchers, invokes the original package manager,
  verifies its result, and restores moved files when an upgrade fails.
- [`native_upstream.rs`](../../engine-rs/src/native_upstream.rs) runs npm and PyPI
  version probes through the same user PATH/PATHEXT resolution. Read-only status
  uses cached probe results rather than performing network requests.

## Runtime path

An npm global package's prefix is derived from its validated native layout:
`<prefix>/lib/node_modules/<package>` on Unix, or
`<prefix>/node_modules/<package>` on Windows. Ordinary and scoped packages use
the same helper for delegated upgrades and adoption. Windows npm `.cmd`, `.ps1`,
and POSIX launchers are backed up only when their targets resolve into the package.

A managed installation has completed version directories and a `current`
directory link. Status and individual-version removal share the same resolver:
the link must resolve to a completed direct child of the program directory.

## Constraints and failure behavior

- Individual-version deletion refuses the active version and refuses to proceed
  when the current pointer is missing, broken, unreadable, or outside the managed
  program. A normal directory named `current` is not a valid pointer.
- Version identifiers cannot address `current`, dot components, Windows device
  names, or trailing-dot aliases. Registered command entries are compared as
  native paths and use extension-free command names in inventory.
- Failed delegated upgrades restore both package directories and regular Windows
  launcher files, including failures after the installer has replaced them.
- Install-time PyPI discovery locates `curl` through user PATH/PATHEXT. Explicit
  executable paths are used for probes; no hard-coded Unix curl path is assumed.
- Queries and plans do not install software or alter program directories.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --lib native_programs
cargo test --manifest-path engine-rs/Cargo.toml --test native_program_lifecycle
cargo test --manifest-path engine-rs/Cargo.toml --test native_programs
cargo test --manifest-path engine-rs/Cargo.toml --test native_programs_windows
cargo test --manifest-path engine-rs/Cargo.toml --test native_upstream
```

The portable lifecycle target uses its own executable as a test-only tool fixture,
with a real `npm.cmd` launcher and a `curl.exe` copy on Windows. It runs CLI
operations against isolated HOME/data directories, covers ordinary and scoped npm
layouts, checks byte-for-byte failed-upgrade restoration and adoption undo, and
creates Windows junctions directly without requiring symlink privileges. These
fixtures exercise process and filesystem behavior without contacting registries.
