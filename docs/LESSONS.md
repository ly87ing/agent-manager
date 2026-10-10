# Lessons from Merged Work

What recent work got wrong, and the rule that would have caught it. Read this before changing code.

Each lesson is a rule, then why, then the evidence (commit, test, or observation). "Seen" counts the separate days a review found it. A lesson seen on three or more days moves into [AGENTS.md](../AGENTS.md), [UI rules](UI-RULES.md), or a guard test, and its entry here shrinks to a pointer.

Add a lesson when a fix had to be redone, a check passed without exercising the change, or an acceptance step was skipped and later mattered. Review the day's commits for these before writing the day's record in [goal-rust-gui.md](goal-rust-gui.md).

## Tests that prove something

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
