# Claude Cleanup Coverage

The requested all-source cleanup is incomplete. This maps all **64** catalog IDs
(55 default and 9 separately listed sources) from the reviewed
[reference checklist](https://github.com/ly87ing/claude-macos-cleanup/blob/9c0acbca4a04c6c88c11773f03b10e31fab2c2d0/docs/cleanup-checklist.md)
to the current Agent Manager cleanup adapter. Adapter status does not establish
presence or absence on any user's machine. Current execution and preservation
rules are in [Claude local data](claude-local-data.md).

## Implemented scope and format evidence

| ID | Source | Adapter state and remaining boundary |
| --- | --- | --- |
| C06 | Active sessions and writers | Guard only: known active Claude writers block deletion; the tool does not terminate sessions. |
| C07 | Default global data directory | Partial: recognized artifacts, exact configuration carriers and local telemetry are selected; projects and unknown content stay protected. |
| C08 | Global state and configuration backups | Partial: default state plus recognized decimal-timestamp backups in the default backup directory and HOME; other copies remain unclassified or outside supported locations. |
| C12 | Code Keychain credentials | Partial: three exact current-user macOS default services; custom and historical namespaces are not integrated. |
| C13 | Credential-file fallbacks | Partial: default-directory credential files; custom roots and external copies are not integrated. |
| C37 | Session Viewer state and watermarks | Not integrated: the installed binary exposes names, but its current mixed-state/migration schema is not verified. |
| C38 | Orca full-text index and cursors | Read-only inspection: schema-6 selected main-file snapshot, exact-agent counts and unattributed records. Cleanup, old FTS content and other profiles remain unverified. |
| C39 | Orca parse caches | Not integrated: current Orca also uses SQLite schema 4 and scope_cwds version 1; the older JSON input alone is insufficient. |
| O09 | Independent Cowork resource candidates | Unverified candidate: no fixed path or independent resource identity is established. |

## Sources without cleanup adapters

| ID | Source |
| --- | --- |
| C01 | Native CLI installations |
| C02 | Homebrew CLI installations |
| C03 | npm CLI installations |
| C04 | Bun/pnpm CLI installations |
| C05 | Dedicated launch wrappers |
| C09 | Custom configuration/account directories |
| C10 | XDG and legacy cache/state directories |
| C11 | Organization-managed configuration |
| C14 | External authentication sources |
| C15 | Shell startup files and completions |
| C16 | Active launch/process environment |
| C17 | Dedicated automatic-start entries |
| C18 | Shell command history |
| C19 | npm cache and logs |
| C20 | Other package-manager caches |
| C21 | External plugin caches |
| C22 | External diagnostic logs |
| C23 | Temporary artifacts |
| C24 | Downloaded installers and archives |
| C25 | Crash and diagnostic reports |
| C26 | Retired-script caches |
| C27 | VS Code extensions and dedicated storage |
| C28 | VS Code settings |
| C29 | VS Code global state and backups |
| C30 | VS Code workspace state and backups |
| C31 | JetBrains plugins |
| C32 | Browser extensions and dedicated storage |
| C33 | User native-messaging manifests |
| C34 | System native-messaging manifests |
| C35 | Agent Manager active records |
| C36 | Agent Manager copies and exports |
| C40 | Orca recovery databases and backups |
| C41 | Orca JSON recovery copies |
| C42 | Orca orchestration records |
| C43 | Orca terminal history |
| C44 | CC Switch active records |
| C45 | CC Switch backups and exports |
| C46 | Claude Code Router stores and exports |
| C47 | macOS privacy permissions |
| C48 | macOS local-network state |
| C49 | Network-configuration backups |
| C50 | LaunchServices registrations |
| C51 | Default URL handlers |
| C52 | System storage-size cache |
| C53 | Cleanup-tool backups |
| C54 | Trash copies |
| C55 | Manual copies and sync targets |
| O01 | Desktop installation |
| O02 | Desktop main data |
| O03 | Desktop network storage |
| O04 | Desktop caches and preferences |
| O05 | Desktop logs and recent documents |
| O06 | Desktop credentials |
| O07 | Browser site data/history/recovery |
| O08 | External backups and snapshots |

Project source, documents, uncommitted worktrees, external skill targets,
other-assistant data and shared support code stay protected across these sources.
The latest user clarification includes Claude telemetry configuration in deletion.
Remote accounts, subscriptions and server-side token revocation are separate from
local-data cleanup.

A source may be reported complete only after exact ownership, a supported format,
quiescent writers, fresh preview/confirmation, target readback and non-target
preservation are verified. Unknown schemas or orphan ownership remain open;
absence of a default directory does not close the rest of this table.
