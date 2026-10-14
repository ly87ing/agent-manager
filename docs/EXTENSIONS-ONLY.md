# Extensions Only

The 2026-10-11 user requirement supersedes earlier account designs: Agent Manager manages MCP, rules, skills, their sources, and their extension dependencies. It must never manage accounts.

The 2026-10-12 user request adds an independent general launch tool: configurable
applications or executables, environment variables, arguments, optional time zones,
and shortcut names. Generating
or copying commands is read-only. Recipes can be saved in Agent Manager without
running the application or changing shell configuration. Application launch is a
separate explicit action. This exception does not enable the retired account or
`aliases` APIs. See
[Launch configurations](subsystems/launch-configurations.md).

## Product boundary

The 2026-10-13 user request adds an independent environment checker for
macOS time, language, region, locale and NTP settings. The desktop and CLI share
`environment:check`; reference-time queries require an explicit action. It reads
no account identity or credentials and changes no system or shell settings.
The follow-up adds named expected-value collections stored in Agent Manager and
authorized one-shot time correction. Only `environment:sync-time --confirm`,
including a collection's explicitly enabled automatic correction during a user
check, may adjust the system clock. It requires a fresh excessive offset and
macOS administrator authorization, then independently checks the result. It
does not alter the time zone, regional preferences, NTP configuration or shell
files, and installs no privileged service. See
[Environment checks](subsystems/environment-checks.md).

Account creation, registration edits, removal, switching, login/logout, account-bound launch commands, credentials, provider/model/runtime settings, and assistant installation/upgrades are retired. Existing account directories, authentication files, sessions, configuration, shell files, and historical launch scripts are preserved. Official assistant tools remain responsible for them.

Historical registrations are immutable location metadata for existing extension targets. Reading a target label does not inspect authentication, account settings, or assistant executables. New account registration is unavailable. The default assistant resource locations remain available.

**Assistants** lists those existing extension targets and their registered configuration,
skill, and MCP paths. It reads metadata only, skips external secret-source loading,
and offers no account or configuration actions. Records do not establish installation
or login status; the page does not discover new accounts or scan assistant directories.

Settings includes **More tips**, a read-only page for historical multi-account and
shell-alias knowledge. It renders bundled guidance and generic `YOUR_*` examples.
Copy buttons write only to the clipboard; there is no account discovery, command
generation or execution, file access, or persistence. This does not restore any
account or alias management operation.

The separate **Launch configurations** tool uses the `launch` domain. It stores
inactive recipes and renders complete commands and alias/function definitions for
manual copying. It does not write `.zshrc`, `.bashrc`, PowerShell profiles, or
`.zprofile`, and it does not restore the historical `aliases` domain.

## Runtime and write constraints

- [Native entry point](../engine-rs/src/native.rs): every `agents`, `accounts`, `profiles`, `profile`, and `aliases` command, including dotted aliases, returns `FEATURE_RETIRED` before loading secrets, preflight, locks, or state creation. Policy writes cannot modify account or alias trees. All remaining policy/import paths compare those trees unchanged.
- [Old controlled launcher](../engine-rs/src/native_account_context.rs): `run_cli` immediately refuses execution without resolving an account, reading its files, or starting an assistant. Existing scripts are left on disk.
- [Desktop bridge](../app/src-tauri/src/engine.rs): its whitelist contains extension operations and the independent general `launch` domain. The resource UI uses `resources:targets` for labels rather than account status queries. Account, historical account-launch, and assistant-diagnostic pages are removed; search only queries extension resources.
- [Backups](../engine-rs/src/native_config.rs): exports omit account and alias definitions; imports preserve local metadata and reject changes. Historical alias sources are skipped without reading or changing shell files or legacy alias rows. Secret values and source paths remain excluded.
- [MCP writes](../engine-rs/src/native_mcp/write.rs): JSON edits splice only the MCP container value, preserving every byte outside it, including login metadata. Missing containers are inserted without reformatting existing fields. Codex uses `toml_edit` and retains non-MCP configuration. Previews, CAS, scope checks, and conditional rollback remain required.
- [Path protection](subsystems/account-path-isolation.md): extension target validation still rejects account identity aliases, cross-target paths, unsafe ancestors, and links to protected files. Regular MCP/rule files with multiple hard links are rejected before reads or writes on Unix and Windows.
- [Dependency programs](subsystems/programs.md): assistant command names and npm packages are excluded at every installation location. Declarations and lifecycle requests refuse them. Plans and each execution step validate all filesystem targets against account boundaries.

Dependency-program metadata and backup descriptions reject file links before writing.
This also protects session files with hard-link aliases outside their account directory.
Backup listing, automatic pruning, and failure rollback check the same account boundary.
Unsafe cleanup is skipped; unsafe rollback reports an error and retains the original backup.
- [MCP secrets](../engine-rs/src/native_secrets.rs): only variables referenced by active or saved MCP entries can be edited. Reserved assistant credentials and variables referenced by historical account metadata are protected and cannot be borrowed by MCP operations. Source changes are blocked when historical account metadata references credentials. Account and application-data files cannot be selected as secret sources. Legacy database secret migration is retired because its contents may include account credentials.

An old custom target without a verifiable credential identity remains read-only for secret writes. The tool refuses the operation and preserves its metadata and source bytes; it does not repair or re-register an account to make an extension operation succeed.

Assistant and provider environment namespaces are reserved using the existing identity-variable guard, including OAuth, AWS, Azure, and Google variables. MCP entries must reference dedicated variable names outside those namespaces.

Unused frontend alias editors and hooks, account runtime drafts, connection presets, authentication command generators and shell selectors, and
account seed/shared-setting builders are removed. Active extension target recognition and grouping remain.

Retired backend account writers, authentication probes, controlled launch rendering,
assistant installers, runtime-setting editors, and shell-alias writers are removed.
Account-named helpers that remain protect extension paths, locks, immutable target
metadata, and MCP credential ownership. They expose no account operation. The old
launcher keeps only an immediate refusal so existing scripts cannot start an assistant.

## Verification

[Scope acceptance](../engine-rs/tests/native_extension_scope.rs) uses temporary HOME/data and fake assistant directories. It verifies retired commands, real CLI extension writes with file readback, immutable registration/import behavior, shared JSON byte preservation, dedicated MCP variables, and identity-file hard links. Existing resource tests retain cross-target, external-change, malformed-path, shared-credential, and rollback cases. Tests whose purpose was successful account mutation are retired; resource fixtures seed historical metadata directly instead of invoking account writers.

Run the focused boundary checks first:

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_extension_scope --test native_account_paths --test native_account_skills --test native_mcp --test native_rules --test native_secrets
cargo test --manifest-path engine-rs/Cargo.toml --test native_extension_program_scope
```

Then run the repository checks:

```sh
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
pnpm --dir app test:unit run
pnpm --dir app typecheck
pnpm --dir app build
./scripts/ci-smoke.sh
```

Browser mock acceptance covers extension navigation, search, and affected pages at 1000 px and 760 px. It establishes frontend behavior, not installed Tauri or real-assistant acceptance. Test/build receipts and remaining failures are recorded in [implementation history](goal-rust-gui.md). No installed application, package, release, or real account environment is changed by this source milestone.
