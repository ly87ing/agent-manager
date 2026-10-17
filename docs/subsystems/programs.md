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
- [`scope.rs`](../../engine-rs/src/native_programs/scope.rs) reserves built-in and
  historically registered assistant command names and npm packages, independently
  of their installation location. Inventory, declarations, and direct lifecycle
  requests use the same exclusion.
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

Windows uv environments use `uv/data/tools` and `Lib/site-packages`. Copied `.exe`
entries are recognized and backed up only when `uv-receipt.toml` names the actual
entry and its bytes match the environment's `Scripts` entry. A changed copy stays
unowned. Receipt reads and byte comparisons are bounded; queries never invoke uv.
Managed PyPI/git installs and delegated uv upgrades explicitly pass
`--no-python-downloads`. Existing interpreter selection is retained; a missing
interpreter fails rather than letting uv provide a runtime. This implements the
[observe-only runtime boundary](../PROGRAM-LIFECYCLE.md#5-object-model). The option
also appears in copyable list and operation-dialog commands. An older uv that
does not recognize it fails before installation instead of silently ignoring a
new environment variable.
This option controls uv's own Python provisioning; it does not isolate arbitrary
package build code or establish an installed-version postcondition.
An absent, empty or relative `SHELL` uses `/bin/bash` on Linux and `/bin/zsh` on
macOS; an explicit absolute shell retains its own failure diagnostics.

Manual execution and failure-recovery commands use PowerShell on Windows and the
existing POSIX form on Unix. Windows executable steps reuse the launch renderer
for literal arguments, batch forwarding, child-only environment, working directory
and exit-code checks. File moves, command shims and `current` junctions use native
Windows operations. Existing command-entry hashes and link targets guard manual
replacement against changes after preview. App metadata and database preferences
remain application-owned; the manual command list does not claim to reproduce its
operation audit or completion metadata. Batch shims preserve literal percent signs
and disable delayed expansion without changing the caller's shell settings.

A managed installation has completed version directories and a `current`
directory link. Status and individual-version removal share the same resolver:
the link must resolve to a completed direct child of the program directory.

The interface labels these resources as extension dependencies. Version action
buttons display complete versions and wrap within their action column; short
status badges do not repeat long version strings. Paths and current versions wrap.

## Constraints and failure behavior

- Assistant packages and commands are excluded from skill declarations, suggestions,
  MCP runners, and managed-program inventory. Direct lifecycle requests refuse
  them, including packages whose completed installation exposes an assistant command.
- Entry directories cannot overlap main or registered accounts, MCP identity files,
  or manager data. Plans validate every filesystem mutation target; execution
  repeats that validation before each step. Historical cleanup links, backup
  restore destinations, and aliases inside the data directory receive the same
  account protection. An unsafe plan performs no filesystem steps.
- Moving a package can temporarily leave its launcher links dangling. Validation
  checks the link location and destination separately so ordinary upgrades and
  rollback still work; it does not relax account identity path validation.
- Program metadata and backup descriptions must be ordinary files with no links.
  Writes refuse hard links even when the other name is an unrecognized session
  file inside an account directory. This is checked again after the installer runs.
- Backup listing and automatic pruning reject account-directory aliases. Failure
  rollback rechecks its targets, skips unsafe moves or cleanup, and reports that
  the retained backup needs inspection. It does not restore through an account alias.
- Individual-version deletion refuses the active version and refuses to proceed
  when the current pointer is missing, broken, unreadable, or outside the managed
  program. A normal directory named `current` is not a valid pointer.
- Version identifiers cannot address `current`, dot components, Windows device
  names, or trailing-dot aliases. Registered command entries are compared as
  native paths and use extension-free command names in inventory.
- Linking and unlinking derive the same platform-specific entry filename:
  `<command>` on Unix and `<command>.cmd` on Windows. Unlink retains the managed
  installation and entries in other directories.
- Failed delegated upgrades restore both package directories and regular Windows
  launcher files, including failures after the installer has replaced them.
- Install-time PyPI discovery locates `curl` through user PATH/PATHEXT. Explicit
  executable paths are used for probes; no hard-coded Unix curl path is assumed.
- Queries and plans do not install software or alter program directories.

## Verification

The interface uses the shared [feedback policy](interface-feedback.md). MCP
version changes report success only when both entry edits and any required
version-preference updates succeed. Partial work keeps its failure records and
does not close the version dialog as a successful change.

```sh
cargo test --manifest-path engine-rs/Cargo.toml --lib native_programs
cargo test --manifest-path engine-rs/Cargo.toml --test native_extension_program_scope
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

The [native uv fixture](../../scripts/verify-native-uv-runtime.mjs) uses an empty
private runtime directory and a loopback mirror that serves only 404 responses.
An unguarded positive control must request a supported Python archive; managed
and delegated CLI operations and their copied list/dialog install commands must
not, even with inherited automatic downloads enabled. It verifies the original delegated
package/entry identities and bytes after missing-interpreter failure. It never
installs an interpreter or contacts a package registry. GUI CI runs it on all
three native systems with uv 0.13.1, a pinned setup action and archive checksums.
Local execution requires `AM_ISOLATED_NATIVE_UV=1` and an absolute real binary in
`AM_NATIVE_UV`; only a copy is executed in fresh private fixture paths.

Windows command-entry metadata uses ASCII hex-encoded UTF-8 paths; legacy plain-text metadata remains readable. Manual filesystem commands reject linked ancestors, compare existing entry hashes, and stage a literal junction before replacing `current`. A staging failure preserves the previous pointer and foreign staging contents. Recursive manual removal refuses nested reparse points.
