/**
 * 各助手的常用启动参数预设。
 *
 * 取值全部来自本机实测的 `--help` 输出，不臆造——各家叫法完全不同，照抄别家的名字会写出
 * 根本不存在的参数：
 * - Claude Code / Antigravity：`--dangerously-skip-permissions`
 * - Codex：`--dangerously-bypass-approvals-and-sandbox`（官方标注 EXTREMELY DANGEROUS）；
 *   中间档 `--ask-for-approval never --sandbox workspace-write`
 * - Grok：`--permission-mode <default|acceptEdits|auto|dontAsk|bypassPermissions|plan>`
 * - Kimi：`--auto`（Never Ask）与 `-y/--yolo`（Ask When Needed，风险动作仍问）
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

/** Antigravity CLI（agy）：与 Claude 同名的参数，另有一个带终端限制的沙箱档。 */
const ANTIGRAVITY_PRESETS: LaunchPreset[] = [
  {
    id: "agy-yolo",
    label: "全自动（自动批准所有工具请求）",
    args: "--dangerously-skip-permissions",
    danger: true,
    hint: "所有工具调用自动批准，不再逐次询问。只在你信任的目录里用。",
  },
  {
    id: "agy-sandbox",
    label: "沙箱模式（限制终端）",
    args: "--sandbox",
    danger: false,
    hint: "在沙箱里跑，终端能力受限，比直接放开安全。",
  },
];

/** Grok：权限档位是一个取值，不是开关。 */
const GROK_PRESETS: LaunchPreset[] = [
  {
    id: "grok-bypass",
    label: "全自动（跳过权限检查）",
    args: "--permission-mode bypassPermissions",
    danger: true,
    hint: "不再做权限检查，改文件与跑命令都直接执行。只在你信任的目录里用。",
  },
  {
    id: "grok-accept-edits",
    label: "自动接受改文件",
    args: "--permission-mode acceptEdits",
    danger: false,
    hint: "改文件不再逐次确认，其余动作仍按默认规则询问。",
  },
];

/** Kimi：两档语义在官方说明里写得很清楚，照它的措辞。 */
const KIMI_PRESETS: LaunchPreset[] = [
  {
    id: "kimi-auto",
    label: "全自动（从不询问）",
    args: "--auto",
    danger: true,
    hint: "什么都不打断你，全部自动决定并执行。只在你信任的目录里用。",
  },
  {
    id: "kimi-yolo",
    label: "常规自动（风险动作仍问）",
    args: "-y",
    danger: false,
    hint: "常规的改文件与命令自动执行；风险动作、提问与计划仍会征求你的确认。",
  },
];

/** 按实际执行的命令名取预设；未知命令没有预设。 */
export function presetsForCommand(command: string): LaunchPreset[] {
  const bin = command.trim().split(/[\s/]+/).pop() ?? "";
  const byBin: Record<string, LaunchPreset[]> = {
    claude: CLAUDE_PRESETS,
    codex: CODEX_PRESETS,
    agy: ANTIGRAVITY_PRESETS,
    antigravity: ANTIGRAVITY_PRESETS,
    grok: GROK_PRESETS,
    kimi: KIMI_PRESETS,
  };
  return byBin[bin] ?? [];
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
