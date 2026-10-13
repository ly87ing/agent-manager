# Subsystem References

Each page describes how one subsystem works now: what it is responsible for, where its facts come from, its runtime path, the constraints it keeps when something fails, and the commands that verify it. Start with the page for the area you change, then follow its source links to check the code before editing.

| Subsystem | Reference |
| --- | --- |
| Retired quota and background presentation | [Quota and tray](quota-and-tray.md) |
| Retired session collection | [Recent sessions](recent-sessions.md) |
| MCP test connection | [MCP test connection](mcp-check.md) |
| Skill assignments and atomic source switching | [Skills](skills.md) |
| Selective extension definition restore | [Extension backups](extension-backups.md) |
| External commands, timeout groups and system proxy fallback | [External commands](external-commands.md) |
| Desktop installation warning and OS login entries | [Desktop startup](desktop-startup.md) |
| Independent account directories and settings targets | [Account path isolation](account-path-isolation.md) |
| Retired account operations and extension safety boundary | [Account context](account-context.md) |
| Signed updater artifacts and platform selection | [Updater manifest](updater-manifest.md) |
| Managed program versions, npm layouts and executable discovery | [Programs](programs.md) |
| On-demand queries and memory constraints | [Resource bounds](resource-bounds.md) |
| English default and saved interface language | [Interface language](interface-language.md) |

Product scope is governed by [Extensions only](../EXTENSIONS-ONLY.md). Earlier account designs and acceptance records are historical. Data semantics remain in [GUI Redesign](../GUI-REDESIGN.md); implementation history and acceptance stay in [goal-rust-gui.md](../goal-rust-gui.md).

## Maintaining a reference

- Update the owning page in the same commit when a change alters a documented responsibility, source of truth, runtime path, failure behavior, or verification command. A refactor that keeps these contracts needs no prose change.
- Keep each fact in one page and link to it. Link source files and named functions rather than copying code that changes often.
- A new page has the same sections: responsibilities and sources of truth, runtime path, constraints and failure behavior, verification.
