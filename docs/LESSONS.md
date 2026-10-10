# Lessons from Merged Work

What recent work got wrong, and the rule that would have caught it. Read this before changing code.

Each lesson is a rule, then why, then the evidence (commit, test, or observation). "Seen" counts the separate days a review found it. A lesson seen on three or more days moves into [AGENTS.md](../AGENTS.md), [UI rules](UI-RULES.md), or a guard test, and its entry here shrinks to a pointer.

Add a lesson when a fix had to be redone, a check passed without exercising the change, or an acceptance step was skipped and later mattered. Review the day's commits for these before writing the day's record in [goal-rust-gui.md](goal-rust-gui.md).

## Tests that prove something

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

**Capture the server's exit status before starting cleanup.**
- Closing stdin can make an unresponsive MCP server exit normally or return a new error. Capture its status before closing stdin; otherwise a timeout can acquire an unrelated exit code. Cover both forced termination and EOF-driven shutdown in the same failure-stage test.
- Seen 1× (2026-10-09).

## Dependencies and lockfiles

**Pin a dependency to a known-compatible version in one step; do not probe versions in a loop.**
- `tauri-plugin-notification` pulled `notify-rust` 4.18.2, which needs rustc 1.89 (local toolchain 1.88). Trying versions with repeated `cargo update --precise` rewrote unrelated Windows dependency edges in `Cargo.lock`. The lockfile was restored from `HEAD` and only the plugin plus `notify-rust` 4.17.0 were added (`341b4ee`).
- Check `git diff --stat Cargo.lock` and the changed `version =` lines before committing a dependency change.
- Seen 1× (2026-10-09).

## Acceptance on the real desktop

**Launch the generated OS entry; a correct-looking string is not startup evidence.**
- The first Linux autostart entry escaped a literal `%` as `%%` according to the Desktop Entry syntax. Real `gio launch` refused it because GIO checks whether the executable exists before expanding field codes. File readback alone had passed. Such paths now fail before writing; a disposable Linux run executed the exact path containing spaces, quotes, `$`, and a backslash.
- Seen 1× (2026-10-09).

**Canonicalize a temporary native executable's launch path on macOS.**
- Launching the warning fixture through `/var/...` produced Tauri's `unknown path`, since `/var` is a symlink and Tauri rejects symlinks in its cached executable path. Launching the same fixture through `/private/var/...` with its resource directory present exercised the native warning. Do not weaken Tauri's protection to repair a test fixture.
- Seen 1× (2026-10-09).

**A development build next to the installed app needs its own identifier.**
- The single-instance plugin hands a second launch to the running instance, so a debug build with the default identifier would only have focused the user's installed app. The tray panel was accepted with `TAURI_CONFIG='{"identifier":"io.github.ly87ing.agent-manager.devcheck"}'`, an isolated `HOME` and `AGENT_MANAGER_DATA_DIR`, and the installed instance kept running (`09675b5`).
- Seen 1× (2026-10-09).
