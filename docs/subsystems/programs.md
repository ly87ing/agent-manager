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
  verifies its result, and attempts restoration after command failures.
- [`uv_postcondition.rs`](../../engine-rs/src/native_programs/uv_postcondition.rs)
  independently checks delegated uv results and identifies version failures that
  must retain current files and the moved backup for inspection.
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
On Unix, an explicit absolute `UV_TOOL_DIR` also supports custom locations.
[`uv_custom_root.rs`](../../engine-rs/src/native_programs/uv_custom_root.rs)
requires a valid PyPI package name, an ordinary executable at the resolved
`<tool>/bin/<command>` location and an ordinary receipt within 64 KiB naming the
exact entry path and command. Receipt links, special files and uncertain entries
do not establish custom-root ownership. Standard path-marker recognition is
unchanged, including its existing fallback when receipt evidence is unavailable.
Installed uv versions come from the matching distribution's ordinary `METADATA`
file, not its directory name. Native Windows uses `Lib/site-packages`; Unix uses
`lib/python<major>.<minor>[t]/site-packages` or `lib/pypy<major>.<minor>/site-packages`,
including free-threaded CPython and PyPy's native layout.
Identity headers are read within 64 KiB,
before the description body, with normalized names and PEP 440 versions.
Directory/version disagreement, missing or malformed headers, unreadable matching
candidates and multiple matches produce an unknown version. Enumeration is
bounded to 64 library entries, 16 Python roots and 8192 entries per package root.
Reads retain directory handles, refuse linked path components and special files,
and permit ordinary hardlinked cache metadata. This is declared-version evidence,
not package authenticity, successful execution or an upgrade postcondition.
Managed PyPI/git installs and delegated uv upgrades explicitly pass
`--no-python-downloads`. Existing interpreter selection is retained; a missing
interpreter fails rather than letting uv provide a runtime. This implements the
[observe-only runtime boundary](../PROGRAM-LIFECYCLE.md#5-object-model). The option
also appears in copyable list and operation-dialog commands. An older uv that
does not recognize it fails before installation instead of silently ignoring a
new environment variable.
This option controls uv's own Python provisioning; it does not isolate arbitrary
package build code. After a successful delegated uv upgrade/switch command,
installed metadata must satisfy the requested PEP 440 equality specifier before
entry checks. Equivalent releases and matching local versions follow PyPA version
matching semantics. Invalid target versions fail during preview before moves.
An unreadable, ambiguous or mismatched version reports failure without automatic
restoration or cleanup after that check. The unverified current install may still
be on PATH. This retains existing backup files for inspection, not a promise of
an independent, runnable or immutable backup. Every later successful action for
that program applies the existing newest-three non-adoption backup retention.
Managed PyPI installs validate their name/target during preview and freshly read
native metadata and declared command entries before writing a completion marker
and before selecting `current`.
Reused completed versions and explicit switches receive the selection check too.
Each check requires a matching PEP 440 equality result and declared entry targets
that resolve to ordinary files inside the selected version, with execute bits
on Unix. Internal entry links remain valid; missing/unreadable entries, directories,
external targets and absent declarations do not establish entry availability.
The plan carries its own declared commands into both gates; a Windows outer shim
existing by itself is insufficient. Checks execute no command and do not prove
interpreter/shebang availability, authenticity or arbitrary program functionality.
This does not make filesystem
publication atomic against uncoordinated external changes after or between reads.
An incomplete existing PyPI version directory is refused, including on a later
retry, so retained candidates are not automatically deleted or overwritten.
Failures retain their candidate and report which completion/selection step was
not performed; an earlier completion marker or previously active contents may
already exist. Existing target/account guards remain in force. Parent path
aliases are normalized while the version root and internal metadata components
retain ordinary-object checks. Git version postconditions and general
rollback/restore conflicts remain separate work.
The `PROGRAM_VERSION_UNVERIFIED` diagnostic identifies this incomplete outcome;
it also identifies refusal to overwrite an incomplete managed PyPI candidate.
The dialog keeps its details visible and announces a warning. Ordinary installer
failures retain `PROGRAM_ERROR` and their existing recovery attempt.
`PROGRAM_ENTRY_UNVERIFIED` identifies failed managed entry evidence before
completion/selection and retains the same inspection warning without automatic
rollback/cleanup after that check. An earlier completion marker can still exist;
later retries refuse incomplete candidates through the existing version diagnostic.
For installed uv tools, cached upstream versions/timestamps remain available, but
current versions and ordering come from actual metadata. Unreadable current
metadata remains unknown instead of inheriting a cached current/latest label.
An absent, empty or relative `SHELL` uses `/bin/bash` on Linux and `/bin/zsh` on
macOS; an explicit absolute shell retains its own failure diagnostics.
The CLI accepts an eligible entry directory in either `~/...` or absolute form.
It compares expanded paths without adopting additional filesystem aliases;
existing stable-PATH, writable-directory and protected-target checks still apply.
Equivalent spellings of an already saved directory do not rewrite its preference.

Manual execution and failure-recovery commands use PowerShell on Windows and the
existing POSIX form on Unix. Windows executable steps reuse the launch renderer
for literal arguments, batch forwarding, child-only environment, working directory
and exit-code checks. File moves, command shims and `current` junctions use native
Windows operations. Existing command-entry hashes and link targets guard manual
replacement against changes after preview. App metadata and database preferences
remain application-owned; the manual command list does not claim to reproduce its
operation audit, completion metadata or native version-verification gates. Batch shims preserve literal percent signs
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
- Failed delegated commands attempt restoration of package directories and owned
  Windows launcher files. A successful uv command with an unverified installed
  version instead retains the current files and backup, reporting both locations.
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
cargo test --manifest-path engine-rs/Cargo.toml --test native_uv_metadata
cargo test --manifest-path engine-rs/Cargo.toml --test native_uv_lifecycle
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

The [offline version fixture](../../scripts/verify-native-uv-versions.mjs) selects
an existing interpreter explicitly with `AM_NATIVE_PYTHON`, generates private
wheels, then performs real uv upgrades, entry execution and public/local version
matching without a package registry. It verifies that the selected interpreter
binary/identity is unchanged and private managed-runtime directories remain empty.
GUI CI runs standard/custom delegated roots and managed install/upgrade/switch
cases, reading back actual metadata, completion markers, selected pointers and
entry output. It selects a preinstalled runner interpreter; this fixture never bootstraps
one and adds no interpreter requirement to the Rust engine test suite.

Windows command-entry metadata uses ASCII hex-encoded UTF-8 paths; legacy plain-text metadata remains readable. Manual filesystem commands reject linked ancestors, compare existing entry hashes, and stage a literal junction before replacing `current`. A staging failure preserves the previous pointer and foreign staging contents. Recursive manual removal refuses nested reparse points.
