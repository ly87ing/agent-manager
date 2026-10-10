# Account Context

## Responsibilities and sources of truth

Actual assistant files hold active authentication and configuration. Registrations hold the account's directory, declared mode, key reference, endpoint, and local generation. The selected external secrets file supplies values. The database holds registration metadata, ownership fingerprints, and inactive MCP snapshots; it does not store plaintext credentials.

| Part | Source |
| --- | --- |
| Controlled command generation, absolute executable resolution, environment, supported CLI contract, and session locks | [native_account_context.rs](../../engine-rs/src/native_account_context.rs) |
| Effective authentication and provider validation | [profile model](../../engine-rs/src/native_profiles/model.rs) and [authentication](../../engine-rs/src/native_auth.rs) |
| Candidate ownership graph, including native and saved MCP references | [native_secrets.rs](../../engine-rs/src/native_secrets.rs) |
| Secret-source permissions, bounded reads, and conditional writes | [secret_values.rs](../../engine-rs/src/secret_values.rs) |
| Account creation, configuration adoption, and updates | [profile apply](../../engine-rs/src/native_profiles/apply.rs) and [transaction](../../engine-rs/src/native_profiles/transaction.rs) |

## Runtime path

`accounts:command --only <agents:reserved-id|profiles:id> --operation <launch|login|logout|status> --shell <posix|powershell>` returns a controlled command without secret values. A draft `--definition` is a preview only. The executable's early `--account-launch` path validates the account binding, holds a session lock, checks local/project/managed configuration, resolves the assistant's absolute executable, and starts it with a clean environment. Accounts, authentication actions, tray copy, the palette, and recent-session resume request this command instead of using stored raw aliases.

`profiles:create` and `profiles:apply-definition` coordinate pending secret values, actual configuration, and registration. Planning is read-only. Writes hold the manager and account locks, compare baselines, read back results, and conditionally compensate normal failures. Adoption and shared settings apply the same identity and path checks. Main configuration parameters use the reserved main account's fixed path; a profile cannot invoke that writer.

## Constraints and failure behavior

- One account cannot share another's directory, final identity file, or credential reference. [Path isolation](account-path-isolation.md) covers symlinks, hardlinks, and overlapping registrations.
- Removing and recreating an account produces a fresh OS-random generation; old bindings remain invalid. Queries cannot initialize a legacy generation. Explicitly save the definition to initialize its metadata without migrating credentials.
- The merged effective configuration must match the declared authentication mode, key, and endpoint. Unknown provider/authentication layers and unsupported CLI versions stop before execution.
- Only the selected account's validated MCP references enter its environment. Copy and automatic fill refuse entries carrying credentials or uncertain fields. MCP connection tests share the account lock and clean-environment contract.
- Multiple launch sessions may share one account's read lock. Authentication and account configuration writes require its exclusive lock. A and B keep separate locks.
- CAS compensation preserves external changes and reports conflicts. There is no crash or power-loss recovery journal; assistant-generated metadata is outside the tool's transaction.
- Policy snapshots permit only the exact existing retirement of unused historical builtin skills performed when opening a write connection. The migration and snapshot check share one read-only projection; account registrations, source fields, and other user policy changes still invalidate the snapshot.
- See [Account Isolation](../ACCOUNT-ISOLATION.md) for the same-OS-user boundary and supported CLI versions.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_paths
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_policy
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_transaction
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_launch
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_settings
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp
```

Acceptance uses fake CLIs and credentials in isolated HOME/data directories, actual generated commands, and main/A/B file readback. Compilation or implementer reports alone do not establish native desktop operation. The task's final acceptance record distinguishes local Mac operation, CI platform tests, and unexecuted physical desktop or real-authentication flows.
