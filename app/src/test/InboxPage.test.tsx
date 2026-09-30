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

/** 分类默认收起：点开要看的那一类。 */
async function openGroup(state: string) {
  const toggle = await screen.findByTestId(`inbox-toggle-${state}`);
  if (toggle.getAttribute("aria-expanded") === "false") await userEvent.click(toggle);
}

describe("待处理：统一状态分类（可折叠），同一原因合成一行，每行一个处理动作，处理完回读后消失", () => {
  it("分类默认收起，标题给出数量和摘要；点开看明细，展开状态会记住", async () => {
    const first = renderWithEngine(<InboxPage onOpen={vi.fn()} />, routes());
    const broken = await screen.findByTestId("inbox-group-broken");
    expect(screen.getByTestId("inbox-toggle-broken")).toHaveAttribute("aria-expanded", "false");
    expect(broken).toHaveTextContent("损坏1alpha");
    expect(within(broken).queryByTestId("inbox-item-skill:skill:c/alpha:broken")).toBeNull();
    await openGroup("broken");
    expect(within(broken).getByTestId("inbox-item-skill:skill:c/alpha:broken")).toHaveTextContent("技能alpha链接损坏：");
    first.unmount();
    renderWithEngine(<InboxPage onOpen={vi.fn()} />, routes());
    expect(await screen.findByTestId("inbox-toggle-broken")).toHaveAttribute("aria-expanded", "true");
  });

  it("同一来源里还没用的技能合成一行：点开看名单，「去查看」到技能页那个来源", async () => {
    const onOpen = vi.fn();
    const r = routes();
    const base = r["skills:status"] as (args: string[]) => Promise<Envelope>;
    r["skills:status"] = async (args) => {
      const envelope = await base(args);
      const data = envelope.data as { elements: ReturnType<typeof makeElement>[] };
      for (const name of ["n1", "n2", "n3"]) {
        data.elements.push(makeElement({ id: `skill:c/${name}`, domain: "skills", spec: { collection: "c", name }, observed: { agents: {}, discovery_inbox: true } }));
      }
      return envelope;
    };
    renderWithEngine(<InboxPage onOpen={onOpen} />, r);
    await openGroup("update");
    const row = screen.getByTestId("inbox-item-group:new:c");
    expect(row).toHaveTextContent("c 里还没用的技能");
    expect(row).toHaveTextContent("3 项：n1、n2、n3");
    expect(screen.queryByTestId("inbox-item-skill:skill:c/n1:new")).toBeNull();
    await userEvent.click(within(row).getByRole("button", { expanded: false }));
    expect(screen.getByTestId("inbox-members-group:new:c")).toHaveTextContent("n1n2n3");
    await userEvent.click(screen.getByTestId("inbox-fix-group:new:c"));
    expect(onOpen).toHaveBeenCalledWith("skills:c");
  });

  it("忽略：不再计入这一类，移到底部「已忽略」，可以恢复；不写任何东西", async () => {
    const { calls } = renderWithEngine(<InboxPage onOpen={vi.fn()} />, routes());
    await openGroup("broken");
    await userEvent.click(screen.getByTestId("inbox-ignore-skill:skill:c/alpha:broken"));
    expect(screen.queryByTestId("inbox-group-broken")).toBeNull();
    const ignored = screen.getByTestId("inbox-ignored");
    expect(ignored).toHaveTextContent("已忽略 1");
    await userEvent.click(screen.getByTestId("inbox-toggle-ignored"));
    await userEvent.click(screen.getByTestId("inbox-restore-skill:skill:c/alpha:broken"));
    expect(await screen.findByTestId("inbox-group-broken")).toBeInTheDocument();
    expect(screen.queryByTestId("inbox-ignored")).toBeNull();
    expect(calls.some((call) => !call.command.endsWith(":status"))).toBe(false);
  });

  it("全部忽略一类", async () => {
    renderWithEngine(<InboxPage onOpen={vi.fn()} />, routes());
    await userEvent.click(await screen.findByTestId("inbox-ignore-all-missing"));
    expect(screen.queryByTestId("inbox-group-missing")).toBeNull();
    expect(screen.getByTestId("inbox-group-broken")).toBeInTheDocument();
  });

  it("能直接做的在这里做：修复发出与技能页相同的动作，回读后这一条消失", async () => {
    const { calls } = renderWithEngine(<InboxPage onOpen={vi.fn()} />, routes());
    await openGroup("broken");
    await userEvent.click(await screen.findByTestId("inbox-fix-skill:skill:c/alpha:broken"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")!.args).toEqual(["--only", "skill:c/alpha", "--action", "update", "--agent", "claude-code"]);
    await waitFor(() => expect(screen.queryByTestId("inbox-item-skill:skill:c/alpha:broken")).toBeNull());
    expect(screen.queryByTestId("inbox-group-broken")).toBeNull();
  });

  it("要看差异或做选择的跳到负责的页面：来源更新去来源页并定位，不在这里直接写", async () => {
    const onOpen = vi.fn();
    const { calls } = renderWithEngine(<InboxPage onOpen={onOpen} />, routes());
    await openGroup("update");
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
