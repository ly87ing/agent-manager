/**
 * 界面文案集中收口（术语映射表的唯一事实源）。
 * 原则：按钮/标题/徽章用「动作/结果」语言；工程原值（state 枚举、命令名）
 * 只保留在 tooltip/详情里做对照。引擎 envelope 的 code/note 属契约内容，不在此列。
 */
import type { ElementState, Freshness, PolicyLayerName, RiskLevel } from "./contract";

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

/** 升级/更新动作的版本对照（当前 → 最新）；缺失的一侧用「未知」占位。 */
export function versionRangeHint(freshness: Freshness): string {
  const current = freshness.current ? shortVersion(freshness.current) : "未知";
  const upstream = freshness.upstream ? shortVersion(freshness.upstream) : "未知";
  return `${current} → ${upstream}`;
}

// ---------- 矩阵单元格 ----------

// ---------- 风险级（操作类型） ----------

export const RISK_ORDER: RiskLevel[] = ["read", "write-local", "exec-local", "network", "install-remove"];

export const RISK_COPY: Record<RiskLevel, { label: string; className: string }> = {
  read: { label: "只读检查", className: "border bg-muted text-muted-foreground" },
  "write-local": { label: "修改配置文件", className: "border-info/30 bg-info/10 text-info" },
  "exec-local": { label: "运行本地命令", className: "border-warning/30 bg-warning/10 text-warning" },
  network: { label: "需要联网", className: "border-orange/30 bg-orange/10 text-orange" },
  "install-remove": { label: "安装/卸载软件", className: "border-destructive/30 bg-destructive/10 text-destructive" },
};

// ---------- 个性化配置来源 ----------

export const POLICY_LAYER_COPY: Record<PolicyLayerName, { label: string; className: string }> = {
  builtin: { label: "内置默认值", className: "border bg-muted text-muted-foreground" },
  sqlite: { label: "本地数据库", className: "border-violet/30 bg-violet/10 text-violet" },
};

// ---------- 定义保存失败提示 ----------

export const TOGGLE_COPY = {
  saveFailed: "保存配置失败",
  saveRejected: "保存被拒绝",
} as const;

// ---------- 定义覆盖层（MCP 服务 / 技能集合的新增·编辑·移除） ----------

export const OVERLAY_COPY = {
  addServer: "新增服务",
  editServer: "编辑",
  removeServer: "移除",
  addCollection: "添加来源",
  removeCollection: "移除",
  /** 移除定义时确认文案（tombstone 语义）。 */
  removeDefinitionNote: "写入移除标记，使该条目停止生效；删除标记即可恢复定义。确认后会清理本工具管理的配置或链接。",
  /** 移除覆盖层条目时确认文案（policy:delete 语义）。 */
} as const;
