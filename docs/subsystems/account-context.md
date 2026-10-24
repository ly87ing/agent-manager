# Extension Safety Context

## Responsibilities and sources of truth

Historical assistant registrations are immutable resource-location metadata.
Actual MCP files, rule files and skill links determine active extension state.
The engine does not create accounts, inspect login state, generate account-bound
commands, change provider settings, or launch account-bound assistant sessions.

The dedicated [Claude local-data maintenance](claude-local-data.md) domain is the
2026-10-23 exception for explicitly confirmed default local session/account-data
deletion. It does not loosen these extension helpers or restore account APIs.

The retained helpers have extension responsibilities:

| Part | Source |
| --- | --- |
| Resolve registered extension locations, scrub MCP subprocess environments and acquire location locks | [native_account_context.rs](../../engine-rs/src/native_account_context.rs) |
| Private per-OS-user locks and existing-version lock exclusion | [native_account_locks.rs](../../engine-rs/src/native_account_locks.rs) |
| Parse immutable registrations and protect resource destinations | [Target metadata](../../engine-rs/src/native_profiles/model.rs) and [path guards](../../engine-rs/src/native_profiles/paths.rs) |
| Protect credential references and ownership fingerprints | [native_secrets.rs](../../engine-rs/src/native_secrets.rs) |
| Validate external secret sources and conditionally write MCP variables | [secret_values.rs](../../engine-rs/src/secret_values.rs) |

Retired account writers, authentication probes, assistant installation, runtime
settings, profile rendering and shell-alias management have been deleted.
The old `--account-launch` path retains only an immediate error; existing scripts
and assistant files stay on disk.

## Runtime path

`resources:targets` reads only labels and registered configuration, skill and MCP
paths. It skips external secret loading, executable probes, authentication and
directory scans. The read-only Assistants page uses this response; it does not
establish installation or login status.

Extension writes validate their selected destination, compare the current file
baseline and hold the appropriate write locks. MCP connection checks use a clean
child environment, dedicated MCP variables and the same location-lock identity.
[MCP checks](mcp-check.md) documents protocol stages and process cleanup.

Policy writes and configuration imports compare historical account and alias
metadata unchanged before committing. Backups exclude those definitions.
Saving general launch recipes uses the separate `launch` domain and does not
resolve account metadata or access MCP secrets.

## Constraints and failure behavior

- Main and registered assistant directories, identity files and manager data are
  protected from cross-target extension writes. Links, hard links, overlapping
  paths and stale baselines fail closed. See [path isolation](account-path-isolation.md).
- Only dedicated variables referenced by MCP can be changed. Assistant identity
  namespaces and historical credential references remain protected. Ownership
  fingerprints retain their original registration identity.
- Runtime locks use a private OS-user location independent of HOME, temporary
  directory overrides and the manager data directory. Reads never initialize a
  manager database or historical lock file.
- Unix private directories/files use 0700/0600. Windows private lock entries use
  protected current-user ACLs. Unsafe links, owners, permissions and ancestors
  are rejected without being repaired.
- Different data directories share the same location-lock identity. Existing
  legacy lock handles remain held through the operation. Stop older manager
  processes and older controlled sessions before upgrading; arbitrary historical
  custom data directories are not discovered by an unbounded scan.
- File rollback is conditional on the current contents still matching this
  operation's write. External changes are preserved and reported.
- General launch requests and independent applications follow
  [launch configurations](launch-configurations.md).

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --lib native_account_locks
cargo test --manifest-path engine-rs/Cargo.toml --test native_extension_scope --test native_extension_program_scope
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_paths --test native_account_skills
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp --test native_rules --test native_secrets
```

Fixtures use isolated HOME/data and test assistant paths. They read back protected
file bytes, selected and unselected targets, immutable metadata and absent state.
Tests named `account` cover current extension safety, not account management.
These checks do not establish real-account, installed-desktop or update acceptance.
