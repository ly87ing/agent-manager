import { beforeEach, describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { agentManagementFixture, makeAuditData, makeElement, makeEnvelope, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/** sqliteLayer=null 表示本机还没保存过任何配置（全新安装）。 */
function routes(sqliteLayer: Record<string, unknown> | null): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline: false }) }),
    "policy:get": makeEnvelope(makePolicyData({ layers: { sqlite: sqliteLayer } }), { command: "policy.get" }),
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:claude-code",
            domain: "agents",
            spec: { display_name: "Claude Code", bin: "claude", preferred_provider: "npm", providers: { npm: { managed: true } } },
            observed: { install_source: "npm", installed: true, management: agentManagementFixture("claude") },
          }),
        ],
      },
      { command: "agents.status" },
    ),
    "aliases:status": makeEnvelope({ domain: "aliases", elements: [] }, { command: "aliases.status" }),
    "profiles:status": makeEnvelope({ domain: "profiles", elements: [] }, { command: "profiles.status" }),
    "sources:get": makeEnvelope(
      {
        sources: [
          { key: "aliases.shell", label: "Shell Aliases", format: "json", revision: "rev-1", value: { aliases: {} } },
        ],
      },
      { command: "sources.get" },
    ),
  };
}

describe("首次使用引导", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("全新安装也直接进入 Provider 列表，不展示长篇首次引导", async () => {
    renderWithEngine(<AgentsPage />, routes(null));
    await waitFor(() => expect(screen.getByTestId("connection-workbench")).toBeInTheDocument());
    expect(screen.queryByTestId("first-run-guide")).toBeNull();
  });

  it("旧版首次引导偏好不会让说明重新出现", async () => {
    window.localStorage.setItem("agent-manager:first-run-guide-dismissed", "false");
    renderWithEngine(<AgentsPage />, routes(null));
    await waitFor(() => expect(screen.getByTestId("connection-workbench")).toBeInTheDocument());
    expect(screen.queryByTestId("first-run-guide")).toBeNull();
  });

  it("已经保存过配置的机器上不出现", async () => {
    renderWithEngine(<AgentsPage />, routes({ agents: { "claude-code": { enabled: true } } }));
    await waitFor(() => expect(screen.getByTestId("connection-workbench")).toBeInTheDocument());
    expect(screen.queryByTestId("first-run-guide")).toBeNull();
  });
});
