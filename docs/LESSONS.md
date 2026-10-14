# Lessons from Merged Work

What recent work got wrong, and the rule that would have caught it. Read this before changing code.

Each lesson is a rule, then why, then the evidence (commit, test, or observation). "Seen" counts the separate days a review found it. A lesson seen on three or more days moves into [AGENTS.md](../AGENTS.md), [UI rules](UI-RULES.md), or a guard test, and its entry here shrinks to a pointer.

Add a lesson when a fix had to be redone, a check passed without exercising the change, or an acceptance step was skipped and later mattered. Review the day's commits for these before writing the day's record in [goal-rust-gui.md](goal-rust-gui.md).

## Tests that prove something

**Audit notification ownership across complete user operations.**
- A shared renderer cannot prevent one workflow from announcing both step success
  and a final result. Inspect execution helpers and their callers together; one
  multi-step operation owns one completion summary while preserving row records.
- Test backend success with warnings, a failed later preference write and unknown
  version checks. These must not announce complete success. Keep the original
  diagnostic in the description when translating the cause or recovery advice.
- Clear notification state between tests, use the actual themed renderer, and
  exercise long stacked messages in both languages and window widths. When closing
  a filtered list of notifications, re-read it after each close; indexes shift.
- Seen 1× (2026-10-14; application-wide feedback audit).

**Open launch editors independently of the saved-list length.**
- Keeping controls sticky inside a page-end editor still leaves the form below
  the viewport when editing the first saved item. Use a modal with an internally
  scrolling body and fixed action footer; verify the triggering viewport,
  unchanged list position, cancellation and focus restoration at both widths.
  Trigger a save failure while checking the footer: a bottom notification can
  still cover fixed buttons, so editor notifications belong at the top.
- Seen 1× (2026-10-13; compact launch-list and modal-editor correction).

**Exercise launch failures from the triggering viewport.**
- A page-end alert can exist in tests while remaining off screen after a launch
  click. Require an immediate dismissible toast for launch/copy failures; verify
  dismissal and expiry remove the message without changing row height or leaving
  a persistent error line. Form failures still need details beside editor controls.
- Verify saved and preview actions at both window widths without scrolling after
  the click, preserve drafts on failure, and test retry without automatic relaunch.
  Test clipboard recovery so an earlier error does not outlive a new action.
- Seen 1× (2026-10-13; placement and stale error rows reported in the preview).

**Verify environment completion feedback from the triggering viewport.**
- Persistent report counts alone do not prove that a user immediately sees the
  conclusion. Check a completion toast without scrolling, and name differences
  and unknown target checks in both the toast and persistent summary.
- Matching selected criteria does not establish complete evidence. Disclose
  untargeted unknowns separately, and never call inventory-only results or an
  unconfirmed clock correction successful. Bound long notification lists while
  retaining full details on the page.
- Seen 1× (2026-10-13; environment completion feedback correction).

**Keep remediation tied to the evidence and process it can actually change.**
- A raw locale conflict can coexist with matching effective categories. Explain
  that distinction inline, and hand expected values to an unsaved launch draft;
  changing the checker process would only hide the original evidence.
- Unknown automatic-time state needs an explicit read-only authorization path,
  separate from clock correction. Cancellation must keep the previous report.
- Frontend unit tests transpile test code without checking its TypeScript API
  options. Run the production type/build gate after adding navigation coverage.
- Seen 1× (2026-10-13; environment remediation and build acceptance).

**Verify saved criteria and authorization controls through actual UI state.**
- The generic local-write badge says that configuration files change. A saved
  expected-value collection changes only Agent Manager's database. Supply an
  explicit criteria-only badge for save/delete dialogs so users can distinguish
  comparison setup from system changes.
- A footer inside the viewport can still be covered by a transient toast from
  Settings. Wait for that toast to clear before visual acceptance, then inspect
  the actual dialog and fixed footer in both supported window widths.
- Accessibility value-setting can update a WebKit input without updating React
  state. A later render then resets the field. Type into the confirmed focused
  input and verify the value after another render and in persisted readback.
- Seen 1× (2026-10-13; collection dialog and native persistence acceptance).

**Check explicit locale conflicts separately from effective locale precedence.**
- A matching `LC_ALL` can hide different `LANG` or category values. Environment
  checks now show optional strict raw-locale comparison alongside effective
  categories; the regression proves matching effective values and conflicting
  explicit values can coexist, while unset variables remain valid.
- Seen 1× (2026-10-13; environment-check acceptance review).

**Run the development mock through each affected page after changing its data shape.**
- Removing retired account fields from the mock also removed `agents` arrays nested
  inside skill discovery groups. The TypeScript build and 561 frontend unit tests
  passed, but opening Sources in the browser failed at `skill.agents` iteration.
  Keep nested skill assignments when removing top-level assistant discovery;
  verify the mock contract and render the affected page at both target widths.
- Seen 1× (2026-10-13; browser mock check after frontend retirement).

**Saving a launch configuration must be tested independently of applying it.**
- The first launch builder retained generated commands only in page memory and
  persisted nothing unless the user wrote local shortcuts. Separate copy and
  apply buttons did not prove that a configuration could be saved without
  applying it. Exercise save, process restart, reload, edit and deletion, while
  reading back unchanged shell files and checking that no application ran.
- Seen 1× (2026-10-12; user correction after `596a88e`).

**Cross-platform launch fixtures must select their shell and avoid piped CLI output.**
- The Linux shortcut test expected `.zshrc` after status used the new Linux default (Bash). Specify zsh in every fixture operation that reads or writes that file.
- The Windows launch test used `Command::output()` and reached its 15-second application deadline before checking whether the application was still running. Redirect CLI JSON to an isolated file and read it after the CLI exits, without waiting for pipe EOF from an independent child.
- Seen 1× (2026-10-12; CI for `0cb1fce`).

**Reject unsafe backup metadata before acquiring a stateful write lock.**
- The first assigned-source restore guard acquired the apply lock immediately
  after parsing. The existing foreign-skill-target regression caught data-directory
  creation for a rejected account import. Account, module and payload validation
  now precede lock acquisition; current bindings are still checked inside the
  write transaction. Retain both no-write rejection and active-source restore cases.
- Seen 1× (2026-10-12).

**Check the actual command entry after success on each platform.**
- Dependency unlink returned success while leaving the Windows `.cmd` entry in
  place: its plan used the extension-free inventory name instead of the same
  platform filename as link. Keep the portable CLI link/unlink readback and
  account identity assertions; a successful envelope alone missed the defect.

**Validate every dependency-program mutation, including backup and internal paths.**
- Checking only a chosen entry directory missed historical cleanup links, backup destinations inside an account, and a `programs/bin` alias into an account. Isolated CLI regressions reproduced each path; plans and execution now share checks for every filesystem mutation step.
- A first version of the step guard rejected legitimate upgrades after moving a package made its launcher link temporarily dangling. Check the link location and destination separately, and retain the existing ordinary/scoped npm upgrade, rollback, and adoption acceptance. Account identity files still reject links.
- Directory ownership alone does not protect arbitrary session files with hard-link aliases. A fake installer linked its metadata to an isolated account session file; the later metadata write overwrote that session. Metadata and backup-description writes now reject linked files before writing, including after installation.
- Planned steps do not cover optional cleanup. A backup-root alias let pruning remove an isolated account's matching directory after an ordinary link action. Backup listing, pruning, and failure rollback now check their own targets; retain original backups when a changed path makes rollback unsafe.
- Program entry arguments use the displayed PATH candidate (for example `~/.local/bin`). An absolute spelling in a fixture failed at candidate selection and did not exercise account isolation. Confirm the expected failure reason and a working ordinary-entry control before accepting a negative result.
- Keep baseline and candidate Cargo output directories separate. Sharing the executable output left the archived baseline binary in the candidate path; the later race probe ran that old binary. Rebuild the package and verify the executable before counting candidate results.
- Seen 1× (2026-10-12).

**Prepare and warm lightweight native fixtures before short process deadlines.**
- The 2026-10-12 unchanged engine binary failed 6 capture tests at its Cargo output path; a SHA-256-identical copy passed all 12 capture tests from a private temporary directory. Several failures occurred before the descendant readiness marker, so they did not establish a cleanup defect. The OS mechanism behind that path-dependent startup delay remains unverified.
- Compile the small std-only capture fixture before the timed region, following the existing native CLI fixture pattern. Keep the same deadlines, readiness and parent-exit markers, delayed-write checks, output limits, and Windows handle-inheritance protection. Do not enlarge production timeouts or remove valid assertions to accommodate test-program startup.
- Compilation alone did not finish preparation: a fresh fixture entered `main`
  about 2.19 seconds after the timed test started during concurrent first launches.
  Tracing reproduced missing parent-exit markers; a completed I/O warmup before the
  timer reduced entry latency to 8–13 ms and passed the same 12 cases. Both compile
  and warmup stages have a 60-second preparation limit. An empty warmup still
  failed on repetition; use the existing output fixture to exercise I/O too.
  The underlying OS cause of the cold-start delay remains unverified.
- A filtered command that reports zero tests is not validation. Check the executed count as well as the exit status.
- Seen 1× (2026-10-12).

**Resolving an ordinary file does not establish its owner.**
- The rc-file symlink compatibility path accepted `.zshrc` pointing at main Codex's `config.toml`; an actual isolated CLI alias write appended shell content to that configuration. Retain ordinary dotfiles links, but check the resolved target against all account directories, known identity files, registered MCP targets and manager data before reading or planning a write.
- Cover external identity links, hard links, disabled/malformed registrations, stale baselines and Windows command-directory junctions. The prior ordinary-dotfile compatibility test remains necessary and passes alongside the rejection cases.
- Seen 1× (2026-10-10).

**Account skill fixtures must distinguish registration IDs from directory names.**
- Skill target fallback used the registration ID rather than the account's configuration directory. A first fixture accidentally used matching names, so its fallback case passed before the fix. The regression now uses different names for both assistant types and reads the actual installed files, while checking that the accidental directories stay absent.
- Saving a `skills_dir` override also allowed an independent account to write inside another account. Cover registration, import, historical fallback, legacy writes, directory aliases and cleanup through the real engine entry point, rather than checking only equal skill target directories.
- Keep source-rule discovery independent of skill destinations. The first guard made an unsafe account's directory hide the Rules status result; the existing blocked-target regression caught this, and source discovery now loads only collection metadata.
- Validate canonical destinations without changing recorded display paths. Returning canonical skill paths broke adoption undo for a trusted HOME alias; the existing adoption-and-undo regression caught the mismatch with its journal.
- Seen 1× (2026-10-10).

**Do not kernel-execute a newly written shell fixture to bootstrap another test.**
- A concurrent fork can retain a writable descriptor until exec, so Linux rejects an immediate script launch with `ETXTBSY` even after the writer in the parent has closed. The fake npm bootstrap now reads the exact script through `/bin/sh`; real product discovery, installation, and rollback still execute the fixture normally.
- A held-writer regression reproduces the old helper failure deterministically and checks the resulting package, symlink, script bytes, and executable permission after the fix.
- Seen 1× (2026-10-10).

**Read-only probes need runtime locks without initializing application data.**
- Moving account locks into the manager data directory made successful MCP connection checks create that directory. Keep runtime locks in a private OS-user namespace and verify both the read-only filesystem boundary and real cross-process exclusion across different data directories. Exercise missing HOME as well as missing data directories, so a safety check does not reject valid first-use writes.
- Moving the lock name alone also loses old-version sessions. Test existing legacy shared locks, first-reader migration rejection, parallel upgraded readers, all known old/new aliases, and release after a failed bridge. Arbitrary unknown old data directories remain an explicit stop-old-programs upgrade prerequisite.
- Seen 1× (2026-10-10).

**Serialize test configuration and synchronize the phase being exercised.**
- Windows path interpolation made a statusline fixture invalid JSON despite successful cross-linking. The regression now forces backslash, space, and apostrophe paths on every host and serializes its JSON.
- A 500 ms transaction fixture could reject before its plan. Ready/release signaling at the real authentication probe plus exact CAS/audit diagnostics now proves the intended race. Old JSON interpolation and removed CAS guards fail separately named negative-control packages.
- Seen 1× (2026-10-10).

**Cross-linking does not prove a Windows fixture satisfies the runtime contract.**
- The new account preflight requires a native assistant executable and private credential-source ACLs. Existing Windows fixtures still installed `.cmd` helpers and relied on inherited temporary-directory permissions; successful compilation could not reveal those runtime mismatches. The fixtures now compile a std-only fake `.exe` and protect only new temporary roots with inheritable current-user ACLs, including atomic credential replacement.
- Keep explicit native-executable, private-source rotation, and broad-ACL rejection cases. Cross-compiled test executables remain build evidence until those cases run on Windows.
- Seen 1× (2026-10-10).

**Policy CAS must distinguish its own known migration from external changes.**
- `open_write` retires unused historical builtin skill definitions before a command's transaction. Comparing the resulting full policy with the raw preflight snapshot rejected legitimate `policy:set`, `policy:delete`, and `config:import` operations. The CAS now uses the same read-only retirement projection as the migration, while every account, source, and other user policy field must still match exactly.
- Keep the legacy retirement integration test, and test that account/source changes and adopted collections are still rejected without writes.
- Seen 1× (2026-10-10).

**A process timeout test must cover pipe EOF after the parent exits.**
- Killing a still-running parent did not exercise the unbounded reader joins or post-exit PATH reads. The program-capture regressions now keep stdout or stderr inherited by a descendant, assert one shared parent/reader deadline, and check that no delayed descendant marker is written. Restoring the original function bodies fails these regressions; the unchanged output/exit-status test still passes.
- Check every capture caller. Version/auth/upstream probes still replaced unfinished pipe reads with empty buffers after a separate two-second wait. Their Rust pipe-holder regression failed with `Some((0, [], []))` before these callers adopted the same shared deadline and cleanup.
- Seen 1× (2026-10-10).

**Startup snapshots must not replace newer runtime events.**
- A first cold-link fix deduplicated delivery IDs but still replayed an older initial snapshot after a newer runtime event, replacing the import dialog's latest URL. The regression now delays the startup read, delivers a newer event first, and verifies that the stale snapshot cannot overwrite it.
- Seen 1× (2026-10-10).

**Negative controls need distinct build artifacts.**
- A before-fix timeout check in a temporary checkout initially reused the shared Cargo target's same-package artifact. Renaming the temporary package forced the original function bodies to compile; six new behavior regressions then failed for the intended reasons. A copied source tree alone is not proof that Cargo executed that copy.
- Seen 1× (2026-10-10).

**Prepare lazy modules before timing navigation; await data loaded on demand.**
- Removing global prefetch made navigation tests time the first Vite module transform,
  and the palette test assumed resource results existed before its query completed.
  Preloading modules in test setup and waiting for the actual result kept the
  functional assertions. Preserve the existing 10-second initial-page wait; allow
  the same loading wait for a newly deferred Settings page under host contention.
  Per-test interaction deadlines remain unchanged.
- Native acceptance initially opened a blank page while the development server was
  unresponsive. Relaunching after checking the server rendered actual Tauri data;
  a blank window alone is not UI acceptance. Ignore Rust target output in Vite HMR.
- Seen 1× (2026-10-10).

**A test for masking must use a value the generic sanitizer does not already mask.**
- `mcp:check` masks resolved variable values in its output. The first test used `sk-am-check-…`; it still passed with the masking removed, because `sanitize_diagnostic_message` already masks `sk-` keys. With a plain value (`plainvalue-qq83hz-moonlight`) it failed without the masking and passed with it (`a97f432`).
- Before trusting a new test, put the old code back and see it fail for the reason it names.
- Seen 1× (2026-10-09).

**Fixtures that sort by time must sit in the past.**
- The recent-sessions cap test set file times from `1_800_000_000` (2027). A file written during the test was older than those, so "newest first" picked the wrong one (`750fc6d`, fixed before commit).
- Seen 1× (2026-10-09).

**A check chained before a commit must fail the chain when a test fails.**
- `cargo test … | grep 'test result' | awk '{…sum…}' && git commit && git push` printed "1 failed" and still committed and pushed `cdd9369`, because `awk` exited 0. The failure was real: the new Codex standalone rule matched the developer's own `~/.codex` while the test used a temporary HOME, so the rule was narrowed to the account's home in the next commit.
- End such a summary with `exit f>0` (or run the tests without a pipe) and use `set -o pipefail`.
- Seen 1× (2026-10-09).

## Messages that say what happened

**Cargo accepts one test-name filter; an invalid command is not a passing test.**
- The interrupted cross-platform follow-up ran `cargo test ... codex_standalone unreadable_content`, then filtered away Cargo's argument error. Run each filter separately or run the full suite, retain the original exit status, and read the result after the final edit.
- Seen 1× (2026-10-09).

**Tell "the program exited" apart from "it timed out".**
- `mcp:check` first reported "no reply to initialize within 30 seconds" for a server that exited after 0.5 s. The wait now returns why it stopped, and the test asserts the exit message (`a97f432`).
- Seen 1× (2026-10-09).

**A stage-specific subprocess test must reach that protocol stage.**
- Linux `2d1fcd4` correctly reported an immediate MCP startup exit, but its secret-redaction fixture expected an initialization exit. The fixture exited before reading the request, so the scheduler chose the observed stage.
- Make the fake server read initialization before exiting. Retain the exact failure message, natural exit code and secret-redaction assertions; also verify the initialization stage and unchanged configuration.
- Seen 1× (2026-10-10).

**Capture the server's exit status before starting cleanup.**
- Closing stdin can make an unresponsive MCP server exit normally or return a new error. Capture its status before closing stdin; otherwise a timeout can acquire an unrelated exit code. Cover both forced termination and EOF-driven shutdown in the same failure-stage test.
- Seen 1× (2026-10-09).

## Dependencies and lockfiles

**Separate Rust and frontend passes do not prove a Tauri installer can be built.**
- Rust notification 2.4.0 and JavaScript notification 2.5.1 passed their own test suites, but Tauri rejected all four installer targets before bundling. Pin both notification dependencies to the same tested version and check the resolved plugin major/minor versions in CI.
- Keep installer construction and native package startup separate from compile evidence. The lockfile regression must fail when the original mismatched JavaScript version is restored.
- Seen 1× (2026-10-10).

**Pin a dependency to a known-compatible version in one step; do not probe versions in a loop.**
- `tauri-plugin-notification` pulled `notify-rust` 4.18.2, which needs rustc 1.89 (local toolchain 1.88). Trying versions with repeated `cargo update --precise` rewrote unrelated Windows dependency edges in `Cargo.lock`. The lockfile was restored from `HEAD` and only the plugin plus `notify-rust` 4.17.0 were added (`341b4ee`).
- Check `git diff --stat Cargo.lock` and the changed `version =` lines before committing a dependency change.
- Seen 1× (2026-10-09).

## Acceptance on the real desktop

**Open secondary actions when checking page copy and layout.**
- Previous Sources acceptance inspected the empty page without opening its recommendation menu. The user then found informal labels and literal ellipses in the action text.
- Inspect open menus and their confirmation dialogs at both required widths. Check complete action labels, text bounds, and long paths; a visible page or a passing interaction test alone does not prove these details are readable.
- Seen 1× (2026-10-12).

**Use committed window geometry for the preview users review.**
- The tips preview overrode the committed 1000 × 650 window with 1000 × 800. Earlier browser captures used a tall viewport, so the checks did not establish the default-window proportions. The user reported the excessive space in the empty Skills page.
- Inherit the committed window settings in isolated native previews; change only the identifier and title. Inspect empty and populated pages at the normal height, plus the required narrow browser width. Keep user-resized window memory intact.
- Seen 1× (2026-10-12).

**Temporary-path fixtures must reach the intended ancestor permission check.**
- The runtime sticky-parent test used macOS's `/tmp` symlink and failed before checking its disposable parent's mode. Canonicalize the fixture root; retain the rejection of untrusted ancestor links in production.
- Absolute Windows volume/share roots need ACL checks too. Skip only drive-relative prefixes; permitting new root children must not permit replacing existing children or changing their ACL/owner.
- Seen 1× (2026-10-10).

**Native Windows CI must diagnose ancestor ACL failures before loosening checks.**
- The first native run passed 214 library tests and rejected 14 lock users at ancestor validation. Generic errors hid whether the runtime or fixture parent failed. Preserve the static ACL rejection cause and report only permission categories for both parent chains.
- Use a fresh protected test directory under the OS user profile for Windows TEMP/TMP. Leave existing directory permissions unchanged; retain unsafe-parent and private-credential rejection assertions. Native CI must confirm the cause and rerun the full suite.
- Seen 1× (2026-10-10).

**Windows system ancestors may be owned by TrustedInstaller.**
- Moving the fixture under the user profile did not repair the 14 lock failures. Native diagnostics on `ab28ed7` identified the same system-root owner as the exact Windows Modules Installer SID for both runtime and original TEMP chains; rejection happened before ACL grants were inspected. The untrusted effective grants only allowed creation/read/traversal.
- Accept that exact SID only as an ancestor owner. Keep private credential/lock owners, ACL grantees, and rejection of deletion/ACL/owner changes strict. Test private-file rejection, nearby service SIDs, and dangerous grants, then rerun the native Windows suite.
- Seen 1× (2026-10-10).

**Legacy-lock fixtures need explicit private Windows ACLs too.**
- After the system-owner repair, Windows passed 228 library tests but the known-default legacy-lock test stopped at the private-entry check before testing old-session exclusion. Its fixture created the lock directory recursively with inherited permissions.
- Create the disposable legacy-lock directory with the same atomic protected-ACL helper used for the runtime directory. Read back both directory and file ACLs before acquiring the simulated old session. Keep production rejection and the expected upgrade-restart assertion unchanged.
- Seen 1× (2026-10-10).

**Rust file access does not prove raw Windows ACL APIs support the same path.**
- Explicit private fixture readback on `ad6ef6c` rejected the longer desktop-default lock file while its directory and file creation succeeded. The ordinary desktop-default filename reaches 261 UTF-16 characters under this CI fixture; the shorter CLI-default filename reaches 243. The failure was ACL lookup, not a broad grant.
- Pass standard-library canonical Windows paths, including the verbatim prefix, to ACL lookup and atomic private-directory creation. Keep the ACL rules unchanged and exercise a real private file beyond MAX_PATH on native Windows.
- Seen 1× (2026-10-10).

**Wall-clock timestamps alone do not identify concurrent probe directories.**
- The macOS `ad6ef6c` adoption regression passed the library suite but failed a later version probe with `File exists`. Probe directories used only the process ID and wall-clock nanoseconds, which need not be unique across calls.
- Add a process-wide atomic sequence and retry only exclusive-creation collisions. Authentication and version probes share private atomic directory creation and RAII cleanup. Force the same timestamp in a regression, keep both directories alive, and verify that cleaning one leaves the other unchanged.
- Seen 1× (2026-10-10).

**General UUIDs also need native Windows randomness.**
- Windows `3c95a07` passed 231 library tests, then two policy fixtures encountered changed registration snapshots. Those parallel fixtures used `uuid_v4`, which always fell back from `/dev/urandom` on Windows. Its timestamp-only bytes discarded the PID and repeated for equal clock values, so fixtures could share one database.
- Use the existing OS-random UUID path on Windows too. The general-purpose fallback includes a process-wide sequence and PID; account generations retain OS-only randomness and fail closed. Force equal-clock fallback calls, run concurrent UUID generation, and make policy fixtures create their roots exclusively. Keep the snapshot-conflict and ownership assertions unchanged.
- Seen 1× (2026-10-10).

**Credential provenance must not change when its directory is created.**
- Windows `2d1fcd4` passed all policy tests, then three transaction checks rejected the account's own claim as retired. The claim hashed an ordinary missing path before creation, but a native verbatim canonical path afterwards.
- Resolve existing ancestors with the same native canonical representation before appending absent components. Prove directory/file creation preserves that identity and a distinct path stays distinct. Retain generation and account checks; mismatched historical claims are not rewritten. Legacy-session fixtures must use the historical lock's display-path canonicalization when hashing its name.
- Seen 1× (2026-10-10).

**Launch the generated OS entry; a correct-looking string is not startup evidence.**
- The first Linux autostart entry escaped a literal `%` as `%%` according to the Desktop Entry syntax. Real `gio launch` refused it because GIO checks whether the executable exists before expanding field codes. File readback alone had passed. Such paths now fail before writing; a disposable Linux run executed the exact path containing spaces, quotes, `$`, and a backslash.
- Seen 1× (2026-10-09).

**Canonicalize a temporary native executable's launch path on macOS.**
- Launching the warning fixture through `/var/...` produced Tauri's `unknown path`, since `/var` is a symlink and Tauri rejects symlinks in its cached executable path. Launching the same fixture through `/private/var/...` with its resource directory present exercised the native warning. Do not weaken Tauri's protection to repair a test fixture.
- Seen 1× (2026-10-09).

**A development build next to the installed app needs its own identifier.**
- The single-instance plugin hands a second launch to the running instance, so a debug build with the default identifier would only have focused the user's installed app. The tray panel was accepted with `TAURI_CONFIG='{"identifier":"io.github.ly87ing.agent-manager.devcheck"}'`, an isolated `HOME` and `AGENT_MANAGER_DATA_DIR`, and the installed instance kept running (`09675b5`).
- Seen 1× (2026-10-09).

**Status and actual startup must use the same explicit account context.**
- Cleaning a status probe while a launch command inherits its shell's tokens, endpoint, or `claude` alias can display one identity and start another. Resolve the executable, clear the child environment, and validate the same directory, authentication mode, endpoint, and owned references for status, launch, login, and logout.
- Run the exact generated command under an interfering shell alias and fake parent credentials; read back the fake child's environment and main/A/B files. A string assertion alone cannot prove isolation.
- Seen 1× (2026-10-10).

**Deleting a reference must not transfer credential ownership.**
- Rendering a referenced MCP credential into its native file can erase the variable name from that file. Removing an account or an MCP entry can erase its remaining ownership evidence. Preserve variable names and account registration bindings in the same transaction as the change; never persist the values.
- Include metadata-only registration, native MCP removal, disabled snapshots, and account recreation in both rejection and legitimate account-owned rotation tests.
- Seen 1× (2026-10-10).

**A rejection test must reach the security check it names.**
- A missing generated-command binding or an unsupported argument can reject a request before its intended alias or account test runs. Build fixtures through the actual command interface and assert the relevant diagnostic. Compare critical tests with the previous source using the same fixtures.
- Seen 1× (2026-10-10).

### 2026-10-10: execution summaries must respect domain result schemas

- Rule: enumerate result shapes before retaining a shared execution summary; copy
  only fields with the schema the list can render, while returning the full result
  unchanged to the caller.
- Why: resource apply steps have name/ok/exit_code, while program operations return
  their own plan steps. Treating them alike threw during summary creation and stopped
  the caller's post-install refresh.
- Evidence: the first focused summary/Skills/MCP checks passed without exercising
  Programs. Full frontend checks caught the failure; a program-step regression and
  the existing program-upgrade/refresh test cover the correction.
- Run TypeScript checking after the final test-fixture edit as well: a passing
  runtime assertion does not validate Testing Library option types. Exact role
  names use an anchored name pattern rather than an unsupported exact option.

### 2026-10-10: inspect every full-suite failure and derive platform fixtures

- Focused runtime integration checks did not exercise an old unit helper that still
  requested installation of the retired status line. Inspect every failure name
  and diagnostic before attributing a mixed suite result to host startup delays.
- Migrate obsolete assertions to the confirmed contract: reject installation,
  preserve unselected/custom configuration, and explicitly clean owned old entries.
- Derive ownership fixture paths through the production path helper. A hardcoded
  Unix separator tested a different command on Windows and missed the intended
  ownership check. Keep its skip/custom-copy assertions intact.

### 2026-10-10: MCP cleanup must include descendants after parent exit

- A successful protocol session does not prove process cleanup. The old finish
  path skipped process-group termination after a natural parent exit, leaving a
  background process and inherited reader pipes alive on Unix.
- The isolated CLI reproduced a delayed descendant write after success. The new
  integration regression confirms the descendant starts, preserves configuration,
  and checks that it cannot write after the check returns. Keep the natural exit
  code observed before shutdown while terminating the remaining group.
- The first fixture exited before consuming `notifications/initialized`, so faster
  Linux/macOS CI correctly rejected the handshake before reaching cleanup. Wait
  for that actual protocol phase; keep the success and descendant checks unchanged.

### 2026-10-10: command cleanup must cover desktop exit and normal completion

- Tauri exits with `std::process::exit`; pending blocking commands do not get to
  drop their guards. Exercise native close while an isolated MCP command is still
  running, then check the descendant's delayed marker after the app has exited.
- A successful capture with closed pipes can also leave a detached descendant.
  Terminate remaining owned groups on guard drop as Windows Job Objects already
  do, and retain independent terminal sessions outside the temporary groups.
- An exclusive lock around every spawn queued unrelated probes behind slow host
  process startup and caused additional deadline failures in the full suite.
  Parallel spawns must share the exit gate; only shutdown takes it exclusively.
  Keep the readiness, late-marker, refusal and independent-process assertions.
- On Windows, NUL standard streams alone do not prevent inheriting the parent's
  other inheritable pipe handles. The detached-descendant fixture must clear
  inheritance on its captured stdout/stderr handles before spawning. Assert that
  capture returns before the delayed marker, then retain the later absence check.
  See [Windows handle inheritance](https://learn.microsoft.com/en-us/windows/win32/procthread/inheritance).


### 2026-10-11: extension-only retirement needs backend and byte-level proof

- Removing account UI is insufficient: reject every historical account command before loading secrets, preflight, locks, or state creation, including old controlled launch scripts and generic policy/import paths.
- JSON semantic equality does not prove account preservation when MCP shares a file with login metadata. Preserve bytes outside the MCP container and verify both existing-container replacement and missing-container insertion.
- Extension tests seed immutable historical target metadata directly. They must not create fixture targets through retired account writers. Keep cross-target path, identity-alias, credential ownership, and external-change negative cases in resource tests; retire tests whose purpose was successful account mutation.
- Verify real CLI resource writes in isolated HOME/data and read back account-file bytes, target metadata, and unselected targets. Development mock success is UI evidence only.
- A short API-key denylist misses OAuth and cloud-provider identity variables. Reuse the existing identity-variable guard and test protected names even when MCP explicitly references them; dedicated MCP names remain editable.
- Parse smoke-test JSON readback structurally. Matching the line after an object key failed once other fields preceded the extension flag, despite a successful write.

### 2026-10-12: exercise local sources and verify test selection

- Git-source switching tests did not cover an assigned local skill, whose link follows its actual source without a content snapshot. Exercise both source types through the CLI and read back the copy, original and unselected assistant.
- An exact test filter with an abbreviated name ran zero tests. Check the executed count as well as the exit code. Old-source negative controls need a separate build target; sharing one reused stale Cargo artifacts until the package was rebuilt.
- Backup previews must match native handling of omitted sections: an absent legacy template section preserves current templates. Do not display removals that the restore will not perform.
- A page-level overflow check missed skill collection badges overlapping fixed assistant columns. Inspect screenshots and check the constrained header content itself; wrapping must keep source text and status marks inside that column.
- After a personal source switch, inspect both same-name source rows. A valid link owned by the other source is not an external takeover candidate; verify read-only ownership presentation and ordinary-add refusal as well as successful switching.
- Compare missing copy destinations through resolved existing ancestors. A lexical `/var/...` destination and its existing `/private/var/...` source looked unrelated on macOS; the overlap regression caught the first guard accepting a directory inside the original source.
- Use that same resolution in preview fingerprints: a failed adoption can leave an empty parent directory. Its creation must not change the destination identity or invalidate an otherwise unchanged preview.
- Apply personal-directory confirmation guards only to personal-copy adoption. The existing duplicate-cleanup test caught the new dirty-path guard blocking an operation that has no copy destination.

### 2026-10-12: language changes need layout and stale-query checks

- Wrapping JSX text in a translation call can remove meaningful boundary spaces.
  The initial English implementation passed type checking but broke existing
  count, breadcrumb and update-version assertions. Preserve JSX whitespace and
  inspect both languages, including screenshots after page animations finish.
- Keep global language updates out of cancelable query functions. A canceled
  older response can finish after a successful save; only the active query
  snapshot and successful save readback may apply a language. Exercise that
  response order explicitly.
- Preserve interpolated user content without a second translation pass, and
  verify localized MCP display hints still produce the same engine sentinel.
- Keep engine error matching in its original language. Translating an MCP error
  prefix changed unsupported-target recognition in English despite a passing
  Chinese suite; verify the same target filtering in both languages.
