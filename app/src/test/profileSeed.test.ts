import { describe, expect, it } from "vitest";
import { ALL_SEED_ITEMS, seedArgs, summarizeSeed, type SeedResult } from "../lib/profileSeed";

describe("从默认环境带设置", () => {
  it("参数按固定顺序给出，凭据相关的项不在可选范围里", () => {
    expect(ALL_SEED_ITEMS).toEqual(["permissions", "statusline", "rules", "mcp"]);
    expect(seedArgs("cc-work", ["mcp", "permissions"])).toEqual([
      "--only",
      "profiles:cc-work",
      "--items",
      "permissions,mcp",
    ]);
  });

  it("汇总：标题说带上了什么，说明列出没带上的原因；有失败时标出", () => {
    const results: SeedResult[] = [
      { item: "permissions", state: "copied", detail: "已带上权限规则" },
      { item: "rules", state: "failed", detail: "规则文件当前不能读写" },
      { item: "mcp", state: "copied", detail: "已复制 fs；含明文凭据或格式不认识、没有复制：gh", skipped: ["gh"] },
    ];
    const summary = summarizeSeed(results);
    expect(summary.title).toBe("已从默认环境带上：权限规则、MCP 服务");
    expect(summary.failed).toBe(true);
    expect(summary.description).toContain("规则文件：规则文件当前不能读写");
    // MCP 即便带上了也要说明哪些没带
    expect(summary.description).toContain("gh");
    expect(summary.description).not.toContain("权限规则：");

    expect(summarizeSeed([{ item: "rules", state: "exists", detail: "" }]).title).toBe("没有需要带上的设置");
    expect(summarizeSeed([{ item: "rules", state: "failed", detail: "x" }]).title).toBe("没有带上默认环境的设置");
  });
});
