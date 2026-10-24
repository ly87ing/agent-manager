# Claude Local Data Maintenance

## Responsibilities and sources of truth

Settings → Advanced → Claude local data maintenance is an opt-in destructive tool.
The Advanced section starts collapsed. Opening the page performs no inspection.
It targets only the current user's default `.claude` directory and `.claude.json`
file. It does not discover accounts or restore the retired account APIs.

The user authorized local session and account-data cleanup on 2026-10-23, while
requiring telemetry configuration to remain. A developer must never exercise this
feature against personal data as acceptance. The live filesystem and native
credential metadata are authoritative; previews are not persisted.

The user subsequently expanded the target to all local Claude-related session
copies and account remnants through Agent Manager, while retaining project code,
documents, worktrees and shared resources. That target is not an implemented
state. The current executable scope remains the default directory and credentials;
the reference catalog's other sources need explicit discovery and safe adapters.

| Constraint | Source | Implementation consequence |
| --- | --- | --- |
| Remove local Claude sessions and account remnants | User request, 2026-10-23 | Discover each source, require a fresh preview and independently read back selected cleanup |
| Preserve project code, documents and worktrees | User request and reference C07 | Never infer ownership from an absent VCS marker; retain unknown content |
| Keep telemetry configuration and dependencies | User request | Retain values, reject uncertain dependencies; broader adapters need a deletion-target union check |
| Keep shared and other-assistant data | Reference C35–C46 | Use exact source identities and supported schemas, not keyword deletion or whole mixed stores |
| Use Agent Manager as the entry point | User request | Show per-source results and remaining work in the app; no external cleanup-script prerequisite |
| Keep development isolated | Repository contract | No real-session deletion, credential probing, app termination or permission changes for tests |

Primary sources:

- [Claude directory](https://code.claude.com/docs/en/claude-directory) documents
  transcripts, memory, file recovery, cached policy and configuration locations.
- [Credential management](https://code.claude.com/docs/en/authentication#credential-management)
  documents macOS Keychain and file-based Linux/Windows credentials.
- [Settings](https://code.claude.com/docs/en/settings) and
  [monitoring](https://code.claude.com/docs/en/monitoring-usage) define settings
  precedence and telemetry controls.
- Public protocol identity was checked against the official npm native package
  [`@anthropic-ai/claude-code-darwin-x64@2.1.310`](https://registry.npmjs.org/@anthropic-ai/claude-code-darwin-x64/2.1.310).
  Its verified SHA-512 integrity is
  `7httDzvIQv+LZF7cSdTu6/hCeOVEbUKmeQ1D1yv0OV1cWH6yjthUucbSFmQKKCUjXmKyI2iomunw+W9Rvbyo5g==`.
  No downloaded implementation or leaked source is included in this repository.
- Apple's installed `SecItem.h` documents macOS persistent-reference matching
  through `kSecMatchItemList`; the adapter never requests password data.

## Runtime path

The desktop and CLI enter [native_claude_data](../../engine-rs/src/native_claude_data/mod.rs)
before external secret-source loading, preflight, or general database access.

1. `claude-data:preview [--files-only]` reads bounded no-follow evidence and hashes
   file contents/identities. It returns category counts, paths, credential counts,
   preserved-field counts, blockers and a SHA-256 base, never credential or session
   values. Preview and cancellation create neither a database nor an audit record.
   Retained project and unclassified paths have explicit reasons and are excluded
   from deletion counts; unknown ownership remains visible in the result.
2. `claude-data:clear --base SHA256 --confirm [--files-only]` re-reads the same
   scope, requires an unchanged base and no blockers, acquires the existing apply
   lock, and revalidates before creating a nonsecret intent audit.
3. It removes previewed credentials, projects telemetry-bearing configuration,
   then unlinks previewed leaves and empty directories through held capabilities.
   Every file is re-read; parent identities and directory emptiness are checked.
   There is no recursive pathname removal and no automatic sensitive backup.
4. Phase records and returned verified counts retain partial progress on failure.
   Independent final inspection must find only retained telemetry configuration
   and no selected credentials before reporting complete. Any attempt requires a
   fresh preview before retrying.
   Returned retained paths come from independent final inspection. The
   `retained_verified` flag is false when only preview evidence remains available;
   complete results require verified retained paths and no unclassified content.

## Constraints and failure behavior

- Native process inspection remains bounded. Windows PowerShell/CIM has a
  thirty-second deadline; macOS process commands have ten seconds. Timeout is
  reported without command lines and always blocks cleanup.

- The default production namespace is the first supported scope. Custom config,
  secure-storage or OAuth namespace variables block macOS credential inspection;
  files-only cleanup remains explicitly available. Historical usernames, custom
  namespaces and shared SDK/cloud profiles are not searched or guessed.
- macOS matches Generic Password items for the current verified native user and
  exact services `Claude Code`, `Claude Code-credentials`, and
  `Claude Code-device-keys`. The last contains device authentication shared with
  Cowork. Matching is non-synchronizable and deletion uses individual persistent
  references after metadata freshness checks. Locked/denied/unknown inspection
  blocks credential cleanup. An isolated HOME cannot inspect the real Keychain.
  The original opaque Keychain search list remains bound through reference
  readback, deletion and final readback; the OS search list is never changed.
- Windows and Linux credentials are ordinary files in the selected directory;
  no Windows Credential Manager sweep runs.
- Preserved JSON fields include reporting/privacy switches, OTel variables,
  tracing controls, feedback configuration and transport dependencies. Mixed
  objects lose other fields; retained values remain unchanged. Already retained
  files remain byte-identical. Headers and other telemetry secrets never enter
  preview, result or audit output. No new opt-out/opt-in variable is written.
  Shared telemetry TLS authentication, CA-store selection and compression controls
  remain too. Unknown Claude-owned runtime controls or Node options in saved
  settings block cleanup until their telemetry role is classified; known OAuth,
  setup-token and provider selectors remain removable account/runtime data.
- Ambiguous JSON, unknown telemetry controls, opaque headers helpers, or known
  file dependencies inside the cleanup root block deletion. Caller environment
  dependencies are checked without changing them. A cached organization policy
  with telemetry settings blocks cleanup rather than rewriting that policy.
- The tool preserves project files, shell configuration, external symlink targets,
  external repositories, managed system policy and Agent Manager data. It does not
  inspect every external policy, project or arbitrary launcher dependency; the
  guarantee covers the explicit configuration in the selected local scope and
  the caller's observable telemetry environment, not every possible future launch.
- Redirected roots/settings, unsupported objects, nested mounts and unsupported
  replacement metadata are refused. Linux uses held-handle mount IDs, including
  bind mounts. macOS replacement copies native metadata; Linux refuses telemetry
  replacement when extended metadata cannot be preserved. Windows uses native
  deletion handles and replacement that retains the destination ACL.
- Known active Claude processes block execution and are checked again at phase
  and directory boundaries. Other writers cannot be locked transactionally by
  Agent Manager. New/changed observed entries stop cleanup; unknown final results
  remain partial. There is no claim of an atomic filesystem/Keychain transaction,
  secure erase, cloud deletion, anonymous identity or remote-token revocation.
- Whole worktree containers and directories with VCS or project-manifest markers
  are retained without traversing their source. Only recognized artifact layouts
  are eligible; opaque directories or unclassified files remain protected, together
  with their parents. An unclassified file at the cleanup root refuses the operation
  before writes. Source classification is conservative and may leave review items.
  Retained-path output is bounded to 1,024 paths and 256 KiB of path bytes.
- Every selected leaf rechecks the root and ancestor project markers. A newly
  observed project stops cleanup, retaining earlier verified progress. This narrows
  the observation window; it does not turn concurrent filesystem writes into a
  transaction. Unknown retained content prevents an overall complete result.
- Deleting local credentials may require login and device pairing again. A partial
  result is not undone automatically. Application rollback cannot recover deleted
  data. Audit intent surviving without completion means inspect before retrying.

## Verification

Run the focused suite first, followed by the complete repository checks:

```sh
cargo test --manifest-path engine-rs/Cargo.toml native_claude_data --lib
cargo test --manifest-path engine-rs/Cargo.toml
cargo test --manifest-path app/src-tauri/Cargo.toml
pnpm --dir app test:unit run
pnpm --dir app build
```

Filesystem fixtures use synthetic HOME/data locations and files-only mode.
They exercise actual deletion/readback, telemetry projection, external link
preservation, stale previews, changed staging files and new files mid-cleanup.
Frontend tests exercise cancellation, typed confirmation, scope invalidation,
partial results and fresh-preview requirements. Mock UI checks do not establish
native credential behavior.

The ignored private-Keychain fixture is invoked explicitly on disposable macOS
CI hosts only. It creates synthetic credentials in its own Keychain, checks that
unrelated items remain unchanged, deletes only its previewed persistent reference,
removes the fixture Keychain and verifies that the original search list remains.
It never runs against personal login items. Native CI and installed-client
receipts remain separate in [remaining work](../REMAINING-WORK.md).
