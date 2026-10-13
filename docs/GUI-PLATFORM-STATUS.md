# GUI Platform Status

The current product manages extensions and independent general launch recipes.
Account management, login, provider settings, account-bound launch commands,
shell-file alias management, quota, and assistant lifecycle capabilities described
in older records below are retired. The Assistants page lists immutable extension-target metadata;
it does not restore account management. See [current architecture](ARCHITECTURE.md)
and [scope](EXTENSIONS-ONLY.md).

## Current acceptance boundary (2026-10-12)

- macOS isolated CLI and UI checks exercise extension operations with temporary
  HOME/data directories; real accounts and sessions are not test targets.
- A Windows CI failure at `41707c4` reproduced dependency unlink reporting success
  while its `.cmd` file remained. `adc5973` fixes the platform filename. Native
  Windows CI at `a21feff` passed the actual link/unlink and private-file ACL tests;
  the ACL fixture now declares its dedicated MCP variable reference so it reaches
  the permission assertions without relaxing secret scope.
  [GUI CI receipt](https://github.com/ly87ing/agent-manager/actions/runs/38252731593).
- Current correction work adds atomic skill source replacement, local-source
  personal copies, selected-module definition restore with stale-preview refusal,
  grouped batch outcomes and failed-only retries, and complete dependency action
  labels. Local acceptance and remaining limits are recorded in
  [the implementation history](goal-rust-gui.md#2026-10-12-extension-workflow-corrections).
- The follow-up [source ownership audit](goal-rust-gui.md#2026-10-12-source-ownership-and-failure-reporting-audit)
  records copy/restore safeguards, duplicate-name refusal, orphan cleanup,
  accurate update outcomes, and native macOS click verification at `26db532`.
  That revision passed all regular and GUI CI jobs, including four installer
  builds and native-architecture macOS DMG / isolated Linux launch verification.
- Source changes, native tests and installer builds are separate evidence. Earlier
  downloadable previews predate extension-only scope and must not be treated as
  acceptance of current source. Interactive Windows and arm64 macOS desktop
  operation, signing/notarization and actual automatic updates remain unverified.
- No new Release, tag, or installer publication is part of this correction.

## Historical platform records

The following dated records apply only to their named revisions. Statements about
accounts, aliases, commands, quotas or secret migration describe those old builds,
not current supported functionality.

## Release Verification on 2026-10-08

Builds for all four platforms and macOS / Linux launch checks passed. GitHub billing conditions blocked the full signed release; the formal Release and actual automatic update verification remain incomplete.

- Both [regular CI](https://github.com/ly87ing/agent-manager/actions/runs/37768672793) and [four-platform artifact CI](https://github.com/ly87ing/agent-manager/actions/runs/37768672559) passed for `77866cd`. DMGs for both macOS architectures passed build identity, complete ad-hoc signature, and isolated HOME launch verification. Linux AppImage / extracted deb executables passed liveness checks on an isolated virtual desktop. Windows NSIS / MSI builds passed; installation and UI on a real Windows machine remain unverified.
- With no sources configured in an isolated HOME, all 7 actual CLI status entry points for accounts, skills, rules, MCP, commands, and secrets worked. Local resources were visible, and no database or repository cache was created. Local regression checks passed: 619 frontend tests and 443 Rust tests. See [Secrets Management](SECRETS-MANAGEMENT.md) for private secrets repository configuration.
- Local Tauri ad-hoc packaging, update archive signing, independent signature verification, and tamper rejection passed. The update private key and password were stored in the maintainer's private repository and configured as Actions Secrets; ordinary users do not need access to them.
- The first private draft, `v0.1.0-rc.1`, exposed an outdated build identity test expectation, fixed by `2a71f12`. All 6 release-mode tests and all 443 regular-mode tests passed. Before any steps started, GitHub rejected the second [GUI Release](https://github.com/ly87ing/agent-manager/actions/runs/37770628640) because payment failed or the spending limit needed to be raised. No formal version was published, and repository visibility was not changed.

To resume, resolve GitHub Billing first, then create a new candidate tag from the latest `main` (the next available candidate is `v0.1.0-rc.3`); do not move existing tags. Full signed release, actual downloading, and automatic update installation still require verification. The distribution scope of installation packages remains to be confirmed. Published preview packages do not contain the update public key; switching to the first signed release requires manual installation.

## Feature Status

| Scope | Status |
| --- | --- |
| SQLite core | Complete: `native-manager.db`, settings, policy, runs, and read/write boundaries |
| Shared CLI / Tauri library | Complete: both call the same `engine-rs` |
| Tauri bridge / React foundation | Complete: native bridge, settings and policy pages, primary resource page layouts, and direct apply interactions |
| Experience improvements (2026-09-17) | Complete: design tokens and light/dark/system themes (including native title bar synchronization); system tray residence, remembered window position and size, and single-instance activation; page/dialog animations, skeletons, and reusable empty states; assistant/skill/MCP/operation record search, ⌘,/⌘F shortcuts, and update badges at the Settings entry. See the [Delivery Receipt](goal-rust-gui.md). On 2026-09-21, the Tauri native tray built from the same source was clicked in an isolated data directory with DEV mocks, verifying that connection entries restored the window, navigated directly to their targets, required explicit confirmation, and refreshed status. Dock Reopen, residence after closing the window, and the packaged tray still lack real-machine GUI verification |
| Python cleanup | Complete: the root repository no longer depends on Python or `manage.sh` |
| Native Agent domain | Complete: read-only main-account status (official account, login, quota) and runtime parameter writes (real-time quota status line), with cross-process locking, result verification, and operation records. Main-configuration switching to third-party providers and connection recognition are retired |
| Native Skills domain | Complete: wired into the dispatcher; built-in defaults are read-only, with writes stored in SQLite; collections and individual skills support updates, enable/disable, and removal; content snapshots and safe symlinks; read-only, zero-write status and exclusive locking/auditing for apply |
| Runs operation records and retention | Complete: lists and details read SQLite `operation_logs`; adding an operation record deletes expired records according to `log_retention_days` (0 disables cleanup); `runs:clear` provides manually confirmed clearing with a second confirmation (clearing itself creates no audit entry and does not delete the database or change configuration) |
| Local structured diagnostic logs and retention | Retired (decision on 2026-09-19): JSONL diagnostic logs, `logs:clear`, and envelope `log_file` were removed together. Error messages are still uniformly redacted before leaving the database; troubleshooting uses `operation_logs` |
| Native MCP domain | Complete (changed to actual entries as the authority on 2026-09-22; independently verified by the lead reviewer in isolation and with browser mocks): `mcp:status` directly lists stdio and remote entries in each assistant's configuration files (redacted, preserving unmodeled fields and providing entry digests); `mcp:apply` performs add/edit/enable/disable/remove/copy/forget on individual entries, with digest CAS before writing and rejection of external changes. Codex/OpenCode use the official `enabled` field for disabling; Claude removes disabled entries and saves snapshots (suspected credentials are transferred to an external secrets file). The old service library and assignment tables migrate on the first write without changing configuration files. Status is read-only with zero writes; apply has locking, auditing, readback, and conditional rollback |
| Native Rules domain | Complete (changed to actual files as the authority on 2026-09-22): `rules:status` returns the actual full contents and write baselines of main Codex/Claude and instance rule files; `rules:apply` directly edits using `--base`, or applies a template/built-in recommendation/copy from another target (saving a backup before overwrite); templates are stored in `rule_templates`; status is read-only with zero writes; apply has exclusive locking, auditing, CAS/rollback, and rejects external changes with a refresh prompt |
| Native Hooks domain | Removed (2026-09-29): UI, bridge, and `hooks:*` engine commands were deleted; new accounts and shared settings no longer carry hooks. Existing hooks in account files remain intact; `hooks.*` from old backups are skipped during import |
| Exact text preservation in definition editing | Complete (`5ae995b`): SourceDefinitionsPanel disabled automatic text substitutions and had unit coverage; Rules/Hooks text was not rewritten by smart quotes or similar substitutions (the panel was later removed when alias sources were retired) |
| System audit | Complete: the Rust dispatcher exposes read-only `system:audit`, aggregating agents/skills/mcp/rules/aliases/secrets and runtime/storage status |
| Secret status | Complete: `secrets:status` returns only variable names, sources, and Boolean status. The user explicitly selects an external `.env` file with permission `600`; the database stores only its path. `secrets:set` and secret transfers while disabling MCP write only to that file; configuration backups contain neither paths nor values. Historical database values require explicitly confirmed migration; the old table is removed only after a successful write. Variable rows come from actual MCP entries and disabled snapshots across assistants; literals are neither listed nor echoed |
| Native Aliases domain | Complete (changed to actual blocks as the authority on 2026-09-22; independently verified by the lead reviewer in isolation): `aliases:status` returns actual aliases in managed blocks in `~/.zshrc` / `~/.zprofile`, read-only aliases outside those blocks, and each file's `base`. `aliases:apply` adds, rewrites, renames, or deletes one alias by name with a separate `base` per file; external changes are rejected. Built-in aliases are recommendations only; unapplied aliases from old `alias_sources` remain suggestions that may be added or discarded (`aliases:discard-legacy`). There is no pending-sync state; `sources:get/set` were removed |
| Configuration tools in Settings | Complete: retains client diagnostics and Aliases / Secrets entries; the update center was folded into the overview, and the raw Policy editor was removed. The Tools domain, snapshots (`snapshots:*`), and `system:converge` are retired; related pages, Tauri allowlists, and contract schemas were removed. `system:refresh-upstream` is wired up (probing agents/mcp/skills and caching results in SQLite `upstream_probes`) |
| Actual business E2E in a standalone package | Main scenarios complete (independent lead-reviewer verification on 2026-09-15, tied to verification commit `5a110be3`): Darwin x86_64 DMG verify script exited 0; bundled commit/date exactly matched the source; unsigned; SHA256 `076d16b098ede8915802c0bc6dceec42dc06e0dcadddd137e7865681c7e90a68`, 7632606 bytes. Actual GUI E2E used the old plan→confirm→apply interaction, now obsolete and unable to prove this round's direct apply interaction. Coverage included zero writes from `system:audit` with a missing database; Rules cancellation with zero writes and application; Hooks to two targets; Aliases to two targets; MCP stdio and remote HTTP plan/confirm/apply; Secrets showing only variable names and existence; Runs; and fail-closed plans for external changes. MCP definitions were written into an isolated SQLite fixture using the shared Rust CLI; the MCP add-service form was not verified because of macOS automation input limitations (fixture results do not establish form success). Historical dry-run evidence: stdio dry-run left database and target hashes unchanged and wrote no logs; remote headers were written to the Codex target as environment-variable references; synthetic values did not appear in targets, diagnostics, or operation logs. Earlier history (2026-09-14): a DMG based on `5ae995b` passed independent package verification (SHA256 `71a386d6ab8e1a7f85ef59c82f379411e9aa1dccaa3c3166309b96b971fce26b`, 7632599 bytes, not a HEAD build); GUI business E2E did not start then because the host returned AX/computer-use permission_denied |
| Cross-platform release, signing, and updates | 2026-10-08: GUI CI `37744659756` (`3378c12`) successfully built all four platform artifacts; preview Release `preview-v0.1.0-20261008` was published in the private repository. `2f88a2d` supports macOS ad-hoc signing / Windows without commercial signing by default; update package signatures remain mandatory. It fixes the Tauri 2 manifest format while retaining draft and source-verification gates. Update private key/password are configured in Actions Secrets and the public key variable is configured; local signing, verification, and tamper rejection passed. Formal Release, public downloads, and actual automatic update download/installation remain unverified. |

| Windows support | Code completed; all engine tests passed under Wine; an NSIS installer was cross-built and installed under Wine. UI on an actual Windows machine remains unverified (2026-09-30; see Windows Support below) |
| Linux support | Verified in an Ubuntu 22.04 container (2026-09-30; see Linux Support below): engine, desktop, and frontend tests passed; deb and AppImage packaging succeeded; AppImage launched on a virtual display, and a launch command created through the UI was written to `~/.bashrc` and ran in bash. A real desktop environment (GNOME / KDE windows, tray, input method) remains unverified |

Release-readiness improvements in this round (`d46b85a`, `47702ed`, `14e6790`): added the MIT LICENSE and public-release metadata, enabled an explicit Webview CSP, updated the application icon, and set the Bundle ID to `io.github.ly87ing.agent-manager`; the frontend gained a top-level error boundary, custom assistant install/upgrade confirmation, and MCP empty-state guidance, and retired code was removed. The release pipeline refactor is described in the Cross-platform release, signing, and updates row above.

Reading with a missing database does not create it; saving initializes it. Settings live in `app_settings`. The policy persistence source is `sqlite`, with writes fixed to `--layer local`. Both Claude Code and Codex default to official `native` commands; brew is handled only when detected as the actual installation source.

Public `*:plan` and `--dry-run` entry points are retired and reject calls. Status/query are read-only with zero writes. Domain writes use actual apply only; apply retains preflight/write validation, precise scope, locking, external change detection, CAS/conditional rollback, and auditing.

MCP supports local stdio and remote HTTP transports. Entries in each assistant's configuration files are authoritative. Adding or editing remote URLs permits only `https://` or local-loopback `http://localhost` / `http://127.0.0.1`. Headers accept only `${VAR}` references or preserved existing values and are written using each client's native reference format. The UI lists all assistants with MCP configuration (including instances and OpenCode) from `mcp:status`'s `targets`.

Multi-account version (2026-09-28): a Darwin x86_64 DMG built from `beddc6d` passed independent package verification with `verify:macos` (read-only mount, copy to a temporary directory, isolated HOME and minimal PATH, 5 seconds of liveness, matching build identity, path redaction, and no Python remnants). Unsigned, SHA256 `7c3ff59818692484fb76b39e4b5726713a7936e5d0fabe01328c139bbe4d3f39`, 8705779 bytes. Actual writes for account creation/edit/removal, skill/MCP/rule filling, and real-time quota were verified by readback in an isolated HOME using the shared engine CLI. UI click flows in the packaged application were not automatically verified.

Multi-account version (2026-09-29): the x86_64 DMG based on `2e70d14` passed `verify:macos` (unsigned, SHA256 `37c07fb92bf3dca5b3502c86a8db5edee535d9a042764ad5fc04244717f702e5`, 8761959 bytes). An API account was then created through its UI in an isolated HOME, with directory, configuration, rules, secrets file, and launch command read back individually. Actual Claude read the rules through the new launch command. See the [Work Plan](goal-rust-gui.md), Delivery Receipt (2026-09-29, Multi-account Version).

Skill source versioning and adoption (2026-09-29): the x86_64 DMG based on `6706505` passed `verify:macos` (matching build identity, 5 seconds of isolated liveness, and graceful exit; unsigned, SHA256 `882b05237e8279c8cc5a9fb23553b31586994323c83105c0ec262cb931b5bde1`, 8891675 bytes). Actual version, preview, and adoption writes were verified by readback against real GitHub in an isolated HOME using the shared engine CLI. Click flows in the packaged application did not receive native-window verification.

Old DMGs and old GUI E2E do not establish this round's new interactions. This round's source, browser, isolated CLI, and uncommitted candidate dirty DMG verification is complete. The final clean DMG was rebuilt after final commit `a01d0f5` and passed independent lead-reviewer verification (commit, SHA256, and results are in the [Delivery Receipt](goal-rust-gui.md)).

Verification entry points: `cargo test --manifest-path engine-rs/Cargo.toml`, `cargo test --manifest-path app/src-tauri/Cargo.toml`, and `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`. macOS packaging and independent package verification use `pnpm --dir app run build:macos` and `./scripts/verify-macos-dmg.sh <path-to-dmg>`. Local verification may accurately report unsigned status; it does not establish Apple signing or notarization and does not replace other-platform or incomplete-domain E2E.

## Windows Support (2026-09-30)

Implementation approach, checked against cc-switch source, official Claude Code and npm documentation, and Codex source:

- **Home and data directories:** Use `USERPROFILE` when `HOME` is unset. CLI data lives in `%APPDATA%\agent-manager`; the desktop application keeps Tauri's application data directory.
- **Launch commands:** Without `~/.zshrc` or `alias`, each command is written to `~/.agent-manager/bin/<name>.cmd`. Values use the same notation as other platforms (for example, `CLAUDE_CONFIG_DIR="$HOME/.claude-cc" claude`). Script generation converts leading assignments to `set "name=value"`, `$HOME` to `%USERPROFILE%`, and appends `%*` to the command. The third script line records the original value. Readback regenerates the script from that value and recognizes it as tool-owned only on an exact match. Manually edited scripts can only be deleted; unmarked `.cmd` files in the directory are listed read-only and left untouched. Values containing cmd control characters such as `& | < > ^` are rejected.
- **No registry edits:** The tool only reads system and user PATH from the registry to determine whether the command directory is present. If missing, the Commands page gives a PowerShell command for the user to run (reads/writes the raw `HKCU\Environment\Path`, appends only this directory, and does not expand existing `%VAR%` references), plus manual instructions using Edit environment variables for your account.
- **Directory links:** Try symbolic links first; when a normal user lacks permission, use directory junctions (no administrator rights or developer mode required). Existing links are removed before replacement (Windows cannot rename over a directory); directory links are deleted with `remove_dir`. cc-switch falls back to copying whole directories when symlinks fail; this tool uses junctions so content follows source updates.
- **Command discovery:** Append extensions according to `PATHEXT` (`claude` → `claude.exe` / `claude.cmd`). Include common installation locations: `~\.local\bin` (official Claude Code installer), `%APPDATA%\npm`, Volta, Bun, and Scoop.
- **MCP:** On Windows, entries newly added or changed to commands such as `npx`, `npm`, or `node` are written as `cmd /c <command> …` (Claude Code `/doctor` warns about bare `npx`; cc-switch handles it similarly). Readback removes `cmd /c` for display and wraps it again on write. Entries whose commands were not changed remain intact.
- **Programs:** Read user PATH from the registry. npm global packages live in `<prefix>\node_modules` and commands are `<prefix>\<command>.cmd`; follow cmd-shim to identify package and version. Tool-installed programs get `.cmd` forwarding scripts in the entry directory (npm `.cmd` files locate files relative to themselves and cannot be linked or copied elsewhere). The npx cache is `%LocalAppData%\npm-cache\_npx` (npm documentation).
- **Paths:** Strip the `\\?\` prefix from `canonicalize` results (`dunce`); display paths such as `~/…` with `/`. Tool-invoked git disables newline conversion (`core.autocrlf=false`, for that invocation only), so downloaded skills match upstream byte for byte.
- **Real-time quota:** A status line script in Git Bash may see `/c/Users/…` or `C:\Users\…`. Both engine and script first normalize to lowercase `c:/users/…`, then encode the filename. Unix encoding is unchanged.
- **UI:** Windows shortcuts display `Ctrl+`. The Commands page explains the platform-specific write location and provides the command and manual instructions above if the command directory is missing from PATH.

Excluded or different from Unix: assistant installation and upgrade actions are not executed on Windows (assistants are detected only). Claude Code registry-managed settings (`HKLM`/`HKCU\SOFTWARE\Policies\ClaudeCode`) are not read, matching macOS's exclusion of configuration profiles. Secrets files lack permission bits such as `0600` and rely on default access controls in the user directory. Commands installed by uv on Windows are forwarding `.exe` files whose ownership cannot be recognized, so they show an unknown source.

Verification:

- Local cross-compilation: engine and desktop compiled for `x86_64-pc-windows-gnu` without warnings.
- Windows engine test executables ran under Wine 11 in Docker (Git for Windows MinGit supplied Windows git): all 399 tests across 19 test executables passed (program installation and rc-file groups run only on Unix and have 0 tests on Windows). The same code passed all 439 tests on macOS. Added Windows coverage includes command-script creation, rewriting, renaming, deletion, external-change rejection, preserving user scripts, and PATH hints; actual generated-script execution in Wine `cmd`, passing environment variables and arguments; and global npm cmd-shim ownership recognition.
- Unix-only tests: program installation and some upstream probes using shell scripts as fake npm / uv / git; Unix permission bits; assistant installation actions.
- Installer: cross-built NSIS in a Linux container according to Tauri documentation (`tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis`, about 6MB, unsigned; MSI requires Windows). Silent installation succeeded under Wine, placing application and uninstaller in the current user's `AppData\Local\Agent Manager`. Wine lacked WebView2 (an installed registry entry was simulated during installation), so the UI could not start, as expected.
- Not performed: desktop execution and installation on a real Windows machine (or VM), including WebView2 download; MSI; actual Claude Code / Codex / Git Bash status lines and networked npm installation. GitHub CI was still unable to start because of account billing at that time. A local Windows VM was not possible (no KVM inside the Docker VM and insufficient free disk).

## Linux Support (2026-09-30)

Linux shares Unix code with macOS. Launch commands are the only Linux-specific difference: most distributions default to bash and read `~/.bashrc` rather than `~/.zshrc`. New command locations now follow `$SHELL` (zsh → `~/.zshrc`, bash → `~/.bashrc`; on Linux with no `$SHELL`, use bash if only `~/.bashrc` exists). Other configuration files are synchronized only if they already contain the tool's block; missing blocks are not created. Other shells such as fish are not yet supported. Assistant discovery also includes Linuxbrew directories.

Verification in Docker Ubuntu 22.04 with Rust 1.88 and Node 22, using the same dependencies as the Linux leg of `gui-release.yml`:

- From a clean checkout: engine tests ran once as root and once as a normal user; all 441 passed as a normal user. All 23 desktop tests, 606 frontend tests, and the production build passed.
- `tauri build --bundles deb,appimage` succeeded: deb about 10MB, AppImage about 88MB.
- AppImage launched as a normal user in Xvfb (isolated data directory, bash user). Accounts, Commands, and Programs pages displayed correctly, including Chinese fonts. The Commands page stated that new commands are written to `~/.bashrc`. A launch command created through the UI targeted `~/.bashrc` in both preview and write, preserved existing content, and ran in `bash -ic`.
- Linux exposed an intermittent issue that was fixed: when two Codex session files had the same modification time, quota could come from the older session (filenames now break ties).
- Unverified: real desktop environments (GNOME / KDE window decorations, tray icons, input methods); arm64 Linux; installer installation on individual distributions.
