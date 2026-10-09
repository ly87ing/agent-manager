# MCP Test Connection

Starts one account's MCP entry once, the way its assistant would, and reports whether it answers and which tools it offers. Runs only when the user asks; it changes no configuration and saves no result.

## Responsibilities and sources of truth

| Part | Responsibility | Source |
| --- | --- | --- |
| Command | `mcp:check --only mcp:<name> --agent <account>`; the entry comes from the same inventory as `mcp:status`, so it is the account's actual file | [`native_mcp/check.rs`](../../engine-rs/src/native_mcp/check.rs), [`native_mcp/inventory.rs`](../../engine-rs/src/native_mcp/inventory.rs) |
| Variables | Whole-value references use `env_ref` / header references; inside strings `${NAME}` (and `${NAME:-default}`) is expanded for Claude and `{env:NAME}` for OpenCode; Codex strings are not expanded. Values come from `secret_values::lookup` (secrets file first, then process environment) | [`native_mcp/check.rs`](../../engine-rs/src/native_mcp/check.rs), [`secret_values.rs`](../../engine-rs/src/secret_values.rs) |
| stdio | Runs the command with the user's login-shell PATH (`locate::user_path`) ahead of the inherited one, `HOME` set, in its own process group; sends `initialize`, `notifications/initialized`, `tools/list`; skips log and notification lines | [`native_mcp/check.rs`](../../engine-rs/src/native_mcp/check.rs) |
| Streamable HTTP | Uses `curl -i`, headers on stdin (`-H @-`), keeps `Mcp-Session-Id`, reads JSON or `data:` lines | [`native_mcp/check.rs`](../../engine-rs/src/native_mcp/check.rs) |
| Interface | Test button on each MCP row; dialog to pick the account and show server, tools, and stderr tail | [`McpCheckDialog.tsx`](../../app/src/components/McpCheckDialog.tsx), [`McpServerCard.tsx`](../../app/src/components/McpServerCard.tsx) |

## Constraints and failure behavior

- A variable without a value stops the check before anything runs and names the variable.
- Resolved values are replaced with `***` in the message and stderr, then the usual diagnostic sanitizer runs.
- stdio waits for the entry's `startup_timeout_sec`, else 30 s; HTTP waits 20 s. The result says whether the program exited or the wait timed out.
- Afterwards stdin is closed and, if the server is still running after 0.5 s, the whole process group is killed.
- Legacy SSE transports are refused with a reason.
- The command takes no apply lock and writes neither files nor the database.

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp check_
cd app && pnpm vitest run src/test/McpPage.test.tsx
```

Interop with a real server was last checked in an isolated HOME with `npx -y @modelcontextprotocol/server-everything@2025.9.25` (10 tools).
