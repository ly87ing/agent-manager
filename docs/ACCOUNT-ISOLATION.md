# Account Isolation

Agent Manager's controlled commands bind one registered account to one configuration directory, authentication mode, credential owner, and endpoint. When supported CLI behavior or local configuration cannot establish that context, the command stops before launching or changing credentials. Existing unsafe definitions receive a diagnostic; the tool does not migrate or overwrite their credentials automatically.

| Constraint | Source | Consequence |
| --- | --- | --- |
| Accounts must remain independent | User decision, 2026-10-10 | Reject overlapping directories and aliased identity files, including disabled registrations |
| Main accounts hold official logins only | Multi-account design | Main Claude uses `HOME/.claude.json`; independent Claude uses `<CLAUDE_CONFIG_DIR>/.claude.json` |
| API and official authentication cannot coexist | User decision, 2026-10-10 | Validate existing and rendered effective configuration before writing or launching |
| Credentials belong to one account | User decision, 2026-10-10 | Reject conflicting secret references and automatic copies of credential-bearing MCP entries |
| Resource destinations belong to the selected account | User decision, 2026-10-10 | Bind main/profile skill destinations to their own configuration directory and reject redirected targets |
| No partial account update on failure | User decision, 2026-10-10 | Compare the preview baselines and conditionally restore this operation's writes |
| Real accounts remain untouched during acceptance | User decision, 2026-10-10 | Use isolated HOME/data directories and fake CLIs and credentials |

## Controlled entry points

The generated launch, login, logout, and status commands use the installed application or standalone engine executable. A binding includes the local registration generation, directory, authentication mode, key reference, and endpoint. Removing and recreating an account invalidates its old generated command, even when its name and settings are reused. Legacy registrations without a generation require an explicit definition save; queries do not initialize them. The command resolves the assistant's absolute executable and prepares a clean child environment; a shell alias named `claude` or `codex` cannot redirect it.

Account and credential checks apply before the child process starts. Identity-changing command arguments and relevant project or managed configuration must pass the supported configuration checks. Configurations that are malformed, unreadable, too large, or ambiguous stop the operation. A configured secrets source must supply the account's key itself; an error or missing key cannot fall back to a parent shell's value. Concurrent sessions may use separate accounts; authentication changes and account writes must respect account locks.

Account session locks are shared across upgraded processes even when they use different manager data directories. Before upgrading from data-directory locks, end all older account sessions and exit older Agent Manager processes; do not run an older binary alongside the upgraded app. Known legacy locks are checked, but arbitrary historical directories and unknown path spellings cannot be discovered reliably. See [the lock migration boundary](subsystems/account-context.md#upgrading-from-data-directory-locks).

Credential variable names remain reserved to their account registration after an entry or account is removed. The database stores the variable name and registration binding, never its value. A recreated account needs a new variable name; deleting a definition cannot make the previous account's credentials available to another account.

The supported CLI contracts are Claude Code 2.1.285 and Codex CLI 0.156.1, 0.159.0, and 0.162.1. Other versions stop with a diagnostic until their identity configuration has been reviewed. Executable hooks and notification commands in relevant account, project, or managed configuration stop controlled startup because their effects on identity cannot be established. Windows assistant batch wrappers require a supported native executable. Credential-bearing MCP entries, including references, headers, authentication helpers, and ambiguous fields, are configured separately for each account. Disabling an entry does not silently move its literal credentials into a shared secrets file. MCP connection tests use the selected account's references, a clean subprocess environment, and explicit HTTP configuration.

An existing Claude MCP entry containing resolved literal credentials cannot be saved as a disabled snapshot through automatic credential adoption. This includes an env reference previously rendered into a literal by Add or Edit. The operation stops without changing the entry; configure a native reference explicitly before retrying. This compatibility restriction applies even when its account has a valid secrets source.

An account update compares its preview baselines and compensates normal errors by restoring only bytes still owned by that operation. It preserves an external change and reports a rollback conflict. This is not a durable cross-file transaction: abrupt termination or power loss has no recovery journal, and metadata written by the assistant CLI itself is outside this compensation.

Stored login records and decoded JWT labels are local status information. They do not verify the server's current account identity or validate a token signature. Tests use fake credentials and cannot establish a real subscription's identity or quota.

## Security boundary

These controls cover Agent Manager's entry points and CLI/configuration behavior that the tool supports and can validate. They do not prevent the same OS user from running an assistant directly, editing files outside the tool, changing identity inside an already running assistant, or another process with that user's permissions from altering files. An assistant executable or MCP server runs with that user's permissions; this preflight is not an OS sandbox for its code. Remote policy or credential changes after a process starts require the assistant's own enforcement. Protection against hostile processes with the same OS identity requires separate OS users or an OS sandbox; this change does not alter system security settings.

Use a generated command again after changing an account or its configuration so the preflight checks run against current files. Resolve a diagnostic explicitly before proceeding; do not reuse an old raw alias to bypass it.

## Verification

Run the engine, Tauri bridge, frontend, and script suites described in [AGENTS.md](../AGENTS.md). Isolation acceptance must additionally run actual generated commands against fake CLIs, compare the same inputs with the previous implementation, and read back main/A/B files after both accepted and rejected operations. A cross-compile or linked test executable proves build coverage only. Native Windows/Linux desktop flows and real signed updates need acceptance on their own operating systems.

Implementation details are indexed in [subsystem references](subsystems/README.md).

### 2026-10-10: configuration and account review

Source `80d82ed` fixes two verified skill destination errors. An explicit
`skills_dir` could direct account A's actual skill write into a nested directory
inside main Codex or another account. Without that field, the target used the
registration ID rather than the profile's configuration directory. Destination
validation now covers saves, imports, historical fallbacks and runtime skill
operations, including adoption and cleanup. Unsafe historical definitions are
rejected without moving or deleting existing files. Source-only rules, aliases
and recommendations remain available independently of skill destination checks.

The six new integration checks all fail against a distinctly named build of the
previous source and pass on the corrected source. They cover both assistant
types, differing registration/directory names, nested foreign targets, import,
fallback edits, links/junctions, cleanup and valid custom-assistant destinations.
The existing skill adoption/undo case also passes with its trusted HOME alias.
Actual isolated CLI execution rejects the unsafe registration before creating
data, applies A and B separately and preserves 13 configuration, authentication,
session and sentinel files. Status-query snapshots remain unchanged.

Source `38b47b3` closes a separate command-write route: an rc symlink could point
to main Codex's `config.toml`, and adding an alias appended shell text to that
configuration. Command observation now checks ownership before reading contents
or returning a baseline. Four Unix regression groups fail on the old source and
pass on the correction; Windows additionally covers command-directory junctions.
They protect main/registered accounts, disabled/malformed registrations, external
identity links, hard links, declared custom MCP targets and manager data. Actual
isolated CLI readback rejects the unsafe link without changing protected bytes,
accepts ordinary dotfiles links, preserves outside edits and rejects stale edits
inside the managed block.

Local evidence includes 66 profile checks, 36 skill checks, 24 rule checks,
18 existing alias checks, one bash check, four new command-target groups and
27 Tauri checks passing. The final normal local engine run still has 12 failures
in subprocess/lifecycle library tests and one transaction fixture that did not
reach its real CLI probe; it is not a pass. Byte-identical private test copies
from the preceding skill fix are recorded separately and do not turn failed
normal runs green. An unrelated bare shell script also exceeded
five seconds when executed directly; reading the same bytes with `/bin/sh`
completed in milliseconds. Test deadlines and safety assertions remain intact.
The unchanged frontend authentication cases passed alone, but timed out in local
full runs; the production frontend build passed.

For source `38b47b3`, native engine CI passes 605 tests on Linux, 606 on macOS and
519 on Windows, with no failures. The new Windows command-directory junction case
executed successfully. Full Linux/macOS engine, Tauri, frontend, type/build and
script CI also passes. All nine CI/GUI CI jobs succeeded, including the four
platform builds. These are source-bound test/build results, not physical desktop
or real-login acceptance. The final documentation-only follow-up changes no
product code and is checked for link/anchor consistency separately.
See [CI](https://github.com/ly87ing/agent-manager/actions/runs/38115020549) and
[GUI CI](https://github.com/ly87ing/agent-manager/actions/runs/38115020553).

No real account login or installed application was changed during this review.
This evidence covers the tool's supported configuration paths; it cannot
establish an absolute absence of defects or extend the same-OS-user and
crash-recovery boundaries described above.
