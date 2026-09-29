import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import SourcesPage from "../pages/SourcesPage";
import type { Envelope } from "../lib/contract";
import { makeApplyResult, makeElement, makeEnvelope, makeFreshness, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

function routes(): Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> {
  return {
    "skills:status": makeEnvelope({
      domain: "skills",
      elements: [
        makeElement({ id: "skill:agent-skills", domain: "skills", spec: { url: "https://github.com/example/agent-skills.git" }, policy: { enabled: true }, observed: { repo_cached: true, actions: ["update", "disable", "remove"] }, freshness: makeFreshness({ state: "behind" }) }),
        makeElement({ id: "skill:agent-skills/alpha", domain: "skills", spec: { collection: "agent-skills", name: "alpha" }, observed: { agents: {} } }),
        makeElement({ id: "skill:local-pack", domain: "skills", spec: { source_path: "~/pack" }, policy: { enabled: false }, observed: { repo_cached: true, actions: ["update", "enable", "remove"] } }),
      ],
      recommendations: [],
    }, { command: "skills.status" }),
    "rules:status": makeEnvelope({ domain: "rules", elements: [], templates: [], source_rules: [{ id: "agent-skills/core", collection: "agent-skills", name: "core", body: "x", revision: "r", local: false }], suggested_sources: [] }, { command: "rules.status" }),
    "policy:get": makeEnvelope(makePolicyData({ effective: { rules: { sources: { notes: { path: "~/notes/rules", enabled: false } } } } }), { command: "policy.get" }),
    "policy:set": makeEnvelope({}, { command: "policy.set" }),
    "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
    "system:refresh-upstream": makeEnvelope({}, { command: "system.refresh-upstream" }),
  };
}

describe("来源页", () => {
  it("技能集合与规则来源在一张表里，写明提供什么和状态，操作是看得见的文字按钮", async () => {
    renderWithEngine(<SourcesPage />, routes());
    const git = await screen.findByTestId("source-row-agent-skills");
    expect(git).toHaveTextContent("git · https://github.com/example/agent-skills.git");
    expect(git).toHaveTextContent("1 个技能 · 1 条规则");
    expect(git).toHaveTextContent("可更新");
    for (const label of ["更新", "编辑", "停用", "移除"]) expect(within(git).getByRole("button", { name: label })).toBeVisible();
    const local = screen.getByTestId("source-row-local-pack");
    expect(local).toHaveTextContent("已停用");
    expect(within(local).getByRole("button", { name: "启用" })).toBeInTheDocument();
    const notes = screen.getByTestId("source-row-notes");
    expect(notes).toHaveTextContent("规则 · ~/notes/rules");
    expect(notes).toHaveTextContent("已停用");
  });

  it("停用、启用、移除都走对应命令；移除先确认", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, routes());
    await userEvent.click(await screen.findByTestId("source-toggle-agent-skills"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")!.args).toEqual(["--only", "skill:agent-skills", "--action", "disable"]);

    await userEvent.click(screen.getByTestId("source-toggle-notes"));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const set = calls.find((call) => call.command === "policy:set")!;
    expect(set.args).toEqual(["--layer", "local", "--key", "rules.sources.notes.enabled", "--value", "true"]);

    await userEvent.click(screen.getByTestId("source-remove-local-pack"));
    const dialog = await screen.findByRole("dialog", { name: "移除来源 local-pack" });
    // 本机目录：没有清理下载的选项，目录不动
    expect(within(dialog).queryByTestId("remove-collection-purge")).toBeNull();
    expect(dialog).toHaveTextContent("目录里的文件一律不动");
    expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(1);
  });

  it("添加：git 仓库或目录打开添加来源表单，规则文件或目录打开添加规则来源", async () => {
    renderWithEngine(<SourcesPage />, routes());
    await userEvent.click(await screen.findByTestId("source-add-collection"));
    expect(await screen.findByRole("dialog", { name: "添加来源" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await userEvent.click(screen.getByTestId("source-add-rule"));
    expect(await screen.findByRole("dialog", { name: "添加规则来源" })).toBeInTheDocument();
  });

  it("推荐来源在表里：「添加…」打开预填好的表单，保存前不写；「不用了」就不再出现", async () => {
    localStorage.clear();
    const r = routes();
    const status = r["skills:status"] as Envelope<{ recommendations: unknown[] }>;
    status.data!.recommendations = [
      { name: "team-skills", description: "团队的技能", url: "https://github.com/example/team.git", skill_paths: ["skills"], added: false, definition: { url: "https://github.com/example/team.git", skill_paths: ["skills"] } },
      { name: "agent-skills", description: "已添加", url: "https://github.com/example/agent-skills.git", skill_paths: ["skills"], added: true, definition: {} },
    ];
    const { calls } = renderWithEngine(<SourcesPage />, r);
    const row = await screen.findByTestId("source-suggest-team-skills");
    expect(screen.queryByTestId("source-suggest-agent-skills")).toBeNull();
    await userEvent.click(within(row).getByRole("button", { name: "添加…" }));
    const dialog = await screen.findByRole("dialog", { name: "添加来源" });
    expect(within(dialog).getByDisplayValue("team-skills")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    await userEvent.click(within(screen.getByTestId("source-suggest-team-skills")).getByRole("button", { name: "不用了" }));
    expect(screen.queryByTestId("source-suggest-team-skills")).toBeNull();
    expect(calls.filter((call) => call.command !== "skills:status" && call.command !== "rules:status" && call.command !== "policy:get")).toEqual([]);
  });

  it("添加推荐来源只保存定义（不限账号、不分配），不下载", async () => {
    localStorage.clear();
    const r = routes();
    const status = r["skills:status"] as Envelope<{ recommendations: unknown[] }>;
    status.data!.recommendations = [
      { name: "team-skills", description: "团队的技能", url: "https://github.com/example/team.git", skill_paths: ["skills"], added: false, definition: { url: "https://github.com/example/team.git", skill_paths: ["skills"] } },
    ];
    const { calls } = renderWithEngine(<SourcesPage />, r);
    await userEvent.click(await screen.findByTestId("source-suggest-add-team-skills"));
    const dialog = await screen.findByRole("dialog", { name: "添加来源" });
    await userEvent.click(within(dialog).getByTestId("collection-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const write = calls.find((call) => call.command === "policy:set")!;
    expect(write.args[write.args.indexOf("--key") + 1]).toBe("skills.repositories.team-skills");
    const definition = JSON.parse(write.args[write.args.indexOf("--value") + 1]);
    expect(definition.url).toBe("https://github.com/example/team.git");
    expect(definition.agents ?? null).toBeNull();
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
  });
});
