# MCP Test Connection

Checks one account's actual MCP entry once. This is a separate protocol session, not evidence that the assistant has loaded the service. Runs only when the user asks; it changes no configuration and persists no result.

## Responsibilities and sources of truth

| Part | Responsibility | Source |
| --- | --- | --- |
| Command | `mcp:check --only mcp:<name> --agent <account> [--base <digest>] [--cwd <absolute-directory>]`; optional `--base` rejects a changed entry before execution. The UI always supplies it | [`native_mcp/check.rs`](../../engine-rs/src/native_mcp/check.rs), [`native_mcp/inventory.rs`](../../engine-rs/src/native_mcp/inventory.rs) |
| Variables | Validate all reference names and exclusive account ownership, including embedded Claude `${NAME}`, OpenCode `{env:NAME}`, and Codex header references. Defaults and malformed references fail closed. Values come only from the explicit private secrets source | [`native_mcp/check.rs`](../../engine-rs/src/native_mcp/check.rs), [`native_secrets.rs`](../../engine-rs/src/native_secrets.rs), [`secret_values.rs`](../../engine-rs/src/secret_values.rs) |
| stdio | Resolve an absolute executable without running shell startup files; use a clean environment with explicit HOME and this account's references, in its own process group. Send `initialize`, `notifications/initialized`, `tools/list`; skip log and notification lines | [`native_mcp/check.rs`](../../engine-rs/src/native_mcp/check.rs) |
| Streamable HTTP | Use an absolute `curl` with `--disable` first to suppress curlrc, explicit URL and headers on stdin (`-H @-`), retain `Mcp-Session-Id`, and read JSON or `data:` lines | [`native_mcp/check.rs`](../../engine-rs/src/native_mcp/check.rs) |
| Interface | Dialog shows account, file, command/address, working directory, effective timeouts, failure stage, natural exit code and sanitized stderr tail. Results stay in page memory per account and entry digest; a changed digest hides them. Switching account or directory clears the dialog result | [`McpCheckDialog.tsx`](../../app/src/components/McpCheckDialog.tsx), [`McpServerCard.tsx`](../../app/src/components/McpServerCard.tsx) |

## Constraints and failure behavior

- A variable without a value stops the check before anything runs and names the variable.
- Resolved values are replaced with `***` in the message and stderr, then the usual diagnostic sanitizer runs.
- Working directory: entry `cwd`, then explicit `--cwd`, then the tool's HOME. Paths must be existing absolute directories. A project-dependent server needs the correct project directory. The tool's process environment, selected secrets file and proxy fallback can differ from the assistant's environment.
- Initialization uses `startup_timeout_sec`, then `startup_timeout_ms`, then Codex's 10 s default (other formats: stdio 30 s, HTTP 20 s). Tool listing gets a separate `tool_timeout_sec`, defaulting to 60 s for Codex and the transport default for others. Codex defaults follow its [official MCP guide](https://learn.chatgpt.com/docs/extend/mcp).
- Invalid initialization, tool-list RPC errors, missing results, early exits and timeouts fail at their respective stage. Servers without the [tools capability](https://modelcontextprotocol.io/specification/2025-06-18/server/tools) succeed at initialization without claiming a tool list. Only the first tool-list page is read; no tools are invoked.
- Results include `base`, `cwd`, `cwd_source`, both effective timeouts and optional `exit_code`. Only exits observed before shutdown are reported; closing stdin or forced termination must not manufacture a failure exit code.
- Afterwards stdin is closed and, if the server is still running after 0.5 s, the whole process group is killed.
- Commands use the shared [external-command layer](external-commands.md) for Windows console flags, timeout jobs and manual system proxy fallback; explicit entry environment variables take precedence.
- Legacy SSE transports are refused with a reason.
- The command holds a shared account lock while its process runs. Account, credential, and MCP writes require an exclusive lock and refuse an active check. It writes neither configuration nor the database.

## Legacy marker recovery

MCP rows expose legacy `_MANAGED_BY` fields as configuration warnings, independently of connection results. `mcp:preview` and `mcp:apply` accept `--action clean-markers --only mcp:<name> --agent <account> --base <digest>`. The action removes only the selected entry's top-level and env-container marker fields, preserving other values, enabled state and other accounts. Preview is read-only; apply rechecks the digest under the normal write lock. See the [configuration guide](../MCP-CONFIG-GUIDE.md).

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp check_
cd app && pnpm vitest run src/test/McpPage.test.tsx
```

The historical real-server check used an isolated HOME with `npx -y @modelcontextprotocol/server-everything@2025.9.25` (10 tools). Current account-isolation acceptance uses fake stdio and HTTP executables and a loopback server; it does not request a real account's service.
