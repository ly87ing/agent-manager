/**
 * 把默认环境（主配置）里的共享设置一次性带进独立环境（引擎 `profiles:seed`）。
 *
 * 独立环境有自己的配置目录，助手不再读主配置里的用户级设置；CC Switch 在同一个目录里切换接入，
 * 这些自然都在。这里改为用户勾选后复制一次：只补环境里还没有的，已有的不动；凭据（env、
 * apiKeyHelper、含明文凭据的 MCP 条目）一律不带。
 */
import type { ProfileKind } from "./profileDefinition";

export type SeedItem = "permissions" | "statusline" | "rules" | "mcp";

export type SeedState = "copied" | "exists" | "absent" | "skipped" | "failed" | "unsupported";

export interface SeedResult {
  item: SeedItem;
  state: SeedState;
  detail: string;
  /** MCP：因含明文凭据或格式不认识而没带的条目名。 */
  skipped?: string[];
}

export const SEED_ITEMS: { id: SeedItem; label: string; source: Record<ProfileKind, string> }[] = [
  {
    id: "permissions",
    label: "权限规则",
    source: {
      claude: "settings.json 的 permissions（允许/拒绝列表与默认模式）",
      codex: "config.toml 的审批与沙箱（approval_policy、sandbox_mode、权限档案）",
    },
  },
  { id: "statusline", label: "状态栏", source: { claude: "settings.json 的 statusLine", codex: "config.toml 的 [tui] status_line" } },
  { id: "rules", label: "规则文件", source: { claude: "CLAUDE.md", codex: "AGENTS.md" } },
  { id: "mcp", label: "MCP 服务", source: { claude: "~/.claude.json 的用户级 MCP 服务", codex: "config.toml 的 mcp_servers" } },
];

export const ALL_SEED_ITEMS: SeedItem[] = SEED_ITEMS.map((item) => item.id);

export function seedArgs(profileId: string, items: SeedItem[]): string[] {
  const ordered = ALL_SEED_ITEMS.filter((item) => items.includes(item));
  return ["--only", `profiles:${profileId}`, "--items", ordered.join(",")];
}

export function seedLabel(item: string): string {
  return SEED_ITEMS.find((entry) => entry.id === item)?.label ?? item;
}

const STATE_TEXT: Record<SeedState, string> = {
  copied: "已带上",
  exists: "环境里已有，没改",
  absent: "默认环境里没有",
  skipped: "没带",
  failed: "没成功",
  unsupported: "不支持",
};

export function seedStateText(state: string): string {
  return STATE_TEXT[state as SeedState] ?? state;
}

/** 一句话汇总：带上了哪些、哪些没成功。 */
export function summarizeSeed(results: SeedResult[]): { title: string; failed: boolean; description: string } {
  const copied = results.filter((r) => r.state === "copied").map((r) => seedLabel(r.item));
  const failed = results.filter((r) => r.state === "failed");
  const title =
    copied.length > 0 ? `已从默认环境带上：${copied.join("、")}` : failed.length > 0 ? "没有带上默认环境的设置" : "没有需要带上的设置";
  const description = results
    .filter((r) => r.state !== "copied" || r.item === "mcp")
    .map((r) => `${seedLabel(r.item)}：${r.detail || seedStateText(r.state)}`)
    .join("；");
  return { title, failed: failed.length > 0, description };
}
