# Claude Local Data Maintenance

## Responsibilities and sources of truth

Settings → Advanced → Claude local data maintenance is an opt-in destructive tool.
The Advanced section starts collapsed. Opening the page performs no inspection.
It inspects the current user's default `.claude` directory, `.claude.json` and
adjacent `.claude.json.*` names. Only recognized automatic backups join deletion;
other copies remain visible for review. It does not discover accounts or restore
the retired account APIs.

The user authorized local session and account-data cleanup on 2026-10-23 and
subsequently included telemetry configuration in deletion. This supersedes the
earlier preservation request. A developer must never exercise this feature
against personal data as acceptance. The live filesystem and native
credential metadata are authoritative; previews are not persisted.

The user subsequently expanded the target to all local Claude-related session
copies and account remnants through Agent Manager, while retaining project code,
documents, worktrees and shared resources. That target is not an implemented
state. Whole-data deletion covers the default directory, recognized configuration
backups and credentials. Separate per-item connection maintenance also covers explicitly
selected user settings and supported startup/user-environment items;
the reference catalog's other sources need explicit discovery and safe adapters.
Orca's search index now has a separate opt-in read-only inspection; it cannot yet
be cleaned. The page explicitly keeps the all-source goal incomplete.

| Constraint | Source | Implementation consequence |
| --- | --- | --- |
| Remove local Claude sessions and account remnants | User request, 2026-10-23 | Discover each source, require a fresh preview and independently read back selected cleanup |
| Preserve project code, documents and worktrees | User request and reference C07 | Never infer ownership from an absent VCS marker; retain unknown content |
| Delete Claude telemetry configuration too | Latest user clarification | Remove exact local configuration files, without following or executing their external references |
| Preserve shared caller dependencies | Project/shared-resource preservation constraint | Compare generic runtime certificate/key dependencies with the actual deletion-target union |
| Keep shared and other-assistant data | Reference C35–C46 | Use exact source identities and supported schemas, not keyword deletion or whole mixed stores |
| Use Agent Manager as the entry point | User request | Show per-source results and remaining work in the app; no external cleanup-script prerequisite |
| Close Claude sessions before deleting session data | User request, 2026-10-25 | Stop previewed current-user Claude processes and verify their exit before deleting credentials or files |
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

### Targeted connection maintenance

The 2026-10-30 request authorizes inspection followed by individually confirmed
removal, including shared startup files, user environment and custom Claude user
directories. This is separate from whole-data deletion below. Opening the panel
does not inspect anything, select fields or execute helpers/scripts.

- `claude-data:inspect-connections [--connection-root /YOUR_CLAUDE_CONFIG_DIR]`
  reports field names, source paths, per-item editability and unavailable evidence,
  never values. It observes `settings.json` and `settings.local.json` in the selected
  user directory. The latter is an observed carrier, not proof of global precedence.
  Fields include `ANTHROPIC_*`, uppercase/lowercase proxy variables, `CLAUDE_CONFIG_DIR`,
  `PROXY_MANAGED` and the root `apiKeyHelper`. Presence does not prove a proxy remnant:
  ordinary API credentials and legitimate helper configuration can use these names.
- Unix lexical inspection covers HOME `.zshrc`, `.zprofile`, `.zshenv`, `.bashrc`,
  `.bash_profile` and `.profile`, without sourcing them. Only standalone literal
  Claude-named assignments in conservatively simple files can be removed. Compounds,
  arrays, arithmetic, multiline/continued constructs, substitutions and uncertain
  references remain read-only. Removal leaves a comment and the original comment,
  line ending and other lines. This does not discover sourced files or ZDOTDIR.
- macOS inspects named values in the current Aqua login domain. It queries the
  fixed catalog and names discovered in selected files/caller environment; it does
  not enumerate every launchd variable. A variable is session-scoped, not persistent
  across logout. Current processes are observations only and are never normalized.
  Windows observes fixed PowerShell profile locations and fixed user/machine registry
  names; these do not cover custom/OneDrive/all-host profiles. Linux/Windows targeted
  writes are unavailable pending native publication and acceptance.
- `claude-data:preview --connection-items '["settings:ANTHROPIC_API_KEY"]'`
  returns a fresh SHA-256 base for exactly one item. `settings:root:apiKeyHelper`,
  `shell:.zshrc:3:ANTHROPIC_BASE_URL` and `user-env:ANTHROPIC_AUTH_TOKEN` identify the
  other supported scopes. Forward the same optional `--connection-root` through
  inspection, preview and clear. `claude-data:clear --connection-items JSON --base SHA256 --confirm`
  removes only that item. Empty/unknown/malformed selections or mixed full-cleanup
  arguments refuse; they never fall through to whole-data deletion.
- File values, identities, parent identities and shared caller dependencies bind
  freshness. Duplicate root/env JSON keys or malformed data refuse, without an empty
  object fallback. Unselected JSON values retain their raw representations; other
  file regions remain unchanged for env removal. Only supported root fields can be
  removed. Redirected/hard-linked files, protected default/resolved `CODEX_HOME`
  locations, Agent Manager data, project ancestors and worktree containers refuse.
- Recognized Claude and CC Switch processes must already be closed by the user.
  This mode never stops them. Unknown process evidence blocks removal. Cancellation
  and preview are zero-write; confirmed attempts record a nonsecret selected-item
  intent/completion audit. Existing whole-data deletion keeps its separate stop phase.
- macOS publication clones from a bound source handle into a fresh private staging
  directory, clears inherited directory ACLs and verifies mode 0700 before copying.
  It retains source ownership/mode/ACL/flags/xattrs and uses native rename-swap to
  inspect the displaced file. Observed external changes stop or restore guarded
  publication. Unknown displaced/replaced versions remain in a reported private
  recovery location, never silently deleted or reported complete. Successful cleanup
  leaves no extra credential copy; it is not secure erase or an OS-wide transaction.
- Shared assignments and login variables require `REMOVE SHARED` in the GUI;
  settings require `REMOVE`. Shared `ANTHROPIC_*` and `CLAUDE_CONFIG_DIR` changes affect
  every future consumer, potentially including custom Codex launch dependencies.
  Codex files are neither read nor modified. General shared proxy variables remain
  read-only; the same variables inside selected Claude settings can be removed.
  launchctl has no compare-and-swap: the selected value/scope is re-read immediately
  before unset and independently checked afterwards, but uncoordinated future writers
  can still race or recreate it. No complete absence or other-consumer independence
  is inferred. Restart and verify the target application yourself.

The [environment variable reference](https://code.claude.com/docs/en/env-vars),
[network configuration](https://code.claude.com/docs/en/network-config) and
[settings reference](https://code.claude.com/docs/en/settings) establish the field
meanings and inheritance limits. The supplied scripts are review inputs only:
their Codex reset, whole-object fallback and raw-value output are not executed.

### Whole default-data deletion

1. `claude-data:preview [--files-only]` reads bounded no-follow evidence and hashes
   selected file contents/identities. It returns category counts, paths, credential counts,
   telemetry policy, blockers and a SHA-256 base, never credential or session
   values. Preview and cancellation create neither a database nor an audit record.
   `configuration_paths` lists each selected configuration and backup, including
   default-directory `keybindings.json` and verified `orca-agent-view.json`, bounded to
   1,024 paths and 256 KiB of path bytes.
   Retained project and unclassified paths have explicit reasons and are excluded
   from deletion counts; unknown ownership remains visible in the result.
   `process_ids` lists recognized processes to close, or NULL on unavailable
   inspection. Native incarnation identities
   join the base; command lines never enter output.
2. `claude-data:clear --base SHA256 --confirm [--files-only]` re-reads the same
   scope, requires an unchanged base and no blockers, acquires the existing apply
   lock, and revalidates before creating a nonsecret intent audit.
3. It stops the previewed Claude processes, waits up to three seconds for graceful
   exit, then force-stops surviving bound processes and waits up to three seconds
   for verified exit. Windows uses forced termination after the initial wait.
   macOS signals audit tokens, Linux uses pidfds, and Windows verifies the current
   user token and creation time on held process handles. It never signals a bare
   PID, closes an IDE/shell, or terminates shared MCPs by keyword.
   Newly discovered processes stop execution; they are not automatically added.
4. After independent zero-active-process inspection, it compares all original
   filesystem, credential and caller-dependency evidence, excluding the expected
   process disappearance. Shutdown writes invalidate the data preview: no data is
   deleted, stopped-process progress is returned, and another preview is required.
   It then removes previewed credentials, exact configuration files including telemetry,
   then unlinks previewed leaves and empty directories through held capabilities.
   Every file is re-read; parent identities and directory emptiness are checked.
   There is no recursive pathname removal and no automatic sensitive backup.
5. Phase records and returned verified counts retain partial progress on failure.
   `stopped_processes`, `forced_processes` and `processes_stopped` describe the
   stop phase independently from deletion counts. Forced counts include only
   processes signaled forcibly and independently confirmed exited. Complete results require the
   stop phase, guarded deletion and final independent readback.
   Independent final inspection must find no selected data or credentials before
   reporting complete. Any attempt requires a
   fresh preview before retrying.
   Returned retained paths come from independent final inspection. The
   `retained_verified` flag is false when only preview evidence remains available;
   `scope_verified` comes from that same independent inspection, even after a
   partial execution. Complete results require both flags and no unclassified
   content. Preview and result require `telemetry_policy:"remove"`; the v5 base
   binds stop-before-delete semantics and rejects older previews.

## Orca search-index inspection

`claude-data:orca-search-preview` inspects only the selected current-user search
database. It accepts no path or confirmation argument. The default path is
`Library/Application Support/orca/ai-vault/session-search.sqlite` below HOME on
macOS, `orca/ai-vault/session-search.sqlite` below APPDATA on Windows, and below
XDG_CONFIG_HOME (or HOME/.config) on Linux. An absolute `ORCA_USER_DATA_PATH`
selects the caller's override instead; the preview identifies that origin.
Default absence or an override never establishes other-profile absence.

The adapter's layout and exact `agent='claude'` identifier were read from Orca
1.4.237's installed scanner and SQLite support modules. Only the schema-6 column
layout and supported ownership keys are counted. Integer session-row references
join messages and file cursors; shared session IDs and keyword matches are not
ownership evidence. Dangling, NULL and non-integer references remain unattributed.
Output contains counts and the database path, never session IDs or content.

The desktop/CLI parent resolves the selected path and runs the packaged native
`agent-manager-claude-data-helper`. Only that process opens source handles: a
rejected hard-link alias must not release concurrent Manager SQLite locks.
The desktop registers its resource path; the CLI uses the adjacent executable.
Build all CLI components with `cargo build --manifest-path engine-rs/Cargo.toml --bins --locked`.
Tauri's build/dev hooks build and copy the helper on each native platform.

The helper supports only the exact inspection operation and database leaf. It
does not enter the general dispatcher, initialize Manager storage, load
credentials or elevate authority. It is a separate process, not an OS sandbox.
The child inherits only OS loader/temp essentials. The parent uses an owned
process group/Job with a twenty-second deadline, and accepts less
than 4 KiB of strictly typed/versioned count output. Missing, redirected,
incompatible, malformed or timed-out components leave inspection unavailable;
there is no in-process fallback. Opening the page still starts no inspection.
Inspector initialization treats an uncertain resource directory as unavailable
inspection and disables the CLI adjacency fallback for that desktop process.
On macOS this preserves startup under the framework's executable-path refusal.
Windows/Linux clock-helper initialization has its separate existing contract.
macOS artifact acceptance validates package identity and signature before running
the copied component. The component verifier also requires its strict signature
check to pass before launching the Rust fixture.

Inside the helper, SQLite only receives a bounded memory copy, never the source
pathname. The adapter holds ordinary ancestor directories, refuses aliases, hard links and any
WAL/SHM/journal entry, and rechecks file evidence and ancestor identities.
Unrelated parent contents may change without invalidating the binding;
directory timestamps, length and link count are not identity evidence. No-follow
opens and the final native-identity rewalk still reject observed replacements.
A 256 MiB image bound and three-second SQL progress deadline refuse oversized or
expensive input. These are inspection limits, not a transactional writer lock.
The snapshot is an observed standalone main-file copy; active writers can make
it unavailable or stale. No application is terminated.

For a closed WAL-format main file without sidecars, only bytes 18/19 of the
memory copy change from 2/2 to 1/1, as documented by
[SQLite deserialize](https://www.sqlite.org/c3ref/deserialize.html). The allocation
includes its recommended 20-byte padding, transfers ownership with FREEONCLOSE,
and uses READONLY. Temporary storage stays in memory. Source bytes, journal mode,
credentials, the Agent Manager database and audit remain unchanged.

Inspection always returns `cleanup_supported:false` and `scope_complete:false`.
Unchecked results contain NULL counts, never inferred zeros. Neither zero owned
rows nor successful inspection establishes removal: ordinary FTS5 deletes can
leave old index content, as documented by
[SQLite FTS5](https://www.sqlite.org/fts5.html#the_secure_delete_configuration_option).
Confirmed deletion still requires writer checks, exact schema validation,
FTS-content removal and independent preservation/readback. Other Orca stores,
custom profiles and the remaining catalog sources are not covered by this probe.

### SQLite write-safety research

The separate `sqlite_maintenance_probe` test executable exercises synthetic
databases only. Its normal build verifies that FILESTAT diagnostics and the
`sqlite_filestat` SQL builtin remain unavailable. Native CI explicitly builds an
experimental variant with `SQLITE_ENABLE_FILESTAT` in a separate target directory;
that step's flags do not configure application builds.

The experimental variant compares the actual SQLite handle with a held file,
rejects a replaced database before writes, and uses a second process to test
writer exclusion across transaction commit, VACUUM and checkpoint. Unix also
has a fault control: closing an extra raw database FD releases the process's
POSIX locks and permits the second writer. The safe path keeps raw FDs alive
until after SQLite closes. The dependency is pinned to SQLite 3.53.2 for this
capability experiment; an upgrade requires a new protocol review.

The Unix diagnostic variant also reproduces that loss through the actual
default VFS: a synthetic WAL pathname is replaced with a hard link to the main
file, its opened native identity is verified, then `xClose` releases the main
connection's locks. The original WAL and single-link main binding are restored;
the connection identity still matches, while a second-process writer can enter.
This is a fault control, not a safe rejection protocol. It does not implement
process isolation or prevent later writes after such a failure.

A second [synthetic schema-6 fixture](../../engine-rs/tests/support/sqlite_fts_maintenance.rs)
exercises exact-agent deletion and FTS
rebuild in one transaction, followed by VACUUM and TRUNCATE checkpoint. It checks
selected row keys independently, and preserves non-target storage classes/bytes,
explicit IDs, token positions, schema, FTS configuration, index incarnation and
an AUTOINCREMENT sequence larger than the remaining maximum ID. Orphans remain
retained. A VACUUM-without-rebuild control removes the freed uppercase body but
leaves a unique lowercase token in FTS data blocks. The complete maintenance and
zero-owned-row retry must remove both. Copied-fixture mutants omitting rebuild or
message deletion fail their specific assertions.
This FTS fixture runs in both the normal production-flag test build and the
separate diagnostic build; FTS maintenance does not depend on FILESTAT.

These experiments do not establish safe sidecar/pathname writes or support for
unknown schemas/configurations. Those boundaries and independent source readback
still require a confirmed cleanup adapter. No production diagnostic flag or
Orca deletion entry point is enabled by these research fixtures.

## Constraints and failure behavior

- Native process inspection remains bounded. Windows PowerShell/CIM has a
  thirty-second deadline; macOS process commands have ten seconds. Timeout is
  reported without command lines and always blocks cleanup.
  Windows inspects candidate processes across logon sessions and checks their user
  tokens before classification. Positively identified other-user processes are
  excluded; unavailable ownership blocks cleanup. Ambiguous Node/Bun launch arguments
  block cleanup. Windows runtime-script matching ignores ASCII case; Unix matching
  preserves case. Known exited zombies are excluded; parent shells are retained.
  Automatic restart services and custom launch wrappers remain outside proven stop
  coverage. Cross-logon-session Windows runtime acceptance remains unverified.
  A later writer still stops guarded deletion.

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
- The exact default-root configuration carriers (`settings.json`,
  `settings.local.json`, `managed-settings.json`, `.claude.json`,
  `remote-settings.json`, `keybindings.json` and `orca-agent-view.json`) and global `~/.claude.json` are deleted whole after
  ordinary-file, single-link and unchanged-byte checks. Unknown fields, duplicate
  JSON keys and damaged JSON do not require interpretation for ordinary whole-file
  removal; the additional Orca ownership requirement is described below.
  Values never enter preview, result or audit output. Helpers are never executed;
  referenced external certificates, keys and scripts are never followed for deletion.
- The Orca Claude shortcut marker additionally requires at most 64 KiB of valid
  JSON with exactly one `owner` field equal to `stablyai/orca/claude-agent-view`.
  Ownership is parsed from the same bounded bytes used for the file fingerprint;
  missing, foreign, duplicate or invalid ownership refuses before writes. The
  installed Orca profile worker defines that filename/owner and its association
  with `keybindings.json`. Confirmed whole-data cleanup removes both selected
  default-directory carriers, not other Orca profiles/preferences or Codex files.
  Quit Orca before cleanup; enabling its integration again can recreate these files.
  Checking only creates a preview. The user must then choose Clear selected scope,
  type `DELETE` and confirm before processes stop or deletion starts.
- Automatic backup recognition accepts nonempty decimal timestamp suffixes for
  `~/.claude/backups/.claude.json.backup.<timestamp>`,
  `~/.claude/backups/.claude.json.corrupted.<timestamp>` and HOME-adjacent
  `.claude.json.backup.<timestamp>`. Decimal matching is a conservative tool
  policy, not a claim that every historical backup uses that format. The
  [official recovery instructions](https://code.claude.com/docs/en/settings#fix-a-broken-settings-file)
  describe the default backup directory; the
  [official sensitive-file example](https://github.com/anthropics/claude-code/blob/main/examples/settings/settings-hipaa.json)
  also lists HOME-adjacent backup copies.
  Root backups stay on the ordinary held-ancestor deletion path; HOME backups use
  the held HOME configuration path. Both locations check single-link evidence.
  A mixed backup directory with unknown source/documents stays protected as a
  whole. Adjacent unclassified copies remain listed and prevent complete status.
  HOME enumeration is sorted and bounded to 100,000 entries. Undisplayable
  candidates refuse inspection rather than being silently skipped or serialized.
- Shared caller file dependencies from `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`,
  `SSL_CERT_DIR` and generic OTel certificate/key variables are compared with the
  selected deletion targets, including resolved aliases. Certificate-directory
  lists use native separators and protect all selected descendants. A match,
  unresolved alias, empty directory entry or uncertain relative dependency refuses
  cleanup. The directory-list semantics follow the
  [OpenSSL environment reference](https://docs.openssl.org/3.0/man7/openssl-env/). This protects shared resources without
  retaining Claude's own saved telemetry configuration. The caller environment
  and shell files remain unchanged; no claim is made that organization reporting
  or every possible future launcher has been disabled.
- The tool preserves project files, shell configuration, external symlink targets,
  external repositories, managed system policy and Agent Manager data. It does not
  inspect every external policy, project or arbitrary launcher dependency; the
  guarantee covers the selected local scope and observable shared caller file
  dependencies, not every possible future launch.
- Redirected roots/settings, unsupported objects and nested mounts are refused. Linux uses held-handle mount IDs, including
  bind mounts. Windows uses native deletion handles. No configuration replacement
  or sensitive backup is created by the whole-file deletion path.
  Windows rechecks ordinary-file link counts on the held deletion handle before
  disposition; a late hard-link alias stops deletion without removing either name.
- Previewed Claude processes are stopped before deletion. New active processes
  block deletion and are checked again at phase and directory boundaries.
  Other writers cannot be locked transactionally by
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
`node scripts/verify-native-claude-connections.mjs` exercises actual targeted
inspection, cancellation, invalid/stale requests, native field/root-helper/startup
publication, custom/protected paths and preservation of other settings, sessions,
credentials and Codex sentinels. Run after Cargo tests. On macOS, explicitly setting
`AM_TEST_CLAUDE_SESSION=1` also creates and clears one random `ANTHROPIC_AM_TEST_*`
login variable, never a personal variable. The fixture retains its owned files and
prints their location for receipt review. No credential-store query is needed.
They exercise actual deletion/readback, whole-file telemetry removal, external link
preservation, stale previews, changed selected files, initial/late backup aliases
and new files mid-cleanup.
`node scripts/verify-native-claude-data.mjs` runs the real built CLI against
synthetic files and owned native-name/Node-script processes. Before every confirmed
clear it refuses any previewed process not owned by the fixture. It checks
cancellation/staleness without stopping, graceful and forced exit before deletion,
and shutdown flushes that require a new preview with zero deletions. Unit-test
dispatch refuses discovered real targets; its native stop test binds only children
it created and verifies an unrelated child remains running.
Run the real CLI fixture after the Cargo suite, never concurrently: both inspect
the same operating-system user's processes despite their separate HOME directories.
Frontend tests exercise cancellation, typed confirmation, scope invalidation,
partial results and fresh-preview requirements. Mock UI checks do not establish
native credential behavior. Linux native fixtures exercise undisplayable file
names; APFS rejects those names during construction, so macOS cannot establish
that case.

The former telemetry projector and configuration-replacement helpers are retired.
Their projection-only assertions do not describe the current policy. Applicable
changed-file, external-link, read-only/junction and late-hard-link safety cases
exercise actual deletion; telemetry removal and shared dependencies have separate
current-policy fixtures.

The ignored private-Keychain fixture is invoked explicitly on disposable macOS
CI hosts only. It creates synthetic credentials in its own Keychain, checks that
unrelated items remain unchanged, deletes only its previewed persistent reference,
removes the fixture Keychain and verifies that the original search list remains.
It never runs against personal login items. Native CI and installed-client
receipts remain separate in [remaining work](../REMAINING-WORK.md).
