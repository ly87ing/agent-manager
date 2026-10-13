# Extension Target Isolation

[Extensions only](EXTENSIONS-ONLY.md) is the current product boundary. Agent Manager does not manage accounts, authenticate users, edit providers or runtime settings, or start account-bound assistant sessions. Historical registrations are immutable location and credential-ownership metadata. Existing assistant files, sessions, scripts, and database rows remain in place.

## Current isolation boundary

- Each MCP, rule, or skill action selects one assistant target from actual files and registered locations. Validation rejects overlapping main or registered directories, manager data, redirected paths, unsafe links, and identity-file hard links before offering a write baseline. See [path isolation](subsystems/account-path-isolation.md).
- Status and preview read without writing. Apply rereads the current target under a lock and compares the baseline supplied by the UI or CLI. Rejected writes preserve protected bytes. A rollback restores only bytes still written by that operation.
- An MCP connection check uses the selected target, a clean child environment, and dedicated MCP variables. Account identity variables and historical credential references cannot be repurposed. MCP secret values belong in a selected external private file; the database and audit records do not store plaintext.
- Policy and backup imports compare historical account and alias metadata unchanged. A source or inactive alternative can be saved without applying it to assistant files.
- General launch recipes are a separate `launch` domain. They do not resolve account registrations or credentials and never install shell aliases.

These controls govern Agent Manager's supported operations. They do not restrict another process running as the same OS user from modifying files or launching an assistant directly. A test with fake assistants cannot establish the state of a real subscription or an installed desktop client.

## Verification

Use isolated HOME, manager data, and test assistant paths. The relevant tests read back both selected and unselected targets after accepted and rejected operations:

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_extension_scope --test native_account_paths --test native_account_skills
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp --test native_rules --test native_secrets --test native_extension_program_scope
cargo test --manifest-path app/src-tauri/Cargo.toml
```

See [extension safety context](subsystems/account-context.md) for locks and credential ownership, and [platform status](GUI-PLATFORM-STATUS.md) for the current acceptance boundary.

### 2026-10-10: configuration and account review

The earlier account-operation acceptance recorded in [implementation history](goal-rust-gui.md) applied to a retired product scope. It does not authorize or verify account operations in the current version.
