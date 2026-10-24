import type { ClaudeDataPreview, ClaudeDataResult, OrcaSearchPreview } from "../lib/claude-data";

export function makeOrcaSearchPreview(): OrcaSearchPreview {
  return { source: "orca_search", path: "/YOUR_HOME/Library/Application Support/orca/ai-vault/session-search.sqlite",
    location_origin: "default", status: "inspected", schema_version: 6,
    counts: { claude_sessions: 4, claude_messages: 32, claude_file_cursors: 4,
      non_claude_sessions: 6, unattributed_messages: 2, unattributed_file_cursors: 1, unattributed_fts_rows: 3 },
    cleanup_supported: false, scope_complete: false, message: "Synthetic read-only main-file snapshot; no data was cleaned." };
}

export function makeClaudeDataPreview(): ClaudeDataPreview {
  return {
    scope: "default_claude_code", platform: "macos", root: "/YOUR_HOME/.claude", config_path: "/YOUR_HOME/.claude.json",
    configuration_paths: ["/YOUR_HOME/.claude/settings.json", "/YOUR_HOME/.claude.json", "/YOUR_HOME/.claude.json.backup.1770000000000"],
    base: "a".repeat(64), can_clear: true, has_changes: true,
    groups: [{ id: "sessions", files: 12, directories: 3, bytes: 190_000 }, { id: "account", files: 2, directories: 0, bytes: 3000 }],
    telemetry_policy: "remove",
    retained: [{ path: "/YOUR_HOME/.claude/worktrees", reason: "project" }],
    credential_items: 2, files_only: false, blockers: [],
  };
}

export function makeClaudeDataResult(): ClaudeDataResult {
  return { scope: "default_claude_code", status: "complete", deleted_files: 14, deleted_directories: 3,
    deleted_credentials: 2, telemetry_policy: "remove", files_only: false, scope_verified: true,
    retained_verified: true,
    retained: [{ path: "/YOUR_HOME/.claude/worktrees", reason: "project" }],
    message: "The selected local scope was checked. Remote tokens were not revoked." };
}
