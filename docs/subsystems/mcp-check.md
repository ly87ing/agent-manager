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

The dialog announces completion through the shared
[interface feedback](interface-feedback.md) policy. Protocol success is decided
by the result's `ok` field, independently of command collection success. The full
result remains in the dialog; request failures stay above its action footer.

- A variable without a value stops the check before anything runs and names the variable.
- Resolved nonempty values are replaced with `***` in messages, stderr, tool
  names and recursively in service-provided metadata values and keys, then the
  usual diagnostic sanitizer runs. Original matching ranges are merged before
  replacement, including overlapping and repeated values; masks are never
  processed as input. Non-string service metadata leaves containing a resolved
  value become the display string `***`. Masking keys may merge otherwise
  distinct keys; this is display-only data, never an executable configuration.
  A shared 8 MiB conservative matching-work budget covers suffix searches,
  value preparation and range marking. Exhaustion masks text and omits service
  metadata (`null`); it never returns an incompletely sanitized field.
- Working directory: entry `cwd`, then explicit `--cwd`, then the tool's HOME. Paths must be existing absolute directories. A project-dependent server needs the correct project directory. The tool's process environment, selected secrets file and proxy fallback can differ from the assistant's environment.
- Initialization uses modeled `startup_timeout_sec` (Codex stdio), then
  `startup_timeout_ms`, then Codex's 10 s default (other formats: stdio 30 s,
  HTTP 20 s). Other entries retain unmodeled fields without treating a seconds
  field as this check's timeout. Tool listing gets a separate `tool_timeout_sec`,
  defaulting to 60 s for Codex and the transport default for others. Codex defaults
  follow its [official MCP guide](https://learn.chatgpt.com/docs/extend/mcp).
- Invalid initialization, tool-list RPC errors, missing results, early exits and timeouts fail at their respective stage. Servers without the [tools capability](https://modelcontextprotocol.io/specification/2025-06-18/server/tools) succeed at initialization without claiming a tool list. Only the first tool-list page is read; no tools are invoked.
- Results include `base`, `cwd`, `cwd_source`, both effective timeouts and optional `exit_code`. Only exits observed before shutdown are reported; closing stdin or forced termination must not manufacture a failure exit code.
- If protocol output closes first, the checker waits up to 0.5 s within the
  remaining protocol deadline for a natural exit, without closing stdin. EOF
  alone does not prove that the process has finished.
- Afterwards stdin is closed and the server gets up to 0.5 s to exit. The whole
  process group is then terminated even if the parent already exited, so background
  descendants cannot retain pipes or outlive the check. Natural exit codes are
  recorded before shutdown.
- Commands use the shared [external-command layer](external-commands.md) for Windows console flags, timeout jobs and manual system proxy fallback; explicit entry environment variables take precedence.
- Legacy SSE transports are refused with a reason.
- The command holds a shared account lock while its process runs. Account, credential, and MCP writes require an exclusive lock and refuse an active check. It writes neither configuration nor the database.

## Editing targets

The MCP editor initially selects only the assistant used to prefill the form.
Users must explicitly select additional assistants to synchronize modeled fields.
Changing the prefill source resets selection to that assistant. Preview and apply
use exactly the selected subset, with each entry's own digest and preserved
literal/unknown values. A stale target fails without blocking other selected
targets or affecting unselected assistants.

## Legacy marker recovery

MCP rows expose legacy `_MANAGED_BY` fields as configuration warnings, independently of connection results. `mcp:preview` and `mcp:apply` accept `--action clean-markers --only mcp:<name> --agent <account> --base <digest>`. The action removes only the selected entry's top-level and env-container marker fields, preserving other values, enabled state and other accounts. Preview is read-only; apply rechecks the digest under the normal write lock. See the [configuration guide](../MCP-CONFIG-GUIDE.md).

## Verification

```sh
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp check_
cargo test --manifest-path engine-rs/Cargo.toml --test native_mcp_protocol
cd app && pnpm vitest run src/test/McpPage.test.tsx
```

The historical real-server check used an isolated HOME with `npx -y @modelcontextprotocol/server-everything@2025.9.25` (10 tools). Current account-isolation acceptance uses fake stdio and HTTP executables and a loopback server; it does not request a real account's service.

The native protocol target uses copied native executables and the actual CLI,
without a shell or an external language runtime for its stdio server. It records
and independently checks request order, literal arguments, working directory,
selected-variable isolation, protocol failures, natural exits after output EOF,
still-running servers with closed output, timeout outcomes,
serialized-result redaction (including numeric echoes and ten thousand metadata
records) and unchanged source/database bytes. Windows also
exercises a real `npx.cmd` behind a stored `cmd /c npx` entry with ordinary space
and Unicode arguments; special-character round trips apply to the direct native
executable case. Compiling the target alone does not establish native acceptance.
