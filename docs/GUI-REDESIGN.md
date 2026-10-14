# GUI Redesign: Current Contract

[Extensions only](EXTENSIONS-ONLY.md) defines the product boundary. The current interface covers MCP, rules, skills, their sources and dependency programs, immutable extension target labels, user-controlled general launch configurations, and macOS environment checks. Historical account and alias designs are archived in [implementation history](goal-rust-gui.md) and [multi-account design](MULTI-ACCOUNT-DESIGN.md).

## Data model: the actual configuration is authoritative

Each assistant's real files and directories are the sole source of truth for active extensions. Agent Manager uses three layers:

| Layer | Content | Location |
| --- | --- | --- |
| Active | MCP entries, rule files, skill links and directories | Assistant files and directories |
| Saved but inactive | Disabled MCP snapshots, rule templates, skill source definitions and assignments, launch recipes, environment comparison collections | `native-manager.db` |
| Recommendations | Suggested skill sources and rule text from those sources | Read-only catalogs and source repositories |

The database also holds app preferences, immutable recognition and location metadata, ownership records with digests, and sanitized operation history. It does not keep a second writable copy of active assistant configuration or plaintext secrets. Recommended rules are maintained in the agent-skills repository; frozen legacy rule copies are migration fingerprints only.

Status, form prefill, search, preview, cancellation, and discovery read actual files and do not initialize or write state. A missing database row never makes a recommendation active. Saving an inactive alternative does not install a program, link a skill, or apply a rule. Every active edit rereads the target under the appropriate lock, compares its displayed baseline, and changes only the selected resource. An external edit rejects the write and asks for a fresh view. Conditional rollback touches only bytes still owned by that operation.

Only supported assistant fields are written. Ownership markers are not inserted into assistant files; the database records location and digest. Any legacy marker cleanup happens only during an explicit MCP entry write. Built-in recommendations appear as choices requiring individual application; they are never automatic pending synchronization. Public `*:plan` and `--dry-run` entry points remain retired.

Historical account registrations locate existing extension targets but cannot be edited, removed, or used to infer login identity. General launch recipes are independent of those registrations and do not read credentials or shell configuration.

## Navigation and resource behavior

Launch opens by default. Environment, Skills, MCP, Rules, Sources, Dependency Programs, Assistants, and Settings follow. Assistants displays target labels and paths from registration metadata only. Settings includes static More Tips; its examples can be copied but are not executed or installed.

`system:discover --only skills` classifies local skill entries as native directories, external links, links belonging to a known collection, or cleanup candidates. It is read-only and network-free. Native directories remain with their original installer. External sources are adopted only on an explicit skill action. A missing upstream skill is actionable only after successful source discovery; failed or unfetched sources do not prove deletion.

MCP status reads actual per-assistant entries and shows sanitized fields, variable resolution, target paths, and per-entry baselines. Add, edit, enable, disable, remove, copy, forget, and marker cleanup act on a specific entry; cross-assistant copy requires explicit destination selection and replacement confirmation when needed. Preview shows the complete candidate target file locally and is invalidated when inputs change. Codex and OpenCode use their supported disabled field; Claude Code stores a disabled snapshot outside the assistant file. Credential-like values must use a dedicated external secret reference before they can enter saved snapshots.

Rules status reads actual target file text and baselines. Users can edit text, apply a saved template, apply a rule from a skill source, or copy another target's actual text. Each write targets one file and rejects stale baselines. Saved templates are inactive database content. Source rules are read from registered skill sources; the tool ships no current recommended rule text. Source follow records help offer an update only when the target still matches the version last applied.

Skill collections use verified git snapshots or user-owned local directories. Assignments and links are applied per assistant. Dependency Programs handles tools required by MCP or skills, with previewed actions and protected filesystem destinations; assistant executable installation remains outside scope. MCP Secrets edits only dedicated variables referenced by MCP and stores values in a selected external private file.

Launch builds copyable commands and alias/function definitions from saved or draft recipes. Saving a recipe changes Agent Manager data only. Explicit Run can start a local graphical application. No launch action writes shell startup files or manages an assistant account. See [launch configurations](subsystems/launch-configurations.md).

Environment compares macOS and caller-process observations with optional saved
criteria. Settings panes, unsaved launch drafts and administrator-authorized
read-only inspection are separate handling actions. Explicitly enabled clock
correction alone may adjust time after a fresh clear mismatch and authorization,
with independent verification and request/completion audit evidence. See
[environment checks](subsystems/environment-checks.md).

[Configuration backups](subsystems/extension-backups.md) can select saved launch
recipes and environment collections independently of extension modules. A stale
preview refuses restore, missing legacy sections preserve local data, and restored
tools remain inactive until a later explicit launch or check.

## Verification boundary

Implementation and acceptance are separate. Isolated HOME/data and test-assistant paths verify real file readback, negative scope cases, and rollback. Frontend mock tests and browser viewport checks verify interface behavior; they do not establish installed desktop or real-assistant acceptance. See [platform status](GUI-PLATFORM-STATUS.md) and [implementation history](goal-rust-gui.md).
