import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import { ReadAgain } from "../components/PageState";
import { useEngineQuery } from "../lib/queries";
import McpPage from "../pages/McpPage";
import RulesPage from "../pages/RulesPage";
import SkillsPage from "../pages/SkillsPage";
import SourcesPage from "../pages/SourcesPage";
import { makeEnvelope, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

describe("Explicit resource reads", () => {
  it("reads changed data on demand and keeps the button disabled until completion", async () => {
    let finish!: () => void;
    let reads = 0;
    const gate = new Promise<void>((resolve) => { finish = resolve; });
    function Probe() {
      const query = useEngineQuery<{ value: string }>("rules:status");
      return <><output>{query.data?.data.value}</output><ReadAgain queries={[query]} /></>;
    }
    const { calls } = renderWithEngine(<Probe />, { "rules:status": async () => {
      if (++reads > 1) await gate;
      return makeEnvelope({ value: reads === 1 ? "original" : "external edit" });
    } });
    await screen.findByText("original");
    expect(screen.getByText(/读取时间：/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "重新读取页面" }));
    expect(screen.getByRole("button", { name: "重新读取页面" })).toBeDisabled();
    finish();
    await screen.findByText("external edit");
    expect(screen.getByRole("button", { name: "重新读取页面" })).toBeEnabled();
    expect(calls.map((call) => call.command)).toEqual(["rules:status", "rules:status"]);
  });

  it.each([
    [SkillsPage, "技能", ["skills:status", "programs:status"]],
    [McpPage, "MCP", ["mcp:status", "programs:status"]],
    [RulesPage, "规则", ["rules:status"]],
    [SourcesPage, "来源", ["skills:status", "rules:status", "policy:get"]],
  ] as const)("refreshes a page's own inventories without writes", async (Page, title, commands) => {
    const empty = makeEnvelope({ elements: [], installed: [], recommendations: [], source_rules: [], templates: [], targets: [], suggested_sources: [] });
    const { calls } = renderWithEngine(<Page />, {
      "skills:status": empty, "mcp:status": empty, "rules:status": empty,
      "programs:status": empty, "policy:get": makeEnvelope(makePolicyData()),
    });
    await screen.findByRole("heading", { name: title });
    await waitFor(() => expect(screen.getByRole("button", { name: "重新读取页面" })).toBeEnabled());
    const before = calls.length;
    await userEvent.click(screen.getByRole("button", { name: "重新读取页面" }));
    await waitFor(() => expect(calls.slice(before).map((call) => call.command).sort()).toEqual([...commands].sort()));
    expect(calls.some((call) => /:(apply|set|save|import|refresh-upstream)$/.test(call.command))).toBe(false);
  });
});
