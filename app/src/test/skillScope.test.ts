import { describe, expect, it } from "vitest";
import { canAutoInclude, readScope, toSkillPaths } from "../lib/skillScope";

const nested = [
  { name: "alpha", path: "skills/alpha" },
  { name: "beta", path: "skills/beta" },
];
const top = [
  { name: "alpha", path: "alpha" },
  { name: "beta", path: "beta" },
];

describe("集合技能范围换算", () => {
  it("skill_paths 是装技能的目录：里面的技能全算在用，且以后新增的自动算进来，不当成找不到的技能", () => {
    expect(readScope(["skills"], nested)).toEqual({ selected: ["alpha", "beta"], auto: true, missing: [], entryOf: {} });
  });

  it("逐个列出的技能（含单段旧写法）只选中它们；来源里没有的列为缺失", () => {
    const scope = readScope(["alpha", "gone"], nested);
    expect(scope.selected).toEqual(["alpha"]);
    expect(scope.auto).toBe(false);
    expect(scope.missing).toEqual(["gone"]);
    expect(scope.entryOf).toEqual({ alpha: "alpha" });
  });

  it("没限定（null）：全部在用并自动包含", () => {
    expect(readScope(null, top)).toMatchObject({ selected: ["alpha", "beta"], auto: true });
  });

  it("原样保存不改写配置：目录项、null、逐个列出的旧写法都保持原样", () => {
    expect(toSkillPaths({ available: nested, selected: ["alpha", "beta"], auto: true, keptMissing: [], initialPaths: ["skills"] })).toEqual(["skills"]);
    expect(toSkillPaths({ available: top, selected: ["alpha", "beta"], auto: true, keptMissing: [], initialPaths: null })).toBeNull();
    expect(toSkillPaths({ available: nested, selected: ["alpha"], auto: false, keptMissing: [], initialPaths: ["alpha"] })).toEqual(["alpha"]);
  });

  it("从目录模式取消一个：改成逐个列出剩下的技能，用它们的实际路径", () => {
    expect(toSkillPaths({ available: nested, selected: ["beta"], auto: true, keptMissing: [], initialPaths: ["skills"] })).toEqual(["skills/beta"]);
  });

  it("从逐个列出改回全选并自动包含：按技能所在目录写，顶层的写 null", () => {
    expect(toSkillPaths({ available: nested, selected: ["alpha", "beta"], auto: true, keptMissing: [], initialPaths: ["alpha"] })).toEqual(["skills"]);
    expect(toSkillPaths({ available: top, selected: ["alpha", "beta"], auto: true, keptMissing: [], initialPaths: ["alpha"] })).toBeNull();
  });

  it("顶层和子目录混放时不能自动包含，全选也逐个列出", () => {
    const mixed = [{ name: "alpha", path: "alpha" }, { name: "beta", path: "skills/beta" }];
    expect(canAutoInclude(mixed)).toBe(false);
    expect(toSkillPaths({ available: mixed, selected: ["alpha", "beta"], auto: true, keptMissing: [], initialPaths: ["alpha"] })).toEqual(["alpha", "skills/beta"]);
  });
});
