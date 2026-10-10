# Recent Sessions

Lists each account's newest conversations on the Usage page and copies a command that goes back to one with the same account. Read-only: it reads the head of session files and never writes.

## Responsibilities and sources of truth

| Part | Responsibility | Source |
| --- | --- | --- |
| Account directories | `session_roots`: the main `~/.claude` and `~/.codex` plus every registered separate account's configuration directory, each once, labelled `claude`, `codex`, `claude:<id>`, or `codex:<id>` | [`native_usage.rs`](../../engine-rs/src/native_usage.rs) |
| Session list | `usage:sessions` returns up to ten sessions per account, newest file first, from the first 256 KiB of each file; a half-written last line is ignored | [`native_sessions.rs`](../../engine-rs/src/native_sessions.rs) |
| Claude format | Files directly under `projects/<encoded dir>/`; the file name is the session id; deeper files are subagent transcripts and are skipped. Title: last `custom-title` (`customTitle`), else last `ai-title` (`aiTitle`), else the first typed prompt (meta lines and text starting with `<` or `#` are skipped) | [`native_sessions.rs`](../../engine-rs/src/native_sessions.rs) |
| Codex format | `sessions/**/rollout-*.jsonl`; the first line `session_meta` gives id, cwd, and source; a `{"subagent": …}` source is skipped. Title: `thread_name` from `session_index.jsonl` (last entry per id), else the first typed user message | [`native_sessions.rs`](../../engine-rs/src/native_sessions.rs) |
| Resume command | Map the account label to its registered account, request a fresh controlled `accounts:command`, and append only the assistant's supported resume arguments. `resumeCommand` enters the recorded directory using the selected POSIX or PowerShell syntax | [`accountCommand.ts`](../../app/src/lib/accountCommand.ts), [`sessions.ts`](../../app/src/lib/sessions.ts), [`RecentSessions.tsx`](../../app/src/components/RecentSessions.tsx) |

## Constraints and failure behavior

- Claude resumes by project directory, so the command always enters the session's recorded directory first.
- An account whose context cannot pass preflight shows its sessions but command copying reports the diagnostic. A saved raw alias cannot bypass account checks.
- Unreadable directories or files are skipped; the command does not fail for one bad file.
- Formats were checked against Claude Code 2.1.295 and Codex 0.161.0 session files; a format change shows up as missing titles, not wrong ones.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml native_sessions
cd app && pnpm vitest run src/test/sessions.test.ts src/test/UsagePage.test.tsx
```
