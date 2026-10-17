# Programs: Lifecycle of Programs Required by Skills and MCP (Design)

> Current scope (2026-10-12): [Extensions only](EXTENSIONS-ONLY.md) supersedes the historical account diagnostics and launch requirements below. Programs excludes assistant commands and packages and protects account directories in every planned filesystem mutation. Current behavior and checks are in the [subsystem reference](subsystems/programs.md).

Status: the user confirmed the direction on 2026-09-30 (section 16). Phases A, B, C and upstream version detection from phase D are implemented; see section 17 for unimplemented items.

## 1. Problem

Many skills require CLI programs (for example, wecom-cli skills call `wecom-cli`), and MCP services also need programs to launch (`npx chrome-devtools-mcp@1.9.0`, `uvx --from git+https://example.com/mcp/server@main example-mcp`). These programs are installed separately through global npm, uv, Homebrew, or other applications. Agent Manager currently does not know who needs them, where they are installed, their versions, or available updates, so it cannot manage installation, upgrades, rollback, or cleanup.

This design makes Programs a resource category alongside Skills and MCP: **declared dependencies, clear ownership, version records, previewable installation/upgrades, rollback, and cleanup when unused**, integrated with the existing Inbox workflow.

**Out of scope:** this tool does not install or upgrade assistants such as Claude Code and Codex (`claude`, `codex`, and bundled programs), and never touches their files or links. It only detects installed assistants, versions, and available updates (shown on Accounts; Check for new versions queries corresponding npm packages `@anthropic-ai/claude-code` and `@openai/codex`; user decision, 2026-09-30).

**Fallback:** when automation is impractical or fails, every action provides an equivalent command that users can copy into a terminal and execute themselves (user, 2026-09-30).

## 2. Constraints

| Constraint | Source | Excludes |
| --- | --- | --- |
| Do not manage assistant lifecycles | User (2026-09-30) | Treating `claude`/`codex` as managed programs; changing files such as `~/.local/bin/claude` |
| Do not add what was absent; handle only changes made by this tool | User (2026-09-29) | Unconfirmed account configuration/rc edits, files in user directories, global installation/deletion |
| Users decide whether to adopt; retain one copy after adoption and clean redundancy | User (skill adoption, 2026-09-29) | Silent takeover of global installations; retaining both managed and global copies |
| Track releases when available, otherwise the main branch; detect and change versions | User (skill versioning) | Checking existence only |
| Actual files are the sole source of truth; queries/previews perform zero writes | [GUI Redesign](GUI-REDESIGN.md) | Inferring status from database records; downloading, installing, or executing programs during previews |
| Same-type accounts share resources | User (2026-09-30) | Separate program installations per account |
| Unified states and complete Inbox workflow, with ignoring | [Concurrent Account Design](MULTI-ACCOUNT-DESIGN.md) 2.1 | A separate program notification model |
| Seek the best result regardless of workload | User (2026-09-30) | Detection-only scope for convenience |

**How this design interprets Do not add what was absent:** anything this tool adds (entry links, managed directories, account configuration keys, MCP entry rewrites) becomes user configuration only after individual preview and explicit consent, never silently. Each item is registered; undo removes only registered items. Deleting someone else's installation happens only during adoption, only for the adopted package, and only after a whole-package backup.

## 3. Current State and Evidence (2026-09-30)

Evidence levels: **Observed** = local read-only inspection or isolated testing; **Inferred** = follows from the mechanism but not tested; **Unverified** = external documentation or no evidence yet.

**Actual dependency forms**

| Form | Example | Installer and location | Level |
| --- | --- | --- | --- |
| Skill declares a program | wecom-cli skill `metadata.requires.bins: ["wecom-cli"]`; installation `npm install -g @wecom/cli` and requirement ≥1.2.1 appear only in the body of `wecomcli-shared` | Global npm `~/.npm-global`, 1.3.4; `bin/wecom.js` is a `#!/usr/bin/env node` script selecting platform subpackages (`optionalDependencies`) using `os.arch()`; whole package 8.4 MB | Observed |
| Skill uses an undeclared program | `command -v playwright-cli` in xiaoyu-bug-loop body | Global npm | Observed |
| Skill bundles scripts requiring only a runtime | archify bundles `bin/archify.mjs`, requiring only node | System node (x86_64) | Observed |
| MCP fetches on demand, pinned version | `npx -y chrome-devtools-mcp@1.9.0` | npx cache | Observed |
| MCP fetches on demand, follows a branch | `uvx --from git+https://example.com/mcp/server@main example-mcp` | uv cache | Observed launch pattern; example URL |
| MCP uses another application's bundled program | node_repl inside `ChatGPT.app` | That application | Observed |
| One npm package provides multiple commands | `chrome-devtools-mcp` provides 2, `lighthouse` 3, `cline` 2 | Global npm | Observed (independent review) |
| Program owns global state | `~/.config/wecom` exists | Program itself | Observed (independent review) |

Reusable capabilities (observed): `system:refresh-upstream` already detects npx package versions and uvx git sources for MCP and stores results in `upstream_probes` (header comment in `engine-rs/src/native_upstream.rs`). The global exclusive lock is an `fs2` file lock, released on process exit (`engine-rs/src/native.rs`). Skill sources already support tracking, version directories, switching/rollback, and adoption/undo ([Skill Versioning](SKILL-VERSIONING.md)). Global npm and this tool's data directories are on the same partition (`df`), so moving a whole directory is a same-partition rename.

**Making programs discoverable by assistants: isolated testing (observed)**

Method: isolate HOME and configuration directories; a local fake model service makes each actual assistant invoke its command execution tool once; read back its `PATH` and `command -v`. Claude Code 2.1.285, Codex 0.159.0.

| Method | Claude Code | Codex |
| --- | --- | --- |
| Prefix launch command with `PATH="<directory>:$PATH"` | Works; directory comes first | Works, but default execution uses a login shell; macOS `path_helper` puts system paths first and the directory last |
| Put `PATH = "<directory>:$PATH"` in account configuration (Claude `env`, Codex `shell_environment_policy.set`) | `$PATH` remains literal, unexpanded | Same; even `ls` cannot be found |
| Point Claude account `env.CLAUDE_ENV_FILE` at a script containing `export PATH="<directory>:$PATH"` | Works; directory comes first | Not applicable |
| Codex `allow_login_shell = false` + launch-command prefix | Not applicable | Works and preserves order, but changes user shell behavior; excluded |

**Directories in user PATH (observed):** local `~/.local/bin` is present in `.zshenv`, `.zprofile`, and `.zshrc` and visible in zsh `-c`, `-lc`, `-ic`, and `-lic` modes. For Dock-launched applications, `launchctl getenv PATH` is empty: **desktop application PATH cannot represent user PATH**.

**Unverified:** PATH seen by assistant sessions launched from IDEs (such as VS Code); whether Claude Code sessions reread PATH (determines when new programs become available to existing sessions); actual field usage in [OpenClaw skill format](https://docs.openclaw.ai/clawhub/skill-format) (no local skill uses it); interaction between `npm ci` and platform subpackages. Custom uv tool/entry directories now have isolated native fixtures described in the [subsystem reference](subsystems/programs.md); this does not establish assistant consumption. Remaining items are listed in their corresponding phases in section 17.

## 4. Goal

Every program required by a skill or MCP has a declaration and clear ownership. External installations default to delegated management: this tool invokes the original package manager for detection, upgrades, and rollback without relocating the installation. Programs explicitly selected for tool management have one installation per version in the tool's directory, pinned versions, previewable upgrades, rollback, and cleanup when unused; assistants discover them through a user-confirmed entry directory. Deployment model: one machine, one user, one shared program copy across accounts. The Agent Manager desktop application and engine CLI may run concurrently and share a file lock.

## 5. Object Model

```
Source ──provides──► Skill ──requires──┐
                                     ├──► Program ──delegated: original package manager (global npm, uv tool)
MCP service ─────────requires─────────┘            └─ tool-managed: tool directory, one copy per version (npm / PyPI / GitHub Release / Go)
Account ──uses──► Skills, MCP (programs shared machine-wide, not per account)
```

**Program** (one package, potentially multiple commands):

| Field | Description |
| --- | --- |
| Identifier | `npm:@wecom/cli`, `pypi:example-mcp`, `git:example.com/mcp/code`, `github:owner/repo`, `go:module` |
| Commands | **All** executables provided by the package, read from package metadata, rather than only declared dependencies |
| Management mode | **Delegated:** remains in place (global npm, uv tool); this tool calls the original package manager for upgrades/rollback. **Tool-managed:** installed in the tool's directory. **Observe-only:** Homebrew, system, bundled with applications, or unknown source |
| Tracking | Automatic (latest stable release when releases exist), version (patterns such as `1.*` allowed), branch, pinned; same model as skill sources |
| Constraints | Intersection of all dependent requirements, such as `>=1.2.1` |
| Version record | Exact version + integrity (npm integrity, wheel hash, downloaded-file sha256, git commit) + installation runtime (node path/architecture) |
| Dependents | Required skills/MCP, each declaration source (section 6), and each dependent's selected version in tool-managed mode |

Runtimes (node, python, uv itself) are observe-only: detect existence, version, and architecture; report deficiencies without installing on the user's behalf.

## 6. Declaration Sources

Merge in descending trust order, intersecting declarations for the same program. Record every dependency's source for cleanup and conflict decisions:

1. **Skill declarations:** `metadata.openclaw.requires.{bins, anyBins, env, config}` and `metadata.openclaw.install` (kind brew / node / go / uv / download); also support shorthand `metadata.requires.bins` used by wecom-cli. [Agent Skills specification](https://agentskills.io/specification) `compatibility` is free text and displayed unchanged.
2. **MCP launch commands:** `npx [-y] <package>[@version]` → `npm:<package>`, pinned when a version is specified; `uvx --from <source>[@ref] <command>` and `uvx <package>[==version]` → `pypi:` or `git:`; absolute paths → ownership from location; `node <script>` / `python <script>` → runtime dependency only.
3. **User-confirmed supplementary declarations:** scan SKILL.md bodies for `command -v X`, `which X`, `npm install -g X`, `brew install X`, `uv tool install X`; show candidates with source lines and record only after confirmation. Store supplements only in this tool's database, without editing skills. Native declarations take precedence after source updates.
4. **Missing installation method:** when `X` is required but its installation source is unknown, show Missing · Unknown installation method with Provide installation method as the action; do not guess.

## 7. Three Management Modes

### 7.1 Observe-only (Default for Homebrew, System, Application-bundled, Unknown Sources)

Show location, version when readable, and constraint satisfaction. Provide original-method upgrade/delete commands for users to run themselves.

### 7.2 Delegated Management (Default for Existing External Installations)

Keep programs in place and invoke the original package manager:

| Action | Global npm | uv tool |
| --- | --- | --- |
| Detect updates | `npm view <package> version` (network during checks) | Git probes as for skill sources / PyPI queries |
| Upgrade / switch version | `npm install -g <package>@<version>` | `uv --no-python-downloads tool install --force <package>==<version>` |
| Roll back | Before upgrading, **move** the entire package directory and its command links into the tool's backup directory (same-partition rename, no network); restore unchanged on failure or undo | Move the uv environment and owned entries first. Command failures attempt restoration; a successful command whose installed version cannot be verified leaves current files and backup for inspection, with no automatic restoration after that check. The current install may remain on PATH; every later successful action retains only the newest three non-adoption backups. General restore conflicts remain open. |

Preview shows the full command, package directory/commands being replaced, and npm configuration (registry and related settings follow user `~/.npmrc`, since this is the user's global environment). Delegated management introduces no entry directory or second program copy and is the lowest-risk primary path.

### 7.3 Tool-managed (Explicit User Selection per Program)

Suitable for multiple concurrent versions (such as separately pinned MCP versions), reliable rollback, global-environment isolation, or new programs not previously installed.

**Directory layout** (under the data directory, wholly tool-owned):

```
programs/
  npm/@wecom__cli/1.3.4/            # One copy per version; includes node_modules and .agent-manager-program.json
  npm/@wecom__cli/current -> 1.3.4  # Current version pointer for tracking dependents
  bin/wecom-cli -> ../npm/@wecom__cli/current/node_modules/.bin/wecom-cli
```

Write `.agent-manager-program.json` last, recording package, version, integrity, all commands, runtime path/architecture at installation, and backend. A version directory without it is an incomplete installation (Broken · Incomplete installation; clean up and reinstall).

**Backends:**

| Backend | Installation method | Integrity | Notes |
| --- | --- | --- | --- |
| npm | `npm install --prefix <version-directory> --cache <data-directory>/npm-cache --no-save <package>@<version>`; registry/authentication follow user `~/.npmrc` (shown in preview); tool sets prefix/cache | npm integrity | Platform subpackages follow the architecture of node running npm; record node path/architecture and mark Broken · Runtime changed if node changes |
| PyPI / git (Python) | Install directly into the final version directory (venvs embed absolute paths; **never install into a temporary directory and rename**); resolve branch tracking to a commit before installing | wheel hash / git commit | Interpreter location and receipt behavior in custom uv directories require testing (phase B in section 17) |
| GitHub Release | Download using OpenClaw `download` or a supplementary asset-name template, verify sha256, then extract | sha256 | |
| Go | `GOBIN=<version-directory>/bin go install <module>@<version>` | Go module checksum | |
| Homebrew | Not tool-managed (global, cannot install into this tool's directory); observe-only | — | |

**Installation flow:** preview (command, target, actual size, install scripts, dependents) → user confirmation → global file lock → install (npm/Release/Go into temporary directories, then same-partition rename; Python directly into final directory) → integrity verification → one version check in isolated `HOME` / `XDG_CONFIG_HOME` when declared, using the same PATH as assistants → write `.agent-manager-program.json` → switch `current` if needed → read back → operation record. Failure at any step removes this installation's version directory without changing existing versions/pointers.

**Upgrade, version switch, rollback:** install a new version in a new directory, verify, then switch `current` atomically. Rollback can use only retained versions, and the UI lists only those. Retention: always keep `current`, the previous version, and any version explicitly pinned by a dependent. Other versions beyond Retained count (default 2, configurable) become Eligible for cleanup.

**Keep MCP on npx / uvx; manage the entry's version (user decision, 2026-09-30):** follow mainstream clients such as VS Code, Claude Code, and Cursor, retaining `npx -y <package>@<version>` / `uvx --from <source>@<ref>`. Manage that version: flag unpinned references (`@main`, `latest`, omitted), offer one-click pinning to the current latest version (git to a commit), and upgrade by changing the version. Change only the version segment in launch arguments through MCP editing (preview changed lines, compare before writing, affect only this entry); preserve all other fields. Pinning runs only that version without silently following new releases, while update checks still show Update available: X for user choice. Unpin and follow latest is also available (npx back to `@latest`, remove `==version` from PyPI, git back to a branch, default `main`). Record that user choice in `programs.follow_latest`; stop warning Unpinned and show Following latest; clear the record when repinned (user, 2026-09-30). Industry comparison: mainstream clients use npx / uvx and recommend pinning; Claude Desktop MCPB bundles services/dependencies and launches absolute paths; Docker MCP Toolkit and ToolHive build pinned containers. The latter approaches need client or Docker support and are not adopted.

## 8. Assistant Discovery of Tool-managed Programs: Entry Directory

Delegated and observe-only programs already reside on user PATH and need no entry directory. Based on section 3 testing, tool-managed programs use an **entry directory**:

- **Probe user PATH through login/interactive shell modes:** use user `$SHELL` with `-c`, `-lc`, and `-lic` (desktop application PATH is unusable and was observed empty). Candidate entry directories must appear **in all three modes**, be user-writable, and belong to neither the system nor Homebrew (local `~/.local/bin` qualifies).
- On first tool-managed installation, let the user choose the entry directory in preview. Add only links to `programs/bin/<command>`, for **all** package commands, not merely declared ones; register every link's location and target.
- **Reject** insertion when the directory already contains a same-name file not registered by this tool; show Drifted · Name conflict and the conflicting path; never overwrite. Assistant-owned files are always outside scope.
- Multiple copies of a command on PATH (for example, tool-managed plus global) show Drifted · Duplicate command, with all locations and discovery order.
- **Alternative if the user wants no links in user directories:** only for accounts launched by this tool, prefix managed launch commands with `PATH="<programs/bin>:$PATH"`; for Claude, also set account `env.CLAUDE_ENV_FILE` to the tool's script to cover IDE launches. Tested costs: same-name global programs take precedence in Codex; IDE-launched Codex sessions cannot discover the program; one new account configuration key. Enable only by explicit user choice, confirmed per account.

## 9. Adoption: Delegated to Tool-managed

Adoption is a user-initiated transition for one program:

1. **Reject** linked package directories (`npm link`), sources other than ordinary registry releases (git, tarballs, local paths), or local modifications (integrity differs from the same registry version). Reinstalling the same version is not equivalent in these cases; retain delegated management.
2. **Preview:** external location/version, all package commands, tool-managed version to install (same version by default), external package directory/command links to move, entry links to add, and excluded data (program-owned configuration/login such as `~/.config/wecom`).
3. **Execution order** (at least one working copy remains on PATH at all times): install/verify the tool-managed version → add entry links and read back that PATH finds it → **move** external package directory/command links into the tool's backup directory (same-partition rename, no `npm uninstall`) → read back that only the tool-managed copy remains on PATH.
4. **Undo** (no network): restore backed-up package directory/command links to their original location and read back usability → remove entry links and the tool-managed version.
5. Backups of relocated external packages become cleanable only after Retention days (same as skill adoption backups).

## 10. Unified States and Inbox

Program states join the unified model ([Concurrent Account Design](MULTI-ACCOUNT-DESIGN.md) 2.1), using the same categories, collapse, merging, and ignoring as Skills/MCP in Inbox. **Each item has one primary cause determining its only action**; other simultaneous causes appear as labels. Select the first matching cause from top to bottom:

| Unified state | Primary cause | Only action |
| --- | --- | --- |
| Broken | Incomplete installation (version directory lacks `.agent-manager-program.json`) | Clean up and reinstall |
| Broken | Integrity mismatch / broken pointer or entry link | Repair (reinstall same version / rebuild link) |
| Broken | Runtime missing or changed (node absent, architecture changed) | Reinstall from records (runtime itself is notification-only) |
| Broken | Version violates constraints (installed delegated/tool-managed version below requirement) | Upgrade to a satisfying version |
| Broken | Conflicting constraints (dependent requirements have no intersection) | Review conflicting dependents and choose a version per dependent |
| Broken | Verification failed, including disabled install scripts | Review cause (allow scripts here and retry if needed) |
| Drifted | Name conflict (someone else's same-name file in entry directory) | Review (choose another entry directory or handle conflict manually) |
| Drifted | Duplicate command (multiple copies on PATH) | Review locations, then adopt or delete extras |
| Drifted | Tool-managed directory changed externally | Repair (reinstall same version) |
| Missing | Absent everywhere, known installation method | Install (tool-managed by default, preview first) |
| Missing | Unknown installation method | Provide installation method |
| Missing | Tool-managed but no entry directory selected | Choose entry directory |
| Missing | Not initialized (declared check fails) | Initialize (provide command) |
| Update available | New version under tracking policy (delegated/tool-managed) | Upgrade (preview dependents and changes) |
| Update available | Known vulnerability (npm/PyPI only, using [OSV](https://osv.dev)) | Upgrade to a fixed version |
| Unmanaged | In-use external observe-only program with readable version | Delegate management (global npm / uv tool) or ignore |
| Unmanaged | Unreadable version (Homebrew, `.app`, unknown source, unknowable without execution) | Ignore (observe-only) |
| Eligible for cleanup | Unused tool-managed program / old versions beyond retention | Delete (preview first; supplementary-only dependencies yield suggestions, excluded from bulk deletion) |

Eligible for cleanup is a new unified state: it is not a problem, but action requires user confirmation. Ignored items reuse the Inbox ignore mechanism. Skill/MCP rows show dependency labels (Requires wecom-cli ✓, Missing playwright-cli); clicking navigates to the corresponding Programs row.

**Version and vulnerability detection** reuse `system:refresh-upstream`: network access occurs only on Check updates or enabled scheduled checks, with results written only to the probe cache. Queries use no network.

## 11. Safety and Concurrency

- All installation, upgrades, rollback, adoption, and deletion require preview then confirmation, create individual operation records, and support undo. Queries/previews neither download nor execute programs.
- Preview lists npm `preinstall`, `install`, and `postinstall` scripts separately. Default to `--ignore-scripts`; if verification fails, explain that install scripts may need to be allowed, with per-package user consent.
- All program execution (version checks) uses isolated `HOME` / `XDG_CONFIG_HOME` to prevent writes to user configuration directories.
- Writes acquire the existing global file lock (released on process exit), mutually excluding desktop and CLI. Before deleting a version directory, check for running processes with executables there (for example, active MCP services); defer and notify if found.
- Downloads use HTTPS only; delete and report failed verification. Database/operation records store no credentials, only package identifiers, versions, and digests.

## 12. UI

- **Add Programs** beside Sources: one row per program with name/all commands, management mode (tool-managed / delegated / observe-only), current version/constraints, tracking, clickable dependents, and unified state. Row actions follow section 10. The top offers Check updates, Upgrade all, and entry-directory settings.
- **Skill/MCP rows:** dependency labels; missing programs count toward the skill/service in Pending only.
- **Sources:** add/update previews list required programs for the source's skills and allow joint installation.
- **Inbox:** classify program states according to section 10.

## 13. Engine Commands and Data

| Command | Purpose | Writes |
| --- | --- | --- |
| `programs:status` | Programs, dependencies with declaration sources, ownership, versions, states, supplementary declaration candidates, user PATH probes | Zero writes, no network, no program execution |
| `programs:preview` | Complete plans for install, upgrade, switch, rollback, adopt, undo, delete, and entry links | Zero writes |
| `programs:apply --action install\|upgrade\|switch\|rollback\|repair\|remove\|link\|unlink` | Execute with global file lock and rollback on failure | Program/backup directories, entry links, operation records; original package manager in delegated mode |
| `programs:adopt [--undo <backup>]` | Delegated → tool-managed / undo | Same as above |
| `programs:declare` | Save/delete supplementary declarations, tracking, management mode, entry directory | Database |
| `system:refresh-upstream` | Extend probing to programs and add OSV vulnerability detection | Probe cache |

Store only supplementary declarations, tracking, management mode, entry-directory choices, entry-link registrations, MCP entry target registrations, adoption backups, and probe caches in the database. Actual files determine version content, current pointers, and link existence. Tauri permits these commands/arguments using its existing allowlist model.

## 14. Impact

| Item | Treatment | Basis |
| --- | --- | --- |
| User PATH and rc files | Unchanged | Design: only registered links in user-selected directories already present in all three shell modes |
| Account configuration | Change only the selected MCP entry's command for tool-managed services, with individual confirmation; alternative adds `CLAUDE_ENV_FILE` | Sections 7.3, 8 |
| Global installations | Delegated: original manager changes only the confirmed package for upgrade/rollback after whole-package backup; adoption moves only the adopted package | Sections 7.2, 9 |
| npm configuration/cache | Delegated uses user `~/.npmrc` and cache; tool-managed retains registry/authentication but uses tool-owned prefix/cache | Section 7 |
| Data-directory usage | Increases by one copy per tool-managed version (wecom-cli whole package 8.4 MB); retain 2 by default plus pinned references | Observed size |
| Multiple accounts | Share programs and entry points | Design |
| Upgrade/rollback unit | One program; tool-managed rollback switches pointers, delegated rollback restores backups; both without network | Section 7 |
| Assistant sessions | Discovery of newly installed programs by running sessions requires testing | Unverified |
| Skill authors | No changes required; use OpenClaw declarations when present, supplements otherwise | Section 6 |

## 15. Objections and Decisions

An independent reviewer, uninvolved in the discussion, read this file and performed local read-only queries on 2026-09-30. All 15 objections were decided individually:

| # | Objection | Decision |
| --- | --- | --- |
| 1 | Desktop cannot read user PATH; insufficient evidence for entry-directory reachability; IDE launches unverified | Accepted: probe `-c`, `-lc`, `-lic` and require all three (section 8). Confirmed empty Dock PATH and local `~/.local/bin` in all four modes; IDE remains unverified and enters acceptance |
| 2 | `npm uninstall -g` deletes all package commands | Accepted: always handle all commands; adoption moves whole-package backups instead of uninstalling (7.3, 9) |
| 3 | Delete-then-reinstall undo leaves a no-copy window; reinstalling the same version is not equivalent | Accepted: keep at least one copy throughout adoption/undo, move backups without network, reject `npm link`, non-registry sources, local modifications (9) |
| 4 | Python venvs cannot be installed temporarily then renamed; uv directory variable semantics unclear | Accepted: Python installs to final directory and records completion metadata last; uv tested in phase B |
| 5 | Pointing pinned MCP at `current` silently changes versions and contradicts conflict handling | Accepted: pinned entries target specific version directories; tracking entries alone use `current`; pinned versions retained (7.3) |
| 6 | npm inherits user global configuration/cache | Accepted with correction: delegated intentionally uses the global environment; tool-managed retains registry/authentication for private sources, while tool controls prefix/cache and shows them in preview |
| 7 | Platform subpackages depend on node architecture running npm; size claim incorrect | Accepted: record node path/architecture; changes mark Broken; size corrected to observed 8.4 MB |
| 8 | Version checks write to real HOME; no state for unreadable versions | Accepted: isolate all execution with `HOME` / `XDG_CONFIG_HOME`; add Unreadable version primary cause |
| 9 | States/actions are not one-to-one and several are missing | Accepted: ordered primary-cause/action table in section 10, adding incomplete installation, missing entry directory, runtime changes, failed verification, and separating name conflicts from duplicates |
| 10 | Multiple processes and running programs during cleanup | Accepted: existing lock is a verified process-exit-released file lock; check running processes before deleting version directories |
| 11 | Tension with Do not add what was absent | Accepted: section 2 explains individually confirmed, registered additions; deletion only during adoption, only that package, after backup |
| 12 | Delegated management (external installation stays in place, original manager called by this tool) not evaluated; mise rejection inaccurate | Accepted: delegated becomes the external-installation default (7.2); tool management is per-program user choice; mise correction below |
| 13 | Several assumptions lack evidence levels | Accepted: section 3 separates observed/inferred/unverified; unverified items enter section 17 acceptance; OSV coverage limited to npm/PyPI |
| 14 | Retention conflicts with rollback; supplementary declarations can misclassify cleanup | Accepted: list only retained rollback versions; always retain pinned references; supplementary-only dependencies excluded from bulk deletion |
| 15 | Self-review omitted issues; phase C lacked counterexamples | Accepted: this section records review; counterexamples enter section 17 |

**mise correction and continued rejection:** its shims directory can indeed work without shell activation by adding one PATH directory, structurally matching this design's entry directory (inferred; its current version was not checked). It remains outside the primary plan because it does not know skill/MCP dependencies; this tool would still implement cleanup, constraint intersection, and Inbox, while mise introduces its own global configuration/data directories under user HOME. It could be an optional tool-managed backend without changing the object model.

**Most likely remaining failure:** individual packages install successfully but cannot run under tool management (global-path dependencies, native-module/runtime mismatch). Protection: never switch if verification fails; retain delegated management for those programs. **Evidence that would overturn the entry-directory approach:** IDE-launched sessions cannot see it. Use the alternative for IDE scenarios in that case; phase B verification in section 17 will decide.

## 16. Decisions (2026-09-30, User-confirmed)

| # | Decision | Result |
| --- | --- | --- |
| 1 | Three management modes: existing external installations default to delegated; users can choose tool installation for individual programs | Agreed |
| 2 | New, previously uninstalled programs use tool-managed installation | Implement recommendation: isolated, multiple versions, pointer-only switching; select entry directory in installation dialog |
| 3 | Assistant discovery of tool-installed programs | Use recommended entry directory (local `~/.local/bin`), selected in installation dialog |
| 4 | Convert MCP entries to absolute paths? | No: keep npx / uvx and manage entry version numbers (end of 7.3) |
| 5 | npm install scripts | Disabled by default, allowed per package, matching defaults in [pnpm 10](https://socket.dev/blog/pnpm-10-0-0-blocks-lifecycle-scripts-by-default), [Bun](https://bun.com/docs/pm/lifecycle), [Deno](https://docs.deno.com/runtime/reference/cli/approve_scripts/), [npm 12](https://github.blog/changelog/2026-06-09-upcoming-breaking-changes-for-npm-v12/); list packages whose scripts were skipped after installation |
| 6 | Assistants themselves | Detect versions/updates only; do not manage installation |
| 7 | Actions impractical to automate | Provide commands for users to run themselves |

## 17. Phases and Acceptance

All verification uses isolated HOME, data directories, and test assistant paths. Assistant-related checks use a local fake model service to invoke the actual command execution tool and read results back.

| Phase | Content | Status (2026-09-30) |
| --- | --- | --- |
| A | `programs:status`: declaration parsing (OpenClaw/shorthand), MCP command parsing, external installation recognition (all commands), three-mode user PATH probing, supplementary declaration candidates; skill/MCP dependency labels; Inbox integration | Implemented. Local read-only readback: wecom-cli (global npm 1.3.4, needed by 4 skills), chrome-devtools-mcp (npx pinned 1.9.0), Git-backed uvx runners (branch tracking, unpinned), node_repl (ChatGPT.app bundled, observe-only), playwright-cli (undeclared candidate). Data/user directories unchanged before/after queries |
| B | Delegated upgrade/switch/rollback (whole-package backups); tool installation: npm/PyPI/git backends, version directories, pointers, entry links, switches, old-version deletion, complete removal, cleanup eligibility | Implemented. Actual npm in isolated HOME: failed upgrade to nonexistent version restored byte for byte; rollback restored byte for byte; foreign same-name entry file rejected and untouched. Engine integration tests preserve these behaviors with fake npm. Actual isolated uv backend testing (uv 0.7.15, PyPI cowsay): tool-managed install/upgrade/switch/delete and delegated upgrade/rollback (uv tool directory byte-identical after rollback). **Unverified:** discovery of entry-directory programs by IDE-launched sessions |
| C | Adoption (delegated → tool-installed), undo; MCP entry version pinning/upgrades | Implemented. At least one copy on PATH throughout adoption, exactly one at completion; undo only moves local directories and restores byte for byte; `npm link` packages rejected |
| D | Update detection (programs/assistants); vulnerabilities, constraint conflicts, joint installation from Sources | Update detection implemented (Check for new versions writes only probe cache). **Not implemented:** OSV detection, constraint intersection/conflicts, integrity checks for tool-installed programs (tampering → Broken), running-process checks before directory deletion, GitHub Release / Go backends, joint installation from Sources |
