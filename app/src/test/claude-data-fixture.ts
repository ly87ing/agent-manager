import type { ClaudeDataPreview, ClaudeDataResult } from "../lib/claude-data";

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
