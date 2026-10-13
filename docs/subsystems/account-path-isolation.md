# Extension Target Path Isolation

## Responsibilities and sources of truth

Historical registrations identify existing assistant extension locations; actual MCP files, rule files, and skill directories determine active state. [Path guards](../../engine-rs/src/native_profiles/paths.rs) resolve those locations and protect main and registered assistant directories, identity files, manager data, and cross-target resources. They do not create, edit, or switch accounts.

The `validate_shell_target` helper has a historical name but remains in use for skill source and fork paths, external MCP secret files, and dependency program destinations. It does not write shell aliases. MCP and rule inventory additionally use [target validation](../../engine-rs/src/native_mcp/targets.rs) to reject unsafe file types, links, and multiply linked files before reading or writing.

## Runtime path

1. Resolve trusted HOME and the registered configuration directory. Reject relative paths, parent traversal, control characters, linked account-directory components, and overlapping main, registered, or manager data directories. Disabled registrations still reserve their locations.
2. Resolve each selected extension destination against its target and protected files. Skill assignments must land in the selected assistant's skill directory; explicit and historical fallback destinations receive the same check. User-owned local skill sources are read in place, while tool-created links remain distinct from their source.
3. Recheck destination and file identity at the resource operation. Existing target content is compared with the status or preview baseline under the write lock. MCP, rules, skills, secrets, and programs apply their own resource-specific validation and rollback.
4. Reject unsafe links, hard links to known identity files, and redirected protected targets. A failed check does not repair historical metadata or move existing assistant files.

Dependency Program plans and every filesystem execution step validate installation, launcher, backup, restore, and cleanup locations. Unsafe cleanup is skipped; unsafe rollback reports the condition and retains its backup. External MCP secret sources cannot be selected from protected assistant or manager data paths.

## Constraints and failure behavior

- Only the selected extension target may change. Other assistants, identity files, sessions, and external source directories remain untouched.
- Existing symlink targets and missing path components are resolved conservatively. Windows junctions and reparse points, Unix links, case aliases on Windows/macOS, and hard-link identity are covered by platform-specific guards.
- Rejected status targets offer no write baseline. Baseline mismatch rejects without overwriting external edits. Conditional rollback cannot make unrelated multi-file operations globally atomic or prevent every concurrent rename by a hostile process with the same OS identity.
- The general launch domain does not use these registrations; its recipe storage and explicit application start are documented in [launch configurations](launch-configurations.md).

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_paths --test native_account_skills
cargo test --manifest-path engine-rs/Cargo.toml --test native_extension_scope --test native_extension_program_scope
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp --test native_rules --test native_secrets
cargo test --manifest-path engine-rs/Cargo.toml
```

Fixtures use isolated HOME/data and test-assistant paths. Negative cases read back protected contents after rejection and cover overlapping directories, linked ancestors, hard links, stale baselines, redirected destinations, and platform path aliases. Native desktop and real-assistant acceptance require separate evidence.
