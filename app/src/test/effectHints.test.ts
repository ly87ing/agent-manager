import { describe, expect, it } from "vitest";
import { effectHint } from "../lib/effectHints";

describe("生效提示", () => {
  it("按目标给出何时生效", () => {
    expect(effectHint("aliases:apply")).toContain("新开一个终端窗口");
    expect(effectHint("profiles:apply")).toContain("下次用这个实例的命令启动");
    expect(effectHint("mcp:apply")).toContain("下次启动");
    expect(effectHint("hooks:apply")).toContain("下次启动");
    expect(effectHint("skills:apply")).toContain("技能目录");
  });

  it("规则按目标区分助手", () => {
    expect(effectHint("rules:apply", ["--only", "rules:codex_global"])).toContain("Codex");
    expect(effectHint("rules:apply", ["--only", "rules:claude_global"])).toContain("Claude Code");
  });

  it("实例的规则目标看不出助手种类，给通用说法而不是猜成 Claude Code", () => {
    const hint = effectHint("rules:apply", ["--only", "rules:profile:cx-work", "--base", "missing", "--recommended"]);
    expect(hint).toContain("助手下次启动时读取新规则");
    expect(hint).not.toContain("Claude Code");
    expect(hint).not.toContain("Codex");
  });

  it("安装类动作本身就是即时结果，不额外解释", () => {
    expect(effectHint("agents:apply")).toBeNull();
    expect(effectHint("policy:set")).toBeNull();
    expect(effectHint("runs:clear")).toBeNull();
  });
});
