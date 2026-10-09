# MCP Configuration Guide

Actual entries in assistant configuration files are authoritative: Claude Code `~/.claude.json` (top-level `mcpServers`, user scope), Codex `~/.codex/config.toml` (`[mcp_servers.*]`), and other assistants with `mcp_config`, including instances and OpenCode. Entries added directly in a client appear on MCP without import. `native-manager.db` stores only saved but inactive MCP items, never a second copy of current configuration.

## Reading

`mcp:status` performs zero writes. `targets` lists assistant configuration files, their format, existence, disable mode, and file-level errors. `elements` groups entries by service name across assistants; same-name entries may have different contents. Each entry includes:

- State: enabled, disabled, or unrecognized structure; source: configuration file or a disabled snapshot saved by this tool.
- Modeled fields: stdio `command`, `args`, `env` (plus Codex `startup_timeout_sec`); remote `url`, `headers`. Variable references remain `${VAR}`; all literals become `<literal>` in engine output. The UI shows a keep-original-value label and restores `<literal>` on save; both representations are accepted. Paste Configuration recognizes common fragments such as `mcpServers`; credential-like plaintext env values and header values are replaced with same-name `${VAR}` references rather than filled into the form.
- Unmodeled official fields, such as Codex `cwd`, `enabled_tools`, and `tools.*`: preserved on write. Non-credential values remain visible; others are `<hidden>`.
- Digest `base`, returned as the write baseline.

A file that cannot be read because of parsing, symlinks, unsupported format, or sharing with another assistant affects only that assistant.

## Writing

`mcp:apply` changes one entry for one assistant per operation:

| Action | Arguments | Behavior |
| --- | --- | --- |
| `add` | `--agent --spec` | Add an entry; reject if that assistant already has the name in a file or disabled snapshot |
| `edit` | `--agent --spec --base` | Change modeled fields only; preserve other fields, key order, comments, and tables |
| `disable` / `enable` | `--agent --base` | See the next section |
| `copy` | `--agent --to --base` | Convert to the target format; reject existing names unless `--replace --to-base` is supplied. Other fields in the replaced target entry are not preserved; omitted source fields are listed in hints |
| `remove` | `--agent --base` | Delete the entry and its disabled snapshot; the UI requires separate confirmation |
| `forget` | None | Delete a saved definition never written to any assistant |

Re-read the file and compare `--base` under the lock. A mismatch reports an external modification and leaves both file and database unchanged; refresh and retry. Validate in memory that the candidate parses and preserves other entries and file contents, then atomically replace against the old bytes and read back. Failures use conditional rollback. Auditing records only action and scope, never command arguments, endpoints, or values.

`--spec` accepts only modeled fields. For env, `${VAR}` is a reference resolved according to the assistant (Claude runtime references remain unchanged); `<literal>` preserves that key's original value. Other inputs are new ordinary values, rejected if they resemble credentials, with guidance to use `${VAR}`. Headers accept only references or `<literal>`, written in native client form (Claude `${VAR}`, Codex `env_http_headers`, OpenCode `{env:VAR}`). Remote URLs allow only `https://` or loopback `http://localhost` / `http://127.0.0.1`; unchanged URLs are not revalidated during edit. Editing cannot switch stdio and remote transports. Written entries contain only official client fields. Legacy `_MANAGED_BY` and `# [agent-manager:managed]` markers are stripped on read and removed when that entry is edited or toggled.

## Enabling and disabling

- **Codex and OpenCode:** use official `enabled`. Disable writes `enabled = false` while keeping the entry. Enable removes `enabled` to restore its default, rather than adding `enabled = true`. New entries contain only supplied fields: no env table without variables, and no `startup_timeout_sec` without an entered timeout.
- **Claude Code:** the official client has only project-specific disabled lists in `/mcp`, with no global per-entry field. Disable removes the entry from `~/.claude.json` and saves a disabled snapshot; enable restores it and rejects an existing same-name entry. Snapshots retain no credential-like values: detected env literals and all header literals move to the external file selected on Secrets and become `${VAR}` in the snapshot, resolving back to their original values on restore. Without a usable file, disable is rejected. Ordinary env values such as `NODE_ENV=production` remain unchanged in the snapshot.

## Legacy database migration

Old service-library definitions and per-assistant assignments are converted on read using the same rules and persisted on the first MCP write, removing the old assignment table and policy definitions. Migration never changes configuration files:

- Assigned with a same-name file entry: the file wins.
- Assigned but absent from the file, or assignment disabled: become a disabled snapshot for that assistant.
- Never assigned, or modified in the library but not applied: become saved definitions not yet written to any assistant, available to add to an assistant or delete.

`mcp.definitions.*` written by `policy:set` and MCP definitions from old backups are validated normally and converted to saved definitions too.

## Secrets and backups

Variables resolve first from the external `.env` selected on Secrets, then from the process environment. The database stores only the file path. `secrets:status` lists references in actual entries and disabled snapshots and their resolvability, never values. Configuration backups contain saved MCP items with references and ordinary values only. Import writes items individually, lets the backup win on same-name items, and does not delete other local saved items. Backups contain neither secret values nor file paths.

## Upstream versions

Check Updates probes npx packages and uvx `git+repo@branch` sources in actual and saved entries. It picks one entry per service name, preferring configuration files and taking the first assistant in order. If assistants pin different versions under the same name, the result reflects only the selected entry.
