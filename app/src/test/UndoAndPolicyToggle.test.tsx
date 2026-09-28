import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import AliasesPage from "../pages/AliasesPage";
import type { Envelope } from "../lib/contract";
import { agentManagementFixture, makeAliasStatus, makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
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
    "aliases:status": makeEnvelope(makeAliasStatus({ claude: "claude" }), { command: "aliases.status" }),
    "aliases:preview": makeEnvelope({
      files: [{ target: "aliases:zshrc", path: "/isolated/.zshrc", exists: true, content: "# 保留用户内容\n" }],
      skipped: ["aliases:zprofile"],
    }, { command: "aliases.preview" }),
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
  it("移除快捷命令直接写回终端配置文件：先确认，写进文件的动作不给撤销", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, agentRoutes());
    await userEvent.click(await screen.findByTestId("remove-alias-claude"));
    expect(await screen.findByTestId("alias-confirm-file-aliases:zshrc")).toHaveTextContent("# 保留用户内容");
    await userEvent.click(await screen.findByRole("button", { name: "确认移除" }));

    const writes = () => calls.filter((call) => call.command === "aliases:apply");
    await waitFor(() => expect(writes()).toHaveLength(1));
    const args = writes()[0].args;
    expect(args[args.indexOf("--remove") + 1]).toBe("claude");
    expect(calls.find((call) => call.command === "aliases:preview")?.args).toEqual(args);
    expect(await screen.findByText(/移除快捷命令 claude/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "撤销" })).toBeNull();
  });

  it("助手定义移除入口随完整定义维护一起撤下，保留定义且零写入", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, agentRoutes());
    await screen.findByTestId("connection-workbench");

    expect(screen.queryByRole("menuitem", { name: "移除管理" })).toBeNull();
    expect(screen.queryByRole("button", { name: "添加助手" })).toBeNull();
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });
});
