# Account Context

## Responsibilities and sources of truth

Actual assistant files hold active authentication and configuration. Registrations hold the account's directory, declared mode, key reference, endpoint, and local generation. The selected external secrets file supplies values. The database holds registration metadata, ownership fingerprints, and inactive MCP snapshots; it does not store plaintext credentials.

| Part | Source |
| --- | --- |
| Controlled command generation, absolute executable resolution, environment, and supported CLI contract | [native_account_context.rs](../../engine-rs/src/native_account_context.rs) |
| Per-OS-user session locks and the bounded legacy-lock bridge | [native_account_locks.rs](../../engine-rs/src/native_account_locks.rs) |
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
- Session locks use a private, stable per-OS-user runtime directory, independent of `AGENT_MANAGER_DATA_DIR`, HOME and temporary-directory environment overrides: canonical `/tmp/agent-manager-account-locks-v1-<uid>` on Unix, and the OS-known LocalAppData directory's `AgentManager-account-locks-v1` child on Windows. Unix directories/files are created with modes 0700/0600; Windows creates a protected, inheritable current-user ACL atomically. Runtime ancestors must also prevent other users from replacing that private child; trusted sticky Unix temporary parents are allowed. Existing links, hardlinks, unsafe owners and permissions fail closed without being repaired automatically. These runtime files contain no credentials or account configuration and must not be removed while sessions run.
- Lock identity resolves existing ancestors and case-folds Windows/macOS paths even when trailing components do not exist. Policy changes take one global lock per identity while retaining every known old/new spelling for legacy checks, avoiding both self-contention and lost legacy aliases. Lock acquisition never initializes a manager data directory, database, or legacy lock file, including on fresh homes and after failed acquisitions.
- Windows ancestor checks include the absolute volume or share root. The exact Windows Modules Installer (TrustedInstaller) SID is accepted as an ancestor owner; it does not qualify as a private lock/credential owner or as a trusted ACL grantee. Root grants to create new children are allowed; rights to delete existing children, delete ancestors, or change their ACL/owner are rejected. Private lock and credential ACLs remain restricted to the current user and trusted system identities.
- CAS compensation preserves external changes and reports conflicts. There is no crash or power-loss recovery journal; assistant-generated metadata is outside the tool's transaction.
- Policy snapshots permit only the exact existing retirement of unused historical builtin skills performed when opening a write connection. The migration and snapshot check share one read-only projection; account registrations, source fields, and other user policy changes still invalidate the snapshot.
- See [Account Isolation](../ACCOUNT-ISOLATION.md) for the same-OS-user boundary and supported CLI versions.

## Upgrading from data-directory locks

Stop all older-version account sessions and exit older Agent Manager processes before upgrading. Do not restart an older binary alongside the upgraded version. This is a migration prerequisite, not a claim that a new binary can control another same-user program.

The bridge checks existing legacy files in the selected data directory and the discoverable CLI and desktop default data directories, including the current platform data-base environment override. It retains their original inodes and names. A first upgraded reader probes the legacy file exclusively; an active older shared session is rejected with instructions to end it and restart using the upgraded application. A short per-legacy-path gate and a runtime cohort lease permit later upgraded readers to share that file safely. All legacy handles remain held through the complete session or transaction; an acquisition failure releases earlier handles without replacing the lock files. Legacy ancestor links or replacement permissions are rejected. Missing legacy directories and files are left missing by both reads and writes.

Arbitrary historical custom data directories, historical case spellings that are no longer known, and older processes started after the migration check cannot be discovered or excluded reliably. There is no unbounded filesystem/process scan and no persisted data-directory registry. Cross-data-directory protection is guaranteed between upgraded processes; it does not silently imply protection from unknown old-version instances. The stop-old-programs prerequisite covers that boundary.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --lib native_account_locks
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_paths
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_policy
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_transaction
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_launch
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_settings
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp
```

Acceptance uses fake CLIs and credentials in isolated HOME/data directories, actual generated commands, and main/A/B file readback. Compilation or implementer reports alone do not establish native desktop operation. The task's final acceptance record distinguishes local Mac operation, CI platform tests, and unexecuted physical desktop or real-authentication flows.
