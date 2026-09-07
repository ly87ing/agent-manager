/**
 * 界面文案集中收口（术语映射表的唯一事实源）。
 * 原则：按钮/标题/徽章用「动作/结果」语言；工程原值（state 枚举、命令名）
 * 只保留在 tooltip/详情里做对照。引擎 envelope 的 code/note 属契约内容，不在此列。
 */
import type { ElementState, Freshness, PlanAction, PolicyLayerName, ProfileSyncState, RiskLevel } from "./contract";

// ---------- 同步状态（收敛轴七态） ----------

export const ELEMENT_STATE_COPY: Record<ElementState, { label: string; hint: string }> = {
  "managed-ok": { label: "正常", hint: "与期望完全一致" },
  drifted: { label: "不一致", hint: "文件被改动过，与应有内容不一致" },
  missing: { label: "缺失", hint: "应该有，但没有" },
  "not-materialized": { label: "尚未生成", hint: "目标文件还没有生成过" },
  "unmanaged-detected": { label: "已安装，未接管", hint: "检测到未经本工具管理的安装" },
  unsupported: { label: "不支持", hint: "当前平台或形态暂不支持管理" },
  disabled: { label: "已停用", hint: "在配置中被停用" },
};

// ---------- 最新版本（上游轴五态） ----------

/** git commit 全 hash 显示短形。 */
export function shortVersion(value: string | null): string {
  if (!value) return "?";
  return /^[0-9a-f]{20,}$/i.test(value) ? value.slice(0, 7) : value;
}

export function freshnessLabel(freshness: Freshness): string {
  switch (freshness.state) {
    case "current":
      return "已是最新";
    case "behind":
      return `可更新 ${shortVersion(freshness.current)} → ${shortVersion(freshness.upstream)}`;
    case "ahead":
      return freshness.current ? `比官方版还新 ${freshness.current}` : "比官方版还新";
    case "unknown":
      return "未检查";
    case "not-applicable":
      return "";
  }
}

export const FRESHNESS_STALE_NOTE = "检查结果较旧";

// ---------- 矩阵单元格 ----------

/** MCP 服务 × 助手 配置状态。 */
export const MCP_CELL_COPY: Record<string, string> = {
  "managed-ok": "已配置",
  drifted: "不一致",
  missing: "未配置",
  absent: "未配置",
  "agent-disabled": "未配置",
  "not-materialized": "尚未生成",
  leftover: "残留",
  "probe-error": "检查出错",
};

/** 技能 × 助手 启用状态。 */
export const SKILL_CELL_COPY: Record<string, string> = {
  linked: "已启用",
  "missing-link": "未生效",
  "dead-link": "失效链接",
  drifted: "不一致",
  "extra-link": "计划外链接",
  disabled: "已停用",
};

// ---------- 风险级（操作类型） ----------

export const RISK_ORDER: RiskLevel[] = ["read", "write-local", "exec-local", "network", "install-remove"];

export const RISK_COPY: Record<RiskLevel, { label: string; className: string }> = {
  read: { label: "只读检查", className: "border-slate-200 bg-slate-50 text-slate-500" },
  "write-local": { label: "修改配置文件", className: "border-sky-200 bg-sky-50 text-sky-700" },
  "exec-local": { label: "运行本地命令", className: "border-amber-200 bg-amber-50 text-amber-700" },
  network: { label: "需要联网", className: "border-orange-200 bg-orange-50 text-orange-700" },
  "install-remove": { label: "安装/卸载软件", className: "border-red-200 bg-red-50 text-red-700" },
};

/** plan 动作清单的人话摘要：「3 处文件修改 · 1 个软件安装」。 */
export function planSummaryPhrase(actions: PlanAction[]): string {
  const count = (risk: RiskLevel) => actions.filter((action) => action.risk === risk).length;
  const parts: string[] = [];
  const writeLocal = count("write-local");
  const execLocal = count("exec-local");
  const network = count("network");
  const installRemove = count("install-remove");
  const read = count("read");
  if (writeLocal > 0) parts.push(`${writeLocal} 处文件修改`);
  if (execLocal > 0) parts.push(`${execLocal} 个本地命令`);
  if (network > 0) parts.push(`${network} 个联网操作`);
  if (installRemove > 0) parts.push(`${installRemove} 个软件安装/卸载`);
  if (read > 0) parts.push(`${read} 项只读检查`);
  return parts.length > 0 ? parts.join(" · ") : "没有实际改动";
}

// ---------- 个性化配置三层 ----------

export const POLICY_LAYER_COPY: Record<PolicyLayerName, { label: string; className: string }> = {
  baseline: { label: "仓库默认值", className: "border-slate-200 bg-slate-100 text-slate-500" },
  synced: { label: "多设备共享", className: "border-sky-200 bg-sky-50 text-sky-700" },
  local: { label: "仅这台电脑", className: "border-orange-200 bg-orange-50 text-orange-700" },
};

// ---------- 多设备同步状态 ----------

export const PROFILE_STATE_COPY: Record<ProfileSyncState, { label: string; className: string }> = {
  clean: { label: "已同步", className: "border-emerald-200 bg-emerald-50 text-emerald-700" },
  ahead: { label: "有未推送的改动", className: "border-sky-200 bg-sky-50 text-sky-700" },
  behind: { label: "有未拉取的改动", className: "border-amber-200 bg-amber-50 text-amber-700" },
  diverged: { label: "两边都有改动", className: "border-red-200 bg-red-50 text-red-700" },
  uninitialized: { label: "远端还是空的", className: "border-zinc-200 bg-zinc-100 text-zinc-500" },
  "local-only": { label: "仅本机（离线）", className: "border-slate-200 bg-slate-100 text-slate-500" },
};

// ---------- 通用动作/状态短语 ----------

export const COPY = {
  /** system:refresh-upstream 的唯一主动作。 */
  checkUpdates: "检查更新",
  checkAllUpdates: "检查全部更新",
  recheck: "重新检查",
  checking: "检查中…",
  neverChecked: "从未检查",
  /** 域级 apply。 */
  syncNow: "立即同步",
  syncState: "同步状态",
  updateThis: "更新此项",
  updateAll: "全部更新",
  executing: "正在执行…",
  /** dry-run。 */
  dryRunChip: "演练",
  dryRunToggle: "演练模式：只列出将要执行的操作，不实际改动",
  /** plan 预览弹窗标题。 */
  planPreviewTitle: "将要执行的操作",
  /** 模式横幅。 */
  offlineBannerBody: "未联网，版本与配置状态显示的是上次检查的结果；需要联网的操作已禁用。",
  dryRunBannerBody: "所有操作只演示，不实际改动。",
  offlineNetworkDenied: "离线模式下不可用（需要联网）",
  /** 漂移解释面板。 */
  expandDetail: "展开同步详情",
  collapseDetail: "收起同步详情",
  driftExplanation: "不一致说明",
} as const;
