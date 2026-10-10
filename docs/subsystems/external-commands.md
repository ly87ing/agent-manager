# External Commands

The engine starts external commands through `process::command` or `command_with_env`. Login-shell PATH discovery is the Unix-only exception. Configuration and credentials stay in their original files.

## Responsibilities and sources of truth

| Part | Behavior | Source |
| --- | --- | --- |
| Window flags | Windows GUI processes without a console use `CREATE_NO_WINDOW`; terminal launches keep console output | [`process.rs`](../../engine-rs/src/process.rs) |
| Timeout groups | Unix uses a separate process group. Windows starts suspended, assigns a kill-on-close Job Object, then resumes; any setup failure stops and reaps the child | [`process.rs`](../../engine-rs/src/process.rs) |
| Proxy fallback | Read macOS manual settings with `scutil --proxy`, or Windows Internet Settings from the current-user registry; cache for 60 seconds | [`process.rs`](../../engine-rs/src/process.rs) |
| Callers | Agent actions/version probes, official status, upstream probes, Git, program execution, MCP checks | [`native_agents.rs`](../../engine-rs/src/native_agents.rs), [`native_upstream.rs`](../../engine-rs/src/native_upstream.rs), [MCP check](mcp-check.md) |

## Constraints and failure behavior

- Explicit command or process proxy variables, including empty values, suppress automatic proxy discovery. Existing `NO_PROXY` / `no_proxy` values are retained.
- Protocol-specific proxy settings remain protocol-specific. Leading `*.` bypass entries become domain suffixes; unsupported wildcard patterns, Windows `<local>`, macOS simple-hostname exclusions and PAC settings suppress fallback because environment variables cannot express them faithfully.
- Linux inherits proxy environment variables only. No system settings or registry values are written.
- Timeout handlers terminate the process group, including descendants. Windows also closes the job when the guard is dropped. Job setup errors are reported rather than silently weakening cleanup to a single child.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --lib process::tests
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp check_
cargo check --manifest-path engine-rs/Cargo.toml --target x86_64-pc-windows-gnu --all-targets
```

Windows process tests execute the test binary as a parent and grandchild, wait for a readiness file, terminate the job, and check that the grandchild cannot write its delayed marker. Wine execution supplements compilation; it does not prove desktop window behavior on physical Windows.
