import { describe, expect, it } from "vitest";
import { rulesFillRows } from "../lib/rulesFill";
import { makeAction, makeElement } from "./fixtures";

function rule(id: string, group: string, kind: string, observed: Record<string, unknown>, writable = true) {
  return makeElement({
    id,
    domain: "rules",
    spec: { name: id, group, kind, profile_id: null, target: "~/x" },
    observed: { content: null, base: null, ...observed },
    actions: writable ? [makeAction("write")] : [],
  });
}

describe("规则补齐", () => {
  it("缺文件的账号从同类主账号复制；主账号没内容、另一种助手、已有文件、不可写的都不补", () => {
    const rows = rulesFillRows([
      rule("rules:claude_global", "global-rules", "claude", { content: "# 规则\n", base: "b1" }),
      rule("rules:codex_global", "global-rules", "codex", { content: "", base: "b2" }),
      rule("rules:profile:work", "profile-rules", "claude", { base: "missing" }),
      rule("rules:profile:cx", "profile-rules", "codex", { base: "missing" }),
      rule("rules:profile:own", "profile-rules", "claude", { content: "自己的\n", base: "b3" }),
      rule("rules:profile:link", "profile-rules", "claude", { base: null }, false),
    ]);
    expect(rows).toEqual([{ elementId: "rules:profile:work", name: "rules:profile:work", from: "rules:claude_global" }]);
  });
});
