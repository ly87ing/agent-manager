# Account Path Isolation

## Responsibilities and sources of truth

Independent accounts own their configuration directory and account settings. Main accounts (`~/.claude`, `~/.codex`), other registered accounts, and Agent Manager's data directory are protected boundaries. Account definitions identify the intended directory; filesystem resolution determines its actual destination.

[Profile path guards](../../engine-rs/src/native_profiles/paths.rs) are shared by profile status, previews, apply/adopt, and the profile-owned seed/shared-settings readers and writers. Rules target selection, login probing, and session-root discovery also use the account-directory guard. Identity and authentication configuration paths reject links; intentional shared skill/rule links remain resource-domain responsibilities.

Skill destinations use the same account path guards. Main Claude and Codex targets are fixed to their own `skills` directories; independent accounts use `<config_dir>/skills`, including when `skills_dir` is omitted. An explicit target or historical `skills.agents` fallback must resolve to that destination. Custom assistants may retain separate destinations, but cannot overlap main accounts, registered accounts, manager data, or Codex's shared `~/.agents/skills`. Destination directories reject links and Windows reparse points; individual installed skill links may still point to a shared source.

## Runtime path

1. Resolve trusted HOME, expand the configured directory, and reject relative paths, parent traversal, control characters, and links in every account-directory component below HOME.
2. Canonicalize existing components, including the nearest existing ancestor when several trailing directories do not exist. Dangling links and resolution errors fail closed.
3. Require the resolved directory to remain below resolved HOME and use its canonical spelling for controlled CLI identity. Reject overlapping main-account, other-account, and data directories, including identical paths, parent/child overlaps, and disabled account registrations.
4. Validate known settings and authentication files before reading them or deriving a write baseline. Reject final links and multiply linked identity files. Check protected directory boundaries and protected final files, including main Claude's `HOME/.claude.json`. Existing hard links are compared by filesystem identity (device/inode on Unix; volume/file index on Windows).
5. Apply recomputes its plan under the write lock, then checks the paths again after the external login check and after directory creation. Existing byte-baseline and conditional rollback checks remain in force.

## Constraints and failure behavior

- Unsafe targets are reported as unsupported, with no write baseline or available write action. Rejected operations leave protected file bytes unchanged.
- Skill metadata fails closed when any registered destination is unsafe, so status/discovery and all skill add, collection, adoption and cleanup paths cannot use an unvalidated destination. Saving or importing a changed destination also validates it before writing registration; invalid historical definitions require explicit correction, with no automatic relocation or deletion of existing files.
- Both assistant types are protected, regardless of the selected account's type. The main Claude login record and known authentication files for every account (including the target account) are also protected, even when linked outside the account directory.
- Windows rejects normal configuration-path components ending in a dot or space before directory creation, avoiding Win32 normalization onto a protected name. Windows uses canonical paths without an unnecessary verbatim prefix. Junction regressions create a real junction directly; they do not depend on administrator access or symlink privileges.
- Windows/macOS comparisons conservatively reject case-only aliases, including not-yet-created names. This also rejects case-distinct account paths on case-sensitive Windows/macOS volumes. Linux keeps case-sensitive path comparisons. Canonicalization and file identity checks handle existing aliases.
- These checks close aliasing and stale-plan errors; they do not make multi-file filesystem and database operations atomic or eliminate every adversarial rename race. No claim of native Windows/macOS acceptance follows from a Linux test or a cross-compile.

## Verification

Run the focused regressions, the existing profile suite, and the engine suite:

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_profiles account_isolation
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_paths
cargo test --manifest-path engine-rs/Cargo.toml --test native_account_skills
cargo test --manifest-path engine-rs/Cargo.toml --test native_profiles
cargo test --manifest-path engine-rs/Cargo.toml
```

The fixtures use isolated HOME/data directories. Negative tests read back the original protected contents and cover ancestor aliases, multiple missing components, data/other-account collisions, file links, redirected main-account dotfiles, hard links, and a link changed after status. Windows/macOS additionally exercise case aliases; the portable ancestor tests use Windows junctions on Windows.
