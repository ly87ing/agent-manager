# Changelog

All notable changes are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and version numbers follow [Semantic Versioning](https://semver.org/).

## [0.1.0] - Unreleased

First public release candidate. At this candidate milestone, public release remained blocked by external repository conditions: GitHub Actions billing, repository Secrets/Variables (both at 0), Apple Developer ID and notarization, Windows signing material, real-device acceptance on Linux/Windows/arm64, and end-to-end updater acceptance were not ready. No tag or GitHub Release had been created, and installers were not yet available. See the README and platform status for subsequent preview delivery.

### Added

- Complete management workflows for six resource domains:
  - Assistants: install, upgrade, and uninstall built-in assistants (Claude Code, Codex) and user-added assistants, with truthful detection of the actual installation source.
  - Skills: update, enable, disable, and remove collections and individual skills; deploy content snapshots through safe symlinks.
  - MCP servers: local stdio and remote HTTP transports, assignments per assistant, Claude JSON / Codex TOML / OpenCode JSON codecs, and discovery/import from existing client configuration.
  - Rules: shared and assistant-specific rules applied to `~/.codex/AGENTS.md`, `~/.claude/CLAUDE.md`, and `~/.config/opencode/AGENTS.md`.
  - Hooks: merge into Claude `~/.claude/settings.json` and write the whole Codex `~/.codex/hooks.json` file.
  - Shell aliases: maintain managed blocks in `~/.zshrc` and `~/.zprofile`; compare only managed blocks, so outside changes do not affect synchronization.
- `native-manager.db` (SQLite) as the source of truth in the original definition model; queries and status reads perform zero writes.
- Direct apply interaction: validation before writing, exact scope, exclusive locking, fail-closed refusal to overwrite external changes, conditional rollback, and operation auditing. Destructive actions such as uninstalling, removing definitions, and clearing logs require confirmation.
- MCP secrets: definitions store only `${VAR}` references. After selecting an external `.env` file with mode `600` on Secrets, values can be entered or adopted directly. The external file takes precedence, with the process environment as a read-only fallback. The database stores only the file path; the UI never reveals values, and operation records and configuration backups omit them. Historical database plaintext is reported only as a count and requires explicit migration; the old table is removed only after all external-file writes succeed.
- Upstream update checks for assistants, MCP, and skills, with probe results cached in SQLite.
- SQLite operation history with configurable retention and confirmation before clearing. Error messages are redacted before leaving storage.
- Assistant connection configuration: multiple connection presets for Claude Code and Codex, switched through a dropdown on the card. Switching and writing are one command; saving writes immediately. Claude writes `env` in `~/.claude/settings.json` (Bearer/x-api-key authentication, model aliases, and a small-task model). Codex writes `[model_providers.<name>]` and `model_provider` in `~/.codex/config.toml` (tokens use an `env_key` variable name rather than being stored on disk).
- Assistant status lines: use this tool's version or leave a user-defined version unchanged. When this tool's version is used, it records live quota supplied by the assistant so card quota matches the status line.
- Usage statistics (`usage:status` / `scan` / `clear`): incrementally total tokens from Claude `~/.claude/projects` and Codex `~/.codex/sessions`, aggregate by UTC hour in storage, and group into local-time days in the UI. Built-in prices for three official Anthropic models were checked against the official pricing page on 2026-09-20; other prices can be entered on Usage. Models without a price show tokens only.
- Connection availability checks (`agents:check`): send a minimal real request to validate endpoint, token, and model, then turn the HTTP result into actionable guidance. Tokens reach curl through a mode-0600 temporary file, never command arguments or the audit trail.
- Provider presets for official Zhipu GLM, Kimi, and DeepSeek (domestic/international) endpoints and models. Presets are separated by API shape: Claude Anthropic and Codex Responses endpoints differ. Only provider-operated endpoints documented by their official sources are included; aggregating relays are excluded.
- Permission-confirmation bypass: write Claude `permissions.defaultMode` or Codex `approval_policy` and `sandbox_mode` only after explicit confirmation.
- Fail-closed writes compare only managed items: Claude key paths (`env.ANTHROPIC_*`, `API_TIMEOUT_MS`, `permissions.defaultMode`, and `statusLine` when pointing to this tool's script), and Codex's four top-level keys and managed `[model_providers.*]` sections. Changes to these items block overwriting; all other content, including comments and MCP-owned sections, is preserved.
- A top-level frontend error boundary, confirmation for custom-assistant installation/upgrades, and MCP empty-state guidance.
- MIT license, public-release metadata, explicit Webview CSP, and bundle ID `io.github.ly87ing.agent-manager`.
- Release pipeline for four platform targets (macOS DMG, Linux AppImage/DEB, Windows NSIS/MSI): create one draft Release first, upload matrix artifacts by releaseId, generate updater `latest.json` once in the final job, support prerelease tags, produce per-platform SHA256 checksums, and gate engine-rs version consistency. Releases remain drafts.
- Command palette (`⌘K`): search assistants, instances, skill collections, MCP, pages, and common actions. Resources open their page; actions use the same execution channel as page controls.
- Tray shortcuts to the main window, command palette, and adoption center. The tray never triggers writes directly.
- Structured hook editing by event and command. Saving reconstructs the on-disk structure and preserves hidden fields; raw editing is available on demand.
- Enable/disable assistant cards. Disabling stops resource maintenance without changing installed software. Removing a definition offers time-limited undo.
- Overview in top-level navigation, with post-success guidance about when changes take effect (restart the assistant or open a new terminal).
- Adoption-center cleanup for broken links and links left by the old system. Only links are deleted; real directories and entries still assigned are rejected.
- Assistant instances: separate configurations of the same assistant, each with its own directory, login, sessions, skills, and MCP. `CLAUDE_CONFIG_DIR` / `CODEX_HOME` isolate concurrent instances without changing global configuration. An instance is an assistant definition, so skill matrices and MCP assignments support it directly. Authentication stores variable names only; Claude resolves values when writing instance configuration, while Codex writes only the variable name.
- Launch commands: configure each assistant's time zone (common zones first; inherit globally or override), language, and persistent extra arguments, with a live preview of the line written to terminal configuration. Settings terminal shortcuts use individual add/edit/delete controls; raw editing is available on demand.
- Adoption center: scan existing local assistants, skills, and MCP resources, classify by source, and adopt selected items in a batch. Official paths and native installation methods are identified and retain their original upgrade/uninstall paths. External resources are managed through symlinks; a skill collection can register a local directory, linking directly to its source so external edits propagate. Disabling removes only the link.
- Launch-argument presets based on each assistant's supported arguments, including fully automatic high-permission modes, selected explicitly by the user. Built-in defaults no longer include permission-bypass arguments. Shortcuts read back actual terminal values and show whether they match the settings.
- Single-file configuration backups. Before import, list additions, changes, and removals by category; diff inspection is read-only. Backups contain definitions and sources, never secret values or external secrets-file paths. Private `.env` files must be moved separately between machines. Older backups with a nonempty `secrets` section are rejected.
- Batch adoption progress showing the current item and count. Failed items are skipped and processing continues.
- First-run guidance with a three-step path and a direct link to the adoption center; dismissible permanently.
- Flexible launch commands and shortcuts: multiple launch commands per instance, each editable, copyable, and removable; shortcuts can be renamed and copied.
- Adopt Existing Content for externally changed rules and shortcuts (previously available only for hooks).
- Merged writes compare only the portion owned by this tool. User edits to unrelated settings in `~/.claude/settings.json` or instance configuration no longer count as conflicts or require repeated adoption.
- Adopting existing MCP entries includes already configured secret values to preserve usability; existing variable references still require one entry on Secrets.
- Skill collection scope selected with checkboxes: all skills or a chosen subset of discovered skills, without typing directory names.
- Overview synchronization status for all six domains, with one action to synchronize safe items. Externally modified items requiring a decision are separate and link to their resource page.
- Launch-argument presets for Claude Code, Codex, Antigravity, Grok, and Kimi, using official permission arguments.
- macOS packaging and standalone acceptance scripts: redact build paths, mount read-only, run in isolation, and truthfully check notarization tickets when signed.

### Removed

- Python implementation and `manage.sh` entry point. CLI and desktop now share one Rust library.
- Public plan/dry-run entry points. Writes occur only through real apply.
