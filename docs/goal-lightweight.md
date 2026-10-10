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
- [ ] Retire usage/cost/session collection and quota display/reminders. Preserve
      existing assistant files and database data; retired commands fail explicitly.
- [ ] Retire enabling OS login entries; retain safe removal of an existing owned
      entry. Do not silently remove externally changed entries.
- [ ] Bound execution summaries and refresh only affected domains; keep necessary
      cross-domain invalidation after source, account, and dependency operations.
- [ ] Bound core memory: stream skill hashes, cap MCP output, and avoid loading
      full Git blobs for previews that will be rejected by the display limit.
- [ ] Keep source lists and dependency discovery on demand; avoid content hashing
      and assistant version probes just to populate selectors.
- [ ] Remove unreachable retired implementation separately from behavior changes.
- [ ] Update current product/subsystem documentation and acceptance records.

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
