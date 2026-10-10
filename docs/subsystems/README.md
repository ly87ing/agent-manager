# Subsystem References

Each page describes how one subsystem works now: what it is responsible for, where its facts come from, its runtime path, the constraints it keeps when something fails, and the commands that verify it. Start with the page for the area you change, then follow its source links to check the code before editing.

| Subsystem | Reference |
| --- | --- |
| Quota readings, tray summary and panel, quota reminder | [Quota and tray](quota-and-tray.md) |
| Recent sessions and resume commands | [Recent sessions](recent-sessions.md) |
| MCP test connection | [MCP test connection](mcp-check.md) |
| External commands, timeout groups and system proxy fallback | [External commands](external-commands.md) |
| Desktop launch location and installation warning | [Desktop startup](desktop-startup.md) |

Product scope stays in [Concurrent Accounts](../MULTI-ACCOUNT-DESIGN.md) and data semantics in [GUI Redesign](../GUI-REDESIGN.md); implementation history and acceptance stay in [goal-rust-gui.md](../goal-rust-gui.md). A reference page is not a design proposal: proposals go in the task document.

## Maintaining a reference

- Update the owning page in the same commit when a change alters a documented responsibility, source of truth, runtime path, failure behavior, or verification command. A refactor that keeps these contracts needs no prose change.
- Keep each fact in one page and link to it. Link source files and named functions rather than copying code that changes often.
- A new page has the same sections: responsibilities and sources of truth, runtime path, constraints and failure behavior, verification.
