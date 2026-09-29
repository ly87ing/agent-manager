import { describe, expect, it } from "vitest";
import { addToAllRows, fillRows, versionUpdateRows } from "../lib/skillFill";
import { makeElement } from "./fixtures";

function skill(name: string, agents: Record<string, { state: string; assigned?: boolean; actions?: string[]; version?: string; available_version?: string }>) {
  return makeElement({
    id: `skill:repo/${name}`,
    domain: "skills",
    spec: { collection: "repo", name },
    observed: { agents },
  });
}

const collection = makeElement({ id: "skill:repo", domain: "skills", spec: { url: "https://example.com/repo.git" }, observed: {} });
const notAssigned = { state: "not-assigned", assigned: false, actions: ["add"] };

describe("技能补齐", () => {
  it("在用的技能补到缺它的账号；停用是例外，新发现的技能不自动加", () => {
    const rows = fillRows([
      collection,
      skill("used", { "claude-code": { state: "linked", assigned: true }, work: notAssigned, ds: { state: "disabled", assigned: true, actions: ["enable"] } }),
      skill("fresh", { "claude-code": notAssigned, work: notAssigned }),
      skill("off-everywhere", { "claude-code": { state: "disabled", assigned: true }, work: notAssigned }),
    ]);
    expect(rows.map((row) => [row.elementId, row.agent, row.action])).toEqual([["skill:repo/used", "work", "add"]]);
  });

  it("只补指定账号（新建账号后只动它自己）", () => {
    const elements = [skill("used", { "claude-code": { state: "linked", assigned: true }, work: notAssigned, other: notAssigned })];
    expect(fillRows(elements, ["work"]).map((row) => row.agent)).toEqual(["work"]);
  });

  it("没有 add 动作的格子（如助手停用、来源不可用）不补", () => {
    const elements = [skill("used", { "claude-code": { state: "linked", assigned: true }, work: { state: "not-assigned", assigned: false, actions: [] } })];
    expect(fillRows(elements)).toEqual([]);
  });

  it("添加新发现的技能：一次加到全部可加的账号", () => {
    expect(addToAllRows(skill("fresh", { "claude-code": notAssigned, work: notAssigned, ds: { state: "extra-link", actions: ["add", "remove"] } })).map((row) => row.agent))
      .toEqual(["claude-code", "work"]);
  });

  it("全部更新：只把在用、来源已有新版本的技能换到新版本；损坏链接、停用、已是最新的不在内", () => {
    const up = { actions: ["update", "disable"], assigned: true };
    const rows = versionUpdateRows([
      collection,
      skill("old", { "claude-code": { state: "linked", version: "v1", available_version: "v2", ...up }, work: { state: "linked", version: "v2", available_version: "v2", ...up } }),
      skill("broken", { "claude-code": { state: "dead-link", version: "v1", available_version: "v2", ...up } }),
      skill("off", { "claude-code": { state: "disabled", version: "v1", available_version: "v2", assigned: true, actions: ["enable", "update"] } }),
    ]);
    expect(rows.map((row) => [row.elementId, row.agent, row.action])).toEqual([["skill:repo/old", "claude-code", "update"]]);
  });
});
