import { describe, expect, it } from "vitest";
import { rollbackPlan } from "../lib/rollback";

const op = (operation_type: string, target: string | null, detail: unknown = {}) => ({ id: "x", operation_type, target, detail, created_at: "2026-09-29T00:00:00Z" });

describe("操作记录的撤回", () => {
  it("技能：首次启用撤回为取消分配，启用与停用互为撤回，只动那一个账号", () => {
    expect(rollbackPlan(op("skills:add", "agent-skills/verification@claude-code"))).toEqual({
      kind: "skills", label: "取消分配 verification @ claude-code", elementId: "skill:agent-skills/verification", agent: "claude-code", action: "remove",
    });
    expect(rollbackPlan(op("skills:disable", "c/s@work"))).toMatchObject({ action: "enable", agent: "work" });
    expect(rollbackPlan(op("skills:enable", "c/s@work"))).toMatchObject({ action: "disable" });
  });

  it("规则：另存过备份模板的覆盖可以写回备份；没有备份的说明原因", () => {
    expect(rollbackPlan(op("rules:apply", "rules:claude_global", { action: "write", backup_template: "tpl-1" }))).toEqual({
      kind: "rules", label: "把 rules:claude_global 恢复成覆盖前的内容", target: "rules:claude_global", template: "tpl-1",
    });
    expect(rollbackPlan(op("rules:apply", "rules:claude_global", { action: "write", backup_template: null }))).toMatchObject({ kind: "none" });
  });

  it("没有保留原内容的写入不能自动撤回", () => {
    for (const type of ["mcp:apply", "aliases:apply", "profiles:apply-definition", "policy:set", "skills:update"]) {
      expect(rollbackPlan(op(type, "x")).kind).toBe("none");
    }
  });
});
