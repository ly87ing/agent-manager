# Lightweight desktop goal

The desktop app is a foreground configuration tool. Keep Accounts, MCP, Skills,
Rules, Commands, Sources, dependency programs, and essential settings. Desktop
and CLI continue to share the Rust engine. This scope supersedes the monitoring
and residency requirements in earlier acceptance records.

## Constraints

| Constraint | Source | Consequence |
| --- | --- | --- |
| Keep accounts, rules, and an independent Sources page | User scope clarification | Reduce query work rather than remove these pages |
| Keep binary dependency support | User scope clarification | Keep discovery and explicit installation/upgrade actions |
| Minimize idle and peak resources | User goal | No tray residency, background monitoring, or unbounded buffers/history |
| Actual assistant files remain authoritative | AGENTS.md | Preserve credentials, sessions, external changes, links, and installed programs |
| Push verified increments to main | User authorization | No release, tag, or installer publication |

## Delivery slices

- [x] Foreground lifecycle: close exits, no tray webview, no global Inbox queries,
      no closed-palette polling, no automatic application update check.
- [x] Retire usage/cost/session collection and quota display/reminders. Preserve
      existing assistant files and database data; retired commands fail explicitly.
- [x] Retire enabling OS login entries; retain safe removal of an existing owned
      entry. Do not silently remove externally changed entries.
- [x] Bound execution summaries and refresh only affected domains; keep necessary
      cross-domain invalidation after source, account, and dependency operations.
- [x] Bound core memory: stream skill hashes, cap MCP output, and avoid loading
      full Git blobs for previews that will be rejected by the display limit.
- [x] Keep source lists and dependency discovery on demand; avoid content hashing
      and assistant version probes just to populate selectors.
- [x] Remove unreachable retired implementation separately from behavior changes.
- [x] Update current product/subsystem documentation and acceptance records.

## Acceptance

Run the smallest relevant tests before the engine, Tauri, frontend unit, and
frontend build checks. Use an isolated HOME, data directory, assistant binaries,
and a distinct desktop identifier for native acceptance. Check affected pages at
1000 and 760 px. Exercise core configuration writes and read the actual files back.
Confirm window close ends the test app, idle time starts no background queries,
large inputs and repeated operations remain bounded, and retired collection does
not read sessions. Record measured memory with its workload; do not promise a fixed
MB threshold from source analysis alone.

Rollback uses the previous executable/source revision. No destructive database
migration or user-data cleanup is part of this goal. Existing deployed status-line
scripts and login entries need an explicit, ownership-checked removal path.

## Progress

- Baseline: `9af369a`; clean main. The 65 relevant frontend tests passed before
  the first change. Implementation and acceptance are in progress.
- Foreground slice: tray setup and the secondary webview are disconnected;
  navigation has no global Inbox collector; closed search issues no queries;
  automatic updater and quota reminders are disconnected. Quota collection and
  login-entry enabling remain for the next slice.
- Native acceptance on the combined `f10d71d` tree: an isolated HOME/data and version-only assistants, under a
  distinct desktop identifier, rendered Accounts through Tauri IPC. Clicking its
  close button ended the process. An initial blank dev-server load was not counted
  as acceptance; the server was checked and the instance relaunched successfully.
- Browser fixtures: all eight retained pages at 1000 and 760 px had no document
  overflow; Accounts and Sources screenshots were inspected. This proves layout,
  not native configuration writes.
- Tauri: 27 tests passed on the combined tree; TypeScript and production build
  passed. Navigation passed all six tests; search passed all ten after restoring
  the existing initial-page wait and awaiting the newly deferred Settings page.
  Full frontend acceptance passed: 72 files, 758 tests, four existing
  platform-conditional skips. All eight retained pages passed layout checks at
  both widths. Local documentation links and diff whitespace checks passed.
- The full engine check on this unchanged engine tree passed 240 library tests
  and failed eight subprocess capture/deadline tests; integration targets were
  not reached. Failures included 10-second fixture timeouts, delayed cleanup,
  and descendants that never started. The host was concurrently compiling other
  software with a load average above 200. This does not establish a passing full
  engine suite. A no-fail-fast rerun is collecting the remaining targets; no
  production behavior or test deadline was changed to bypass the failures.
- Before submission, main advanced from `9af369a` to `f10d71d` (nine commits,
  including account isolation and Windows fixes). Local changes were preserved
  in a patch and stash, then main fast-forwarded. All incoming commits are ancestors
  of HEAD and `HEAD..origin/main` is empty. The 127 incoming paths landed intact.
  Five patch conflicts were resolved semantically: keep the controlled launch path
  and queued import-link delivery in native startup; keep validated account commands
  in search while removing tray publishing; combine the navigation test imports;
  keep controlled-command expectations with on-demand data waits; retain both sets
  of lessons. Automatic merges retained Secrets navigation and import-dialog keys.
  Controlled-command queries are also disabled when search closes, including when
  their account metadata is cached. Acceptance is being rerun on this combined tree;
  pre-sync full suites were interrupted and do not establish its acceptance.
- Skill hashing increment: reuse one 64 KiB content buffer. The existing path,
  exclusion, symlink, and digest format is retained. A golden digest spanning
  multiple buffers and an empty file, content mutation, and unreadable-file tests
  passed (two tests); all 35 Skills integration tests passed.
- Isolated CLI measurement: a source with one 256 MiB sparse asset plus SKILL.md
  used 273,883,136 bytes peak RSS before and 6,418,432 after (261.2 vs 6.1 MiB,
  97.7% lower). The baseline executable came from `9af369a`; its hash source is
  identical through `a2141cd`. Full `skills:status` data, including fingerprints,
  matched exactly. This measures that workload, not total desktop/WebKit memory.

- Retirement slice: Usage navigation, quota cells/sorting/installation actions, and
  OS-login enabling are removed. Retired engine commands fail without data access;
  legacy database rows and assistant files are preserved. The built-in quota
  status line cannot be installed or copied. Main-account cleanup remains explicit
  and ownership checked; custom status lines are preserved.
- Retirement checks: 27 runtime-settings tests, two account quota-retirement tests,
  the seed guard test, 67 affected frontend tests, all 27 Tauri tests, and the
  frontend production build passed. The profile suite passed 64/66; two refused
  writes because fixture CLI version checks timed out. Full frontend passed 757
  with four platform skips and one temporary-script spawn timeout. A standalone
  newly written shell script containing only exit 0 also timed out after five
  seconds, without project code. These are recorded failures, not full-suite passes.
- Actual native retirement UI, under isolated HOME/data and a separate identifier,
  rendered Accounts without quota and Settings with only legacy login-entry removal.
  The removal action returned the read-back disabled state. All eight retained
  pages passed 1000/760 px overflow checks; Accounts and Settings pixels were viewed.

- Bounds slice: completed execution history is capped at 50 summaries; running
  work is retained. Domain refresh, one batch refresh, non-overlapping progress,
  active-only audit polling, and 60-second inactive cache expiry are implemented.
  Sources defers per-skill fingerprints; account selectors skip CLI versions.
  MCP stdout and Git preview capture now have explicit retention limits.
- MCP integration: 52 passed; Skills: 35 passed before the new collection-view
  regression. Pure MCP reader guards: three passed; bounded Git blob check: one
  passed. The full frontend check caught incompatible program-step summaries;
  it was corrected and 22 execution/program/audit tests then passed. No assertion
  was weakened. The remaining shell-fixture startup failures are tracked separately.
- Actual isolated CLI acceptance: metadata status returned both main assistants
  without adding any fake-assistant probe-log entry. A source with a 256 MiB asset
  returned only its collection row; all source metadata matched full status after
  excluding the deliberately deferred per-skill health field.
- The collection-view regression passed after correcting its fixture policy key
  to skills.repositories; metadata status passed and preserved database bytes.
  Tauri's 27 tests and the production build passed again. Sources/Accounts and all
  retained pages passed 1000/760 layout checks after the query changes.
- The corrected full frontend rerun passed 761 tests with four platform skips and
  one Settings navigation timing failure. The assertion remains intact; navigation
  now supplies the initial account-status fixtures, preloads the lazy Accounts
  module as the navigation suite does, and waits for that page. An empty route map
  had rendered an account read error rather than that page. All 25 Settings tests
  passed after this correction.

- Cleanup: removed 24 retired source/test files, the global collector hook, quota
  installation helpers, unreachable startup-enable branches, and production
  status-line deployment. Frozen status-line bytes remain for legacy recognition
  and tests. Notification dependencies and tray-only features are removed; the
  active dependency tree has no tray-icon consumer. Retirement and ownership/CAS
  guards remain. No database, real assistant file, or installed program was removed.
- Cleanup checks: production build, all 27 Tauri tests, updater lockfile guard,
  shell syntax, documentation links, and all 16 layout cases passed. Full frontend
  at two workers passed 689 tests with four platform skips and one lazy Settings
  navigation wait failure; preloading both navigation modules as the navigation
  suite does then passed all 25 Settings tests, without changing assertions or
  timeouts. Full engine acceptance is running with two build/test workers after
  the host reached a load average near 489; the initial overloaded runs were
  stopped, not counted as passes.
- Actual CLI acceptance on the cleanup tree wrote MCP, rules, and a command to
  isolated assistant/shell files and read them back. A stale rule write was rejected
  with the file preserved; dependency discovery succeeded and usage returned
  FEATURE_RETIRED. Native UI acceptance and the final full-suite results follow.
