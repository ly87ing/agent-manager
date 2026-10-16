# External Commands

The engine starts external commands through `process::command` or `command_with_env`. Login-shell PATH discovery is the Unix-only exception. Configuration and credentials stay in their original files.

## Responsibilities and sources of truth

| Part | Behavior | Source |
| --- | --- | --- |
| Window flags | Windows GUI processes without a console use `CREATE_NO_WINDOW`; terminal launches keep console output | [`process.rs`](../../engine-rs/src/process.rs) |
| Captured output | `capture_group` concurrently drains stdout/stderr with a retention cap; one deadline covers both parent exit and reader EOF | [`process.rs`](../../engine-rs/src/process.rs), [`program execution`](../../engine-rs/src/native_programs/apply.rs), [`PATH discovery`](../../engine-rs/src/native_programs/locate.rs), [`version/upstream capture`](../../engine-rs/src/native_upstream.rs) |
| Timeout groups | Unix uses a separate process group. Windows starts suspended, assigns a kill-on-close Job Object, then resumes; any setup failure stops and reaps the child | [`process.rs`](../../engine-rs/src/process.rs) |
| Proxy fallback | Read macOS manual settings with `scutil --proxy`, Windows Internet Settings from the current-user registry, or GNOME/KDE manual proxy settings; cache for 60 seconds | [`process.rs`](../../engine-rs/src/process.rs) |
| Callers | Extension upstream probes, Git, program execution, MCP checks | [`native_upstream.rs`](../../engine-rs/src/native_upstream.rs), [MCP check](mcp-check.md) |

## Constraints and failure behavior

- Explicit command or process proxy variables, including empty values, suppress automatic proxy discovery. Existing `NO_PROXY` / `no_proxy` values are retained.
- Protocol-specific proxy settings remain protocol-specific. Leading `*.` bypass entries become domain suffixes; unsupported wildcard patterns, Windows `<local>`, macOS simple-hostname exclusions and PAC settings suppress fallback because environment variables cannot express them faithfully.
- Linux reads GNOME GSettings or KDE `kioslaverc` through bounded native read tools for the current desktop. PAC, reverse bypass, proxy authentication and malformed values suppress fallback; credential fields are not queried and embedded user information is rejected. Explicit proxy environment variables retain precedence. No system settings or registry values are written.
- GNOME proxy reads share the [environment check's GSettings file-write restriction](environment-checks.md).
  Missing Landlock ABI 3 support suppresses discovery; there is no unguarded
  retry or change to a different settings backend.
- Program installation, Unix shell PATH discovery, and version/upstream capture keep their deadlines active after the parent exits. On timeout they terminate the group/job and poll child/reader cleanup for at most 250 ms; they never block joining an unfinished reader. Capture retains bounded output and drains the rest. Returned output stays capped at 4 MiB per stream for programs, 256 KiB for PATH, and the caller's stdout cap plus 256 stderr bytes for version/upstream probes. A pipe timeout fails the probe; it cannot substitute an empty successful result.
- npm/curl upstream probes and PyPI installation tool resolution use the detected user PATH and platform command candidates, including Windows PATHEXT and command shims.
- Timeout handlers terminate the process group, including descendants. Dropping
  the guard also terminates remaining descendants after normal completion, even
  if they closed the captured pipes. Windows closes its kill-on-close job; Unix
  terminates the owned process group. Job setup errors are reported rather than
  silently weakening cleanup to a single child.
- Desktop exit cleans up active temporary commands. Parallel spawns share an exit
  gate; shutdown waits for group/job setup and refuses further group creation.
  Unix terminates its tracked groups; Windows closes OS-managed kill-on-close jobs
  as the process exits. Commands started outside these owned temporary groups
  remain outside cleanup. No polling thread is added.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --lib process::tests
cargo test --manifest-path engine-rs/Cargo.toml --lib process::linux_proxy
cargo test --manifest-path engine-rs/Cargo.toml --lib native_programs::apply::capture_tests
cargo test --manifest-path engine-rs/Cargo.toml --lib native_programs::locate::capture_tests
cargo test --manifest-path engine-rs/Cargo.toml --lib native_upstream::capture_tests
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp check_
cargo check --manifest-path engine-rs/Cargo.toml --target x86_64-pc-windows-gnu --all-targets
```

Windows process tests execute the test binary as a parent and grandchild, wait for a readiness file, terminate the job, and check that the grandchild cannot write its delayed marker. Wine execution supplements compilation; it does not prove desktop window behavior on physical Windows.

Program and upstream capture regressions compile a small [std-only native fixture](../../engine-rs/tests/common/capture_fixture.rs) once per test process, then complete its existing output mode before starting capture deadlines. This warmup suppresses both streams and requires the fixture's expected exit code 7. Compilation and warmup each have a 60-second preparation limit. The fixture acts as both parent and descendant on all platforms, avoiding startup of the entire engine test executable inside a two-second timeout. Coverage retains inherited stdout and stderr after parent exit, delayed-marker cleanup, exit status, diagnostic tails or per-stream limits, and output above the retention cap. Unix PATH tests call the real shell probe with an exited shell and a pipe-holding background process, plus verbose startup output. These portable fixtures enable Windows CI coverage; compiling them alone is not native Windows acceptance.
