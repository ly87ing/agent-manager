import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import InboxPage from "../pages/InboxPage";
import type { Envelope } from "../lib/contract";
import { makeApplyResult, makeElement, makeEnvelope, makeFreshness } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

const empty = (domain: string) => makeEnvelope({ domain, elements: [] }, { command: `${domain}.status` });

function routes(): Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> {
  let repaired = false;
  return {
    "skills:status": async () =>
      makeEnvelope({
        domain: "skills",
        elements: [
          makeElement({ id: "skill:c", domain: "skills", spec: { url: "https://example.com/c.git" }, policy: { enabled: true }, observed: { repo_cached: true, actions: ["update"] }, freshness: makeFreshness({ state: "behind", upstream: "v2" }) }),
          makeElement({
            id: "skill:c/alpha",
            domain: "skills",
            spec: { collection: "c", name: "alpha" },
            observed: { agents: { "claude-code": repaired ? { state: "linked", actions: ["disable"] } : { state: "dead-link", actions: ["update", "disable"] } } },
          }),
          makeElement({
            id: "skill:c/beta",
            domain: "skills",
            spec: { collection: "c", name: "beta" },
            observed: { agents: { "claude-code": { state: "linked", actions: ["disable"] }, codex: { state: "not-assigned", actions: ["add"] } } },
          }),
        ],
      }, { command: "skills.status" }),
    "skills:apply": async (args) => {
      if (args.includes("update")) repaired = true;
      return makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" });
    },
    "mcp:status": empty("mcp"),
    "rules:status": empty("rules"),
    "agents:status": empty("agents"),
    "profiles:status": empty("profiles"),
    "aliases:status": empty("aliases"),
  };
}

describe("待处理：统一状态分组，每条一个处理动作，处理完回读后消失", () => {
  it("按状态分组：损坏、缺失、有新版本；每条写明属于哪一页", async () => {
    renderWithEngine(<InboxPage onOpen={vi.fn()} />, routes());
    const broken = await screen.findByTestId("inbox-group-broken");
    expect(broken).toHaveTextContent("损坏");
    expect(within(broken).getByTestId("inbox-item-skill:skill:c/alpha:broken")).toHaveTextContent("技能alpha链接损坏：");
    expect(screen.getByTestId("inbox-group-missing")).toHaveTextContent("还没有：");
    expect(screen.getByTestId("inbox-item-source:c:behind")).toHaveTextContent("可更新到 v2");
  });

  it("能直接做的在这里做：修复发出与技能页相同的动作，回读后这一条消失", async () => {
    const { calls } = renderWithEngine(<InboxPage onOpen={vi.fn()} />, routes());
    await userEvent.click(await screen.findByTestId("inbox-fix-skill:skill:c/alpha:broken"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")!.args).toEqual(["--only", "skill:c/alpha", "--action", "update", "--agent", "claude-code"]);
    await waitFor(() => expect(screen.queryByTestId("inbox-item-skill:skill:c/alpha:broken")).toBeNull());
    expect(screen.queryByTestId("inbox-group-broken")).toBeNull();
  });

  it("要看差异或做选择的跳到负责的页面：来源更新去来源页并定位，不在这里直接写", async () => {
    const onOpen = vi.fn();
    const { calls } = renderWithEngine(<InboxPage onOpen={onOpen} />, routes());
    await userEvent.click(await screen.findByTestId("inbox-fix-source:c:behind"));
    expect(onOpen).toHaveBeenCalledWith("sources:c");
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
  });

  it("什么都生效时是空态", async () => {
    const r = routes();
    r["skills:status"] = empty("skills");
    renderWithEngine(<InboxPage onOpen={vi.fn()} />, r);
    expect(await screen.findByTestId("inbox-empty")).toHaveTextContent("没有需要处理的");
  });
});
