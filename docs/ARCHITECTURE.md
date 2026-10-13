# Current Architecture

Agent Manager is a Tauri 2 desktop interface over a shared native Rust engine.
The desktop bridge and Rust CLI use the same dispatcher and validation. The
application does not depend on Python, legacy adapters, or its source checkout.
Product scope is governed by [Extensions only](EXTENSIONS-ONLY.md).

## Product boundary

The primary pages are Skills, MCP, Rules, Sources, Dependency Programs, and
Assistants. Assistants lists immutable extension-target metadata only: type,
display name, and extension paths. It does not inspect login identity or invoke
assistant executables. Account, profile, historical alias, account launch, authentication and
assistant lifecycle operations are rejected before secret loading, preflight,
locks, or state creation. Historical registrations remain local and immutable.
Multi-account and alias examples in More Tips are static text with clipboard
copy only.

Settings → Launch configurations uses the independent `launch` domain to build
general commands, preview and confirm local shortcut writes, and explicitly launch
graphical applications. It never resolves account identity or credentials. The
desktop uses the shared Rust renderer; actual shell files and PowerShell scripts
are authoritative. See [launch configurations](subsystems/launch-configurations.md).

Dependency Programs manages programs explicitly required by skills or MCP.
Assistant packages and command names are excluded, and filesystem mutation
paths cannot overlap protected account or manager identity data. See
[programs](subsystems/programs.md) and [account context](subsystems/account-context.md).

## Configuration and storage

Actual assistant files and skill links are authoritative. `native-manager.db`
stores preferences, recognition definitions, source registrations, assignments,
ownership digests, inactive alternatives, and sanitized operation history.
Queries, previews, cancellation, and diagnostics do not apply resources. Saving
a source or alternative does not install software or activate it.

MCP operations modify one entry for one assistant, checking the original digest
under the write lock. Unmodeled fields and each target's own literal values are
preserved. JSON writes preserve bytes outside the MCP container. The editor
initially selects one assistant; synchronization requires explicit additional
selection. See [MCP configuration](MCP-CONFIG-GUIDE.md) and
[MCP checks](subsystems/mcp-check.md).

Skills use verified snapshots or user-owned local directories. Source replacement
updates the link and assignments together, with conditional rollback on failure;
other assistants remain untouched. See [skills](subsystems/skills.md).
Rules read actual rule files; saved templates and recommended source rules are
applied only after preview and confirmation. Recommended text lives in the
agent-skills repository; sanitized legacy copies are migration fingerprints.

MCP Secrets shows variable names and resolution status. Values belong to a
user-selected external private file and never enter the database, diagnostics,
or backups. Only explicitly referenced MCP variables are editable; identity and
unrelated variables remain read-only. Historical secret migration is retired.

[Extension definition backups](subsystems/extension-backups.md) exclude accounts,
aliases, credentials, active assistant files, assignments, and ownership records.
Restore can select skills, MCP, or rules, with a preview baseline checked again
before committing the transaction. It never applies resources to assistants.

## Queries and execution records

Pages load on demand. There is no tray webview, quota/session collection,
background version polling, or account discovery. Extension targets use
`resources:targets`, which skips secret loading and executable probes. Sources
uses collection-only skill inventory; writes fetch full status when needed.
[Resource bounds](subsystems/resource-bounds.md) describes query and retention limits.

Each write has a run identifier and sanitized audit entry. Batch results group
per-target successes and failures during the session and retain individual audit
records. Failed-only retry re-reads current status and checks action eligibility;
external takeover requires fresh confirmation on the resource page. Historical
records remain in SQLite, subject to configured retention. Generic automatic
undo is offered only when a correct inverse operation is available.

## Acceptance

Implementation is distinct from compilation, isolated execution, platform CI,
and actual installed-client acceptance. [Platform status](GUI-PLATFORM-STATUS.md)
records these boundaries. Earlier account, alias, quota, and installer milestones
in [implementation history](goal-rust-gui.md) describe historical versions and do
not expand the current product scope or prove current installation acceptance.
