# External Commands

The engine starts external commands through `process::command` or `command_with_env`. Login-shell PATH discovery is the Unix-only exception. Configuration and credentials stay in their original files.

## Responsibilities and sources of truth

| Part | Behavior | Source |
| --- | --- | --- |
| Window flags | Windows GUI processes without a console use `CREATE_NO_WINDOW`; terminal launches keep console output | [`process.rs`](../../engine-rs/src/process.rs) |
| Captured output | `capture_group` concurrently drains stdout/stderr with a retention cap; one deadline covers both parent exit and reader EOF | [`process.rs`](../../engine-rs/src/process.rs), [`program execution`](../../engine-rs/src/native_programs/apply.rs), [`PATH discovery`](../../engine-rs/src/native_programs/locate.rs), [`version/auth/upstream capture`](../../engine-rs/src/native_upstream.rs) |
| Timeout groups | Unix uses a separate process group. Windows starts suspended, assigns a kill-on-close Job Object, then resumes; any setup failure stops and reaps the child | [`process.rs`](../../engine-rs/src/process.rs) |
| Proxy fallback | Read macOS manual settings with `scutil --proxy`, or Windows Internet Settings from the current-user registry; cache for 60 seconds | [`process.rs`](../../engine-rs/src/process.rs) |
| Callers | Agent actions/version probes, official status, upstream probes, Git, program execution, MCP checks | [`native_agents.rs`](../../engine-rs/src/native_agents.rs), [`native_upstream.rs`](../../engine-rs/src/native_upstream.rs), [MCP check](mcp-check.md) |

## Constraints and failure behavior

- Explicit command or process proxy variables, including empty values, suppress automatic proxy discovery. Existing `NO_PROXY` / `no_proxy` values are retained.
- Protocol-specific proxy settings remain protocol-specific. Leading `*.` bypass entries become domain suffixes; unsupported wildcard patterns, Windows `<local>`, macOS simple-hostname exclusions and PAC settings suppress fallback because environment variables cannot express them faithfully.
- Linux inherits proxy environment variables only. No system settings or registry values are written.
- Program installation, Unix shell PATH discovery, and version/auth/upstream capture keep their deadlines active after the parent exits. On timeout they terminate the group/job and poll child/reader cleanup for at most 250 ms; they never block joining an unfinished reader. Capture retains bounded output and drains the rest. Returned output stays capped at 4 MiB per stream for programs, 256 KiB for PATH, and the caller's stdout cap plus 256 stderr bytes for version/auth/upstream probes. A pipe timeout fails the probe; it cannot substitute an empty successful result.
- npm/curl upstream probes and PyPI installation tool resolution use the detected user PATH and platform command candidates, including Windows PATHEXT and command shims.
- Timeout handlers terminate the process group, including descendants. Dropping
  the guard also terminates remaining descendants after normal completion, even
  if they closed the captured pipes. Windows closes its kill-on-close job; Unix
  terminates the owned process group. Job setup errors are reported rather than
  silently weakening cleanup to a single child.
- Desktop exit cleans up active temporary commands. Parallel spawns share an exit
  gate; shutdown waits for group/job setup and refuses further group creation.
  Unix terminates its tracked groups; Windows closes OS-managed kill-on-close jobs
  as the process exits. Independent account terminal sessions run outside these
  groups and continue after the desktop closes. No polling thread is added.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --lib process::tests
cargo test --manifest-path engine-rs/Cargo.toml --lib native_programs::apply::capture_tests
cargo test --manifest-path engine-rs/Cargo.toml --lib native_programs::locate::capture_tests
cargo test --manifest-path engine-rs/Cargo.toml --lib native_upstream::capture_tests
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp check_
cargo check --manifest-path engine-rs/Cargo.toml --target x86_64-pc-windows-gnu --all-targets
```

Windows process tests execute the test binary as a parent and grandchild, wait for a readiness file, terminate the job, and check that the grandchild cannot write its delayed marker. Wine execution supplements compilation; it does not prove desktop window behavior on physical Windows.

Program and upstream capture regressions compile a small [std-only native fixture](../../engine-rs/tests/common/capture_fixture.rs) once per test process, before starting capture deadlines. It acts as both parent and descendant on all platforms, avoiding startup of the entire engine test executable inside a two-second timeout. Coverage retains inherited stdout and stderr after parent exit, delayed-marker cleanup, exit status, diagnostic tails or per-stream limits, and output above the retention cap. Unix PATH tests call the real shell probe with an exited shell and a pipe-holding background process, plus verbose startup output. These portable fixtures enable Windows CI coverage; compiling them alone is not native Windows acceptance.
