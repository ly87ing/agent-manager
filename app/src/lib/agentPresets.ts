/**
 * 各助手的常用启动参数预设。
 *
 * 取值全部来自本机实测的 `--help` 输出，不臆造：
 * - Claude Code `--dangerously-skip-permissions`：Bypass all permission checks
 * - Codex `--dangerously-bypass-approvals-and-sandbox`：跳过所有确认且不沙箱，官方标注 EXTREMELY DANGEROUS
 * - Codex `--ask-for-approval never --sandbox workspace-write`：不再逐次确认，但命令仍在可写工作区沙箱内执行
 *
 * 预设只是把参数填进输入框，用户可以继续改；危险项在界面上明确标出后果。
 */

export interface LaunchPreset {
  id: string;
  label: string;
  args: string;
  /** true = 会让助手不再征求确认，界面要显著标出。 */
  danger: boolean;
  hint: string;
}

const CLAUDE_PRESETS: LaunchPreset[] = [
  {
    id: "claude-yolo",
    label: "全自动（跳过所有确认）",
    args: "--dangerously-skip-permissions",
    danger: true,
    hint: "助手不再逐次征求确认，直接执行改文件、跑命令等操作。只在你信任的目录里用。",
  },
];

const CODEX_PRESETS: LaunchPreset[] = [
  {
    id: "codex-yolo",
    label: "全自动（跳过确认且不沙箱）",
    args: "--dangerously-bypass-approvals-and-sandbox",
    danger: true,
    hint: "跳过所有确认并在沙箱之外执行，官方把这个参数标为极度危险。只在本身已隔离的环境里用。",
  },
  {
    id: "codex-auto-sandboxed",
    label: "自动执行，保留沙箱",
    args: "--ask-for-approval never --sandbox workspace-write",
    danger: false,
    hint: "不再逐次确认，但命令仍限制在可写工作区沙箱内，比完全放开安全。",
  },
];

/** 按实际执行的命令名取预设；未知命令没有预设。 */
export function presetsForCommand(command: string): LaunchPreset[] {
  const bin = command.trim().split(/[\s/]+/).pop() ?? "";
  if (bin === "claude") return CLAUDE_PRESETS;
  if (bin === "codex") return CODEX_PRESETS;
  return [];
}

/** 参数是否已经在当前取值里（按空白切分逐段比对，避免子串误判）。 */
export function hasPresetArgs(current: string, preset: LaunchPreset): boolean {
  const tokens = current.trim().split(/\s+/).filter(Boolean);
  const wanted = preset.args.split(/\s+/).filter(Boolean);
  return wanted.every((token) => tokens.includes(token));
}

/** 切换预设：已在取值里就整段移除，否则追加到末尾。 */
export function togglePresetArgs(current: string, preset: LaunchPreset): string {
  const tokens = current.trim().split(/\s+/).filter(Boolean);
  const wanted = preset.args.split(/\s+/).filter(Boolean);
  if (hasPresetArgs(current, preset)) {
    return tokens.filter((token) => !wanted.includes(token)).join(" ");
  }
  return [...tokens, ...wanted].join(" ");
}
