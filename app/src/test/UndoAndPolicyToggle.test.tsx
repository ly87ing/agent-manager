import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import AliasesPage from "../pages/AliasesPage";
import type { Envelope } from "../lib/contract";
import { agentManagementFixture, makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function agentRoutes(): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "policy:get": makeEnvelope(
      makePolicyData({
        layers: {
          sqlite: { agents: { definitions: { "claude-code": { display_name: "Claude Code", bin: "claude" } } } },
        },
      }),
      { command: "policy.get" },
    ),
    "policy:set": makeEnvelope(makePolicyData(), { command: "policy.set" }),
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:claude-code",
            domain: "agents",
            spec: { display_name: "Claude Code", bin: "claude", providers: {} },
            policy: { enabled: true },
            observed: { install_source: null, installed: false, management: agentManagementFixture("claude") },
            state: "missing",
            freshness: makeFreshness(),
            actions: [],
          }),
        ],
      },
      { command: "agents.status" },
    ),
    "profiles:status": makeEnvelope({ domain: "profiles", elements: [] }, { command: "profiles.status" }),
    "agents:apply": makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }),
    "sources:get": makeEnvelope(
      {
        sources: [
          {
            key: "aliases.shell",
            label: "Shell aliases",
            format: "json",
            revision: "rev-1",
            value: { timezone: "America/Los_Angeles", lang: "en_US.UTF-8", aliases: { claude: "claude" } },
          },
        ],
      },
      { command: "sources.get" },
    ),
    "sources:set": makeEnvelope({ key: "aliases.shell", revision: "rev-2" }, { command: "sources.set" }),
    "aliases:status": makeEnvelope({ domain: "aliases", elements: [] }, { command: "aliases.status" }),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
  };
}

describe("助手启停", () => {
  it("助手主路径不再提供启停或安装维护菜单，不会误触定义与生命周期写入", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, agentRoutes());
    await screen.findByTestId("connection-workbench");

    expect(screen.queryByTestId("agent-menu-agents:claude-code")).toBeNull();
    expect(screen.queryByTestId("policy-enabled-agents:claude-code")).toBeNull();
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(calls.some((call) => call.command === "agents:apply")).toBe(false);
  });
});

describe("移除后的撤销", () => {
  it("移除快捷命令给出撤销，点击后原样写回", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, agentRoutes());
    await userEvent.click(await screen.findByTestId("remove-alias-claude"));
    await userEvent.click(await screen.findByRole("button", { name: "确认移除" }));

    const sourceWrites = () => calls.filter((call) => call.command === "sources:set");
    await waitFor(() => expect(sourceWrites()).toHaveLength(1));
    const removedCall = sourceWrites()[0];
    const removed = JSON.parse(removedCall.args[removedCall.args.indexOf("--value") + 1]);
    expect(removed.value.aliases.claude).toBeUndefined();

    await userEvent.click(await screen.findByRole("button", { name: "撤销" }));
    await waitFor(() => expect(sourceWrites()).toHaveLength(2));
    const restoreCall = sourceWrites()[1];
    const restored = JSON.parse(restoreCall.args[restoreCall.args.indexOf("--value") + 1]);
    expect(restored.value.aliases.claude).toBe("claude");
  });

  it("助手定义移除入口随完整定义维护一起撤下，保留定义且零写入", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, agentRoutes());
    await screen.findByTestId("connection-workbench");

    expect(screen.queryByRole("menuitem", { name: "移除管理" })).toBeNull();
    expect(screen.queryByRole("button", { name: "添加助手" })).toBeNull();
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });
});
