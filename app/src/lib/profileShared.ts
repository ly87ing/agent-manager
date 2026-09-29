/** 默认环境与独立环境实际共用设置的结构摘要；不包含配置原文。 */
export interface SharedItem {
  item: string;
  state: "new" | "different" | "same" | "absent" | "blocked";
  source: string;
  target: string;
  changed_fields: string[];
  field_diffs: Array<{ field: string; source: string; target: string }>;
  source_base: string;
  target_base: string;
  blocked_reason: string | null;
}

export interface SharedStatus { items: SharedItem[] }

export interface SharedPreview {
  files: Array<{ path: string; exists: boolean; content: string }>;
  results: Array<{ item: string; state: "ready" | "failed"; detail: string }>;
}

export function sharedItemLabel(item: string): string {
  if (item.startsWith("mcp:")) return `MCP · ${item.slice(4)}`;
  return ({ permissions: "权限规则", statusline: "状态栏", rules: "规则文件", mcp: "MCP 服务" } as Record<string, string>)[item] ?? item;
}

export function sharedApplyArgs(profileId: string, item: SharedItem): string[] {
  return ["--only", `profiles:${profileId}`, "--item", item.item,
    "--source-base", item.source_base, "--target-base", item.target_base];
}

export function sharedPreviewArgs(profileId: string, items: SharedItem[]): string[] {
  return ["--only", `profiles:${profileId}`, "--selection", JSON.stringify(items.map((item) => ({
    item: item.item, source_base: item.source_base, target_base: item.target_base,
  })))];
}
