import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { toast } from "sonner";
import AgentsPage from "../pages/AgentsPage";
import AliasesPage from "../pages/AliasesPage";
import type { Envelope } from "../lib/contract";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

function agentRoutes(): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "policy:get": makeEnvelope(
      makePolicyData({
        layers: {
          sqlite: { agents: { definitions: { "my-agent": { display_name: "My Agent", bin: "my-agent" } } } },
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
            id: "agents:my-agent",
            domain: "agents",
            spec: { display_name: "My Agent", bin: "my-agent", providers: {} },
            policy: { enabled: true },
            observed: { install_source: null, installed: false },
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
  it("停用只写本机偏好、说明影响范围，不触发任何安装或同步", async () => {
    const successSpy = vi.spyOn(toast, "success");
    const { calls } = renderWithEngine(<AgentsPage />, agentRoutes());
    await userEvent.click(await screen.findByTestId("agent-menu-agents:my-agent"));
    await userEvent.click(screen.getByTestId("policy-enabled-agents:my-agent"));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const write = calls.find((call) => call.command === "policy:set")!;
    expect(write.args[write.args.indexOf("--key") + 1]).toBe("agents.my-agent.enabled");
    expect(write.args[write.args.indexOf("--value") + 1]).toBe("false");
    expect(calls.some((call) => call.command === "agents:apply")).toBe(false);
    const lastCall = successSpy.mock.calls[successSpy.mock.calls.length - 1];
    expect(String((lastCall?.[1] as { description?: string } | undefined)?.description)).toContain("已安装的软件不受影响");
    successSpy.mockRestore();
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

  it("移除助手定义给出撤销，点击后把定义原样加回", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, agentRoutes());
    await userEvent.click(await screen.findByTestId("agent-menu-agents:my-agent"));
    await userEvent.click(screen.getByRole("menuitem", { name: "移除管理" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByRole("textbox"), "my-agent");
    await userEvent.click(within(dialog).getByRole("button", { name: "确认移除" }));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const removal = calls.find((call) => call.command === "policy:set")!;
    expect(JSON.parse(removal.args[removal.args.indexOf("--value") + 1])).toEqual({ _delete: true });

    await userEvent.click(await screen.findByRole("button", { name: "撤销" }));
    const policyWrites = () => calls.filter((call) => call.command === "policy:set");
    await waitFor(() => expect(policyWrites()).toHaveLength(2));
    const restore = policyWrites()[1];
    const restored = JSON.parse(restore.args[restore.args.indexOf("--value") + 1]);
    expect(restored.display_name).toBe("My Agent");
    expect(restored._delete).toBe(false);
    expect(restored._replace).toBe(true);
  });
});
