# Rust Native GUI Goals and Boundaries

> Current scope (2026-10-11): [Extensions only](EXTENSIONS-ONLY.md) supersedes every account-management, authentication, launch-command, account-credential, and assistant-lifecycle requirement below. Earlier implementation and acceptance entries are historical; they do not authorize these operations or establish acceptance of the new scope.

## 2026-10-12: Clear Source Actions and Complete Labels

Verification base: `07a3671f2cac420dc608d014ab437e1716984538`, plus the source-copy changes. Sources now uses explicit action labels: **添加并启用**, **仅添加来源（不下载）**, and **不再推荐**. Existing-source menus use **切换版本**, **编辑来源**, **停用来源**, and **移除来源**. Buttons retain their complete text; source URLs, paths, descriptions, and problem details wrap. Recommendation copy no longer refers to the retired terminal-shortcuts page. Operation handlers and account protections are unchanged.

Verification generated and read back locally:

- The unchanged page reproduced the literal ellipses and informal dismissal label at both widths. Three updated interaction assertions failed against that source. Focused frontend checks then passed 196 tests; the final source/mock suites passed 53. Full frontend: 57 files, 590 passed and 4 existing skips.
- The focused native recommendation case passed. Full serial engine: 456 passed across 22 test targets, including 8 extension-boundary cases. Desktop bridge: 27 passed. TypeScript checking, production frontend build, and the native debug build without bundling passed. Existing dynamic-import and engine dead-code warnings remain.
- Browser mock at 1000 × 650 and 760 × 650: recommendation and existing-source menus, confirmation, draft cancellation, dismissal, and long Git/local/rule paths passed. Rendered text bounds fit the controls and paths; no horizontal overflow or page errors occurred. Opening and cancelling dialogs made only resource reads; dismissal made no engine call. Screenshots were visually inspected.
- The isolated macOS preview displayed the revised recommendation, open menu, and confirmation through the actual desktop bridge in a 1000 × 650 window. Confirmation was cancelled. Its HOME contains no account or shell paths; no inherited credential environment was used.
- Formatting, documentation links, and whitespace checks passed. One intermediate browser rerun missed the timestamped fixture module after a source edit; matching that module and tracing its actual client restored the same checks without a product change.

The revised preview remains open for review. Installed-package, Windows/Linux desktop, and user acceptance are not established by these checks. No real account configuration, installer, tag, or release was changed or published.

## 2026-10-12: List Assistant Extension Targets Read-only

Verification base: `494ddd8e5d6c521c21b221562d544f63c201af5f`, plus the assistant-list changes. **Assistants** appears in navigation and search. It lists built-in and historical extension-target labels and their registered paths. It does not detect installation or login status, scan directories, execute assistants, or offer account/configuration actions.

`resources:targets` adds nullable configuration-directory, skill-directory, and MCP-file strings to its existing label response. It skips external secret-source loading. Account commands, immutable registration checks, and all extension write protections remain unchanged.

Verification generated and read back locally:

- Before implementation, both navigation/search tests and the native path-response assertion failed as expected. After implementation, the focused frontend checks passed 168 tests and the navigation suite passed 5. The new empty/error cases reject successful-looking fallbacks. A secret-cache regression proves listing skips refresh and that the missing-source fixture changes the cache when refresh is explicitly invoked.
- Full serial engine: 456 passed, no failures or skips across 22 test targets. This includes 8 extension-boundary cases. Full desktop bridge: 27 passed. Full frontend: 57 files, 590 passed and 4 existing skips. The first frontend run found the old exact navigation expectation; it now includes Assistants without weakening the assertion. TypeScript, production frontend build, and the native debug build without bundling passed; existing dynamic-import and engine dead-code warnings remain.
- Real isolated CLI: both command spellings returned metadata, and five retired account commands were refused. HOME/data remained absent. Native tests preserved fake authentication/session bytes and registration metadata.
- Browser mock: default, long-path, empty, and read-error states passed at 1000 × 650 and 760 × 650. Search opened Assistants; list rows had no buttons or editable fields. There was no page overflow, browser error, account command, or resource write. Default and long-path renders were visually inspected.
- The isolated macOS preview was opened with fresh HOME/data, a separate identifier, and no inherited credential environment. Its 1000 × 650 window displayed Claude Code and Codex through the actual desktop bridge. No account or shell paths were created in its HOME. Documentation links and whitespace passed.

The preview remains open for review. Installed-package, Windows/Linux desktop, real-account, and user acceptance are not established by these checks. No real account files, installer, tag, or release were changed or published.

## 2026-10-12: Compact Empty Pages at the Default Window Size

Verification base: `eeb2ca3cd05d029c5bb8fca7890d317f1520d028`, plus the layout changes. Shared empty states now use a small icon, left-aligned guidance, and an adjacent action. Skills' actual-content disclosure uses a secondary text entry. Data, resource actions, and account protections are unchanged.

The earlier native preview explicitly used 1000 × 800 instead of the committed 1000 × 650. The replacement preview inherits the committed geometry, with only a separate identifier and title. Production defaults and remembered user geometry are unchanged. [UI rules](UI-RULES.md) and [Desktop startup](subsystems/desktop-startup.md) now require this verification setup.

Verification generated and read back locally:

- Before/after browser comparison with the same empty Skills fixture: the empty-state panel decreased from 234 px to 86 px at both 1000 px and 760 px. The source navigation button and actual-content disclosure remained usable.
- All five shared-component pages were inspected at both widths and 650 px height: Skills, MCP, Sources, Programs, and task records. No page-level horizontal overflow or browser errors occurred. Populated Skills, search with no matches, clearing the search, and expanding/collapsing actual contents passed. Light and dark Skills renders were inspected. These fixtures performed no resource writes or account operations.
- Focused component suite: 4 passed before and after. Affected suites: 228 passed. Full frontend: 56 files, 584 passed and 4 existing skips. TypeScript checking and production build passed; the existing dynamic-import warning remains.
- Native debug build without bundling passed, with the existing engine dead-code warnings. The new macOS window was read back at exactly 1000 × 650 and visually inspected. It uses fresh HOME/data and no inherited credential environment. No account or shell files were created in the isolated HOME.
- Documentation links and whitespace checks passed. Full engine and desktop-bridge tests were not rerun because their implementation is unchanged. Installed-package, Windows/Linux desktop, and user acceptance of the revised proportions are not established by this receipt.

The replacement isolated preview remains open for review. No real account configuration, installer, tag, or release was changed or published.

## 2026-10-12: Share Read-only Account and Alias Tips

Verification base: `0cb7e6799673c06e4492223b752842489b32ddca`, plus the tips page. Settings and search now open **More tips**, with guidance about independent environments and terminal aliases. The page contains bundled text and generic `YOUR_*` examples. Its only action copies example text to the clipboard. It has no engine, filesystem, or process access and does not restore account or alias management.

The guidance distinguishes configuration directories from verified login isolation. It asks users to confirm identity through official tools, avoid copying credentials or sessions between environments, and check inherited environment variables. Bash/Zsh alias and function examples are illustrative; users must check and apply any command themselves.

Verification generated and read back locally:

- The two new interaction cases failed against the unchanged base because the entry and page were absent, then passed with the implementation. Affected navigation, settings, search, and UI-copy suites: 186 passed.
- Full frontend: 56 files, 584 passed and 4 existing skips. TypeScript checking and production build passed; the existing ineffective-dynamic-import warning remains.
- Isolated extension-boundary CLI suite: 7 passed. Fake account, settings, and session contents were preserved. Both example snippets passed Bash and Zsh syntax checks without executing their contents.
- Browser mock: Settings, More tips, and search were inspected at 1000 px and 760 px. Text and controls were readable, with no horizontal overflow. Both copied snippets were read back exactly. The tips page made zero engine calls; search made only resource-status reads. There were no page errors or account commands.
- Local documentation links and whitespace checks passed. Full engine and desktop-bridge suites were not rerun: this slice changes only frontend guidance, navigation, tests, and documentation. The focused CLI boundary check covers the continuing account restriction.

This receipt establishes source and browser-mock acceptance. Installed-application and native Windows runtime acceptance remain outstanding. No real account files or shell files were accessed by the tips page, no example command was executed, and no installer, tag, or release was published.

## 2026-10-12: Keep Dependency Programs Outside Account Boundaries

Verification base: `29d3b250b51fc051d70099df130ea42b6918d586`, plus the dependency-program guards. Assistant commands and npm packages are excluded by built-in and immutable historical recognition metadata, regardless of installation location. Direct lifecycle requests and declarations refuse them. Account metadata is not edited.

Programs now validates filesystem targets during planning and before each execution step. The checks cover account entry directories, historical cleanup links, backup restoration, internal directory aliases, metadata hard links, optional backup pruning, and failure rollback. Unsafe rollback preserves the original backup and reports the skipped targets. Ordinary dependency installation, upgrade, adoption, switching, and rollback still work. [Program Lifecycle](subsystems/programs.md) owns the contract.

Verification generated and read back locally:

- Isolated CLI boundary suite: 8 passed. The same first 7 cases against the untouched base passed the ordinary dependency control and failed all 6 boundary cases. Before their guards were added, further fixtures reproduced an overwritten fake account session through a metadata hard link and a deleted fake account backup directory through optional pruning.
- Runtime fixtures: redirected entry directories and rollback paths were refused; exact fake session/account bytes and the original rollback package were read back. All 9 Unix program tests and the portable ordinary/scoped lifecycle harness passed. No network registry or real assistant account was used.
- Full engine, serial (`cargo test --manifest-path engine-rs/Cargo.toml -- --test-threads=1`): 454 passed, 0 failed, including 241 library tests and all integration targets. The final default run stopped at 240 library passes and one `Parent did not finish` capture-fixture failure; its standalone retry also failed. The unchanged assertion passed in the complete serial run. Both failed logs remain evidence; no timeout or assertion was weakened.
- Desktop bridge: 27 passed. Frontend: 56 files, 580 passed and 4 existing skips; TypeScript checking and production build passed. The existing ineffective-dynamic-import warning remains. No frontend source or layout changed in this slice.
- Windows GNU source and test cross-check passed with the explicitly selected installed Rustup compiler. CLI smoke performed a real extension-policy write and structured readback under temporary HOME/data. Formatting, local links, documented test-target checks, and whitespace checks passed.

This receipt establishes source and isolated CLI verification. Native Windows runtime and installed-application acceptance remain outstanding. No real account, credential, session, installed application, installer, tag, or release was changed or published.

## 2026-10-12: Remove Unused Account Frontend Code

Verification base: `fda270f`, plus the frontend cleanup. Import-graph inspection from `main.tsx` and a repository reference sweep confirmed that 8 account/launch modules had no application callers: alias dialogs and their hook, runtime argument drafts, launch presets, connection presets, account seeding, and shared settings. They were removed with their 3 dedicated test files (31 tests). Active target recognition/grouping, resource assignment, and all 11 previously failing Rust tests remain.

Frontend verification: 56 files passed, 580 tests passed, and 4 existing skips; TypeScript checking and the production build passed. The prior frontend count of 611 decreases only by those 31 obsolete tests. The existing ineffective-dynamic-import build warning remains. Local links and whitespace checks passed. No active UI layout or real assistant environment changed; no installer was published. Backend legacy implementations still unreachable behind the retired-command gate are not claimed to be removed by this frontend slice.

## 2026-10-12: Reliable Command Capture Fixtures

Verification base: `67317b331f6dc549023b44604f08416e6cbe48ba`, plus the test-fixture changes. The unchanged engine test binary failed 6 of 12 focused capture tests at its Cargo output path; a SHA-256-identical private-directory copy passed all 12. The copied binary also passed the previously failing npm CLI probe and detached-descendant test. This establishes a startup-path effect, not its underlying OS mechanism or a product cleanup defect.

Short capture tests now compile a small std-only native parent/descendant fixture before starting their deadlines, following the existing integration-fixture pattern. Production command execution, timeouts, output limits, and account protections are unchanged. All readiness, parent-exit, delayed-write, and Windows handle-inheritance assertions remain. The helper moved out of the library test harness; it is no longer counted as a separate test.

Verification generated and read back locally:

- Full engine: 442 passed, 0 failed, including all 240 library tests and all integration targets. The 11 failures recorded on 2026-10-11 did not recur. Extension-only boundary acceptance remains 7 passed.
- Focused capture checks: 12 passed at the original Cargo output path. A negative-control copy with Unix guard-drop cleanup disabled failed the retained detached-descendant assertion with `Descendant survived the timeout`.
- Desktop bridge: 27 passed. Windows GNU source/tests and the standalone Windows fixture metadata cross-check passed using the explicitly selected installed Rustup compiler. Earlier compiler-selection attempts stopped at missing target standard libraries and are not project-validation results.
- Formatting and whitespace checks passed. No real account files, credentials, sessions, installed application, package, tag, or release were changed. Native Windows and installed-application acceptance remain outstanding.


## 2026-10-11: Extensions-only Source Candidate

Verification base: `3d9ef60c4e47391c98f92c50cf6ce6968c7252e1`, plus the extension-only changes. The user authorized commit and push after receiving the scope and test limitations below. [Extensions only](EXTENSIONS-ONLY.md) owns the current product boundary.

Account, authentication, launch-command, provider/runtime-setting, and assistant-lifecycle operations are retired at the native entry point and desktop command whitelist. Navigation and search expose extension resources. Existing registration metadata is frozen and used only to locate and protect resource targets. Backups omit account/alias trees and refuse imports that change them. MCP edits preserve bytes outside the extension container; extension secrets cannot edit or borrow protected account variables.

Verification generated and read back locally:

- Boundary acceptance: 7 passed, including actual CLI MCP/rules/skills writes under temporary HOME/data, preserved identity/settings/session bytes, untouched other targets, immutable imports, shared-configuration byte preservation, and protected secret sources. The same 7 tests against the base source failed, establishing a negative control.
- Resource safety: account-path guards 5, skill-target guards 6, configuration backup 9, core 16, discovery 7, MCP 53, rules 24, secrets 15, skills 36, system audit 6, and upstream 7 passed. The final secrets check explicitly refuses a legacy target without a verifiable credential identity and preserves its source and registration instead of repairing the account.
- Desktop bridge: 27 passed. Frontend: 59 files, 611 passed and 4 existing skips; typecheck and production build passed. The production JavaScript contains none of the retired account-status, account-command, alias-status, or secret-migration endpoints.
- Browser mock: skills, MCP, rules, sources, programs, settings, MCP secrets, and search were checked at 1000 px and 760 px, with screenshots and no page-level horizontal overflow. Account and launch-command navigation and search entries were absent. This is frontend acceptance, not installed Tauri or real-assistant acceptance.
- CLI smoke: real extension-policy write and JSON-structured readback passed under temporary HOME/data. Formatting, shell syntax, local Markdown file links, and whitespace checks passed.

The full engine check is **not green**. The complete serial run finished with 432 passed and 11 failed; every integration-test target passed, and only the library target failed. Its failures concern CLI probing, program capture, shell-path capture, upstream capture, and detached-process cleanup. The earlier default run also had two MCP initialize timeouts; both cases passed in the complete 53-test serial run. Fixture expectations that still attempted retired account writes were corrected and rechecked; no deadline was enlarged and no valid failing assertion was skipped.

Windows source and test cross-check passed for `x86_64-pc-windows-gnu` using the installed Rustup toolchain. The first attempt with the active Homebrew compiler stopped before project validation because that compiler lacked the target `core` crate. No toolchain was installed or changed. Windows runtime acceptance remains outstanding. No real account, credential, session, installed application, package, tag, or release was changed or published. This receipt establishes source verification, not installer acceptance or a full-suite pass.

## Historical Conclusions

- **Current version (2026-09-29)**: implementation follows the [multi-account design](MULTI-ACCOUNT-DESIGN.md). Resource domains are accounts, skills, MCP, rules, launch commands, and sources. Hooks, third-party switching in the main configuration, connection alternatives, runtime settings forms, Overview, and operation history have been retired. The x86_64 DMG based on `2e70d14` passed independent package acceptance. An API account was created through the GUI under an isolated HOME and read back with real Claude; see "Delivery Receipt (2026-09-29, Multi-account Version)". The following conclusions describe earlier stages; references to Hooks, six resource domains, Overview, provider switching, and switching back to the official account no longer apply.
- The core is ready: a shared Rust/SQLite/Tauri library. Since 2026-09-22, actual configuration is the source of truth (see the decision below and `GUI-REDESIGN.md`). Source, isolated live checks, and x86_64 package acceptance on a real machine are complete; see the 2026-09-23 delivery receipt.
- On 2026-09-24, authentication was corrected against official standards: independent environments no longer inherit shell authentication, login checks inspect only the directory's own login, invalid Codex configurations are identified, and usage is deduplicated and counted by instance. Source, full tests, and real CLI E2E passed; the package was not rebuilt. See "2026-09-24: Authentication Corrections Against Official Standards".
- Independent review that day led to 7 more corrections, including forked usage attribution, status redaction, invalid definitions, subprocess isolation defaults, and Linux restrictions, plus system-level settings notices. See "2026-09-24 Follow-up: Corrections After Independent Review".
- Complete wiring covers six domains (Agents, Skills, MCP, Rules, Hooks, Aliases), read-only `system:audit`, `secrets:status`, `system:discover` (takeover discovery, below), and `system:refresh-upstream` (agents/mcp/skills probes cached in SQLite `upstream_probes`, freshness filled into status for all three domains, and explicit `--action` added to frontend convergence calls).
- The 2026-09-12 advanced-management decision is implemented: native Aliases (`b7f30db`); Tools, snapshots, and `system:converge` retired (`9080c1d`).
- Remote HTTP MCP is implemented (`b353aac`): all three clients persist references without plaintext, with the same security semantics as local stdio.
- 2026-09-14 milestone (historical): a clean Darwin x86_64 DMG based on `5ae995b` passed the lead reviewer's independent acceptance; four-platform release workflow source (`a2cc864`) and blocking Windows CI (`9f56f00`) are ready.
- 2026-09-15 milestone: the x86_64 DMG at acceptance baseline `5a110be3` passed verify and the main real GUI E2E checks independently. An arm64 DMG cross-built from the same source passed static acceptance (thin arm64, ad hoc linker-signed, not run on arm64 hardware). The complete matrix had 9/9 exit 0: Rust 269 tests, Vitest 246 tests, typecheck/build passed with 0 warnings.
- Current behavior: public `*:plan` and `--dry-run` entry points are retired and rejected. Status/query is read-only with zero writes. Actions directly execute real apply, retaining internal preflight, write-time validation, exact scope, exclusive locks, external-change detection, CAS/conditional rollback, and audit. Saving a definition still only saves; destructive definition removal and log clearing retain separate confirmation.
- This direct-apply interaction passed source, browser, isolated CLI, and uncommitted dirty candidate DMG acceptance: Engine 278 tests, Tauri 11 tests, Vitest 28 files / 248 tests, typecheck/build, 8 schemas, and isolated smoke checks. Old DMG/GUI E2E evidence does not prove the new interaction. A final clean DMG was rebuilt after `a01d0f5` and independently verified; see the receipt below. Still outstanding: real GUI acceptance of the MCP add-service form (macOS automation input limitations); public cross-platform release prerequisites (GitHub billing, real keys, Developer ID/notarization, Windows signing, real Linux/Windows/arm64 installation and runtime checks, and signed updater download/install/readback). No tag, GitHub Release, or public package was created; existing authorization excludes these actions.
- 2026-09-16 release-readiness repository changes completed in three commits: `d46b85a` added the MIT license, public release metadata, explicit Webview CSP, and app icons, with Bundle ID `io.github.ly87ing.agent-manager`; `47702ed` added a top-level frontend error boundary, custom-agent installation/upgrade confirmation, MCP empty-state guidance, and retired-code cleanup; `14e6790` changed the release workflow to draft-first, single-writer orchestration (create-release creates the sole draft, matrix jobs upload by releaseId, final job alone writes latest.json), with prerelease tags, per-platform SHA256 checksums, engine-rs version consistency gates, and truthful notarization-ticket checks in the signed verify branch. Public release remains undone: GitHub billing, repository Secrets/Variables, signing/notarization material, Linux/Windows/arm64 machine acceptance, and the real updater chain are not ready. No tag or Release was created.
- 2026-09-20 usability milestone: the agent page focuses on the connection workspace, runtime settings, and account instances. AI CLI installation/upgrade/removal scripts and full definition editing are removed from the main product path. Adding a connection saves its full definition and optional credentials before enabling it. Client installation status, version, and source appear only in read-only Settings diagnostics. Existing database lifecycle fields remain for phase-one compatibility and are not physically removed in this round.

## Goal

Share one native Rust library between CLI and Tauri. Actual assistant files determine effective configuration; `native-manager.db` stores preferences, ownership records, and inactive alternatives. Settings live in `app_settings`, with no writable JSON settings or profile layer.

## Completed

- `engine-rs` native/store/envelope core serves CLI and Tauri.
- SQLite reads without a database perform zero writes; transactional saves survive reopening and readback.
- Tauri bridge command, argument, run ID, and asynchronous boundaries.
- Policy builtin/sqlite sources and fixed `--layer local` writes.
- Official `native` default channels for Claude Code and Codex; brew is handled only when detected as the actual source.
- Root-repository Python and old `manage.sh` entry points removed.
- Runs list/details use SQLite `operation_logs`, pruning expired records when adding a record. Local structured diagnostics rotate daily at `data_dir/logs/agent-manager-YYYY-MM-DD.jsonl` (strict redaction, Unix 0700/0600, zero-write reads, symlinks rejected), pruning expired files after writes and at app startup. Both use `log_retention_days` (0 disables automatic cleanup). Execution Center `runs:clear` and Settings `logs:clear` each require a second confirmation and clear their own log category independently.
- Native agent runtime configuration and read-only installation-source detection. Existing lifecycle apply, locks, and audit remain only for compatibility, without normal GUI entry points.
- Complete native Skills domain: builtin repositories are recommendations, becoming user collections only after addition (since 2026-09-22; previously frozen on the first database write). Collections and Skill×Agent bindings independently support update/enable/disable/remove. Content-addressed snapshots, safe symlinks, rollback protection, zero-write status, cross-process apply locks, and operation audit are implemented.
- Native MCP/local stdio: Rust dispatcher provides read-only `mcp:status` and real `mcp:apply`; SQLite stores definitions and assignments. Claude JSON, Codex TOML, and OpenCode JSON are supported with the corresponding Agent target configured. Per-agent add/update/enable/disable/remove and library enable/disable/remove are supported. Status performs zero writes; apply uses locks, audit, CAS, and rollback. SQLite definition/binding env fields persist only literal `${VAR}` references. Status, diagnostics, and operation logs never expose or save resolved values. Real apply resolves process environment values and writes the target codec's Claude/Codex/OpenCode configuration; target plaintext is protected by file permissions and client security boundaries.
- Native `sources:get/set`: rule sources fixed to `rules.shared`, `rules.codex`, `rules.claude`, `rules.opencode`; revision CAS saves only SQLite without synchronization. (These rule keys were retired on 2026-09-22 in favor of actual files plus templates; see "Actual Configuration as the Source of Truth".)
- Native Rules: read-only `rules:status` and real `rules:apply`; fixed targets `rules:codex_global` (`~/.codex/AGENTS.md`), `rules:claude_global` (`~/.claude/CLAUDE.md`), `rules:opencode_global` (`~/.config/opencode/AGENTS.md`), rendered from shared and assistant-specific sources. Zero-write status; apply uses a global exclusive lock, audit, file-byte CAS, transaction/file rollback, and fail-closed external-change protection without a backup promise. Sources render directly into targets without an intermediate directory. Tauri's argument allowlist and Rules page match the real schema and support per-target `--only`.
- Native Hooks: read-only `hooks:status`, real `hooks:apply`, and Hooks-only revision-CAS `sources:get/set` (`hooks.claude/hooks.codex`) writing only SQLite. Fixed targets: Claude `~/.claude/settings.json` signature-set merge preserves user fields/unmanaged hooks; Codex `~/.codex/hooks.json` uses whole-file writes. Zero-write status; apply uses the global exclusive lock, Immediate transaction, exact old-byte CAS, bindings, audit, and conditional rollback, rejecting external changes. Both frontend targets and JSON editors are wired.
- Native Aliases: read-only `aliases:status`, real `aliases:apply`, and Aliases-only revision-CAS `sources:get/set` (`aliases.shell`) writing only SQLite. Alias name/value structure is validated and suspected credentials rejected. Managed marker blocks in fixed `~/.zshrc` and `~/.zprofile` targets preserve outside bytes. Zero-write status; apply uses the global exclusive lock, Immediate transaction, whole-file byte CAS, bindings, audit, and conditional rollback; external changes fail closed without a backup promise. Existing frontend Aliases and source panels work without changes.
- Read-only `system:audit` aggregates agents/skills/mcp/rules/hooks/aliases/secrets and runtime/storage status, supporting Tauri offline bridge injection.
- Secrets workflow: `secrets:status` exposes only variable names, sources, and resolvability. After explicit selection of an external mode-`600` `.env` file, entry and takeover write only there. SQLite stores only the path; configuration backups exclude paths and values. Historical `secret_values` is used only for explicit migration: successfully write the file before deleting the old table.
- 2026-09-12 retirement cleanup: Tools, snapshots (`snapshots:list/restore`), and `system:converge` commands, frontend pages/types/text, contract schemas, seeds, and documentation status entries removed. Retired commands return UNKNOWN_DOMAIN/UNKNOWN_VERB; six-domain apply output no longer includes snapshot fields. Automatic-backup promises became fail-closed descriptions. The physical `agent-tool/` directory remains untouched.
- `system:refresh-upstream`: probes enabled npm agents, MCP npx packages and uvx `git+repo@branch` sources, and enabled skills collections with URLs. Probes (`npm view`, `git ls-remote`, `git rev-parse`, npx cache reads, `bin --version`) use argument arrays without a shell, bounded capture, and process-group termination on timeout (network ≤30s). Individual failures become unknown plus fixed error categories without failing the batch. Comparison matches the old behavior (comparable semver → current/behind/ahead; incomparable hashes → different means behind). SQLite `upstream_probes` caches per-element upserts in Immediate transactions, storing only version/commit and error category. Status reads freshness without writes/network; missing tables/rows mean unknown. `--offline` returns OFFLINE_NETWORK_DENIED; `--only` targets individual elements without the apply lock. Updates directly triggers real apply with internal preflight/write-time checks and explicit `--action` (agents→upgrade, mcp/skills→update).
- Remote HTTP MCP: `url` accepts only https or loopback http, never echoed in errors; optional `headers` contain only `${VAR}` placeholders. Remote/stdio fields are mutually exclusive; codecs infer `type`, which is not persisted. All three codecs write references without plaintext: Claude `type:"http"` with literal `${VAR}`, Codex `env_http_headers` variable-name mappings, OpenCode `type:"remote"` with `{env:VAR}` and fixed `oauth:false`. Managed markers move to entry-level `_MANAGED_BY`, retaining the old stdio location for compatibility. Save-time policy:set validates full definitions; status/apply audit masks headers and env alike as `<literal>`. secrets:status includes headers and distinguishes `kind`; missing variables fail closed.
- Visible Skills discovery failures (`8a428de`): collection observed elements include `discovery_error` (category, skill_paths entry, fixed message), rather than silently appearing empty. All Skills empty states guide users based on collection availability; collection cards show discovery errors.
- MCP discovery/import/takeover (`a4ae118`): `mcp:status` adds read-only `import_candidates` scanned from unmanaged entries in all three target formats. Env/header values are `<literal>`; Codex `env_http_headers` references pass through. Managed entries, library name collisions, bindings, and invalid names are excluded. Frontend converts candidates to `${VAR}` definitions in one step without plaintext leaving the renderer; saving performs no action. Explicit `--allow-takeover` (only per-agent add/update) replaces unmanaged same-name entries; default fail-closed behavior remains. Apply audit always includes takeover.
- macOS candidate build identity (`3b28f45`): release candidates embed trusted commit/date identity and strictly reject artifacts mismatched with source.
- Verbatim definition saves (`5ae995b`): SourceDefinitionsPanel disables automatic text substitution with unit coverage; Rules/Hooks originals are no longer rewritten by smart quotes or similar features.

## Contract Requirements

Resources such as Skills and MCP, and their per-agent assignments, must support add, edit/update, enable, disable, and delete. Saving definitions must not execute actions. UI directly triggers real apply with internal preflight/write-time validation, exact scope, locks, external-change detection, CAS/conditional rollback, and audit. Ordinary synchronization, enable/disable, and updates need no second confirmation; destructive actions require separate confirmation. The normal AI CLI interface manages only connections, runtime parameters, and account instances. Installation, upgrade, removal, and full definition editing belong to the original software channel; existing lifecycle fields and engine capabilities remain for phase-one compatibility.

## Acceptance Baseline (2026-09-12, Fourth Round)

Bound commits: Aliases `b7f30db`, retirement `9080c1d`, refresh-upstream `7d9e83b`, remote HTTP MCP `b353aac`.

- Automation: engine-rs 242 tests, app/src-tauri 11, frontend 229, all passed; `pnpm typecheck`, `pnpm build`, and `cargo fmt --check` passed.
- Real isolated CLI: the previous twelve Aliases negative cases, UNKNOWN_DOMAIN/UNKNOWN_VERB retired-command checks, and refresh-upstream shim matrix still passed. Remote HTTP MCP: three save-time negative cases (http URL, mixed fields, plaintext headers) returned USAGE_ERROR without echoing content; all three codecs rendered without plaintext (zero sentinel grep matches); missing variables failed closed; removing a marker refused takeover; unmanaged entry fields failed closed; `_delete` spared unrelated data; secrets:status included `kind:"header"`.
- Frontend mocks passed, including Updates inline convergence `--action upgrade` and remote MCP form/card masking.
- Boundary: this does not establish real Tauri frontend/backend business E2E for the current package. Real-client entry-marker/reference-expansion integration belongs to package E2E.

## Acceptance Baseline (2026-09-14, Fifth Round)

Bound commits: package baseline `5ae995b`, release workflow source `a2cc864`, blocking Windows CI `9f56f00`.

- Independent package acceptance by the lead reviewer: clean Darwin x86_64 DMG at `5ae995b` passed `verify:macos`: read-only mount, copy outside the repository, architecture/version, no Python, source/HOME path redaction, and 5-second survival in isolation from `/`. Embedded commit/date matched source. SHA256 `71a386d6ab8e1a7f85ef59c82f379411e9aa1dccaa3c3166309b96b971fce26b`, 7632599 bytes. Later `a2cc864` and `9f56f00` change workflows only; this package still represents product code but contains neither workflow and is not a HEAD build.
- GUI business E2E never started: host AX/computer-use returned permission_denied for Agent Manager and Finder; 8 scenarios were not run. Isolated HOME/data had no database/target writes. This is acceptance-infrastructure blockage, neither product success nor failure.
- Release workflow (source acceptance only): `a2cc864` gui-release.yml covers macOS arm64+x86_64, Linux x86_64, Windows x86_64; app/dmg, AppImage/deb, NSIS/MSI artifacts; updater signing/latest.json, Apple/Windows signing, Apple notarization, fail-closed missing-key/placeholder/dirty-tree/source checks, temporary certificate cleanup, and draft-only Releases. Local independent acceptance on 2026-09-14 proves YAML/structure, positive/negative gates, configuration merging, simulated path remapping, and cargo fmt only. No real build, signing, notarization, or updater publication occurred.
- Blocking Windows CI (`9f56f00`): engine/app gates are mandatory (cargo fmt/test/check, engine release build, NSIS/MSI build, no continue-on-error). GUI CI run 34935947828 had six jobs with steps=[]; GitHub payment/spending-limit failure prevented startup, rather than tests failing.
- External state: GitHub Actions secrets=0, variables=0, valid local codesigning identities=0. Signing, notarization, actual arm64/Linux/Windows packages, and updater publication remain unverified. No tag, GitHub Release, or public package was created; current authorization excludes those actions.

## Acceptance Baseline (2026-09-15, Sixth Round)

Bound acceptance commit: `5a110be3da66d1bedee906eb0c30f04b5b95b245`.

- Complete automated matrix: 9/9 exit 0; engine-rs 258 + app/src-tauri 11 = Rust 269 tests passed; Vitest 28 files / 246 tests passed; `pnpm typecheck`/`pnpm build` passed with 0 warnings.
- x86_64 DMG independently accepted: verify exit 0; SHA256 `076d16b098ede8915802c0bc6dceec42dc06e0dcadddd137e7865681c7e90a68`, 7632606 bytes; embedded commit/date exactly matched source; unsigned.
- arm64 DMG (cross-built, static acceptance only): SHA256 `a9540a2690b9d2ede16ef627f310e277ca49130e608c518b876aeaed0bbfd3f8`, 7298163 bytes; thin arm64; ad hoc linker-signed, not Developer ID; not run on arm64 hardware.
- Real GUI E2E (x86_64 package, independently reviewed, old plan→confirm→apply interaction now obsolete and not evidence for direct apply) covered: system:audit zero-write missing-database reads, Rules cancel zero-write/apply, both Hooks and Aliases targets, MCP stdio+remote HTTP GUI plan/confirm/apply, Secrets names/presence only, Runs, and fail-closed external-change plans. MCP definitions were inserted into isolated SQLite fixtures through the shared Rust CLI. The add-service form was not accepted due to macOS automation input limitations; fixture success does not substitute for form acceptance.
- Zero-write/redaction readback (obsolete old dry-run evidence): stdio dry-run left database/target hashes unchanged without operation-log writes. Codex remote headers were written as environment references; synthetic values appeared in neither target nor diagnostic/operation logs.
- External state rechecked 2026-09-15: GitHub Actions secrets=0, variables=0, valid local codesigning identities=0, GitHub Releases=0. GUI CI run 34936884572 bound to `5a110be3` had six jobs with steps=[]; payment/spending-limit failure prevented startup, not test failure.
- Public cross-platform release prerequisites remain unmet: billing, real keys, Developer ID/notarization, Windows signing, actual Linux/Windows/arm64 installation/runtime checks, and signed updater download/install/readback. Current authorization excludes tags, Releases, and public packages.

## Delivery Receipt (2026-09-16, Final Clean Direct-apply DMG)

Bound final commit: `a01d0f5b182c9da346a6663688b49ed4a83f31f4` (2026-09-15T17:32:20-07:00).

- Automated matrix independently rerun 2026-09-16: engine-rs 278, app/src-tauri 11, Vitest 248 tests passed; `pnpm typecheck`, `pnpm build`, both `cargo fmt --check` passed.
- Final clean x86_64 DMG rebuilt locally after the final commit on 2026-09-16, independently accepted: all `verify:macos` checks passed (platform/metadata, exact `a01d0f5` commit/date, no Python linkage/legacy artifacts, source/HOME redaction, 5-second survival and graceful exit under isolation from `/`); unsigned. SHA256 `1ba291cc794c00a1c68152276dad0b04f792b1b8ae1139156f0e6c24decf9898`, 7572035 bytes.
- GUI CI external blockage rechecked: run 35040441270 at `a01d0f5` had six steps=[] jobs failing before startup due to payment/spending limits, not tests.
- External state rechecked 2026-09-16: GitHub Actions secrets=0, variables=0, valid local codesigning identities=0; public release blockers unchanged.
- Repository acceptance of direct apply is complete. Remaining work: real MCP add-service GUI acceptance (macOS automation input limitations) and external public-release prerequisites.

## Delivery Receipt (2026-09-16, Final Clean Release-readiness DMG)

Bound final commit: `5a1582d004c7fe75d6744614b3a475efe3c75c12` (2026-09-16, including four release-readiness commits `d46b85a`/`47702ed`/`14e6790`/`5a1582d`).

- Matrix rerun by the lead reviewer 2026-09-16: both `cargo fmt --check`, all engine-rs cargo tests, 11 app/src-tauri tests passed; Vitest 29 files / 259 tests passed (+11: ErrorBoundary 4, MCP empty state 3, custom-agent confirmation 4); `pnpm typecheck`/`pnpm build` passed.
- Final clean x86_64 DMG rebuilt locally after the final commit on 2026-09-16, independently accepted: `verify:macos` passed Bundle Identifier `io.github.ly87ing.agent-manager`, version 0.1.0, exact `5a1582d` commit/date, no Python linkage/legacy artifacts, redacted paths, 5-second survival and graceful exit under isolation from `/`; unsigned, notarization ticket reported "not applicable" per script semantics. SHA256 `5936656ad180f2ec42e9aa6829f762acbcfd153c45c753bb3e9b2e7dbe27a175`, 7572750 bytes.
- Workflow `14e6790` received YAML/script static acceptance and positive/negative checks (tag regex, version gate, latest.json assembly, checksums, stapler branch), without real execution. External blockers (billing, secrets, signing/notarization, machine acceptance, updater chain) remain; no tag or Release.
- Repository release-readiness work is complete and accepted. Remaining work is unchanged: real MCP add-service GUI acceptance and external public-release prerequisites.

## Delivery Receipt (2026-09-17, UI Polish)

Bound final commit: `bcdcb6e` (2026-09-17, four polish commits `e4debd2`/`8b4e753`/`6255aa1`/`bcdcb6e`, drawing on cc-switch design/usability).

- Changes: design tokens (CSS variables and semantic color tokens across all components); light/dark/system themes with native title-bar synchronization and first-paint flash prevention; tray residency (close to tray, restore/quit, macOS Dock Reopen, explicit icon removal on exit); remembered window position/size intentionally excluding visibility/fullscreen; single-instance restore; page/modal transitions with prefers-reduced-motion fallback; skeleton loading and reusable empty states; agent/skills/MCP/history search composed with existing filters; global ⌘,/⌘F shortcuts; update badge at Settings (delayed startup check, per-version ignore, no check without configuration).
- Matrix rerun by the lead reviewer 2026-09-17: both `cargo fmt --check`, all engine-rs tests, 15 app/src-tauri tests (+4: theme parsing, window flags, exit classification), Vitest 35 files / 298 tests, `pnpm typecheck`/`pnpm build` passed; one harmless dynamic-import notice.
- Final clean x86_64 DMG rebuilt locally 2026-09-17 and independently accepted: `verify:macos` passed exact `bcdcb6e` commit/date, 5-second isolated survival and graceful exit covering tray startup; unsigned. SHA256 `29188131ae0f46c1112a8f0b12987e3da82c21bff0887ff61555a15f68a9d809`, 7873725 bytes.
- Unaccepted: real GUI tray clicks, Dock Reopen, close-to-tray (to avoid disturbing the host desktop; isolation tested startup/survival only). Dark appearance was spot-checked with dev-server screenshots.
- Repository UI polish is complete and accepted; external public-release conditions (billing, signing/notarization, machine acceptance, updater chain, tag/Release authorization) remain unchanged.

### 2026-09-17 Follow-up: Unplanned Skills Link Semantics (`847d703` + `b313716`)

- Real-machine feedback: legacy link rows in All Skills showed the contradictory "Linked + Unplanned link + Add"; adding an unmanaged link inevitably failed closed on external-conflict protection.
- Changes: extra-link actions become "Adopt" (add) and "Unlink" (remove). Per-skill `skills:apply` add/remove supports explicit `--allow-takeover` for unmanaged symlinks only; real files/directories still fail closed. Audit always includes takeover, matching MCP. Status cells add `managed`; Tauri allows the flag. Frontend extra-link labels become Adopt/Unlink with confirmation for unmanaged links; Current Assigned Version checks state for link existence, fixing not-assigned rows falsely showing Linked.
- Verification: engine-rs 279 (new full-chain positive/negative extra-link takeover cases), app/src-tauri 15, Vitest 303, typecheck/build passed.
- DMG rebuilt at `847d703`, independently accepted: `verify:macos` passed; SHA256 `555b744ef97c85614bcdce750bdf822cf043b052a64484882427a8d9ddbe8830`. Single-instance behavior caused acceptance copies to exit immediately when another instance ran, falsely failing survival checks. `b313716` added a running-instance guard to the verify script; recheck passed. The DMG excludes `b313716`, which changes only acceptance scripts, not product code.

### 2026-09-17 Follow-up: Skills Matrix Batch Actions (`26b94b3`)

- All Skills adds selection and batch enable/disable (direct execution) and adopt/unlink (one aggregate confirmation with `--allow-takeover` when unmanaged rows exist). Frontend applies rows sequentially, each with its own run_id/lock/audit/rollback; failures do not stop remaining rows. An aggregate toast reports results; the execution channel's new `silent` option suppresses per-row toasts without changing individual operations.
- Verification: Vitest 36 files / 313 tests passed (8 new batch cases + 1 individual regression), typecheck/build passed; rebuilt `26b94b3` DMG independently passed `verify:macos`.

### 2026-09-17 Follow-up: Execution Feedback and Version Hints (`0419d3c`)

- Real-machine feedback: Upgrade gave no immediate response, leaving users unsure whether it started or which versions it involved.
- Changes: card actions immediately show an Executing spinner and disable other actions on that card, including uninstall/custom-confirmation paths. Upgrade labels/titles show Current → Latest, Unknown for missing versions, abbreviated git hashes, and the same range in toasts/history. UpdatesPage inline/batch actions also show versions and execution state.
- Existing jsdom flakiness under default parallelism was fixed by increasing vite.config.ts testTimeout/hookTimeout to 20s; all 36 files / 320 cases then passed in default parallel execution.
- Verification: Vitest 320, typecheck/build passed; rebuilt `0419d3c` DMG independently passed `verify:macos`; SHA256 `05f1018010a0cecc1cfbc318ad790778be9865b0bacc34b2e0d3f323d5fa6f73`.

### 2026-09-17 Follow-up: Takeover Center (`04c99e6` + `f5e0273` + `763a988` + `d3faf30`)

- User request: discover, classify, and adopt local agent/mcp/skill resources in one operation. Official paths/native installations retain native management; others use symlinks.
- Before implementation: agents detected installation sources only for defined agents, not undefined ones. Skills exposed extra links only within defined collection matrix cells, missing unassigned target entries; collections required git URLs, with no external-local registration. MCP had manual `import_candidates` without source classification.
- Engine: read-only `system:discover` (`--only agents|skills|mcp`, zero writes/network, no apply lock) returns `manage` classifications (native / symlink / collection / register / cleanup). Local skills collections add `source_path`, mutually exclusive with `url`; they directly link registered source directories without content snapshots. Registered roots become managed baselines; collection updates only rescan/repair links without network. Known-agent CLI discovery suggests definitions by source, supplying management commands only after successful npm reverse lookup. MCP reuses masked scanning and adds origin/manage. Tauri commands/argument allowlists admit `system:discover`.
- Frontend: Takeover Center is the fifth navigation item, with categories, batch selection, one aggregate confirmation, and sequential unified execution (independent run_id/history/audit); failures do not interrupt later entries. Agent registration saves definitions only; Skills/MCP use existing real domain apply.
- Separate fix `04c99e6`: `definitionKeyPrefix("agents")` wrote unread top-level `agent_definitions`, making custom definitions ineffective. It now writes the engine's real `agents.definitions`.
- Completion `763a988` / `d3faf30`: dev mock supports `system:discover`, preventing errors under `?mock=1`; collection forms choose git repository or local directory, avoiding git-URL validation failures and destructive URL insertion when editing adopted local collections.
- Automation: engine-rs 289, app/src-tauri 15, Vitest 37 files / 330 tests, `pnpm typecheck`/`pnpm build`, both `cargo fmt --check` passed.
- Real isolated CLI acceptance (HOME + AGENT_MANAGER_DATA_DIR, no mocks): Skills replicated external-tool links, real official-directory folders, and broken links. All 11 checks passed: discovery classification, source-root grouping, registration/per-agent add, direct external-source links, no snapshots, external-update propagation, disable removing only links with sources intact, managed entries leaving candidates, and unchanged database hashes for reads. All 7 MCP checks passed: masked discovery, import/takeover, stdio env markers, SQLite reference-only definitions/status without plaintext, preservation of unrelated user fields, and fail-closed same-name unmanaged entries without `--allow-takeover`. Registered `agents.definitions.<id>` appeared in `agents:status` with state=unmanaged-detected, definition_origin=custom, and no upgrade/removal actions.
- Final clean x86_64 DMG rebuilt after `d3faf30`, independently accepted: `verify:macos` passed Bundle Identifier/version, Mach-O architecture, no Python linkage/legacy artifacts, path redaction, exact `d3faf30` commit/date, 5-second survival/graceful exit from `/` with isolated HOME/DATA_DIR/minimal PATH. Unsigned; notarization "not applicable" per script. SHA256 `51c33f05882170f2f01e1711beeb79ecc27781c6de4517209d89393ed3131687`, 7942641 bytes.
- Browser acceptance (dev server + mock, Chrome DevTools): six source categories rendered correctly; Select All Adoptable selected 6; confirmation stated that 3 replace same-name external entries; toast reported success 6/failure 0 and cleared selection. No errors except dev-server favicon 404; no horizontal scrolling. This proves rendering/orchestration, not real engine writes.
- Unverified/unfinished: broken links/legacy remnants are identified only, requiring manual removal. Discovery uses a fixed known-CLI list, excluding others. Package GUI takeover clicks were not accepted to avoid real host writes; isolated CLI acceptance covers write semantics.

### 2026-09-17 Follow-up: Launch-command UI and Usability Review (`873c63f` + `d01a18e`)

- User request: configure each agent's launch alias/arguments, timezone, and additional parameters through the UI.
- Six-domain review counted steps and whether users must handwrite structured text:

  | Domain | Add/edit | Enable/disable | Delete | Execute | Editor | Conclusion |
  | --- | --- | --- | --- | --- | --- | --- |
  | Agents | Form | Read-only card | Unmanage | Direct install/upgrade/remove | Structured | Enable/disable and auto-install/update cannot be changed; see below |
  | Skills | Form | Collection + per-agent matrix, including batch | Yes | Direct | Structured | Complete |
  | MCP | Form | Library + per-agent | Yes | Direct | Structured | Complete |
  | Rules | Text | — | Clear to remove | Synchronize | Text; content is Markdown | Appropriate |
  | Hooks | Handwritten JSON | — | — | Synchronize | Raw JSON | Still requires schema-based handwriting; see below |
  | Launch commands | Added this round | — | Added this round | Synchronize | Structured | Completed this round |

- Agent cards gain Launch Commands (effective synchronized line, per-item editing, direct synchronization when configured). Editor fields cover name/command/timezone/language/additional arguments with live preview and common timezones first. Unsupported environment-variable-plus-command syntax switches to original-text editing without rewriting user syntax. Settings Terminal Shortcuts becomes structured (global timezone/language, per-item CRUD, expandable raw editor). Engine unchanged: timezone/language substitution, injection checks, and managed-block-only writes already existed.
- Automation: Vitest 39 files / 360 tests passed (13 pure-function, 9 shortcuts-page, 4 card-command cases); `pnpm typecheck`/`pnpm build` passed.
- Real isolated CLI: all 12 checks passed, covering save without shell writes, global timezone substitution, explicit timezone independence, preserving user content/zprofile synchronization, global changes affecting only followers, rejecting single-quote values, and removing only the selected line.
- Browser (dev + mock): launch sections/editor prefill, global shortcut defaults, and item lists rendered correctly without console errors.
- Final clean x86_64 DMG after `d01a18e`, independently accepted: `verify:macos` passed exact commit/date, 5-second isolated survival/graceful exit; unsigned. SHA256 `4d901ab66775383ca7d5fb183d215567f458922272162bf106665c29d63af517`, 7947490 bytes.
- Gaps identified but deferred for a decision: Hooks remains handwritten JSON without event/command forms; enable/disable and auto-install/update cards remain read-only, changeable only through handwritten Personal Configuration; broken/legacy links remain identification-only in Takeover Center.

### 2026-09-17 Follow-up: Agent Instances and Custom Launch Commands (`2204621` + `8893e91`)

- User decision: cc-switch's global-configuration switching allows only one account at a time. The proposed independent directory plus launch alias (`CLAUDE_CONFIG_DIR=… claude …`) supports concurrent clients; the lead reviewer verified and adopted it.
- Measured prerequisites, not documentation inference: pointing `CLAUDE_CONFIG_DIR` at an empty directory makes `claude mcp list` report no configuration and creates a separate `.claude.json` there, confirming configuration/credential isolation. `--settings <file-or-json>` exists; Codex supports `$CODEX_HOME` and `-p/--profile <name>` overlaying `$CODEX_HOME/<name>.config.toml`. Official docs also move settings/skills/agents/plugins/credentials/history with `CLAUDE_CONFIG_DIR` and use this alias pattern for multiple accounts.
- Trade-off: isolated directories allow concurrency, preserve global files, avoid switching races, and revert by directory removal; each instance needs its own login and session/Skills/MCP configuration storage.
- Instance definitions are agent definitions with a `profile` section, reusing Skills matrix and per-agent MCP allocation without another assignment model.
- New profiles domain: zero-write status shows only variable names/resolvability; apply writes only the fully owned instance file (Claude `settings.json`; Codex `agent-manager.config.toml` plus launch `-p` overlay, avoiding whole-file conflicts with MCP-owned `config.toml`). Directory 0700, file 0600, atomic replacement, transaction-failure rollback. Existing third-party content/external edits fail closed unless explicit `--allow-takeover`. Authentication persists only `${VAR}`; Claude resolves on write because its client does not expand references; Codex writes names only. Directories must be under HOME, excluding the main instance, data directory, and symlinks.
- Frontend Instances section: create/edit with launch preview and same-name shortcut saved together; cards write/take over configuration, synchronize launch commands, manage skills, and remove. Launch editor adds custom timezone/language and arbitrary environment-variable CRUD.
- Automation: engine-rs 303, app/src-tauri 15, Vitest 41 files / 379 tests, `pnpm typecheck`/`pnpm build`, both `cargo fmt --check` passed.
- Real isolated CLI: all 19 checks passed: definition-only saves; resolved instance values with global `~/.claude/settings.json` byte-identical; 0700/0600; **assigned skill links land in instance skills directories without affecting the main instance**; **MCP goes into the instance `.claude.json` without creating global `~/.claude.json`**; main/instance commands coexist in managed blocks; external edits remain untouched unless explicitly taken over. Another 12 launch checks cover custom/global/explicit timezones and removal.
- Final clean x86_64 DMG after `8893e91`, independently accepted: all `verify:macos` checks passed, exact identity; SHA256 `cdec9c38375ae1f4dcc2cbdc709a2f2f6d483ac6a9ea1eef1132b5bd451f914c`, 7984635 bytes; unsigned.
- Unverified: instance skill resolution relies on official docs and measured `.claude.json`/credential isolation; **real Claude loading skills from the instance directory has no E2E evidence**, because new directories need login before sessions. Only Claude Code/Codex instances are supported. cc-switch usage queries, speed tests, proxies, and cloud synchronization are outside this product's scope.

### 2026-09-17 Follow-up: Interaction Efficiency, Gaps, and Undo (`44c4bf7`…`a270499`)

- After reviewing cc-switch/industry practice, the user authorized all items: three interaction improvements, three earlier usability gaps, prominent Overview, and undo.
- Overview/effect notices (`44c4bf7`): move Overview from Settings→Advanced Management to first sidebar item. Successful actions state when they take effect (next agent launch, new terminal window, next instance-command launch; rules identify the target agent). Installation is immediately observable and needs no extra notice.
- Agent preferences/undo (`544e4f3`): Auto-install/Auto-update/Enabled become switches that save local preferences without installation/synchronization. Removing definitions (agents, instances, shortcuts, MCP, collections) offers timed undo with precise scope: definition-only removal fully restores; already-cleaned targets restore only definitions and require another apply.
- Command palette (`99e8216`): `⌘K` searches pages/agents/instances/collections/MCP/common actions; resources navigate to their page with agent-skills preselection where needed; actions use the existing execution channel. Arrow/Enter/Esc keyboard support; initial resource queries occur only when opened.
- Leftover-link cleanup (`b3a0182`): `skills:apply --action remove-link` names `--agent` and `--link`, without `--only` because no collection owns it. It removes only links, never source targets; real files/directories fail closed. Assigned links must use their collection's remove. Failed audit restores the original link. Takeover Center permits batch cleanup and distinguishes adoption from link deletion in confirmation.
- Structured Hooks (`bc9ecbb`): event/matcher/command rows save the native structure and merge same-event/matcher commands. Unknown `timeout` and hidden `removed_signatures` retain original values and are identified in UI. Writes compare versions; raw editing is expandable.
- Tray shortcuts (`ae0a1b8`): Search and Execute restores the window/palette; Takeover Center navigates there. Tray entries navigate rather than write, retaining the existing lock/audit path.
- Automation: engine-rs 305, app/src-tauri 15, Vitest 47 files / 408 tests, `pnpm typecheck`/`pnpm build`, both `cargo fmt --check` passed.
- Browser (dev + mock): `⌘K` groups/resources rendered 30 items; both structured Hooks editors/empty states rendered correctly without errors.
- Final clean x86_64 DMG after `a270499`, independently accepted: `verify:macos` passed exact identity; SHA256 `9be8e340f3052b23fa013cc22a216a94c11b0a3f446f3a4fd7ba29f2a33c6775`, 8004560 bytes; unsigned. An earlier `-dirty` build omitted three test assertions/fmt results from commits; they were committed and rebuilt, and the dirty artifact was not used as the acceptance result.
- Unverified: real tray-menu clicks were avoided to prevent host interaction; compilation/event wiring checked, UI subscriptions unit-tested. Palette/Hooks checks used mocks, without package machine clicks.

### 2026-09-18 Follow-up: Gap Corrections and Real-machine GUI Acceptance (`52e6bd6`…`25388e9`)

The user authorized correcting every weakness listed in the previous self-review.

- Instance rules/Hooks (`52e6bd6`): previously called complete, instances still had rules/Hooks targeting main paths. Profiles now owns all managed instance files (Claude: merged settings.json + CLAUDE.md; Codex: agent-manager.config.toml + AGENTS.md + hooks.json), one domain per directory to avoid competing whole-file writes; bindings track each file's fingerprint.
- Launch presets (`05b9263`, added user request): command-specific common flags, Claude full automation, Codex full automation and Keep Sandbox. Values come from measured local `--help`; this Codex lacks `--full-auto`, so it is not offered. Dangerous choices state consequences.
- Builtin OpenCode definition (`81f7798`) extends Skills/MCP assignment. No install command without reliably verified official installation; observation channel only.
- Configuration backup (`5f50c12`): read-only `config:export`; whole-replacement, single-transaction `config:import` through the same save gate. Backups exclude assignments/write fingerprints; restored resources still need individual apply.
- MCP command availability (`25388e9`): status reports local stdio command resolution, with missing-command notices; remote is always Not Applicable, without network probes.
- **Real-machine GUI acceptance, filling the previously missing stage**: packaged app launched under isolated HOME/data; AX fixed position, screenshots captured only the window rectangle. Keyboard checks covered startup/rendering, `⌘K` open/search/no-match empty state, Enter navigation to Takeover Center, `⌘,` Settings, and palette navigation to MCP. HOME/data had 0 writes; no panic/error logs. Previous package-click acceptance was the obsolete 2026-09-15 interaction; subsequent features had only isolated CLI/mock tests.
- Automation: engine-rs 310, app/src-tauri 15, Vitest 49 files / 422 tests, typecheck/build, both `cargo fmt --check` passed.

Unfinished or unavailable:

- **Linux/Windows cannot be verified locally**: cross-compilation lacks `aarch64-linux-gnu-gcc` and `x86_64-w64-mingw32-gcc`; CI/machines required, but GUI CI still fails before startup on GitHub billing. macOS arm64 `cargo check` passed, proving compilation only.
- **Skill version rollback declined**: content-addressed hash snapshots lack per-skill history and would require a new table; direct-link local collections have no version concept; git rollback belongs naturally at the source. Cost/benefit does not justify it; revisit if needed.
- **Observation awaiting user decision**: builtin `claude` shortcuts preselect `--dangerously-skip-permissions`, making fresh environments fully automatic. New presets allow explicit choice, but changing defaults affects existing users; left unchanged pending a decision.
- Instance skills loading still lacks E2E evidence; the instance needs login before a session.

### 2026-09-17 Follow-up: Launch Parameters, Backup Differences, and OpenCode Rules (`3478f54`…this commit)

The user asked for selectable common parameters, visible evidence of effect, and all remaining self-review items.

- Explicit high-privilege opt-in (`3478f54`): builtin defaults remove `--dangerously-skip-permissions`. `aliases:status` reads expected/actual shell values; UI shows Effective / Still old / Missing from block / Never written. It reports files, not whether a terminal session reloaded.
- Global timezone location/substitution (`ce4a28f`): followers show where to change defaults; wrapper raw editors explain `{timezone}` / `{lang}` substitutions.
- Import differences (`a12cf0c`): whole replacement is irreversible; UI first reads current configuration and groups additions/changes/removals by category. Diff reads perform no writes.
- OpenCode becomes the third Rules target (this commit): fixed Codex/Claude/OpenCode `rules:status/apply`, target `~/.config/opencode/AGENTS.md` per [official docs](https://opencode.ai/docs/rules/). New `rules.opencode` defaults empty because no verified OpenCode-specific content exists in this repository; shared rules still apply and users supply dedicated content. No OpenCode Hooks target: extensions are JS/TS plugins rather than JSON configuration.
- Batch progress in Takeover Center (this commit): item index/total, current item, progress bar, and explanation that failures are skipped while later items continue.
- First-use guidance (this commit): an empty local layer means nothing saved here; landing page presents scan/adopt, assign, then Synchronize Now to write, with direct Takeover Center navigation. Don't Show Again dismisses it permanently.
- Automation: engine-rs 313, app/src-tauri 15, Vitest 51 files / 439 tests, `pnpm typecheck`/`pnpm build`, both `cargo fmt --check` passed.
- Isolated CLI evidence for OpenCode rules: under isolated HOME/data, `rules:status` listed three Missing targets with zero writes; apply created `~/.config/opencode/AGENTS.md` from the same shared source; second apply did nothing. Appending a user line changed status to Externally Modified; preflight refused apply and preserved it.
- Packaged-machine acceptance (isolated HOME/empty data, AX-fixed window-only screenshots): real engine data showed first-use guidance; `⌘K` navigation to empty Takeover Center worked. HOME/data had 0 writes; no panic/error logs. Screenshots confirmed builtin claude commands no longer skip permission confirmation and readback said Not Yet Written to Terminal Configuration.
- Final clean x86_64 DMG after `31440f9`: all `verify:macos` checks passed, exact commit/date; SHA256 `f8a2ac041c272c877ef27d106e4ca1d84de948a99473020c7646175ca95982c5`, 8034356 bytes; unsigned.
- Browser (dev + fresh-machine `?mock=fresh`): three-step guidance, Takeover Center navigation, persistent Don't Show Again, and 7 batch progress messages (1/7 → 7/7) worked. Progress disappeared/buttons reset afterward; no horizontal scrolling at 900px or console errors/warnings. Adoption made the local layer nonempty and automatically dismissed guidance without manual closure.

Unfinished or unavailable:

- **OpenCode independent instances are unavailable and excluded**. Claude/Codex each provide an official directory-isolation variable (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`). Per [official configuration docs](https://opencode.ai/docs/config/), `OPENCODE_CONFIG` changes only a configuration **file**; `OPENCODE_CONFIG_DIR` adds an agents/commands/modes/plugins search directory. Neither moves global `~/.config/opencode/AGENTS.md` or isolates credentials/sessions in `${XDG_DATA_HOME:-~/.local/share}/opencode/`. Isolation would require both generic `XDG_CONFIG_HOME` and `XDG_DATA_HOME`; OpenCode is not installed locally and cannot be measured. Revisit when official isolation variables exist.
- **Six concurrent agent-page queries are not a real problem; unchanged**. Warm local `agents:status` / `policy:get` / `aliases:status` / `profiles:status` / `system:audit` / `sources:get` each take about 10 ms; all six concurrently also take about 10 ms. First-run 1.05 seconds is binary cold startup, not concurrency. Desktop calls are in-process and avoid even that. Earlier timeouts were jsdom waits, not slow engine execution.
- Linux/Windows machines, signing/notarization, and instance-skills E2E remain unverified as recorded previously.

### 2026-09-18 Follow-up: Usability Trials and Six Corrections (`64cd48d`…`0aa5669`)

The user requested functional usability testing/evaluation. Real journeys used isolated environments with existing resources rather than code inference.

Observed and corrected:

- **Editing outside shortcut blocks permanently blocked synchronization** (`64cd48d`). Whole-file hashes meant adding `export EDITOR=vim` to `~/.zshrc` blocked all later synchronization, contradicting managed-block-only writes and byte-preserving outside content. Compare only the managed block; add `applied_block_digest` to bindings, with empty-string legacy fallback to whole-file semantics. Schema additions occur only on writes; status remains zero-write.
- **Instance launch commands were broken** (`a0e33a6`). `CLAUDE_CONFIG_DIR="~/.claude-x"` does not expand quoted `~`; the assistant creates a literal `~` directory beneath cwd while this tool writes an absolute path. Generate `"$HOME/..."`; recognize old broken commands on readback and offer correction.
- **One instance command, no alias rename/copy** (`a0e33a6`). Instance cards now reverse-match `CLAUDE_CONFIG_DIR` / `CODEX_HOME` and list all commands for the directory, without new fields or a second alias writer. Each supports edit/copy/delete; true renames leave one entry. Instances can be copied with a different directory by default.
- **Externally edited rules/shortcuts had no recovery** (`2e5c105`). Externally Modified status had no actions, permanent synchronization failures, and no engine/UI takeover support, while Hooks already supported it. Add three corresponding points per domain, allowing only readable current-content conflicts; errors name the target and next action.
- **Secrets recognized only process environment, usually unavailable to GUI** (initially `4d60857`, later security boundaries tightened). Secrets now selects an external private `.env` before variable entry; resolution prefers that file, then process environment; plaintext no longer enters the local database.
- **Adopted skill directories included only previously linked skills** (`0aa5669`). Include all source-root entries containing SKILL.md and label existing usage.
- **Unsupported matrix labels implied an agent could not use skills** (`0aa5669`). Rename to Unassigned and state the next step directly.

- Automation: engine-rs 317, app/src-tauri 15, Vitest 51 files / 451 tests, typecheck/build, both `cargo fmt --check` passed.
- Final clean x86_64 DMG after `7e9cf47`: all `verify:macos` checks passed, exact commit/date; SHA256 `8761d7f2bc6ae0d2a11792265a87eac57c38054bf0ae14c07273438a23616adb`, 8048422 bytes; unsigned.
- Packaged-machine acceptance: isolated HOME/empty data, successful rendering, 0 HOME/data writes, no panic/fatal logs.
- Browser (dev + mock): ⌘K search for Secrets navigated directly; variable states showed Entered / Configured from Environment / Unconfigured with entry actions. Every shortcut had Copy, prefilled `claude-copy`; editing allowed renames. No console errors.
- Real isolated CLI per item: outside-block edits retained managed-ok and synchronized without losing user lines; inside edits refused overwrite but allowed takeover. Rules takeover succeeded for one exact target and rejected multiple targets. Secret status resolved/stored matched entered values, apply used them, operation logs/backups omitted plaintext, and clearing revoked them. Discovery returned all 3 source skills when only 1 was linked.

Unfinished or unavailable:

- **MCP takeover must preserve usable existing configuration**. Discovery still contains no plaintext. On confirmed takeover the engine reads only the target service's literal values and transfers them to the external file selected in Settings → Secrets. Receipts show variable names only. Same-name/different-value conflicts or unavailable files stop the entire item before assistant overwrite.
- **Backups exclude secret values** (default implemented this round without another user instruction). A new machine needs re-entry; the backup itself is not a secret container.
- **Six concurrent agent-page queries are not a real issue; unchanged**. Each warm read is about 10 ms, all six together about 10 ms; first 1.05 seconds is cold binary startup.
- Linux/Windows machines, signing/notarization, and instance-skills E2E remain unverified as previously recorded.

### 2026-09-18 Follow-up: Merged-target Detection and Value-preserving Takeover (`f1b7123`…)

The user requested remaining items after evaluation: two more members of the same bug family.

- **Claude Hooks settings.json** (`f1b7123`): signature-set merging explicitly permits user fields, but whole-file hashes marked any model/permissions/statusLine edit as an external change requiring takeover. MCP already compared entries. Compare only managed-hook slices; fully owned Codex hooks.json still uses the whole file.
- **Instance settings.json** (`e406c0d`): merged writes own only env/model/hooks but compared the whole file. Compare those three keys. The old error claiming this was not the tool's file was also wrong: it was the tool's file with later user additions.
- Both retain compatibility: old whole-file bindings remain valid when the file is unchanged, then upgrade to slice digests on the next write. No schema change or one-time false conflict for existing users.
- **MCP takeover now preserves actual values** (`e69d443`): `secrets:adopt --agent --server` reads only that assistant's configuration and plaintext literals, skipping `${VAR}` and `_MANAGED_BY`. It derives variable names by UI rules and stores them in the local database. UI order is per-agent adopt → definition save → writeback, with order assertions; late adoption would synchronize another value. Previously confirmation merely asked users to refill secrets.
- Command palette adds four Settings subpages: Secrets, Terminal Shortcuts, Update Center, Policy.
- `agent-rule/CLAUDE.md` still described `tests/*.py` as active gates although Python retirement removed `tests/`. It now says the discipline remains but no automatic gate enforces it, and tools must be restored from git history.

**Instance-skills E2E evidence is now available**, previously always unverified. Login is unnecessary: Claude discovers skills before authentication. An instance was created under isolated HOME, assigned a skill, and launched with `claude -p` using its directory and debug logging:

```
Loading skills from: ... user=~/.claude-work/skills, project=[]
Loaded 1 unique skills (... user: 1 ...)
Watching for changes in skill/command directories: ~/.claude-work/skills
```

The instance loaded its assigned skill; main `~/.claude/skills` did not exist and was unaffected. This proves `profileSkillsDir(configDir) = <configDir>/skills`.

- Final clean x86_64 DMG after `c7f58c3`: all `verify:macos` checks passed; SHA256 `31ef45d8ba79d7a4030a289058bd549ad2dd1829e48485a046ffb05fc915e2d5`, 8080588 bytes; unsigned.
- Previous DMG after `04555d3`: all `verify:macos` checks passed; SHA256 `520ab39ffbe94df44ae039d5119cd0e978ace04adb4e97d259b633b33565b084`, 8077673 bytes; unsigned.

- **Backups now include credential values** (`c7f58c3`, user decision). Export includes `secrets`; import restores it wholesale. Machine A export → fresh B import → immediate resolvability → direct synchronization writes A's values. The backup is therefore equivalent to a secret file. UI no longer displays JSON, only "N definitions · M sources · K credential values" and storage guidance; clipboard copying still returns the full backup. Import differences add Credential Values, showing names/additions/changes/removals only. Status/history/diffs exclude values. Also add the previously omitted `rules.opencode` source to backups.

Other measurements confirmed Hooks/instances synchronize and preserve user-owned edits; managed-slice edits reject overwrite, and takeover retains user content. Takeover writeback retained original values; status/backups/receipts had no plaintext.

### 2026-09-18 Follow-up: Real-machine Feedback Fixes and UI Redesign (`575bbc6`…`b0f8519`)

The user tried every page on a real machine and reported functional failures, parameter overwrites, missing global synchronization, and four pages effectively lacking design. Each was investigated before correction.

**Three functional failures found through real clicks** (`cc5880e`), with causes from operation/diagnostic logs:

- MCP takeover reported 4 failures: 3 residual managed entries rejected add and required remove, even though this tool had written them and Takeover Center recognized/badged them but exposed no remove path. Explicit `--allow-takeover` now replaces remnants (not external user content); without it rejection remains with a clear next step. The fourth failure was a genuinely unconfigured credential variable requiring entry.
- Update Center sent library-level MCP `--action update`, but MCP updates are per-agent. Expand to assigned agents in `observed.agents`; when none are assigned, explain Assign First rather than send a guaranteed failure.
- "4 failures; see task history" was unhelpful. Show each object's failure cause directly.

**UI**: move toasts from top-right, where they obscured titles/main buttons just clicked, to bottom-right; launch commands scroll horizontally instead of breaking inside `LC_ALL`; merge Update Center's two time columns.

**Launch presets expand to five agents** (`0785797`). All flags come from measured local `--help`; copying another client's spelling would produce nonexistent flags: Claude/Antigravity `--dangerously-skip-permissions`; Codex `--dangerously-bypass-approvals-and-sandbox` and `--ask-for-approval never --sandbox workspace-write`; Grok `--permission-mode bypassPermissions|acceptEdits` (values, not switches); Kimi `--auto` and `-y`. Each has full-automation and compromise presets with explicit dangerous consequences.

**Global Synchronization Center** (`0785797`): Overview collects changes across six domains. Executable actions from the engine support batch execution with skipped failures and aggregate results; externally modified/legacy items are excluded, with reasons and navigation. Decisions use engine state/actions only; intentionally disabled resources do not prompt. Pure `syncPlan` has six cases.

**Three pages redesigned consistently** (`b0f8519`). Review confirmed problems centered on these three; Agents, Takeover Center, Rules/Hooks, history, and service library were healthy. The common cause was rendering raw engine structures as tables:

| Page | Before | After |
| --- | --- | --- |
| Overview | 2.2 screens / 11 blocks; Agents/MCP/Skills each duplicated | 1 screen / 8 blocks |
| All Skills | 25 rows (5 skills × 5 agents) | 5 rows |
| Agent Services | 35 rows (7 services × 5 agents) | 7 rows |

Three principles, already satisfied elsewhere: (1) resources × agents use a matrix rather than Cartesian-product rows. Shared `ResourceAgentMatrix` gives one resource per row, one agent per cell, expandable per-agent actions, and multiple expanded rows for comparison without lost capabilities. (2) Unknown-value columns take no space: remove all-Unavailable version columns and Version columns containing Linked/Unconfigured, which duplicated status confusingly; details move to hover. (3) Hide engine jargon: replace The Current Engine Provides No Actions with a blocking reason, or Nothing to Do Here when none exists. Overview answers whether users need action now; duplicate Update Center versions become one conclusion/link, and health details default collapsed.

UI Leftovers becomes Previously Configured, which is meaningful to users.

- Automation: engine-rs 326, app/src-tauri 15, Vitest 52 files / 472 tests, typecheck/build, both `cargo fmt --check` passed.
- Final clean x86_64 DMG after `07583cc`: all `verify:macos` checks passed; SHA256 `311805bebb50908f1e6b71164f4cdf63680fa9331c559de080ca7c5c72c3e259`, 8085021 bytes; unsigned.
- Measurements: isolated leftover takeover reached `leftover → managed-ok, assigned=true`; browser row/screen counts, empty-information columns, and jargon occurrences were measured individually, as above.

### 2026-09-20 Follow-up: Connection Workspace, First Increment

> Historical milestone: Installation and Maintenance / Manage Agents were removed in the fifth increment later that day.

- Cause: connections were hidden in full-definition dialogs/card menus; add/switch/test were scattered while installation maintenance dominated the page. The user requested cc-switch's low interaction cost and a more complete, trustworthy workflow.
- Change: a task-oriented top workspace selects an agent, then displays official/default and connection cards with current state, endpoint, model, credential variable, test result, and one-click activation. Installation and Maintenance remains separate below; full-definition editing becomes Manage Agents. Add Connection first selects a Claude/Codex-compatible vendor preset or Custom, then confirms required fields; advanced fields expand on demand.
- Boundaries: adding replaces only the target profile, preserving unknown fields, other connections, and active. Claude optionally saves credentials separately through `secrets:set`; Codex registers only variable names. Immediate activation invokes `agents:configure --use` only after successful definition/credential saving; completion requires status readback.
- Automation: Vitest 55 files / 501 tests, Rust `agent_settings` 24, `native_secrets` 5, Tauri 14 passed; `pnpm build` passed. New coverage includes full-field preservation, current/noncurrent cards, default return, execution exclusion, read-only tests, presets, credential write order/retry, zero-write cancellation, validation, agent switching, unsupported-agent read-only boundaries, and focus restoration.
- Browser (dev + `?mock=1`): Claude/Kimi and Codex/Kimi presets; Claude Anthropic endpoint/optional credential input, Codex Responses endpoint/no plaintext input; Esc restores Add Connection focus. No overflow at 900×800/default viewport, errors, or warnings.
- Boundary: frontend orchestration/tests/docs only, reusing accepted Rust `agents:configure`, `agents:check`, `secrets:set`; package not rebuilt; browser mock success does not prove package writes.

### 2026-09-20 Follow-up: Palette Connection Switching, Second Increment

- Cause: the workspace still required navigation. cc-switch makes frequent vendor switching immediately available; existing `⌘K`/tray search can do this without a second tray write path bypassing confirmation/audit.
- Change: palette indexes Claude Code/Codex defaults and connections, searchable by display/internal name, endpoint, model, and vendor/connection/switch terms. Current entries are marked/disabled; selecting another switches directly, with `--use-default` for defaults. Writes use unified history; success requires `agents:status` active matching the target. Only external-change rejection prompts takeover and `--allow-takeover` retry; other failures leave displayed facts intact.
- Account-free acceptance: DEV mock adds Codex/Kimi and in-memory `agents:configure`/`agents:check`, loaded only in development and excluded from production. Real vendor connectivity was skipped without a suitable account; simulation is not a substitute.
- Automation: Vitest 55 files / 506 tests, Rust `agent_settings` 24, Tauri 14, `pnpm build` passed. Added profile/default switching, operation IDs, readback, current-entry disablement, external-change confirmation, ordinary failure, plus mock check/switch/readback coverage.
- Browser `?mock=1`: keyboard palette search Kimi/Enter switched, with restart-effect notice; searching again showed disabled current Kimi. No overflow at 900×800 or console errors/warnings.
- Boundary: no Rust commands/schema/tray changes; package not rebuilt; DEV mock cannot prove real assistant-file writes.

### 2026-09-20 Follow-up: Inline Connection Editing/Removal, Third Increment

- Cause: endpoints/models/deleting old connections still required full definitions despite workspace add/switch/test.
- Change: nondefault cards gain Edit/Delete. Editors prefill existing fields; current connections default to saving and updating actual configuration, noncurrent ones to save-only without switching. Internal unique names are fixed during editing. Only noncurrent connections can be deleted; current Delete is disabled with switch-first guidance. Confirmation removes only the profile, retaining reusable credential variables.
- Data: strip form-owned fields before merging new values so optional fields can clear, preserving unknown profile fields. Removal preserves other profiles, active, runtime/unknown definition fields. Helpers defensively clean dangling active references, although UI prevents current deletion.
- Automation: Vitest 55 files / 512 tests, `pnpm build` passed, covering unknown fields, optional clearing, current/noncurrent editing, delete confirmation, retained credentials, current deletion prohibition, and no unintended writes. Unchanged Rust/Tauri reuse this round's passed `agent_settings` 24/Tauri 14.
- Browser `?mock=1` Codex/Kimi: prefill/advanced fields/Esc focus/delete confirmation/current-delete disablement passed; no 900×800 overflow/errors/warnings.
- Boundary: no Rust/schema changes or package rebuild; real vendor checks skipped without suitable accounts.

### 2026-09-20 Follow-up: Safe Connection Copy, Fourth Increment

- Cause: similar connections required preset reselection/manual fields. cc-switch's routine copy action is useful, while credentials/current activation must not be implicitly copied.
- Change: Copy on every connection card prefills endpoint/model/authentication/credential variable/advanced parameters. Unique defaults use `<original-name>-copy` / `<original-name>-copy-2`, safely truncating 64-character names. New names are editable; save-only by default, activation only by explicit selection.
- Data/security: unknown extension fields copy, credential values are never read and their input stays empty; source profile, active, and other definitions remain unchanged.
- Verification: 48 targeted helper/page tests passed for collisions, 64-character limits, unknown-field copying, unchanged sources, empty credentials, no default switch, and zero `secrets:set`; full Vitest 55 files / 515 tests and `pnpm build` passed. DEV browser checked prefill/editable unique names/collision blocking/Esc focus; no overflow at 900×800, warnings, or errors.
- Boundary: no Rust/schema changes or package rebuild; real vendor checks skipped without suitable accounts.

### 2026-09-20 Follow-up: Runtime-first UI and Client Diagnostics, Fifth Increment

- Cause: real-window screenshots still showed installation cards, full definitions, and commands dominating runtime/account configuration. cc-switch comparison narrowed responsibilities to Provider/connections, variables, runtime settings, and instances; CLI software remains managed by official installers/Homebrew/npm.
- Change: remove install/upgrade/remove cards, full aliases/definitions, and agent add/delete/disable controls. Compact Runtime Settings supports Claude statusline/permission mode and Codex permission mode. Read-only Client Diagnostics in Settings shows installation/version/source without lifecycle actions. Takeover Center/Overview remove tool-managed upgrade implications and AI CLI version updates.
- Data boundary: `native-manager.db` remains the sole source of truth at this historical stage. Runtime saving owns only `settings.yolo/statusline`, preserving profiles, active, unknown/lifecycle fields. Phase one retains Rust schema/compatibility; later removal first audits callers, CI, packaging, docs, and migration boundaries.
- Verification: Vitest 56 files / 495 tests, `pnpm build`, engine-rs 373, Tauri 14 passed. Real 1792×1044 Tauri window showed all main agent content in one screen, runtime dialog without scrolling, diagnostics read-only without install/update buttons.
- Boundary: real vendor connectivity skipped as authorized without suitable accounts; package not rebuilt. Native development-window layout/interaction is not package acceptance.

### 2026-09-20 Follow-up: CC Switch Capability Transfer and Authentication Safety

- Scope: current product supports only Claude Code/Codex. Remove OpenCode from builtin discovery, Skills/MCP allocation, and current Rules targets. Preserve old database definitions/internal compatibility codecs to avoid upgrade data loss.
- Provider: keep cards/presets/add/switch/test/edit/delete/copy/palette switching; add ordering and key-free shared JSON import/export. Ordering/import/save change definitions only; imports omit `apiKey` and automatic activation, excluding CC Switch deep links that can carry keys and switch immediately.
- Authentication: redacted read-only fixed-file status rejects symlinks/abnormal content, reports method/availability only. Normal configuration never writes Claude `.credentials.json` or Codex `auth.json`. Login/logout copy official commands, never execute in the background. Instance commands include isolated `CLAUDE_CONFIG_DIR` / `CODEX_HOME` for user review and explicit terminal execution.
- Instances: independent directories isolate login/sessions/Skills/MCP/commands; Skills/MCP read actual shapes from `profiles:status`, retaining Claude/Codex instances without admitting other clients.
- Skills: preserve content-addressed snapshots, controlled symlinks, per-instance allocation, fail-closed external edits, reversible disablement; no second ZIP/history-backup model. MCP retains placeholders/exact scope/unmanaged takeover protection/CAS/conditional rollback/audit, adding only clearer filtering/batching.
- Verification boundary: automation/browser mocks prove command generation/status/no-unintended-write orchestration only. No real `claude auth login/logout` or `codex login/logout`; simulations are not authentication acceptance.
- Results: Vitest 57 files / 521 tests, typecheck/production build, engine-rs 384, Tauri 14 passed. Browser mocks showed redacted Claude/Codex authentication, copy-only controls, Codex `auth.json` preservation notice, connection sorting/import/export, and instance Skills/MCP matrix rows.
- Actual configuration readback correction: workspace previously read only definitions and labeled empty `settings` Current without parsing existing files. `agents:status` now reads redacted observations from fixed targets; UI separates actual configuration/managed selection and runtime editing/write preview/noneditable Actual Local Values. Zero-write reads reject symlinks/abnormal files; Claude plaintext tokens yield only Configured, never status JSON/logs/database.
- Consistency/inline repair: compare safely observable managed provider fields as Consistent / Inconsistent / Incomparable. Drift repair explicitly reapplies managed selection using existing external-change confirmation/audit/readback, never automatic takeover. Vitest 57 files / 523 tests, engine-rs 384, Tauri 14, typecheck/build passed; browser mock repaired drift to consistent, with no 320 px overflow/errors. No real HOME configuration writes, provider network/authentication, or package acceptance.
- Batch connection checks: explicit concurrent checks for all custom connections retain per-card success/failure/latency, independent failures, and aggregate available-count/fastest-line results. Existing read-only `agents:check` only; no switching/configuration writes. Vitest 57 files / 524 tests, typecheck/build passed; browser mock exercised Check All through fastest-line summary. Without real credentials, mock results are not network acceptance; backend/Tauri unchanged this batch.
- Narrow-window correction: real 320 px screenshots showed the fixed sidebar compressing text vertically despite no page overflow. Narrow layouts now use horizontal top navigation, retaining the desktop sidebar. Recheck measured both viewport/scroll width at 320 px with normal text/no console errors. Navigation/shortcut targeted tests 33/33, full frontend 57 files / 524 tests, typecheck/build passed.
- Fastest connection/readback gate: after batch checks, users can enable the fastest successful noncurrent route using existing `agents:configure`, external-change confirmation, and `agents:status`. Previously `refetch()` occurred without checking results; now both database `active` and redacted `observed.current_config` must match. Either mismatch preserves actual UI facts and reports failure instead of false switching/reapply success. Vitest 57 files / 525 tests, typecheck/build passed. Browser mock repaired Claude drift and switched from default via Kimi Code 128 ms recommendation with both sides consistent/no errors. No real Provider network/switch acceptance.
- Palette dual readback: `⌘K` shares full profile parsing/redacted file comparison with the agent page, rather than checking only database `active`; changed active without effective files is not success. Vitest 57 files / 526 tests; browser keyboard switched Codex/Kimi, then showed disabled current item and consistent managed/actual state without errors. Backend/Tauri/write protocol unchanged.
- Palette current-connection recovery: managed selection with inconsistent/unconfirmed files shows Configuration Mismatch · Reapply rather than disabled Current Connection. Explicit execution restores disabled-current only after dual readback. Vitest 57 files / 527 tests; DEV browser changed Claude default from actionable mismatch to consistent/disabled without errors. No backend/Tauri/protocol changes or real configuration/package verification.
- Direct tray connections: dynamic Claude/Codex default/custom groups mark trustworthy current entries checked/disabled and inconsistent entries Reapply. Noncurrent clicks only restore the window and precisely filter the palette; Enter/click then invokes existing configure, external-change confirmation, audit, dual readback. This preserves `ae0a1b8`'s no-silent-tray-write decision. Vitest 57 files / 529 tests, Tauri 15; tests prove tray events alone write nothing, explicit confirmation switches, external changes are not silently overwritten. With isolated data and frontend DEV mock, real macOS tray clicks on the same-source Tauri app selected Codex/Kimi Code, restored/filtered the palette without switching, then Enter produced success. Reopened native menu showed Kimi Code checked/disabled. No real `~/.claude`/`~/.codex` reads/writes or running-client operations; package, actual files, and Provider network unverified.
- Connection notes/search: at most 200 characters per connection, displayed on cards and searched by `⌘K`. Database-only notes never render to Claude/Codex, but copy/key-free sharing retains them. Vitest 57 files / 530 tests, typecheck/build passed. DEV browser showed notes and uniquely filtered Codex/Kimi Code by Expires at Month-end without switching. No native app/real-client reads/writes/operations.
- Keyboard switching: `⌘⇧K` opens only Claude/Codex defaults/custom connections, filtering names/notes/endpoints/models. Esc then `⌘K` restores all commands; text input retains shortcuts. Execution still uses confirmation/locks/audit/dual readback. Vitest 57 files / 531 tests, typecheck/build passed; DEV browser saw 3 connections, unique note filtering, restored full scope. No switching/native app/real-client operations.
- Linked usage filters: Agent → Model selectors combine with time range for summaries/unpriced notices/details. Changing time clears both; changing agent clears model. Filters use already-read `usage:status` only, without scanning/writes. Vitest 57 files / 533 tests, typecheck/build passed; DEV browser confirmed summaries/details change together. No native app/recount/real-client reads/writes/operations.
- Provider/runtime consolidation: create/edit/apply remain independent. Active custom Providers require switching/reset before deletion with second confirmation; official defaults only offer confirmed reset. Claude/Codex automation has Manual/Smart Automatic/Fully Unrestricted adapter-backed native fields. Main Agent directly reviews/adds/edits/removes/explicitly synchronizes its own aliases; isolated instances remain directory-owned. Definitions/shell writes are separate, with `aliases.shell` authoritative. Vitest 57 files / 539 tests, typecheck/build, engine-rs 386, Tauri 15 passed. Isolated HOME completed Provider create/apply, `automation_mode=auto`, official reset/deletion, two aliases synchronized/read back managed-ok. Real native window read-only checked aliases/reset and opened/canceled both confirmations. No real-client operations/package build/acceptance.
- Environment information architecture: Agent → Default/Independent Environment → Environment Content. Default has Provider, Automation and Permissions, Launch Commands, Actual Configuration tabs. Provider addition is a text action; independent environments move from dialogs to the same Agent-filtered workspace, with separate creation. Permission flags show Override Environment Automation and effects. Actual Configuration shows files/redacted readback. SQLite/Provider/profile/alias protocols unchanged; historically `native-manager.db` remains authoritative here. Vitest 57 files / 542 tests, build, engine-rs, Tauri 15 passed. Native read-only checks covered four default tabs/independent empty state without apply/sync/confirmation or real-client changes.
- Provider takeover review: after external-change rejection, confirmation shows software/client values per field. Users may overwrite only managed fields or save actual endpoint/model/redacted credential metadata as a new Provider through a focused form. Internal names hidden; missing required fields block saving. No plaintext reads/saves; cancellation leaves client files untouched.
- Global action clarity: restore persistent Import Existing Configuration sidebar entry and matching palette/tray terms. Overview Synchronize All first lists planned writes and asks confirmation, excluding Needs Your Decision entries. MCP import explains the engine's literal-secret transfer boundary instead of simultaneously claiming plaintext is unavailable.
- Runtime field readback/structured review: Actual Configuration groups Provider/runtime fields rather than internal JSON. Runtime fields pair saved/actual values; nonautomation drift also appears in default summary/Automation and Permissions. Claude observes builtin/custom statusline and hidden-attribution switches. Success requires target ready plus matching Provider/all adapter runtime fields. Item editors remove duplicate low-level paths; Write Preview centralizes real keys. Narrow views use vertical field cards instead of squeezed three-column tables. Vitest 58 files / 546 tests, typecheck/build, Rust engine 390, Tauri 16 passed; desktop/320px browser mock had no errors/warnings. No native app/real-file reads/writes this batch.
- First-use consolidation: fresh installs open Providers directly; Import Existing Configuration is reachable through sidebar/palette/tray. Remove uncalled old three-step long-form guidance and retain regression preventing it from obstructing the daily workspace.

## Remaining Roadmap (Unscheduled)

All items have no schedule or assigned owner.

| Item | Prerequisite | Deliverable | Completion criterion |
| --- | --- | --- | --- |
| Current package and GUI E2E | All participating domains complete (met) | x86_64 DMG and main real GUI E2E at `5a110be3` accepted 2026-09-15 for old interaction; new source/browser/isolated CLI/uncommitted dirty candidate accepted | Final clean DMG rebuilt after `a01d0f5` and independently verified; receipt above. MCP add-service form awaits macOS automation input capability |
| Release | Resolve GitHub billing; supply updater/Apple/Windows signing outside repository and in Secrets (Actions secrets/variables/local codesigning identities currently all 0) | Signing/notarization, arm64/Linux/Windows/update release; draft-first single-writer `14e6790` ready, replacing `a2cc864` | Real green GUI CI at current product baseline; independent real-platform install/runtime and signed updater download/install/readback. Release/tag not authorized; refer to lead reviewer when reached |

The 2026-09-15 x86_64 `5a110be3` DMG passed verify/main real GUI E2E independently under old plan→confirm behavior; arm64 cross-build passed static acceptance. New direct-apply source/browser/isolated CLI/dirty candidate acceptance is complete; old package/E2E cannot prove new behavior. Final clean `a01d0f5` DMG was rebuilt and independently verified, as above. Public release still lacks GitHub billing, real keys, Developer ID/notarization, Windows signing, actual Linux/Windows/arm64 runtime/install acceptance, and signed updater download/install/readback.

## Advanced-management Decisions (2026-09-12)

- Tools: retired. Only playwright-cli (two npm commands) remained; implementing a new large domain was not justified. Users manage it manually; cleanup completed `9080c1d`.
- Aliases: retained as the first resumed small increment; implemented/independently accepted `b7f30db`.
- Update Center: `system:refresh-upstream` implemented for agents/mcp/skills, cached in SQLite `upstream_probes`, freshness read-only in status, offline rejected, no apply lock. `system:converge` retired `9080c1d`; Updates directly executes real apply with internal preflight/write-time validation and explicit `--action` (agents→upgrade, mcp/skills→update).
- Snapshots: retired in favor of fail-closed overwrite refusal and operation-local transaction rollback, without persistent prewrite backups. Automatic-backup promises removed `9080c1d`.
- Probe cache: new SQLite table, following operation_logs precedent for volatile state; no second writable state file.
- Implementers: Kimi YOLO was chosen then. Since Kimi lost implementation permission on 2026-09-28, current collaboration rules are updated in [AGENTS.md](../AGENTS.md).

## 2026-09-15: Public plan/dry-run Retirement

- All public `*:plan` (`mcp:plan`, `rules:plan`, `hooks:plan`, `aliases:plan`, etc.) and `--dry-run` entry points are retired/rejected: UNKNOWN_VERB for plan, USAGE_ERROR for dry-run.
- Status/query/diagnostics remain zero-write; six-domain writes use real apply only.
- Apply retains internal preflight/write-time validation, exact scope, exclusive locks, external-change detection, CAS/conditional rollback, and audit.
- Definition saves do not install/link/synchronize. Destructive removal/log clearing retain separate second confirmation.
- MCP secret boundaries remain: SQLite literal `${VAR}`, no resolved values in status/diagnostics/history; apply may write values through target codecs.

## 2026-09-22: Marker-free Assistant Configuration

- Trigger: Codex rejected remote MCP `_MANAGED_BY`. Inventory found all three codecs writing it (stdio env/environment passed it into MCP processes), Codex MCP/`[model_providers.*]` comments `# [agent-manager:managed]`, and statusline `.agent-manager-usage.json` inside Claude configuration.
- Decision: follow Marker-free Assistant Configuration in `docs/GUI-REDESIGN.md`. Ownership exists only in `native-manager.db`, using location + digest as Rules/Hooks/Claude runtime already do. This supersedes entry-level `_MANAGED_BY` requirements in earlier HTTP MCP wiring.
- MCP ownership is assigned/enabled `mcp_bindings`; old markers stripped read-only. Same-name entries without records, including old marked remnants, are user-owned and need explicit takeover.
- Codex runtime `agent_settings_bindings.owned_providers` records written provider IDs; old comments serve one-time migration identification only.
- Statusline quota cache moves to data-directory `usage/`, no longer reading/writing assistant-directory caches.
- Shell alias block comments remain: rc files are not assistant configuration and lack structural fields. Codex instance `agent-manager.config.toml` remains an official Codex profile-file format.

## 2026-09-22: Actual Configuration as the Source of Truth

- Trigger: users required all configuration to read real environments except confirmed builtin recommendations. Inventory found the opposite: builtin policy frozen on first write, database copies in rules/Hooks/aliases/MCP/runtime forms, takeover-only database overwrite recovery, and builtin aliases/repositories/Hooks counted pending.
- Reference: CC Switch `farion1231/cc-switch@8e478b2` imports actual configuration as current on first startup and appends inactive official presets (`src-tauri/src/lib.rs:728-782`), fills actual configuration back before switching (`services/provider/mod.rs:5836-5884`), and refreshes active prompts from files (`services/prompt.rs:62-85`). Do not copy whole settings.json replacement, database-authoritative imported MCP, or write-on-read behavior.
- Decision: use the three-layer Actual Configuration as the Source of Truth model in `GUI-REDESIGN.md`; replace shared + adapter rules with one actual file per assistant plus Copy to Another Assistant. Codex MCP disable uses official `mcp_servers.<id>.enabled = false`.
- Stages/receipts, all pushed to `rust`: contract `1ce4a15` → recommended skill repositories `1e60a7b` → actual-value runtime/connection prefill, preswitch readback, external-secret-only tokens `1acc84a` → actual rules/templates/readback without composition `e89abc5` → actual MCP/official disable `e318d8d` → actual Hooks/cleanup recommendations `94037ef` → actual alias blocks/recommendations `b2eb229` → actual instance files with overwrite/adopt choice `4268bad`.
- Acceptance: both cargo suites, frontend units/typecheck/build passed each stage; critical fixes were tested in reverse. Built CLI readbacks used isolated HOME, including old databases created by old binaries. Connections/MCP/instances also had DEV mock browser checks; real `codex mcp list` read back disablement. Package-machine receipt is dated 2026-09-23.

## 2026-09-23: Runtime Settings and Raw Configuration

- Trigger: real-machine users requested more runtime parameters, then asked to replace all-values Actual Configuration/full-file presentation with meaningful adjustable fields like CC Switch, while displaying plaintext in the editor.
- Reference: CC Switch `farion1231/cc-switch@8e478b2` main UI has vendor cards (name/endpoint/quota); editors prioritize API Key/endpoint, collapse Advanced, and put three-line JSON last with 6 direct switches (`src/components/providers/forms/CommonConfigEditor.tsx`). Current-provider JSON is full live settings.json (`EditProviderDialog.tsx` reads `read_live_provider_settings`) with plaintext tokens. Do not copy whole-file overwrite/common-fragment merging.
- Decision: Claude reasoning/Artifact use official persistent `effortLevel`/`enableArtifact`, not session overrides; users remove old variables through Clear Override Variables. Remove Actual Configuration tab, retaining Provider/Runtime Settings/Launch Commands. Embed runtime forms; other fields use collapsed Edit Configuration File at the bottom, showing/writing exact originals with whole-file digest protection. Database/audit excludes full content/values. Credential-name detection uses whole categories like CC Switch.
- Commits: official persistent keys `1b5ed7a` → raw editor command/UI `05cf21d` → credential-category detection `d643243` → plaintext original `503839c` → tab consolidation/embedded runtime `dba3f27` → lock wait for brief subprocess inheritance `f7060ba` → Disable Auto-update `82e8170` → CodeMirror (same commit as this entry).
- Execution lock: other threads' spawned processes inherit lock descriptors before exec, briefly holding a released lock and falsely rejecting immediate writes. High-load `tests/agent_settings.rs` failed 7/40, single-thread 0; retry acquisition within 500ms yielded 0/40 under the same load.
- Acceptance: both cargo suites, frontend units/typecheck/build passed every commit. Isolated CLI readback additionally covered raw editing/Disable Auto-update; DEV browser showed syntax highlighting/line numbers/inline errors. Consolidated package launched in the real environment for user trials; itemized machine acceptance remains pending.

## 2026-09-23: Moving Rule Content Out

- Trigger: bundled rules required a package rebuild for any edit. Inventory showed recommended `core.md` was the author's personal global rules, edited 32 times, rather than product defaults. Connection presets, Hook cleanup signatures, and statusline scripts are code-coupled tool knowledge.
- Decision: move rules to agent-skills `rules/` with subtree history, size/anchor gates, and behavior-evaluation runner. Read through skill sources, reusing collection definitions/mirrors/Update Resource Library. No binary fallback rules, only frozen legacy-migration copies. Personal timezone/language aliases also move to agent-skills `aliases/`, read from sources by the alias page.
- Commits, all pushed: agent-skills `001ae92` (rule history), `948aaa0` (organization/gates), `50a8cde` (aliases); this repository `7c6eee7` (source rules) → `2b0f9ac` (remove `agent-rule/`) → `0bba6fe` (recommendation generation to Skills) → `329e2b1` (source aliases) → `107c30a` (remove `config/shell-aliases.json`) → `37c6933` (Tauri `--source-rule` allowlist).
- Acceptance defects fixed: from `7c6eee7`, bridge denied `rules:apply --source-rule`, so desktop Apply Source Rule only failed; engine/frontend tests bypassed the bridge, exposed by package clicks. `37c6933` fixed/added bridge coverage. `329e2b1` omitted Tauri tests before commit; direct-bridge alias tests still expected builtin recommendations and were corrected. Update Source used wrong `skills:` rather than `skill:` prefix; implementation/tests shared the error, caught by isolated CLI.
- Acceptance: engine 387, Tauri 20, frontend 558, typecheck/build passed; agent-skills 39 tests passed with a negative case for every gate. Isolated CLI with temporary agent-skills upstream completed recommendation → register → fetch → apply with backup → upstream new version → update availability → one-click update without another backup. Aliases completed recommendation → fetch → source-labelled recommendation → add → both rc blocks/`zsh -n`. Audit stores rule ID/digest/upstream commit or alias source, not content. `37c6933` package passed all `verify:macos`. Its DMG app, isolated HOME/data and substitute CLIs, was clicked through System Events: fetch agent-skills → source rules → Apply To writes byte-identical core.md → upstream commit → Update Source → New Source Version → Update writes new content without backup template. Shortcuts labelled From agent-skills; grok addition wrote both rc blocks, passed `zsh -n`, audited source. Runtime showed Disable Auto-update and CodeMirror raw editor with tokens/format/diff. Real Claude/Codex/rc digests remained unchanged throughout. Real agent-skills mirror was not fetched yet; user should click Fetch agent-skills on Rules.

## 2026-09-23: agent-skills Directory Organization

- Trigger: seven skills, rules/aliases/tests were mixed at root, tests included model evaluation runners, and 33KB README combined directory/measurement/reading/contribution material.
- Decision: skills move to `skills/`; model runners/builtin fixtures to `tools/evals/`; tests retain offline gates. README splits into `docs/authoring.md` (design/package/runtime/contribution) and `docs/evaluation.md` (runners/reading/host truncation/results/dead ends), leaving scope/directories/structure/validation/usage. This tool recommends `skill_paths: ["skills"]`; absent single-segment legacy root names resolve read-only as `skills/<name>` without database changes, preserving definitions/backups. Takeover scans discovered skill names to avoid treating existing same-name directory-source links as external new collections. Recommendation/confirmation replace Contains N Skills with Skills from skills/, since one skill_paths item can contain many.
- Commits, all pushed: agent-skills `fb35311` (runners/docs), `8566c8e` (skills directory); this repository `30a624e`.
- Acceptance: full engine cargo, Tauri 20, frontend 558/build passed; new fallback/name-matching cases fail when their fixes are reverted. agent-skills 39 passed, adding No Root Skill Packages and docs/ English gates, each with a negative case. All four runners locate 7 skills, 196 trigger cases, and rules evaluation files from arbitrary directories (load only, no models). Isolated HOME/local bare upstream: fetch old layout/enable verification → update reorganized upstream → old definition still discovers 7, unchanged link targets, working rules/aliases → `["skills"]` definition yields identical results.
- Real environment: back up database (`before-reorg`), CLI collection-update equivalent to Update Resource Library (`cbf1fd2` → `8566c8e`), then change definition to `["skills"]` retaining url/agents. `skills:status` rows identical; 24 links across `~/.claude/skills` and `~/.codex/skills`, and configuration digests unchanged. Both rule files match agent-skills/core; four recommended aliases already exist in both rc files and cease being recommended. CLI mirror updates do not refresh probes; card still showed 3-hour-old cbf1fd2 until `system:refresh-upstream --only skill:agent-skills` updated cache to 8566c8e/latest. `30a624e` package SHA256 `d1cb0ae5d782ec47fb2da328ca0e9c1b314aa9542a317cba20e4b51259dba74b`, 8689586 bytes, all `verify:macos` passed (unsigned, x86_64 only). New GUI shows Skills from skills/ · Added and Healthy · 7 Skills, no discovery failure. Existing frontend-verification bindings, already merged into verification, remain missing-link for both agents; predated organization and untouched.

## 2026-09-23: Official and Third-party Credential Isolation

- Trigger: users required complete separation, no official-account interference, following official standards. Injected fake `claude auth status` credentials showed a directory containing both authentication types could fall back to official login for endpoint-without-token/unconfirmed API Key while still sending to the third-party endpoint. CC Switch/claude-code-router/ccs/ccm/opencode/Cline research found reliable approaches preserve official login, isolate sessions/directories, and clear overriding keys.
- Decision: one credential type per directory. Main `~/.claude`/`~/.codex` contain only official accounts; third-party `agents:configure --use` returns `AGENT_MAIN_OFFICIAL_ONLY`. All third-party connections use independent instances. Never read/write keychain, `.credentials.json`, `auth.json`, or account fields in `~/.claude.json`.
- Commits, all pushed to `rust`: `f4c4cbc` executor exit-code/stderr refactor; `812a81c` official-command login absence check before third-party writes, Claude definition rules/empty shell overrides, Codex launch clearing `CODEX_API_KEY`/`CODEX_ACCESS_TOKEN`; `f918db6` one-click connection→instance, x-api-key form preserving other env; `192121b` main official-only UI/palette/tray, ⌘⇧K Account Switching; `c75c993` dead explicit-connection switching cleanup refactor; `7dbd73b` missed test fix; `2808579` on-demand official `agents:auth-status`, no auth-file reads, official login in third-party instances marked mixed; `9a3af04` detect/correct missing old Codex clearing prefixes; `1578b76` read-only external-override notices.
- Basis: Claude official authentication priorities/gateway/settings-empty-env cancellation/keychain keyed by CLAUDE_CONFIG_DIR/login sessions in ~/.claude.json; Codex rust-v0.156.0 source and config reference (env_key priority, missing variable errors without fallback; requires_openai_auth official only without env_key; CODEX_HOME-digest keyring; CODEX_API_KEY/CODEX_ACCESS_TOKEN override stored login). Third-party requires_openai_auth rendering retained only to identify legacy main-config tables.
- Acceptance before each push: full engine, Tauri 20, Vitest/build passed, final 57 files / 572 tests. Isolated HOME with real claude 2.1.281/codex 0.156.0 (isolated CLAUDE_CONFIG_DIR/path-keyed keychain): writes to new/existing directories after real auth status, rejection after login-session records. Real Claude empty strings blocked shell CLAUDE_CODE_OAUTH_TOKEN/ANTHROPIC_API_KEY/ANTHROPIC_AUTH_TOKEN. Real codex login status stayed Not Logged In with CODEX_API_KEY. Main third-party write refused/unchanged; returning official transferred token externally, cleared main fields, and left no plaintext in data. Real HOME read-only agents:auth-status corrected Claude Missing to Logged In, Codex logged in via chatgpt; credential_overrides empty on both, temporary data directories empty.
- Process defect: prepush `grep | head` truncated FAILED output for `c75c993`, allowing one stale test to ship. `7dbd73b` fixed it; subsequent reviews inspect only test result and FAILED.
- Not done: packaged-desktop click acceptance (mixed-use warnings, return to official, independent-environment creation, login checks, launch-command correction); isolated environments cannot produce real official login, so the "official command reports logged in" branch is covered only by stand-ins and read-only real-main queries; no packet capture of headers or real third-party relay integration.

## 2026-09-23 Fixes after credential-isolation workflow review

- Trigger: review found three reproduced blockers. Finder-launched desktop PATH contains only system directories, hiding Homebrew/npm assistants and rejecting existing third-party instance writes as "Cannot confirm login." Third-party Codex instances wrote only `env_key`; without exporting it, real Codex reported `Missing environment variable` despite healthy status. Saving an instance wrote only its command, so launching an empty directory prompted official login and later blocked third-party credentials. The user requested mature approaches such as CC Switch, avoiding tight version dependencies; where unavoidable, old versions need not be supported.
- Decision: follow CC Switch's "Preserve official login" mode by writing Codex instance `[model_providers.agent-manager].experimental_bearer_token` in `config.toml` (supported since 0.48; official semantics send `Authorization: Bearer` for programmatic configuration), matching Claude settings.json writes with variable names only in the database. Do not use `model_providers.<id>.auth` commands (since 0.118, requiring a bundled value-producing helper) or layer `agent-manager.config.toml` with `-p` (profile-v2 replaces `-p` since 0.134). Add common installation directories to command lookup as CC Switch does, putting the executable's directory first in PATH during execution. No real local database or rc instance / `-p agent-manager` command exists, so no legacy migration is needed.
- Commits, all pushed to `rust`: `ef9cc86` adds lookup in `~/.npm-global/bin`, `~/.volta/bin`, `~/.bun/bin`, `/opt/homebrew/bin`, `/usr/local/bin` and execution PATH for version/login probes; `27fa934` formats that commit; `d4210e9` writes Codex config.toml in place, owning only `model`, `model_provider`, and its provider table, with those three in the baseline so MCP changes are independent; writes `experimental_bearer_token`, removes `-p`, omits official-instance model_provider, requires credential variable with address, and optionally accepts Codex form values; `c41c558` writes missing third-party configuration before commands and suppresses commands if writing fails; `6a4a5f6` creates temporary instance files at 0600 and removes residual links first; `3b4b9a0` deletes an unused test helper.
- Acceptance: affected full suites ran before every push. Final `3b4b9a0`: engine-rs 401 tests, Tauri 20, Vitest 57 files / 574 tests, build, and `cargo fmt --check` passed. Each added test failed when its fix was removed (npm lookup/same-directory interpreter, Codex form constraints, two save-order cases, temporary-file symlinks). Isolated-HOME engine CLI saved/wrote a third-party Codex instance with real codex 0.156.0 login checks: config.toml 0600, managed-ok, no values in status. Real `codex exec` via its launch command, with intentionally fake shell `CODEX_API_KEY`/`CODEX_ACCESS_TOKEN`/`OPENAI_API_KEY`, produced captured `POST /v1/responses` with only `Authorization: Bearer <fake-instance-token>`; shell values were absent. Removing the clearing prefix still used the instance token in 0.156; the prefix remained as defense in depth and to avoid shell credentials appearing as official login. No instance auth.json was created; real `~/.codex/auth.json` and config.toml metadata stayed unchanged.
- Not done: packaged-desktop clicks for Finder lookup/save order; real third-party relay integration; automatic main MCP/skills/rules carryover; UI explanation of external-secret backup responsibility; blocking exported third-party variables in official instances.

## 2026-09-24: Authentication Corrections Against Official Standards

- Trigger: the user requested an unassailable tool informed by CC Switch and industry practice, with authentication following each assistant's official standards.
- Sources/method: official Claude authentication precedence, settings `env`/`apiKeyHelper`, environment variables, managed settings, WIF, and 2.1.281 binary; Codex `rust-v0.156.1` source (npm latest 0.156.1); CC Switch `f8788719`, ccs, ccm, and vendor-published preset code (`@z_ai/coding-helper`, `MiniMax-AI/cli`, `MoonshotAI/kimi-code`) where available. Every behavior conclusion was tested with real claude 2.1.281 / codex 0.156.1 against local fake Messages/Responses streaming endpoints capturing headers, isolated HOME/config directories. The cloud container injects root Claude login, so logged-out tests used an ordinary user unable to read it. Full engine permission tests also used an ordinary user: two root failures arose because read-only directory restrictions do not constrain root; all passed as the ordinary user.
- Observed and fixed:
  - Instances did not block shell inheritance: `CLAUDE_CODE_USE_BEDROCK=1` redirected third-party instances to Bedrock; shell address/key redirected official instances to relays; `ANTHROPIC_CUSTOM_HEADERS` reached instance endpoints; shell `CODEX_ACCESS_TOKEN` overrode official-instance directory login (`f13cd7a`).
  - Login checks misclassified shared Anthropic federation as login in every directory, rejecting all third-party writes; Console login hidden under third-party tokens appeared logged out; MCP-only `.credentials.json` appeared logged in. Official `--settings` now cancels directory credentials, the profile directory points nowhere, and `apiKeySource` identifies Console (`b210305`).
  - Codex accepted fatal `wire_api = "chat"` (`fdd294c`). Nine fatal forms could enter raw editing and break every command; availability checks used obsolete 0.156 openai `OPENAI_API_KEY`; provider name `OpenAI` was misrecognized as built-in (`ac4209b`).
  - Deprecated `ANTHROPIC_SMALL_FAST_MODEL` was still written (`4c6341b`); new `fable` aliases were not removed on return to official (`3c61fac`).
  - Instance env tokens reached Bash/hooks/MCP subprocesses; official `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB` blocked them in testing and became the third-party default (`3c61fac`).
  - Claude `--fork-session` copied replies and double-counted usage; instance-only third-party usage was entirely absent (`cda5940`).
  - Kimi Code membership/Open Platform shared one variable and had wrong provenance; Z.ai models lagged one generation (`af8eacf`).
- Also added: main Official account checks and copy-only login/logout commands, Chinese login-method labels, directory-change/removal warnings that login does not move, first-use x-api-key confirmation (`9dc43f1`); external-source warnings for two more cloud switches, named/federated profiles, extra headers, managed settings including `forceLoginMethod` / `forceLoginOrgUUID`, and Codex `openai_base_url` (`b210305`). See corresponding `GUI-REDESIGN.md` sections.
- Preserved with official backing: instance Codex `experimental_bearer_token` remains in 0600 files, matching source comments on programmatic configuration and CC Switch. `auth` requires a bundled credential command, so the prior rejection remains. Claude user `.claude.json` MCP header `${VAR}` expands in real tests, validating plaintext-free remote MCP.
- Verification: full suites before each push. Final engine-rs 420 (ordinary user), Tauri 20, Vitest 57 files / 588 tests, `pnpm build`, and `cargo fmt --check` passed, with a correction: Vitest also had one unhandled error and exit 1, overlooked then; see the next section. Each new case failed with its fix removed. Real CLI E2E: engine-written third-party Claude files under malicious shell sent only instance tokens to their own address, while official instances sent nothing externally; Codex exec used instance tokens only. Scanning real CLI sessions including forks counted correctly and rescans did not increase totals. Browser mock showed main login checks and no horizontal overflow at 360px.
- Not done: rebuilt desktop package/macOS clicks (Linux container); real third-party or official-account integration; reloading Codex with the official binary after writing (source-confirmed static detection only); VS Code instance environment snippets; tag or Release publishing.

## 2026-09-24 Follow-up: Corrections After Independent Review

- Trigger: independent adversarial read-only review of `a4fdbad..790ea74`, with explicit failure scenarios, plus self-review of system settings. Every conclusion was reproduced before editing; invalid findings are recorded too.
- Added (`70c48f3`): system settings unaffected by directory isolation. Real tests showed `/etc/codex/managed_config.toml` openai_base_url overriding main and receiving official credentials; `/etc/codex/requirements.toml` model_provider redirecting third-party instances to organization endpoints; system forced_login_method rejecting other login; Claude managed env overriding instance address/token and forceLoginMethod blocking third-party startup. Main/instance status reports origins/key names only; cards explain consequences.
- Confirmed and fixed findings:
  - Unhandled instance-status failure during connection-to-environment creation stalled the dialog silently (`d7b90f0`); the preceding claimed green Vitest result included it.
  - Nested Codex forks/resumed parents contained parent markers in copied prefixes, causing premature end and double-counting; half-written forks were counted as legacy whole files; truncation inside a line missed an event (`91343c4`, markers now identify `thread_id` ownership).
  - Instance status exposed `ANTHROPIC_CUSTOM_HEADERS` and `user:pass@` URLs; login parsing loaded complete `.claude.json`, including Console keys, into memory (`8ee4f47`: hide/mask, reject such definition URLs, check key existence only).
  - `requires_openai_auth = false` was falsely incompatible with `auth`; empty `auth.command` was missed (`9c2df8e`, real 0.156.1 reproduction).
  - Invalid legacy definitions disappeared and could not be edited/removed; instance-config isolation fields were mislabeled shell exports and silently cleared (`169032f`).
  - Connection-created third-party instances lacked subprocess isolation by default; existing `true` appeared disabled and was removed on save (`f9a158a`). Further testing found Linux 2.1.281 requires bubblewrap for isolation and rejects startup without `bwrap`; the container already had it, masking the limitation earlier. Downloaded macOS code always reports sandbox unavailable and lacks that error. Linux therefore defaults off with explanation.
  - Domestic/international vendor accounts shared one secret variable (`d4dfd4f`, naming follows MiniMax CLI and Z.ai helper).
- Invalid/partial findings: first probes in never-started Claude directories do create official first-launch records, with later probes unchanged; the contract now says this accurately. The claim this later makes MCP writes reject external changes was false because MCP compares entry content. Fatal rules start with 0.156 and may overreject old clients; warnings state the version and were not relaxed by installed version.
- Also added exported `CLAUDE_CONFIG_DIR` / `CODEX_HOME` warnings that main configuration is inactive (`517dd9e`); tool launch aliases do not count.
- Also checked: initial interactive third-party startup proceeds through theme, safety notice, and directory trust without official-login selection, because instance credentials take effect.
- Verification: full suites on clean `9acf29e` before push: engine-rs 428 ordinary-user tests (429 on `517dd9e`), Tauri 20, Vitest 58 files / 598 tests with no unhandled errors and exit 0, `pnpm build`, `cargo fmt --check`. Each added case failed with its fix removed (fork ownership, prefix waiting, connection-created defaults, true backfill, invalid definitions). Real E2E under isolated HOME and `/etc/codex` / `/etc/claude-code`: status reported system origins without values and queries created no HOME files; real sessions including resumes/forks retained counts on rescan; third-party Claude under malicious shell sent only instance tokens to its address, official instances nothing externally.
- Not done: real macOS subprocess-isolation execution (static macOS code only); copy-based fork files generated by CLI (constructed from source/protocol comments; exec fork is reference-based); macOS-profile-delivered settings inspection; rebuilt package.

## 2026-09-24 Further continuation: adopting CC Switch connection/authentication features

- Trigger: the user requested focused review of assistant configuration, especially provider/authentication completeness, learning from CC Switch.
- Comparison: CC Switch 3.20.4 (`f878871`) source for providers, presets, usage/balance scripts, speed checks, model lists, common snippets, MCP/skills sync. Each was assessed against actual-file authority, official-only main configuration, and instance-only third-party use.
- Adopted, all pushed to `rust`:
  - `b07f643` instance "Test": `agents:check --only profiles:<instance>` sends one minimal real request read-only.
  - `2068bf8` Secrets lists connection/instance credential variables, usage locations, presence, and environments needing rewrites after value changes.
  - `0de6349` connection cards list associated instances, expose creation when absent, and allow "Save and create independent environment."
  - `b9ebb7c` forms fetch model lists (`agents:models`, credentials not persisted), point aliases to default in one action, and support official `CLAUDE_CODE_SUBAGENT_MODEL`.
  - `2e89b68` expands presets to 25 with key/docs links, plus DeepSeek/Kimi Open Platform balance (`agents:balance`, documented APIs only).
  - `6e52095` carries permissions/statusline/Hooks/rules/MCP from default through `profiles:seed`, adding absent content only, excluding credentials, invoking native domain writes. Creation selects these by default; cards can carry them later.
- Rejected: local proxy/failover (turns the tool into a credential intermediary rewriting requests, losing direct official CLI connections); managing OAuth/auth.json/keychain (official commands own login); plaintext database secrets; TCP/HTTP-only "speed tests" (minimal real requests catch model/token failures); user JS usage scripts; credential-bearing import deep links; full-file snapshot switching (overwrites external edits). CC Switch common snippets correspond to one-time seed here, without continuous merging.
- Verification before `6e52095` push on a clean tree: engine-rs 442 ordinary-user tests, Tauri 20, Vitest 60 files / 619 tests exit 0, `pnpm build`, both `cargo fmt --check`. Cases cover argument allowlists, credential skips, existing content preservation, corrupt-file failure isolation, 0700 directories, and repeat zero writes. Isolated-HOME compiled CLI copied all five items to Claude/Codex instances, preserving main bytes, 0600 files/0700 directories, and omitting main credentials from instance/data paths. Real `claude mcp list` / `codex mcp list` showed only nonplaintext entries and Codex loaded merged configuration; repeating returned exists/skipped and unchanged bytes. Prior model lists, instance checks, and variable consumers also used local fake endpoints/isolated HOME, trying list candidates individually without persisting tokens.
- Not done: real macOS/rebuilt packages; Alibaba Cloud Bailian/Tencent/Baidu presets (documentation blocked by environment egress policy, so addresses/models remained unverified); Coding Plan quota APIs (none documented found); concurrent multiroute speed ranking; VS Code configuration; installed-binary Codex post-write load check (static 0.156+ checks only). Seed excludes skills, assigned per environment on Skills.

## 2026-09-24 User feedback: default-environment state and risk are unclear

- Trigger: an old-package screenshot showed Codex "Current actual configuration (unsaved) · In use" and an "Assistant default account" Apply card. The user could not determine state, next action, risk, or whether Apply loses authentication.
- Reproduction with isolated HOME, real codex 0.156.1, fake API-key login: configuration had only `model = "gpt-6-luna"` and runtime parameters, yet engine reported `default: false`. Returning left auth.json bytes/login unchanged but removed model and created "Imported configuration." Current code also falsely reported main third-party use.
- Fix (`136dff5`): classify accounts by address/credentials; model is an official-account setting (`current.official`). Returning when already official writes nothing. Default rows identify account/login storage/model source/automation. Connection tabs separate account from third-party instance-only connections. Return appears only for mixed use, with explicit scope in confirmation.
- Verification: engine-rs 443 ordinary-user tests, Tauri 20, Vitest 61 files / 625 tests exit 0, build, both format checks. New tests cover official Claude/Codex models with zero writes and screenshot/dialog behavior. Reproduction now reports official, returns write-free, preserves model/login; browser mock checked both states.
- Not done: UI editing of official default model (raw editing at bottom of Runtime settings required); rebuilt package.

## 2026-09-24 User feedback: CC Switch is clear, this tool is confusing

- Assessment: CC Switch has one object (vendor cards), exactly one active item (startup imports actual configuration into default, plus an official preset), and one action (Enable rewrites one directory). Here unusable-on-default Provider cards sat in the default tab while usable instances sat elsewhere, with competing "connection," "independent environment," "instance," and "default account" terms. The user previously required credential isolation by directory, so instead of copying main rewrites, each card becomes a launchable environment with its command.
- Fix (`9290dec`): remove default/independent tabs; select an assistant and show default, instances, then third-party templates on one page. Top-right New environment (vendor or another official account) is primary; third-party form says "Next: create independent environment." Instance headers show account type and command.
- Verification: Vitest 61 files / 626 tests exit 0, build, tsc; navigation tests no longer switch tabs, new tests cover both creation routes/commands; mock screenshots checked sections/menu/form and no horizontal overflow at 360px. Engine/Tauri unchanged; previous full baseline 443/20.
- Not done: New environment → Third-party remains two steps (save connection, then confirm command/carryover in prefilled instance form), not one form; no rebuilt package or desktop clicks.

## 2026-09-24 User asks whether credential questions remain

- Walkthrough: official storage, noninterference, and on-demand official checks are clear. Four third-party API Key problems remain: first use requires manually creating a 600 .env or reports no external secret file; variable name plus Key hides whether a value exists; changed keys appear as generic definition drift with meaningless credential "Update definition"; cards omit key destination.
- Fix (`ac894ac`): `secrets:source --create` exclusively creates missing 0600 files outside data directories and removes invalid new files; `secrets:status` lists existing names. Forms put API Key first, explain presence/destination, and automatically create `~/.config/agent-manager/secrets.env` before saving when absent. Secrets offers "Create one." `profiles:status` marks credentials `credential`; cards show variable/file/consistency and offer direct "Write new key" after changes.
- Verification: engine-rs 445 ordinary-user tests, Tauri 20, Vitest 61 files / 632 tests exit 0, build, both format checks. Engine cases cover permissions/no overwrite/data-directory and relative-path rejection/changed-value actions; UI covers creation order, presence text, confirmation-free new-key action, and file creation. Isolated E2E created 0600 files/0700 directories; real claude 2.1.281 sent the first key to a local fake endpoint, then reported credential mismatch without values or definition-update action after rotation. Rewriting sent the new key; no key remained in the data directory.
- Not done: real Codex rotation E2E (shared comparison covered only by engine cases); rebuilt package.

## 2026-09-27 CC Switch experience improvements (ongoing implementation)

> Retired: under the multi-account design from 2026-09-28, Provider libraries, return-to-official actions, runtime forms, Hooks previews, and related features in this section and the following 09-28 records were deleted in P6 and the 2026-09-29 cleanup. These records are historical only.

The scope is daily assistant/Provider configuration, preserving separate directories, official/third-party credential isolation, actual-file authority, and target-specific write baselines. Presets, copy, sorting, import/export, real-request checks, model lists, balance, and initial carryover described in the earlier CC Switch section already have source/tests; historical verification does not establish current-package acceptance.

| Remaining workflow | Current code evidence | Completion criterion |
| --- | --- | --- |
| Use independent environments through daily entry points | `⌘⇧K` Accounts and environments lists official accounts, instances, and existing commands; environments locate cards, commands copy names. Cards distinguish "Copy launch name" from "Copy one command." Tray lists actual names and opens cards for missing commands; events open exact entries without switching/launching assistants | An official independent environment was created in an isolated native app and the tray opened its palette item; repeated clicks preserved instance/rc/database digests. New shortcut scope has frontend cases but no current native interaction. Multiple/missing/unwritten command branches have local cases; package and platform menus remain unaccepted |
| Maintain instances after Provider edits | Edit lists associated environments/field differences, unselected by default. Fields still matching old Provider can follow edits; self-modified or previously unselected fields remain by default and can later be selected individually. Legacy association uses original address/variable, rejecting ambiguity. `profiles:apply-definition` checks actual-file/definition digests and writes files/definition/ownership under lock with rollback; UI reads back each environment | Isolated native UI verified current updates and later synchronization of previously unselected fields, with file/database readback. Multitarget failure isolation and legacy ambiguity lack native interaction. Legacy environments diverged from original values without provenance need manual checking |
| Maintain shared settings between environments | Creation seed adds missing fields only. Existing cards compare actual permissions/statusline/Hooks/rules/each MCP through `profiles:shared-status` / `profiles:shared-apply`, unselected by default, checking source/target digests and reading back per item. Differences hide free text; selection calls read-only `profiles:shared-preview` for complete target text/results | Continue native isolated coverage of Hooks/rules/MCP/Codex and sequential updates, checking failure isolation. Installed-client MCP loading is verified in isolated directories; other runtime effects and packages remain pending. No background sync |
| Keep broader connection coverage trustworthy | Adapters currently support Claude Code/Codex only; presets require vendor-backed protocols/endpoints, balances require official APIs | Expand only after native format, isolation, and real request paths are confirmed per client. Every preset needs vendor docs, editable fields, and isolated verification; unconfirmed capabilities cannot appear usable |

Implement in table order through incremental commits/pushes, targeted checks first, then affected complete Rust/frontend suites. UI mocks prove interaction only; real client behavior requires isolated HOME/data/test-assistant readback. Current packages need separate builds/acceptance. Rejections remain governed by the 2026-09-24 security/product decision; this list does not authorize global overwrites, OAuth management, or proxies.

Entry-point verification: frontend 61 files / 636 tests and build passed; Tauri 21 and full engine tests passed. A native debug app with isolated HOME/data/stand-ins created an environment; `⌘K` located/focused its card, tray listed environment/name and opened its palette item. Four isolated file digests stayed unchanged across tray clicks. This proves entry points only, without real Claude/Codex or rebuilt packages.

Per-environment Provider updates: three new isolated engine cases; frontend 61 files / 642 tests, build, full engine, Tauri 21, both format checks passed. Native isolated debug UI edited a Provider address, listed differences, selected/saved one instance, and reported/read back the update. Direct instance-file/database/`profiles:status` readback agreed on address and managed-ok; no main configuration directory appeared and the database contained no test credential value. No real assistants/package rebuild. Default-unselected, multitarget failures, and legacy ambiguity had local tests only.

Later Provider synchronization: frontend 61 files / 644 tests and build passed. Isolated native debug data had new Provider address/old environment address; reopening edit without changing Provider showed an unselected old-address difference. Explicit selection updated successfully. File/database/status all showed the new address and managed-ok; no main files or database test values. No real assistants or package rebuild.

Provider multitarget failure isolation (2026-09-28): isolated HOME/data/stand-in Claude created two managed environments. `profiles:preview` confirmed both new-address candidates without writes; then an external edit changed the first owned API address. Per-environment `profiles:apply-definition` using the same batch baselines rejected the first, preserved its external address, and updated the second address/model to managed-ok. Main settings.json SHA256 stayed unchanged. Frontend regression continued after first failure, reported "1 unconfirmed," and disabled autoassociation for two matching Providers. Full frontend 652/build passed. The system listed the native debug window, but accessibility repeatedly refused reads despite reported authorization; native interaction evidence remains absent. No real assistants/package rebuild.

Shared-setting updates: isolated Rust covers Claude permissions/statusline/Hooks/rules/same-named MCP replacement, plaintext-MCP blocking, and stale source/target digests. Codex covers approval/sandbox/statusline preserving provider and fake auth files. Frontend 61 files / 646, build, full engine, Tauri 21, both format checks passed. Native isolated Claude showed permissions/statusline differences unselected. Selecting permissions only yielded consistent readback and source permissions while retaining instance statusline; main/fake-login digests stayed unchanged. External source-statusline change after preview rejected native updating and preserved the target. Native Hooks/rules/MCP/Codex actions remained untested; no real assistants/package rebuild.

Shared-setting value previews: `profiles:shared-status` shows source/target fixed approval/sandbox values, while free text remains counts/hidden notices without commands, rule text, MCP arguments, or credentials sent in differences. Isolated CLI updated Claude Hooks/rules/MCP and Codex permissions/statusline/rules across two instances; all six read back `same`, and seven main/fake-login digests stayed unchanged. Targeted frontend cases prove sequential failure isolation and two status readbacks. Native debug launched, but accessibility refused reads, leaving no new interaction evidence; real loading/package acceptance remained pending.

Account/environment shortcuts: `⌘⇧K` groups official accounts, instances, and existing launch names. Frontend cases cover search/copy/locate/missing or unwritten commands and assert no configuration writes. Full frontend 59 files / 631 and build passed. No current native interaction evidence or real assistant configuration verification.

Codex key-rotation acceptance (2026-09-27): current-source Rust CLI, isolated HOME/data, real `codex-cli 0.156.0`, and loopback-only `127.0.0.1` test endpoint. Initial writes produced `/v1/responses` using test credential A. Changing the external secret to B made status drifted; explicit rewrite/readback of config.toml then produced B requests with no A in the file. The server logged credential categories only; database lacked A/B values, and default config.toml/auth.json were absent. Repeating preserved three host Claude/Codex configuration/auth digests. This proves only local isolated rotation requests, not native UI, packages, real vendors, or complete conversation responses.

Shared settings with real clients: disposable isolated HOME/data/fake login created Claude/Codex instances; copied main permissions/statusline/Hooks/rules/one MCP each. Shared status readback was same; seven main/fake-login digests stayed unchanged. Installed Claude Code 2.1.283 / Codex 0.156.0 ran mcp list in isolated directories, both exit 0 and listing copied MCP. Real main/auth digests stayed unchanged before/after. This proves MCP loading only, not other settings' runtime effects, model requests, native UI, or packages.

Runtime complete-file immediate previews (2026-09-28): `agents:preview-runtime` rereads read-only and shares runtime rendering with configure. Input changes show affected keys then complete actual-merged JSON/TOML; stale responses cannot replace newer inputs. Isolated Rust checks byte-identical preview/write, preserved Codex comments/user fields, stale rejection/unchanged files, and missing-file preview without file/database creation. Frontend 650, complete engine/Tauri, build, and format checks passed; DEV mock read back complete files. Mock evidence covers interaction only, not native/current-package acceptance; real assistant files were neither read nor written.

Additional native-window verification (2026-09-28, current-source debug build): Tauri with isolated HOME, `AGENT_MANAGER_DATA_DIR`, and version-only stand-ins. Claude Smart automatic immediately showed complete settings.json with existing theme/env and new permissions.defaultMode; Codex showed preserved comments/model/history and approval_policy/sandbox_mode/approvals_reviewer. Both target SHA256 values stayed unchanged and data remained empty. Changing new-command Codex automation to Smart automatic immediately showed `--approve-for-me` in alias preview; cancellation created neither rc file. Only these current-debug interactions were verified; package acceptance remained separate and real configurations untouched.

New-instance selected-setting immediate previews (2026-09-28): read-only `profiles:preview --items` calculates permissions/statusline/Hooks/rules/MCP in creation order, reusing individual MCP copy plans. Isolated Rust for both types compares preview text with seed main/additional bytes while no instance directory/database exists before preview. Plaintext MCP is skipped without values leaking; existing names stay intact. Both forms refresh previews on selection, displaying paths/text/results. Frontend 59 files / 652/build, full engine/Tauri 22/both format checks passed. Isolated debug with stand-ins launched without data writes, but macOS accessibility refused reads despite authorization; no native interaction evidence or real assistants/rebuilt package.

Existing-instance complete shared previews (2026-09-28): `profiles:shared-preview --selection` merges selected items in actual write order in memory, rejecting changed source/target digests. Differences still hide free text; selection displays complete local files/paths/failures. Isolated Rust compares multi-item permissions/statusline/Hooks/rules/MCP per-file bytes against apply and verifies zero writes/stale-source rejection. Frontend 59 files / 653, build, engine, Tauri 22, formats passed. Native isolated debug opened a card; selecting permissions/statusline/rules displayed merged settings.json/CLAUDE.md. Four isolated config/database SHA256 values stayed unchanged. External source-statusline edits invalidated preview and disabled update; refreshed selection showed new text with targets/database still unchanged. No real assistants/package rebuild.

MCP add/edit complete previews (2026-09-28): `mcp:preview` shares `mcp:apply`'s read-only plan, accepting add/edit. Valid input shows complete files for selected assistants; input/target changes invalidate old results. Isolated Rust compares Claude JSON/Codex TOML preview/write bytes and checks zero writes/stale external-entry digests; isolated CLI separately reads files/database. Single-worker frontend 59 files / 655, build, engine, Tauri 22, formats passed. Concurrent full frontend twice timed out on an existing environment-form wait assertion; single-file/single-worker passed without weakening it. Native isolated debug showed existing/new Claude entries; editing command invalidated/refreshed text, selecting Codex added new TOML preview. Cancellation preserved Claude digest and created no Codex file/database. No real assistants/package rebuild.

Hooks complete previews (2026-09-28): structured/raw editing shares read-only `hooks:apply` plans and automatically shows complete target JSON/path. Incomplete rows/nonobject raw input send no request; input changes hide old results. Isolated Rust compares both assistant preview/write bytes and checks zero writes/stale rejection. Targeted frontend 33, single-worker full 59 files / 656, build, engine, Tauri 22 passed. Concurrent frontend/two Cargo runs timed out on an existing environment wait; its 46-test file and nonconcurrent full rerun passed with assertions intact. Native isolated Claude showed complete settings.json including theme; changing to `echo new` updated text. Cancellation preserved target SHA256 without creating data. No real configuration operations/package rebuild.

MCP cross-assistant copy complete previews (2026-09-28): preview adds copy, sharing conversion/name conflicts/source-target digests. Target/replacement selection shows complete text/path; unchecking replacement immediately hides it. Isolated Claude → Codex compares preview/write bytes, verifies unchanged target/database before preview, and rejects unconfirmed conflicts. Targeted frontend 18, single-worker 59 files / 656, build, engine, Tauri 22, formats passed. Native isolated Codex preserved comments/model with copied MCP command; unchecking hid text and closing preserved both config SHA256 values without creating data. No real configuration operations/package rebuild.

MCP deletion complete previews (2026-09-28): preview reuses deletion plans. Codex shows complete resulting files; Claude saved disabled database-only deletion explains unchanged configuration, labeled "Delete saved content." Preview failures disable confirmation; reopening rereads instead of reusing old results. Isolated Rust checks byte-identical deletion and unchanged preview files/database. Targeted frontend 12, full 59 files / 658, production build, engine, Tauri 22 passed. DEV mock showed paths/remaining entries/database-only explanation, with demonstration text rather than real files. Isolated debug launched with targets unchanged and no data creation; accessibility refusal prevented native clicking/visual acceptance. No real configuration operations/package rebuild.

Terminal shortcut complete previews (2026-09-28): new `aliases:preview` shares save plans, showing actual .zshrc/.zprofile paths/complete candidates on add/edit/rename. Input or new baseline invalidates old results; failure disables save. Isolated Rust/CLI compare both preview/write files byte for byte, preserving outside-block content, creating no preview files/database, rejecting stale baselines, and passing zsh -n afterward. Full frontend 59 files / 660, production build, engine, Tauri 22, formats passed. DEV mock checked both files/input changes with demonstration data only; no native/current-package or real-config acceptance.

Terminal shortcut confirmation previews (2026-09-28): remove/recommendation/legacy-add and assistant-page command deletion all preview complete candidates before confirmation; failed previews disable it. Discarding legacy database records still explains unchanged terminal files. Isolated Rust compares three write types and confirms preview zero writes; CLI final-alias removal matched preview and passed zsh -n. Targeted frontend 41, single-worker full 59 files / 663, production build, engine, Tauri 22 passed. An old test initially lacked a preview fixture; adding success/parameter assertions made targeted/full pass without skipping or weakening. DEV mock viewed/canceled two assistant deletion candidates; demonstration only, no current-native/package or real-configuration operations.

Terminal shortcut preview-failure recovery (2026-09-28): add/edit/remove/recommendation/legacy and assistant launch edit/delete offer "Close and reread" on failure. This discards stale forms/dialogs, calls `aliases:status`, and requires a new operation; old input never acquires a new baseline. Targeted frontend 40, full 59 files / 665, production build passed, covering refreshed external values, disabled confirmation, and no `aliases:apply` during recovery. Frontend/docs only; no native/package or real-configuration reads/writes.

Return-to-official complete previews (2026-09-28): agents:preview-default rereads main and validates page baseline, sharing agents:configure --use-default rendering. Dialog shows complete paths/text; unfinished/failed previews block confirmation, with close/reread recovery. Isolated Rust compares both assistant bytes, zero writes, Codex user tables/comments, official no-op, and stale rejection. Single-worker frontend 59 files / 667, production build, typecheck, engine, Tauri 22, Rust formats passed. Concurrent frontend/build hit existing UI timeout; single-worker rerun passed. DEV mock viewed complete demonstration candidates; no native/package or real-configuration reads/writes.

Return-to-official full-file validation after preview (2026-09-28): dialog passes original SHA256 from agents:preview-default into agents:configure --use-default. Writes still validate owned baseline and additionally reject later unowned changes, preventing stale complete-file confirmation. Other writes retain owned-field baselines. Isolated Rust checks bytes and an unowned Codex comment changing the full digest but not owned baseline, rejecting without file/definition changes; isolated CLI returned AGENT_CONFIG_CHANGED with zero preview writes and unchanged Claude file. Single-worker frontend 59 files / 667, production build, engine, Tauri 22, formats passed. Native window was listed but accessibility refused reads; no native evidence/real config/package rebuild.

Runtime complete-preview confirmation gate (2026-09-28): agents:preview-runtime returns original digest; only a successful current-input complete preview with digest permits submission, carrying it through fully-unrestricted confirmation. Unowned changes also return AGENT_CONFIG_CHANGED; quick repairs without full previews retain owned baselines. Isolated Rust checks both byte sequences, missing-file baseline, and external Claude Hooks edits rejecting without definition changes. CLI confirms zero writes/rejection/identical writes when unchanged. Targeted frontend 60, single-worker full 59 files / 668, typecheck, production build, engine, Tauri 22, both formats passed. No native evidence, real configs, or package rebuild.

Desktop legacy-lifecycle entry cleanup (2026-09-28): product code no longer calls `agents:apply`; desktop allowlist removes it with entry-only parameter types/messages/fixtures. Rust CLI and old database definitions remain readable. Tauri 22, single-worker frontend 59 files / 668, production build/formats passed. Isolated CLI still recognizes the command and rejects missing explicit target/action without writes. No real configuration involvement, native interaction, or package acceptance.

Pre-save new-environment preview gate (2026-09-28): manual creation, Provider-preset creation, and explicitly selected existing-environment updates require successful current-input file previews. Provider-only saves/existing-definition-only edits remain direct. Targeted cases cover waiting/input changes/failure/manual reread disabling/original save paths. Single-worker full 59 files / 672, typecheck/production build passed. Frontend/docs only; no native/package or real-configuration reads/writes.

Command-palette return-to-official preview gate (2026-09-28): standalone refactor `e1408c2` extracts the assistant confirmation unchanged into a shared component. Palette/tray-reopened official actions show complete files, then submit preview status/full-file baselines. Unsaved-token/unrepresentable-content confirmations preserve both; rejected external changes close stale confirmation and reread. Targeted palette 21, single-worker full 59 files / 676, typecheck/production build passed, covering waiting/failure/cancel zero writes/tray/readback/external changes after secondary confirmation. No native/package or real-config reads/writes.

Saved-instance complete pre-write previews (2026-09-28): Write configuration, Overwrite from definition, and Write new key all call read-only `profiles:preview` with real values/apply plans, showing complete settings.json/config.toml; failure blocks confirmation. Confirmation carries original and candidate SHA256; unowned file, definition, or secret changes invalidate writing. CLI without these optional digests keeps prior semantics. Isolated Rust checks zero writes, byte-identical candidates, external-file/secret rejection, unchanged main, no database credentials; UI covers all entries/failure/cancel. Engine, Tauri 22, single-worker frontend 59 files / 678, typecheck/production build/both formats passed. Parallel frontend timed out; single-worker rerun passed. No native/package acceptance or real configuration writes.

Provider-selected existing-environment exact previews (2026-09-28): `profiles:preview` candidate mode adds explicit `--exact` to render current values; unsaved form keys temporarily override the corresponding variable for this read-only request. Complete candidates and original/content digests are returned; apply-definition rechecks both before writing and under lock. CLI without exact retains references and no digests. Key/option changes invalidate previews; absent digests disable submission. Only selected environments update, with no plaintext keys in write requests. Isolated Rust checks zero writes/exact bytes/external-file or key rejection/main unchanged/no database keys; UI checks missing-digest blocking/new-key repreview/key-save-before-config order. Full engine passed twice; Tauri 22, single-worker frontend 59 files / 678, typecheck/production build passed. No native/package acceptance or real-config writes.

Tighter post-import restoration guidance (2026-09-28): remove "Go to overview and sync all" and its exclusive parameters. Bulk sync cannot restore imported Providers/instances in one action. Success instead directs users to individual previews/application and separate secret setup. Engine import/difference-preview paths unchanged. Targeted backup/navigation 12, single-worker full 59 files / 678, typecheck/production build passed. No real imports/writes or native acceptance.

Overall usability review and sources (2026-09-29, `a7042d4`…`60c1517`): aligned skill/MCP/rule/command tables with account toggles. New top-level Sources supports git/local directories/rule files, add/update/edit/disable/remove, optional recommendations with "Add…" / "No longer needed." Skills can "Make my own version." Source removal may clean downloads but touches only links to that source. Login defaults to local read-only checks; version probes use temporary configuration, leaving HOME unchanged. Eleven global review items: consistent terminology (collections → sources and broader wording guard); Accounts Get started (main login, optional sources) and headers; Hooks renamed Automatic commands and tabled; unsupported MCP accounts omit columns; Paste MCP configuration converts plaintext secrets to variables; command secondary content moves to More; repeated counts/recommendations removed; top-bar Search ⌘K; errors explain what happened/what to do with raw detail collapsed. Verification: engine 441, Tauri 21, frontend 60 files / 559, typecheck/production build/both formats/`cargo check --all-targets` passed locally; dev mock screenshots checked pages. Earlier isolated CLI verified source removal/custom versions/query zero writes, finding and fixing removal of user-version links. Not done: GitHub billing prevented every CI job (no green since 2026-09-08), replaced by local equivalents without Windows; packaged app opened only, with no real/isolated item clicks because native automation unavailable; share-link system deep links covered parsing/dialog tests only.

Hooks removal and narrow top bar (2026-09-29, `30ab8af`, `4169d95`, `bfad127`): user decision removes Hooks UI/editor/bridge `hooks:*` allowlist/`native_hooks`/tests; creation carryover/shared sync/system audit exclude hooks; backups omit them, with imported hooks.* skipped and reported. Actual account hooks/legacy database tables remain unchanged. Isolated release CLI reports `hooks:status` as UNKNOWN_DOMAIN, no audit segment, rejects hooks seed, and creates new settings without it when omitted. Shared settings retain permissions/statusline/rules/MCP, main bytes unchanged. At 900px minimum width, top bar hides app name and uses search icon; dev checks at 900/1000/1280 show no horizontal overflow. Engine 403, Tauri 20, frontend 534, typecheck/build/formats passed; remote CI still blocked by billing.

Residual connection-recognition cleanup (2026-09-29, `2283a9b`): `agents:status` current reports default/official only, without saved-profile matching; configure results drop active/current. Connection-switch-only rendering/matching functions/tests removed; DEV mock rejects switching as engine does. Saved connections still parse for Secrets variable references. Full engine (lib 153), Tauri 20, Vitest 58 files / 534, typecheck/build passed. Isolated status reported third-party main `official: false` and missing Codex `official: true`, with no tokens/data creation. Intermittent frontend case also fixed (`1eca352`, `2e70d14`): it inspected first-preview parameters, but busy-machine debounce could preview partial input first.

Skill-source versioning phases A/B (2026-09-29, `921675c`…`dfe1a55`; [Skill Sources](SKILL-VERSIONING.md)): A adds read-only Other sources, scans Codex shared `~/.agents/skills`, identifies origin via npx lock/skill-release.json/git, flags duplicate names loaded twice. B adds git tracking: automatic follows latest stable version tag when available, otherwise main; alternatives are version rule/main/specific branch/matching branch/pinned. Only skill_paths/rules/aliases download via tree:0 partial clone and sparse checkout. Version dialog updates latest or switches any tag/branch, then moves in-use skills. Verification: full engine with real-git fixtures for automatic/main/pinned/prerelease/rollback/scoped downloads/update checks/bad input; Tauri 21, Vitest 58 files / 544, typecheck/build passed. Real GitHub isolated-data readback upgraded agent-skills full mirror to only `skills/`, `rules/`, and `aliases/` with all 7 skills; new archify download was 14 MB and v3.0.1 ↔ v3.0.0 contents/update checks agreed. Not done: file-level preview, C adoption/redundancy cleanup, rebuilt-package acceptance.

Skill-source B previews and C adoption (2026-09-29, `0c3c344`…`333a491`): `skills:preview` lists file changes/affected accounts before confirmed version switching. `skills:adopt-preview` / `skills:adopt` identify source and compare local files, adopting the matching older version when necessary; retain one source, back up redundant copies before cleanup, restore by manifest on failure, and support undo. Managed duplicates receive cleanup only; unknown sources may preserve local content under `~/my-skills`. Verification: full engine 411 with real-git fixtures for identical/old/edited/renamed/duplicates/local preservation/stale fingerprints/partial restoration/undo/ineligible; Tauri 22, Vitest 58 files / 550, typecheck/build. Read-only real previews classified archify/orca-cli/find-skills/xiaoyu duplicates/synced correctly with no database/directory writes. Isolated HOME copied real archify/find-skills/lock; real GitHub adoption left one source, both accounts linked, shared directory empty, and undo restored identical bytes. Version lookup on thousand-tag repositories fell from 14 to 4 seconds; first orca adoption preview took about 80 seconds. Native selection unaccepted (buttons/dialogs unit-tested only); independent package acceptance is in the next receipt.

Windows support (2026-09-30): platform layer covers home directories, directory junctions, PATHEXT lookup, command-forwarding scripts, registry PATH reads only. Commands become `~/.agent-manager/bin/<name>.cmd`, sharing rc query/edit/baselines. Registry remains unchanged; absent command-directory PATH produces instructions for users. npx-style MCP uses cmd /c. Programs recognize global npm cmd-shims; installed entries use .cmd forwarding. git disables newline conversion and quota filenames normalize Windows paths. See [Platform Status](GUI-PLATFORM-STATUS.md#windows-support-2026-09-30). Engine/desktop x86_64-pc-windows-gnu cross-compilation was warning-free; Docker/Wine 11 Windows engine 399 tests passed, including executing generated commands in cmd. macOS engine 439, Tauri 23, frontend 605, production build passed. Not done: physical Windows desktop/package, real Claude/Codex/Git Bash statusline; GitHub CI remains billing-blocked.

## Delivery Receipt (2026-09-29, Multi-account Version)

Bound commit: `2e70d14` (product code; later changes are documentation only).

- Package: clean x86_64 DMG, SHA256 `37c07fb92bf3dca5b3502c86a8db5edee535d9a042764ad5fc04244717f702e5`, 8761959 bytes; all verify:macos checks passed (matching build identity, no Python, sanitized paths, isolated 5-second survival/graceful exit). Unsigned/unnotarized.
- UI: DMG-extracted app launched under isolated HOME/data, with PATH containing only real claude 2.1.285 and codex 0.159.0. macOS accessibility pressed buttons and focused/pasted fields. Accounts added `gui-api` targeting a local fake Anthropic endpoint; drawer listed 6 planned actions before creation and readiness was 4/4 afterward. Readback: `~/.claude-gui-api/` 700 contained settings.json 600 with address/model/token/main permissions, and CLAUDE.md copied main rules. API Key was only in external 600 secrets.env, with no plaintext in database/data. rc files added only managed lines, preserving user aliases and passing zsh -n; main hashes unchanged.
- Real Claude: interactive zsh ran `gui-api -p`, requesting the fake service with fake-model and the main-rule marker (P4 real rule loading). Enabling live quota pointed main/gui-api statusLine to the data-directory script while preserving permissions. Real interactive pseudoterminal rendered model/directory/context/cost statusline.
- Login isolation (P2 risk): real 0.159.0 codex login status in isolated CODEX_HOME reported Not logged in; main credentials were file-stored auth.json (metadata only). Isolated CLAUDE_CONFIG_DIR `claude auth status` reported `loggedIn: false`.
- Not covered: quota requires subscription rate_limits absent in API-key sessions, so no quota file was written and P5 still needs subscription testing; proving real isolated login preserves main requires user login and was not tested; official/Codex account UI creation absent; external rejection not UI-retested (engine/CLI cover it); WebKit still uses real-home webview storage; unsigned, x86_64 only.

## Delivery Receipt (2026-09-23, Actual-configuration x86_64 DMG Machine Acceptance)

Bound commit: `c0065c090e2ed89e6adedac60bc025a159baefda` (product code; later `0ceb7e0` changes acceptance scripts only).

- Package: locally rebuilt clean x86_64 DMG, SHA256 `56cd1ab2a10da097d6a321215948fbbbb1150e00be4b3cf6218af5554de3358d`, 8514164 bytes; verify:macos passed metadata, exact commit/date build identity, no Python, sanitized paths, isolated 5-second survival/graceful exit. Unsigned/unnotarized.
- Environment: DMG-extracted app ran directly with isolated HOME/AGENT_MANAGER_DATA_DIR and version-only stand-in claude/codex PATH. macOS System Events/JXA clicked native UI and captured window areas, with item-by-item file/database readback. No real assistants, login/logout/check/install/upgrade actions.
- Passed: startup/page status zero writes, empty data and byte-identical HOME/fixtures; built-ins excluded from pending sync; skills show real directories/repository recommendations only; MCP shows manual entries, Codex disable adds only `enabled = false` preserving comments/cwd/enabled_tools/user providers, and Claude plaintext-token disable without selected secret file rejects visibly. Rules show actual files; external edits cause write rejection/preservation; reopening applies recommendation after backup-template preservation. Hooks prompt/command do not block. Recommended aliases write both blocks preserving outside aliases and passing zsh -n. Returning assistant to default first warns unsaved token; explicit discard removes only three ANTHROPIC_* keys, preserving hooks/permissions/theme and importing actual content without token. Instance differences are listed; confirmed definition overwrite merges while preserving user env/theme; after external model changes, definition-from-actual updates only database, not files.
- Found/fixed: Claude disabled snapshots detected values only, allowing prefixless GITHUB_TOKEN into plaintext database. `c0065c0` now moves values when either name or value resembles credentials, with regressions. Acceptance build-identity whole-line layout comparison falsely failed; `0ceb7e0` extracts markers instead and preserves counterexamples.
- Real environment: unchanged digests for ~/.claude/settings.json, `~/.claude/CLAUDE.md`, `~/.claude.json` mcpServers, ~/.codex/config.toml/AGENTS.md/hooks.json, `~/.zshrc` / `~/.zprofile`; auth.json metadata only unchanged, as was Application Support. Exception: WebKit uses real `~/Library/WebKit/io.github.ly87ing.agent-manager`; access statistics/localStorage WAL/shared memory were touched, main localstorage.sqlite3 unchanged and WAL 0 bytes, unrelated to assistant accounts/configuration.
- Limits: nonstandard stand-ins show assistants as Your decision needed/system install source, an environment artifact. Instance definitions were created by CLI before GUI actions; new-form keyboard entry was not tested on the machine. Unsigned, x86_64 only.

## Current stopping point and recovery entry

- Current stopping point (2026-09-29): multi-account `2e70d14` DMG/account-creation acceptance complete, per receipt above. Pending: subscription quota-file writes P5; real isolated login preserving main; GitHub Actions billing because payment failure prevented jobs. The following two entries are earlier stopping points.
- Stopping point: old `5a110be3` x86_64 DMG/GUI E2E and arm64 static acceptance complete. Direct apply completed source/browser/isolated CLI/uncommitted dirty-DMG acceptance; final clean DMG rebuilt after a01d0f5 and independently verified, per receipt above. Three commits d46b85a/47702ed/14e6790 completed in-repository release readiness (LICENSE/metadata/CSP/icons/error boundary/custom-assistant confirmation/MCP empty states/draft-first single-writer pipeline refactor). Release pipeline 14e6790 and Windows blocking CI 9f56f00 ready; external conditions still block public publishing, without tag/Release.
- Recovery in order: resolve billing; provision updater/Apple/Windows signing outside repository/in Secrets; obtain real green GUI CI for current code; independently install/run physical Linux/Windows/arm64; complete macOS MCP-add forms when automated input is available; with separate user authorization tag (hyphen suffix produces prerelease draft), then workflow creates draft Release/four-platform artifacts/checksums and single-writes latest.json. Public release requires independent real signing/notarization/updater download-install readback per platform.

## Verification

`cargo fmt --manifest-path engine-rs/Cargo.toml -- --check`, both Cargo test/check suites, and `cd app && pnpm test:unit run && pnpm typecheck && pnpm build`. Package/independent acceptance use `pnpm --dir app run build:macos` and `./scripts/verify-macos-dmg.sh <path-to-dmg>`.

## 2026-10-07: Learning from CC Switch and Magpie (source and local integration passed)

Goal: make accounts easier to find/use under local single-user deployment with isolated directories, editing actual configuration, explaining failures, and preserving external edits.

Sources fetched this round: CC Switch `5ae6ad3888ba4543f6fad343c87656a97bd69da4`; Magpie interpreted as `yetone/magpie@3aeb7d1bf8609eac8e67cb6b41677c548847e433` (clarification unanswered). Upstream source informs decisions, not copied implementation.

| Constraint | Source | Scope this round |
| --- | --- | --- |
| Independent accounts; official login only in main | AGENTS.md, multi-account design | Do not restore main connection switching or cross-client subscription reuse |
| Actual-file authority, zero-write previews, preserved external edits | GUI redesign | Do not hide reality behind old database values; rollback also compares |
| Simple local changes without extra writable configuration | AGENTS.md | Reuse existing status/queries/forms/write paths; no migration |
| No package publishing | User authorization boundary | Source improvements/local verification only, no tag/Release |

| Observed upstream idea/source | Implementation here | Acceptance criterion | State |
| --- | --- | --- | --- |
| [Magpie field-write transactions](https://github.com/yetone/magpie/blob/3aeb7d1bf8609eac8e67cb6b41677c548847e433/internal/agent/agent.go#L442), [CC Switch failure recovery](https://github.com/farion1231/cc-switch/blob/5ae6ad3888ba4543f6fad343c87656a97bd69da4/src-tauri/src/services/provider/mod.rs) | Account-write readback/conditional rollback, partial-removal feedback | SQLite failure restores old/removes new files; repeated external edits remain with conflicts; command deletion failure is not complete success | Passed locally |
| [CC Switch list search](https://github.com/farion1231/cc-switch/blob/5ae6ad3888ba4543f6fad343c87656a97bd69da4/src/components/providers/ProviderList.tsx#L255), shortcut entries in both | Account search/read-only refresh/quota reset explanations/keyboard menus | Search account/command/address/model; refresh neither writes nor autoprobes login; keyboard open/move/execute/close; no narrow overflow | Passed locally |
| [Magpie actual reads/precise edits](https://github.com/yetone/magpie/tree/3aeb7d1bf8609eac8e67cb6b41677c548847e433/internal/agent) | Actual account-row/form values, stale-edit protection, key explanations | External models show actual values, edits do not silently overwrite, explanations match credential destinations | Passed locally |

No new service, background task, database schema, routes, or outside team is needed. Scope is profiles writes, Accounts UI, and shared row menus. CLI shares profiles engine; other menu callers require regression coverage.

Gateway/automatic routing/dynamic catalogs would reduce manual selection but alter account/credential boundaries. Connection checks/model retrieval retired in 1082269; competitor features do not restore user-removed capabilities. Likely regressions are stricter checks rejecting valid symlinks or refresh disrupting forms, covered by target-link cases and preserved opening baselines.

Starting b7f6b1e baseline: engine 441, Tauri 23, frontend 65 files / 606, TypeScript passed. pnpm 11 automatic-install checking failed because it required deleting dependencies; existing app/node_modules/.bin/vitest and tsc were used without reinstalling. These pre-change numbers do not prove improvements; native GUI/real accounts/packages remain unaccepted.

Account-improvement receipt (later desktop/package results below; unpublished):

- Conclusion/level: source, automated regression, and isolated browser-to-Rust CLI integration passed; native installation delivery/user acceptance was not reached. Target was this workspace's changes atop b7f6b1e via local Vite Accounts.
- Full checks: `cargo test --manifest-path engine-rs/Cargo.toml` 443 and `cargo test --manifest-path app/src-tauri/Cargo.toml` 23; app `./node_modules/.bin/vitest run --maxWorkers=1` 65 files / 617; tsc --noEmit, vite build, engine `cargo fmt -- --check` and `git diff --check` passed. A run spanning edits mixed old modules/new tests; discarded and rerun completely on final source without removed tests/weaker assertions.
- Write counterexamples: isolated HOME/SQLite inject audit-commit failure for existing/missing originals crossed with unchanged/re-edited files. Old implementation overwrote newer external content; the same test failed before and passed after. Name/connection parameters share one commit, restoring both and files on audit failure.
- Actual/sensitive values: Claude/Codex rows read actual address/model; prefilling reuses read-only parsing without restoring missing old models. Unsafe credential/header prefilling explains why and disables connection save while retaining rename. Credentials return references only; suspected keys in ordinary fields block prefilling.
- Browser: screenshots/dimension readback at 1000×650/320×740 showed no horizontal overflow; search/refresh/arrows/End/Escape were operated. Hidden menus could not focus before positioning; fixed post-position focus with regression. Reset/stale quota uses fixed-clock tests, not real subscription data.
- Real engine integration: temporary local transport connected browser account queries/edits to Rust CLI; stand-in assistants and HOME/data/secrets used temporary paths. Forms showed actual address/model. External model edits after opening rejected saves, preserving external files/name/main and creating no secret file. Close/refresh/reopen edited successfully, agreeing on file/registered model/name, preserving address/theme, keeping database credential references and external secret 0600. Independent files/`policy:get` confirmed results beyond UI messages.
- Boundary: transport substitution does not prove Tauri IPC/packages. Temporary service allowed account actions only; other-page HTTP 400 responses reflected test scope, not those pages' acceptance, whose regressions rely on full frontend tests. No real login/model probing/subscription/native-window acceptance or packaging/signing/publishing/tag/Release.
- Partial success remains explicit: external secrets and account configuration are not cross-file transactional. If exact preview precedes successful secret save but failed account write, UI says "API Key saved, but account configuration was not updated." Command deletion after registration removal is also reported separately, never complete success.

### Subsequent UI and local-package acceptance

- Adjusted light/dark contrast, form backgrounds, table spacing, and action visibility. Narrow tables scroll horizontally; headings/navigation/dialogs remain readable. Browser checked 13 pages in light/dark/320px layouts and keyboard menus.
- Programs show Upgrade only for an available target version; current/newer-local/unknown/unchecked states are distinct. Shorter default text moves original/manageability to details. Real local programs show Up to date without Upgrade.
- Final frontend 65 files / 619, TypeScript/Vite passed; before push, engine 443/Tauri 23/engine fmt reran successfully.
- As requested, built local macOS x64 app/DMG and opened native UI. Package passed independent isolated HOME/data/minimal PATH execution. Build identity is b7f6b1e plus workspace changes, not a rebuilt later commit SHA.
- Local package unsigned/unnotarized/unpublished; no tag/Release. Real login, network model calls, and subscription quotas unverified.

## 2026-10-09: Magpie comparison follow-up

Scope agreed with the user after a source-level comparison with [yetone/magpie](https://github.com/yetone/magpie) at `62b1c99`: borrow display, reminder, and engineering ideas; do not add account switching, a gateway, credential handling, or warm-up requests. Each slice is committed and pushed after its checks.

| Item | Implementation | Acceptance | State |
| --- | --- | --- | --- |
| Quota display and reminder | Tray/⌘K summaries carry the reset countdown; Accounts sorts by soonest reset or most left (display order only); opt-in Settings reminder sends one system notification per window round when less than a tenth of the window remains and at most half is used; account status rereads local files every 10 minutes in the background | Unit tests for the rule, de-duplication, sorting, and page order; Settings and Accounts checked at 1000 and 760 px in the mock dev server. A real system notification was not observed | `341b4ee` |
| Tray panel | Left click on the tray icon (macOS/Windows) toggles a 380×520 webview panel placed under a menu-bar icon or above a taskbar icon and clamped to the work area; it hides on blur and Escape; rows show launch command, quota bars, reset countdown, and Copy; footer opens the main window or quits. Right click and Linux keep the native menu | Placement unit tests; panel component test (copy writes only the clipboard, only `:status` reads). A debug build with a separate identifier, isolated HOME and data directory ran on macOS: real clicks opened the panel centred under the icon, Copy put `claude` on the clipboard, blur and a second click hid it, Open main window and Quit worked, and the user's installed instance kept running. Windows placement and the right-click menu were not exercised | `09675b5` |
| Recent sessions and cache hit rate | Read-only `usage:sessions` lists each account's ten newest sessions from the first 256 KiB of each file: Claude titles from `custom-title`, then `ai-title`, then the first typed prompt, skipping subagent transcripts; Codex skips `subagent` sources and takes titles from `session_index.jsonl`. Usage shows them with a Copy command `cd <cwd> && <launch> --resume <id>` (Codex `resume <id>`; `cd /d` for drive paths) and adds the cache hit rate, cache read ÷ (input + cache write + cache read) | Engine fixtures for both formats, the per-account cap, and a half-written last line; the engine read the real HOME in 0.09 s with correct titles and directories; helper and page tests (no write commands); mock UI at 1000 and 760 px. Resume commands were not run in a terminal | `750fc6d` |
| MCP test connection | `mcp:check --only mcp:<name> --agent <account>` starts that account's entry once: stdio servers run with the user's login-shell PATH and resolved variables, then `initialize`, `notifications/initialized`, and `tools/list`; Streamable HTTP uses curl with headers passed on stdin and keeps the session id; legacy SSE is refused with a reason. A variable without a value stops before anything runs; resolved values are masked in messages and stderr; the process group is killed afterwards. MCP rows get a Test connection button and dialog (account choice, server, tools, stderr tail) | Engine tests with a fake stdio server (tools, no writes, missing variable never starts the command, secret masked — the masking test was confirmed to fail with masking removed), a fake Streamable HTTP server (session id and resolved Authorization header), and argument errors; in an isolated HOME the real `@modelcontextprotocol/server-everything` via npx returned 10 tools in 14 s with no data directory created and no leftover process; page test and mock UI | `a97f432` |
| Data-destination notices | The API account form says which host prompts and the key go to and which file stores the key in plaintext; share-link import says which host a remote MCP talks to, which command a stdio MCP runs, or where skills download from | Form and dialog tests (host, file, no key value in the notice) | `21a6468` |
| App dropdowns and UI rules | `components/Select` (combobox + listbox in a body portal, keyboard and type-ahead, value on `data-value`) replaces all 13 native selects; `docs/UI-RULES.md` records the layout checklist plus no native select, no coloured left-border stripes, no scroll on click, and data-destination notices; the guard test fails on a native select or a left-border stripe | Select keyboard, outside-click, and dialog Escape tests; the 21 tests that drove native selects now use `chooseOption`/`readOptions`; mock UI: Usage filter list placement, the 421-entry timezone list inside a dialog keeps the dialog open after choosing | `859d897` |
| New skills in a source update | The version preview marks a skill whose `SKILL.md` is added as a new skill ("appears on Skills after switching; you choose which accounts get it") and one whose `SKILL.md` is deleted as removed by that version. Checked first that updating only moves the mirror and links nothing new | Sources preview test with modified, added, and deleted skills | `0dd358c` |
| MCP variable references (checked, no change) | Claude `${NAME}` in env and headers, OpenCode `{env:NAME}`, and Codex `env_http_headers` are already written in each assistant's own syntax. Codex env values are resolved to literals by design because Codex does not expand them; Codex's `env_vars` forwarding would need the variable exported where Codex starts, which launch commands do not guarantee | Source reading of `native_mcp/entry.rs` `render_new` | — |
| Engineering process | `docs/LESSONS.md` (rule, why, evidence; promoted after three days) seeded with today's five lessons; `docs/subsystems/` references in Magpie's format for quota and tray, recent sessions, and MCP test connection; AGENTS.md asks to read both and keep them current | Relative links resolve; every verification command in the references was run and passed | `4910d83` |
| Considered and not changed | Atomic writes stay separate: profile files force 0600 because they hold keys, MCP writes keep the file mode and refuse symlinked targets by the 2026-09-11 fail-closed design, the secrets file writes only to its validated path; OpenCode JSONC with comments already fails closed instead of losing comments. Per-change backup folders were not unified (each module's backup has its own restore path). CC Switch import was not built: it is optional and there is no CC Switch data on this machine to accept it against. Two-line rows and per-assistant folding already exist on Accounts (two-line rows, per-assistant tabs); a default-first model picker has no counterpart here because API accounts have no assistant default to show | Source reading and history (`27ca903`) | — |
| Negative test from Magpie's lessons | Magpie once treated two unreadable folders as equal and deleted a copy. Here every deletion path that compares skill contents treats a failed hash as unknown (adopt undo keeps the directory); a unit test now locks that `hash_skill_dir` errors on an unreadable file or subdirectory instead of returning a comparable fingerprint | `unreadable_content_is_an_error_not_a_comparable_fingerprint` (skips when run as root) | This commit |

## 2026-10-09: External-command cross-platform follow-up

The interrupted command-layer changes now use one launch path for GUI console flags, manual system proxy fallback and timeout cleanup. Windows starts suspended before joining a kill-on-close Job Object; job setup failure stops the child. Explicit command/process proxy variables and bypass settings take precedence. Unsupported bypass/PAC settings are not approximated. See [External commands](subsystems/external-commands.md).

- Passed: engine 462 tests (`--test-threads=2`), Tauri 24 tests, frontend 727 tests (`--maxWorkers=2`), production build, warning-free Windows engine all-target check, and 7 Windows command-layer tests executed in Wine 11.
- Actual operation: the rebuilt engine connected to a Node MCP test assistant under an isolated HOME, read back its tool, and left the application data directory absent. Unix and Wine tests waited for a real grandchild before terminating its group and checked that its delayed marker never appeared.
- Initial concurrent runs failed one mock-CLI status test and one frontend shortcut test. Both targeted reruns and the subsequent full suites passed with lower concurrency; no assertions or timeouts were relaxed.
- Not covered: physical Windows console appearance, real Windows desktop use, and a live macOS/Windows system-proxy network (the available macOS system proxy was off). Wine is process evidence, not physical-desktop acceptance.

## 2026-10-09: Desktop installation-location warning

macOS reads the current executable's mount flags and AppTranslocation path component. Read-only or translocated launches show an installation banner and suppress automatic/manual update checks and installation. Writable external volumes remain allowed. The warning is runtime information, not a saved setting. See [Desktop startup](subsystems/desktop-startup.md).

- Native acceptance: a separate-identifier debug executable with isolated HOME/data and version-only test assistants rendered the warning through Tauri IPC from a controlled AppTranslocation path. A normal development launch had no warning. This controlled path is not Gatekeeper or DMG acceptance.
- Mount tests read the actual read-only system and writable development filesystems; path cases cover writable external volumes. Browser mock settings exercised the banner on all ten main navigation pages at 1000 and 760 px without document overflow; Settings screenshots confirmed disabled updates and readable wrapping.
- Tauri 26 tests, frontend 730 tests and production build passed. Frontend regressions cover the banner and both update-check paths. No installer, signing, updater download/install, tag or Release was produced.

## 2026-10-09: Explicit OS login-launch control

Settings now reads and changes the application's OS login entry on explicit clicks. Opening the page does not register anything, and the state is not copied into SQLite. A small native adapter uses a serialized LaunchAgent on macOS, quoted current-user registry values on Windows, and a quoted Desktop Entry on Linux. Existing lockfile versions are reused; no autostart plugin, Rust toolchain upgrade or helper installation is required. See [Desktop startup](subsystems/desktop-startup.md).

- Native macOS: a separate-identifier debug app with isolated HOME/data and version-only assistants started with no login entry. Clicking On created the expected plist and read back On. An external file edit produced an error and disabled the controls without overwriting it; restoring and rereading recovered, and Off removed the entry. A controlled AppTranslocation launch displayed the warning and disabled On. A disposable launchd job using the same flags started a probe whose child survived its parent's exit; the job was unloaded afterwards.
- Cross-platform checks: the current Windows app passed an all-target compile. A source-module harness with a Tauri facade executed the registry test in a fresh Wine 11 hive: quoted path, Task Manager disable/re-enable, foreign-value preservation and removal. A Linux source-module harness passed three tests, including real `gio launch` of a path with spaces, quotes, `$` and a backslash. An initial percent-path launch failed; unsupported paths now fail before writing and have a regression test.
- Automated coverage checks zero-write reads, on/off readback, external edits, symlinks, write errors, and read-only enable refusal while allowing disable. Tauri 27 tests, frontend 733 tests (70 files), production build, formatting and documentation-link checks passed.
- Limits: physical Windows/Linux Tauri windows and reboot/login were not exercised. The launchd probe verifies process-group survival, not a signed updater download/install. macOS policy can still prevent a registered job from starting. No real assistant login, package, tag or Release was changed or published.

## 2026-10-09: MCP marker recovery and connection diagnostics

MCP rows now distinguish configuration enablement from a manual connection check. Legacy `_MANAGED_BY` warnings have a per-account cleanup preview and confirmation; the existing writer preserves other fields and accounts and rejects an external modification before applying. Check results stay only in page memory, tied to the account and entry digest. Retrying clears the old result and refreshes configuration; changing the account or working directory clears the dialog result. See [MCP checks](subsystems/mcp-check.md) and the [configuration guide](MCP-CONFIG-GUIDE.md).

- Checks use the configured working directory, then an explicitly selected project directory, then HOME. They report initialization and tool-list timeouts separately, failure stage, natural exit code and sanitized stderr. Invalid initialization and failed tool listing no longer report success. A server without tools capability can complete initialization without claiming a tool list.
- Isolated CLI acceptance read back the exact cleanup diff, preserved unrelated entries and fields, rejected a stale digest without creating state, and confirmed the configured project directory from the actual child process. Successful and failed checks left database contents unchanged; a synthetic secret was masked and a natural exit code was returned. These checks used isolated HOME/data and test-server paths, with no inherited assistant credentials.
- Browser acceptance at 1000 and 760 px exercised cleanup preview, cancellation, confirmation, success, failure diagnostics, retry and result invalidation. Screenshots were inspected; no document overflow occurred and dialog footers remained visible. Browser checks used development fixtures; actual writes and protocol exchanges were verified separately through the shared engine CLI and integration tests.
- Final full engine and Tauri suites, 738 frontend tests in 70 files, production build, Rust formatting and documentation links passed. Full engine tests used one test thread and frontend tests used two workers after host contention caused earlier waits to fail; assertions and timeouts were not weakened.
- Limits: actual assistant sessions and a native Tauri window were not launched for this slice. No real MCP configuration, installer, tag or Release was changed or published.

## 2026-10-10: Cross-platform safety regressions

This candidate fixes the eight issues reviewed at `cbe73bf`: resolved account-path
isolation, junction-backed current-version protection, explicit authentication
shell commands, Windows npm prefix/launcher lifecycle, PATH/PATHEXT tool probes,
format-specific DEB updater manifests, cold/runtime import-link delivery, and one
deadline for child exit plus inherited pipe closure. It also fixes the existing
Linux autostart `OsString` to `PathBuf` compilation error exposed by a real Tauri
build. Subsystem contracts are linked from [Subsystem References](subsystems/README.md).

- Passed on Linux: 489 engine tests plus the portable program-lifecycle CLI
  harness; 30 Tauri tests; 749 frontend tests across 72 files; 13 script tests;
  TypeScript checking and production frontend build; engine/Tauri formatting and
  all-target checks; engine release build. Two frontend cases that require native
  Windows `.cmd` execution were skipped by their platform condition.
- Windows GNU all-target checking and compilation/linking of every engine test
  executable passed. This is compile coverage, not Windows execution.
- Actual operations used isolated HOME/data and disposable fixtures: protected
  account/rules bytes stayed unchanged after rejected writes, package lifecycle
  fixtures checked ordinary/scoped npm layouts and rollback readback, process
  regressions measured post-parent-exit deadlines and descendant cleanup, and
  generated authentication commands ran in POSIX and PowerShell 7.5.3 on Linux.
  Negative controls restored the original implementations in separate build
  artifacts and demonstrated the unsafe behavior before these fixes.
- A separate-identifier native Linux debug binary with embedded frontend built
  successfully. Runtime acceptance could not start because the execution
  environment denied Xvfb Unix sockets, including after scoped escalation. Wine
  also failed during its own initialization, before any application test.
- Still unverified: native Windows/macOS execution, installed OS protocol
  registration and single-instance delivery, physical desktop UI, real assistant
  login, and signed DEB/AppImage download/install/restart. Synthetic updater
  signatures verify manifest bindings only. No installer, tag or Release was
  published, and no real account configuration was used.

## 2026-10-10: Strict account isolation and publication decision

The latest user decision supersedes inherited credentials and raw assistant
aliases. Controlled status, launch, login, and logout now use the same registered
account context, absolute assistant executable, clean environment, authentication
mode, endpoint, and account-owned credential references. Paths and identity files
must be unique across registrations; conflicting references and automatic copies
of credential-bearing MCP entries are refused. Legacy unsafe definitions receive
diagnostics without automatic credential migration. The supported contracts and
limits are documented in [Account Isolation](ACCOUNT-ISOLATION.md).

Verification before integration with the newer remote MCP commit:

- The last complete engine run had **563 passed and 14 failed**, plus the portable
  lifecycle harness. It is not an accepted full-suite result. Four alias failures
  used obsolete raw-command fixtures; two audit fixtures lacked the newly required
  explicit source; one failure exposed a real CAS bug caused by the existing
  unused-builtin-skill migration. Those changes have targeted passing evidence:
  aliases 18+1, audit 6, and migration/config/core/skills 68. Other failures included
  CLI version timeouts and two assertions without enough diagnostic output; their
  individual causes were not all established. Original logs were retained.
- The later Windows-fixture follow-up passed Mac agent-settings 27 and transaction
  14, with one transaction failure: the test expected to reach a real fake-CLI probe
  but received "external credential source changed; no write." Its cause remains
  unresolved. The command stopped before the remaining four targets.
- Frontend full rerun passed **751**, with **4 conditional skips**: two PowerShell
  execution cases because the runtime is unavailable and two native Windows `.cmd`
  cases. An earlier run had two timeout-related failures; unchanged targeted cases
  and the subsequent full run passed. Tauri passed **31**, scripts **13**, and
  TypeScript checking passed. These checks preceded the newer remote MCP changes
  and are not a final integrated-snapshot acceptance claim.
- Windows GNU test-executable compilation/linking passed before the last capture,
  CAS, and fixture updates. Later Windows test checking and the alias target check
  passed. Two newest Windows fixture regressions were not compiled or executed.
  No native Windows or Linux desktop acceptance is claimed for this combined change.
- Native Mac interaction created isolated official/API accounts, generated their
  commands, and exercised fake login, logout, and a launch under an interfering
  alias and polluted parent credentials. Disk readback preserved main and the other
  account. All credentials and CLIs were fake; no real upstream login was attempted.
  A 760 px API drawer overflow was found and fixed. The final rebuilt UI width
  recheck and final-snapshot native acceptance were not completed. Clipboard
  delivery readback was not verified.
- Host load exceeded 250 during later runs. Build/test concurrency was limited to
  one without weakening assertions or extending timeouts. Resource contention is
  recorded as a condition, not a proven explanation for every failure.

After seeing the failed full-suite result and outstanding acceptance, the user
explicitly requested publication of the completed cross-platform and isolation
changes to `main`. Publication therefore proceeds without claiming acceptance or
waiting for another full run. The newer remote MCP commit must be preserved; no
force push, branch-protection bypass, PR, tag, release, or billing change is authorized.

## 2026-10-10: Cloud follow-up verification before publication

The follow-up starts from `753ae821` and retains `9af369a`. Notification is
pinned to 2.4.0 in both manifests, with resolved-version CI guards. Statusline
fixtures serialize unusual paths correctly, transaction fixtures synchronize at
the real authentication probe, and the fake npm bootstrap avoids Linux's
inherited-writer `ETXTBSY` without changing production executable dispatch.

Account locks now use one private OS-user runtime namespace across manager data
directories. The bounded legacy bridge preserves existing inodes, verifies the
first upgraded reader exclusively, and retains known old/new path spellings.
Fresh homes, absent optional defaults, unsafe lock ancestors, Windows drive
prefixes, multiple upgraded readers, and failed acquisition are covered. Upgrades
still require stopping all old sessions and older binaries; unknown historical
locations and later old-version starts are outside this protocol. See
[the migration contract](subsystems/account-context.md#upgrading-from-data-directory-locks).

Cloud evidence from input tree `786ea2f2`, before the local publication repairs:

- Two complete Linux engine runs each passed **606 tests, zero failures**, plus
  the portable program-lifecycle harness. Every previously recorded group of 14
  failures passed. A subsequent all-target check and formatting check passed.
- Tauri passed **31 tests**, formatting, and all-target checking. Frontend passed
  **756 tests** across 72 files, with **4 existing platform-conditional skips**;
  type checking and the production frontend build passed. Script tests passed
  **15**, including both JavaScript/Rust plugin-version guards.
- Windows GNU all-target checking passed. Relevant fixture targets had also
  cross-linked; neither result is native Windows execution.
- Actual release-mode Linux **DEB and AppImage bundles were built** using the
  existing user-space system-library tree. The AppImage builder needed its
  documented GTK library-path override for that layout; no system installation
  or permission change was made. Bundling does not establish desktop startup.
- Separate-package negative controls fail with the old cross-data-directory
  locks, old JSON interpolation, and removed transaction CAS guards. The original
  fake npm bootstrap also fails deterministically with a held writable descriptor;
  an inherited-descriptor reproduction confirms the Linux mechanism. The exact
  child responsible for the earlier intermittent failure was not traced.
- Independent source review found and closed fresh-HOME, optional-default,
  runtime-parent replacement, and Windows drive-prefix edge cases. No generated
  binary, credentials, local pnpm policy file, or test data is part of the change.

These are pre-publication local results. Native CI must be checked against the
exact published commit, including all four installer targets. Physical desktop
interaction, real upstream account login, and signed update/restart acceptance
remain distinct from these fake-account tests and package builds. No Release or
tag is authorized by this follow-up.

The Mac publication check found one temporary-path fixture failure: `/tmp` was
still a symlink when the sticky-parent assertion ran. The fixture now uses its
canonical temporary root. Review also found that absolute Windows volume/share
roots skipped ancestor ACL validation. Roots are now checked, permitting only
new-child creation while rejecting replacement, deletion and ACL/owner changes;
private credential checks remain unchanged. After these bounded repairs, the
isolated Mac library run passed **243 tests**, account-transaction integration
passed **15**, and script tests passed **15**, all with zero failures. Formatting
and Windows GNU library checking also passed. Cross-checking does not establish
physical Windows desktop operation.

## 2026-10-10: foreground desktop increment

The [lightweight goal](goal-lightweight.md) supersedes historical monitoring and
tray requirements. Starting from `f10d71d`, the desktop now closes normally,
creates no tray webview, and does not query global Inbox status or run automatic
update checks. Search queries run only while open; quick account search does not
scan Skills or MCP. Controlled account commands and queued cold/warm import links
remain intact. Accounts, Rules, Sources, and dependency programs remain available.
Usage collection and login-entry enabling are still pending retirement.

Acceptance: Tauri passed 27 tests; frontend passed 758 tests across 72 files with
four existing platform skips; TypeScript and production build passed. All eight
retained pages were checked at 1000 and 760 px. A separately identified native
app with isolated HOME/data and version-only assistant fixtures rendered Accounts
through IPC; closing its window ended its process. No real assistant data was used.
The unchanged engine's full check passed 240 library tests and failed eight
subprocess timing/capture tests under heavy concurrent host compilation; integration
targets were not reached. A no-fail-fast rerun is collecting further evidence.
This record does not claim a passing full engine suite or completion of the goal.

The next memory increment streams skill contents through a reused 64 KiB buffer.
Two snapshot tests and all 35 Skills integration tests passed. An isolated
`skills:status` run on the same 256 MiB asset produced identical status data and
fingerprints; peak process RSS fell from 261.2 to 6.1 MiB. The baseline hash source
is unchanged between `9af369a` and `a2141cd`. This is a workload-specific CLI
measurement, not the entire desktop footprint. Other resource bounds and feature
retirements remain in progress.

### 2026-10-10: retire monitoring and OS-login enabling

Usage/cost/session commands fail before data access. Accounts retains configuration,
login, and controlled launch actions, with no quota columns or installation prompt.
The built-in quota script cannot be installed or copied; explicit main-account
cleanup preserves custom status lines. OS-login entries can only be removed after
ownership checks. Existing database/session/script contents are preserved.

Runtime settings: 27 passed; account quota retirement: two passed; seed guard: one
passed; affected frontend: 67 passed; Tauri: 27 passed; production build passed.
All eight retained pages passed layout checks at 1000/760 px. Native isolated
Accounts/Settings IPC was exercised. Full frontend: 757 passed, four existing skips,
one shell-fixture startup timeout. Profiles: 64 passed, two CLI-version fixture
timeouts; the safety guard refused writes. A standalone new exit-0 shell fixture
also timed out without application code. Full-suite acceptance remains unresolved;
no timeout or authentication assertion was weakened.

### 2026-10-10: resource bounds and retired-code cleanup

Completed in-memory execution summaries retain 50 records; caller results remain
unchanged. Batch/domain invalidation, non-overlapping progress polling, active-only
audit polling, and inactive query expiry reduce repeated work. Account selectors
skip CLI version probes; source lists defer per-skill fingerprints. MCP output
and Git blob previews have bounded capture. See [resource bounds](subsystems/resource-bounds.md).

The separate cleanup removes 24 retired source/test files plus notification/tray
dependencies and unreachable installation branches. Legacy recognition, retirement
rejections, account isolation, actual configuration, installed programs, and
ownership/CAS cleanup remain. No destructive migration is included.

Cleanup checks: production build and Tauri's 27 tests passed; updater compatibility,
shell syntax, local links, and all eight retained pages at 1000/760 px passed.
Full frontend: 689 passed, four platform skips, one lazy Settings navigation wait
failure. Explicit dynamic preloading of both lazy modules then passed all 25
Settings tests without changed assertions/timeouts. Isolated CLI wrote MCP, rules,
and commands and read actual files back; stale rule writes were rejected. Full
engine acceptance and final native UI results are still being collected.

### 2026-10-10: lightweight implementation acceptance

The final source at `f9b2c94` passed full Linux/macOS CI, including engine, Tauri,
frontend and production-build checks. Local frontend passed 690 tests with four
platform skips. Isolated native IPC, close-to-exit, actual configuration write
readback and all 16 retained-page layout cases passed. The 65-second browser idle
observation made no additional engine queries.

At that revision, Windows retirement tests passed; the unchanged controlled-command
test failed to locate the OS-user lock directory, as on the pre-change `f10d71d`.
The failed local full engine run and Windows failure were explicit acceptance
limits; the subsequent corrections and acceptance are recorded below. See the
[initial lightweight verification](goal-lightweight.md#final-verification-2026-10-10)
for CI links, counts and workload-specific memory evidence.

### 2026-10-10: lightweight defect follow-up

The follow-up corrects Windows runtime-lock discovery with altered profile
environment, MCP descendants surviving natural parent exit, repeated discovery
after every account resource write, false task-clear success on engine
rejection, and temporary processes surviving normal completion or native desktop
exit. Account/credential guards remain intact. Three native test fixtures now
preserve valid temporary-directory setup, complete the intended MCP protocol
phase, and prevent extra Windows pipe inheritance; their original safety and
cleanup assertions remain, with a stronger return-before-marker assertion.

Source `dcb5aa0` passed full CI and GUI CI: native engine tests/checks on
Windows/Linux/macOS, Tauri/frontend/type/build/script checks, and all four GUI
platform builds. Native Windows passed 229 library tests and all integration
targets. The final byte-identical private macOS library copy passed 241 tests;
the normal local Cargo run passed 583 and failed 13 and remains a failed result.

The [defect record](goal-lightweight.md#defect-follow-up-2026-10-10) distinguishes
native Windows test/check evidence, real isolated CLI cleanup, UI fixtures,
the same-byte local test-binary comparison, and unverified physical desktop/login
acceptance. It does not convert the failed normal local Cargo run into a pass.

### 2026-10-10: configuration target and account review

Sources `80d82ed` and `38b47b3` close actual cross-account skill writes, incorrect
profile skill defaults, and shell rc links that redirected alias writes into
account configuration. Unsafe targets stop before content reads or write plans;
ordinary dotfiles links remain supported. Old-source negative controls and actual
isolated CLI writes/readback accompany the regressions. No real account or login
was changed. See [the account review](ACCOUNT-ISOLATION.md#2026-10-10-configuration-and-account-review)
for exact source/CI evidence, retained local failures and acceptance boundaries.

### 2026-10-12: extension workflow corrections

The current product remains extensions-only; historical account registrations are
immutable target metadata. This correction includes explicit single-assistant MCP
edit defaults, atomic personal skill switching, local-source personal copies,
selected-module definition restore with a stale-preview digest, per-target batch
outcomes and checked failed-only retries, and complete dependency action labels.
Assistant and MCP-secret labels replace account-management wording in active
extension workflows. Skill collection summaries wrap independently of assistant
columns; long dependency versions and actions wrap without overlapping.

Local verification passed 459 engine tests, 27 Tauri tests and 601 frontend tests
(4 platform-specific skips), TypeScript checking and the production frontend build.
The staged batch snapshot separately passed type checking and 85 related tests.
Isolated CLI acceptance copied an assigned local skill, switched only Claude,
preserved Codex's link, restored rules without changing skill definitions, rejected
a stale restore and retired account commands, and read back simulated account
files byte-for-byte unchanged. The skill regression rejects stale versions and
external links and restores the old link and binding after an injected database
failure; old source accepted the stale version and failed the regression.

Browser acceptance checked 14 populated/empty pages, menus and dialogs at each of
1000 and 760 px. Command traces verified single-assistant MCP previews, explicit
multi-assistant selection, cancellation without apply/import writes, mixed batch
outcomes and retries that did not repeat successful targets. Screenshots were
inspected in addition to overflow and button-bound checks. The isolated macOS
debug app starts at the committed 1000 by 650 geometry with a separate identifier.

Native Windows CI at `a21feff` passed actual command-entry unlink and private-file
ACL checks after the MCP-reference fixture correction. Native Windows/Linux/macOS
engine checks also passed at `bb9edd6`. These are revision-specific receipts, not
physical desktop or current-installer acceptance. See [platform status](GUI-PLATFORM-STATUS.md).
Real Windows installation, real arm64 execution, signing/notarization and actual
automatic updates remain unverified. No release, tag or installer was published.

Native populated-preview readback then found that a same-name skill owned by
another source was incorrectly offered as an external-link takeover. Status now
identifies the actual other assignment as read-only `other-source`, and ordinary
add, including takeover, refuses another active assignment. Switching requires
the original-source/version replacement flags. Integration and actual isolated
CLI readback verify ownership and unchanged links; the UI regression verifies
that the misleading takeover/removal controls are absent.

### 2026-10-12: personal skill collection directories

New personal copies default to `~/skills/<collection>/<skill>/SKILL.md` and
support a custom collection directory. Existing local collections retain their
registered paths. Unmanaged-skill adoption previews the complete destination;
changing it requires a fresh digest and preserves the selected assistants.
Account directories, manager data, assistant skill roots, directory aliases,
and overlapping local sources are rejected as personal-copy destinations.
Assistant entries remain compatible one-level skill links; existing content is
not migrated and nested assistant discovery is not assumed.

Local checks passed 462 engine tests, 27 Tauri tests and 604 frontend tests
(four platform-specific skips), plus the production frontend and native debug
builds. Isolated CLI readback verified default and custom directories, custom
directory reuse, single-assistant source switching, stale-directory refusal,
adoption undo, and unchanged simulated account files. All three new integration
cases failed against the previous source for the expected missing behavior.
Browser acceptance inspected 12 dialog states across 1000 and 760 px, including
long paths, read-only preview refresh and cancellation without writes.

The isolated macOS native preview launched at the committed window geometry.
Native click acceptance was blocked by macOS Accessibility access and remains
unverified. Current Windows runtime acceptance is pending CI; prior platform
receipts do not establish acceptance of this revision. No real account files,
registrations, sessions or launch scripts were changed. No release, tag or
installer was published.

### 2026-10-12: source ownership and failure reporting audit

The audit corrections are pushed in `1f8cc7f`, `7a5dc35` and `26db532`.
Skill copies share the apply lock and recheck their destination. Definition edits
and restores refuse to move or remove assigned sources, including disabled
assignments. Duplicate names show both conflicting paths and block deployment.
Missing definitions retain visible assignments that can be removed individually.

Source-query failures show errors instead of an empty inventory. Updates wait
for deployment results and report partial failures or unreadable fresh status.
MCP writes refresh dependency status. Uncalled authentication-command generators
and their retired tests were removed; active extension boundary tests remain.

At `26db532`, local checks passed 466 engine tests with `--test-threads=1`,
27 Tauri tests, 610 frontend tests without skips, 14 repository-script tests,
and production frontend and native debug builds.

Isolated CLI checks compared the candidate with `d7d0737`. The old binary accepted
duplicate-name deployment, removal of an assigned source during restore, and
copying while the apply lock was held; the candidate refused all three. Orphan
cleanup preserved source content and simulated identity/session files.

Browser checks inspected source and skill states at 1000 and 760 px, including
long errors and missing definitions. Native macOS checks inspected empty and
populated Skills/Sources pages at the committed 1000 by 650 geometry, with isolated
HOME/data and a separate identifier. Orphan menus exposed removal only.

A native confirmation removed one orphan skill and preserved the other
assistant's link. This closes the earlier macOS Accessibility verification gap
for these paths. Empty-page queries created no database or source cache.

Default parallel engine runs retain intermittent subprocess capture-timing
failures under local build load. The final serial run passed without changing
assertions or timeouts; the cause of the intermittent failures is unconfirmed.

At final readback, `26db532` passed all seven
[GUI CI jobs](https://github.com/ly87ing/agent-manager/actions/runs/38332619824)
and both [regular CI jobs](https://github.com/ly87ing/agent-manager/actions/runs/38332619815).
This includes three native engine jobs, four installer builds, native-architecture
macOS DMG verification and isolated Linux package launch checks. Windows frontend
tests passed, superseding the retired timeout in
[the prior GUI CI](https://github.com/ly87ing/agent-manager/actions/runs/38327719809).

Interactive Windows and arm64 macOS desktop operation, signing/notarization and
automatic updates remain unverified. No real account files or registrations
were changed, and no Release, tag or installer was published.

### 2026-10-12: capture fixture startup preparation

The capture fixture now completes its existing output mode before short test
deadlines begin. Both streams are suppressed and its expected exit code 7 is
required. Compilation and startup each retain a 60-second preparation limit.
The change is confined to `cfg(test)` setup; production capture code, fixture
modes, deadlines and descendant-cleanup assertions are unchanged.

Before correction, the focused run passed 10 cases and failed two parent-exit
assertions. Tracing observed fixture entry about 2.19 seconds after the test
started; completed I/O preparation reduced entry latency to 8–13 ms. An empty
warmup still failed on repetition and was replaced with the existing output
mode. The underlying OS cause of the cold-start delay remains unconfirmed.

The final setup passed three consecutive 12-case capture runs, including a run
with 12 test threads, and all 466 engine tests under default parallel execution.
Tauri passed 27 tests, frontend passed 610 tests, and the production frontend
build passed. Evidence was collected from actual native fixture processes and
their readiness, parent-exit and delayed-write markers. Native Windows checks
passed at `f67ed1a`, including all 10 Windows capture cases read back from the
job log. All seven [GUI CI jobs](https://github.com/ly87ing/agent-manager/actions/runs/38336759194)
and both [regular CI jobs](https://github.com/ly87ing/agent-manager/actions/runs/38336759120)
passed for that revision. Interactive Windows/arm64 macOS desktop and
automatic-update acceptance remain open. No real account files were changed,
and no Release or installer was published.

### 2026-10-12: English default and Simplified Chinese interface

The desktop interface now defaults to English. Settings → Language switches
between English and Simplified Chinese after an explicit save and retains the
choice across restarts. Language uses the existing application-preference table;
reading missing or legacy preferences remains read-only and defaults to English.
Saving preserves the secrets-file path owned by the MCP secrets page.

The catalogue covers application-owned navigation, pages, forms, search labels,
instructions and existing operation messages. Resource names, user content,
paths and original engine diagnostics remain unchanged. No notification module
was added. Language changes add no account operations or assistant-file writes.

Acceptance passed 466 engine tests, 27 Tauri tests and 621 frontend tests, plus
the production frontend build and native macOS debug compilation. Regressions
cover English defaults with a Chinese browser language, explicit save, failed
save/retry, canceled stale queries, preserved user text, both-language MCP
unsupported-target filtering and unchanged keep-existing wire values.

Browser checks captured all ten page surfaces at 1000 and 760 px in both
languages, with empty and populated skill/source states, plus twenty source,
MCP, rule-template, restore-preview and search-dialog checks. Final captures wait
for loaded content and page animations; no document overflow was found.

Native macOS acceptance used a separate application identifier, isolated HOME
and data directory, and the committed window geometry. It saved Chinese through
the actual UI, read `"zh-CN"` from SQLite, restarted into Chinese, saved English,
read `"en"`, and restarted into English. Three simulated identity/configuration
files remained byte-identical. Initial copied-binary attempts failed Tauri's
resource-path lookup; the accepted run used the actual Cargo debug executable.

Interactive Windows/arm64 macOS operation, signing/notarization and automatic
updates remain outside this acceptance. No real account files were changed and
no Release, tag or installer was published. Downgrading to a pre-language binary
requires removing only the new app preference row as described in
[Interface language](subsystems/interface-language.md).

### 2026-10-12–13: General launch configurations

Settings now offers a general launch builder independent of assistant accounts.
Users select either a graphical application or a terminal command, then add
generic environment variables, literal arguments, optional per-shortcut time
zones and shortcut names. This distinguishes values such as `LANG` and `LC_ALL`
from program arguments such as `--lang=en-US`. Recipes save in Agent Manager
without generating a command, running a process, or changing shell configuration.
Opening a saved recipe makes it editable. Recipes saved before the generic
environment field was added remain readable.

Each saved shortcut retains copy actions for the complete launch command and its
complete POSIX alias or PowerShell function. A saved local graphical application
also has a direct launch action; that click is the explicit launch request and
does not add a second confirmation dialog. Terminal commands remain copy-only so
their normal interactive input and output stay in the user's terminal.
`launch:status`, `launch:preview`, and `launch:apply` were removed after user
feedback established that direct shell-file management was the wrong boundary.
System environment and time-zone settings are never changed, and each
application's handling of injected variables remains unverified until the
application itself is checked.

Local acceptance used 481 full engine tests, 28 Tauri tests and 634 frontend
tests, with no failures, plus the production frontend build. An isolated CLI save
survived a new process and read back both variants, `LANG`, `LC_ALL`, and a
component-specific argument; `.zshrc`, `.zprofile`, `.bashrc`, and a PowerShell
profile remained byte-identical and the target did not run. The launch runtime
regression started an isolated compiled application and read back
`Asia/Singapore`, `LANG`, a custom variable, and literal arguments from the child
process; it kept running after the CLI exited and rejected a repeated launch.
The browser mock at 1000 by 650 and 760 by 650 exercised environment/argument
editing, saving, saved-card copying, and direct launch. Both widths matched their
document scroll widths, the direct launch opened no dialog, and console/page
errors were empty.

macOS rendering and local launch execution were accepted on this host. Windows
PowerShell rendering is covered by source tests, and Linux command rendering is
executed through available local shells. Native Windows/Linux desktop operation
and real applications' displayed time remain unaccepted.

### 2026-10-13 Follow-up: Launch as the default top-level page

- Launch configurations are a frequent-use workflow rather than an advanced
  setting. Launch is now the first top-level navigation item and the page shown
  when Agent Manager opens.
- Saved graphical applications keep direct Launch actions at the top of the
  page; terminal tools keep copy-only commands and definitions. The Settings
  entry and command palette route to the same top-level page.
- Acceptance covers default-page routing, the retained Settings shortcut,
  command-palette routing, and navigation/layout at the committed 1000 × 650
  window and the required 760 px width.

### 2026-10-13 Follow-up: Saved launchers first and explicit login startup

- The default Launch page shows saved applications and terminal commands without
  mounting the editor. New configuration and Open configuration reveal it on
  demand, while saved graphical applications retain their direct Launch actions.
- Settings again exposes explicit Enable and Disable actions for opening Agent
  Manager at OS login. Reading Settings remains zero-write. The backend writes and
  reads back the owned macOS LaunchAgent, Linux Desktop Entry, or Windows current-user
  Run value only after a click, preserving external-change and symlink protection.
- macOS refuses enabling from translocated or read-only locations. Agent Manager
  does not enable startup automatically as part of installation, launch, or this
  change.

### 2026-10-13: Remove retired backend implementations

Retired account writers, authentication probes, controlled command rendering,
assistant installation, runtime settings, shell-alias writers and old database
credential migration are deleted. Immutable extension metadata, path protection,
location locks, clean MCP subprocesses and credential ownership remain. The old
launcher only refuses execution; user files and database contents are untouched.

The backend slice passed the full engine suite and dependency lifecycle harness,
28 Tauri tests, 635 frontend tests, type checking, production frontend build,
Rust formatting and all-target engine checking. Focused isolated CLI scope,
MCP/rules/secrets, dependency and path regressions passed with file readback.
Independent source review found no removed active extension guard. The two
candidate-path unit tests now exercise the retained resource guards directly.
Local links in the four updated architecture references passed checking.

No installed application, user environment, installer, tag or Release was changed.
Native Windows/Linux interaction and desktop update acceptance were not rerun.
Frontend helper, mock, bridge-argument and contract cleanup remains a follow-up.
