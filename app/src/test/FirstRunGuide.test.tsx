import { beforeEach, describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { makeAuditData, makeElement, makeEnvelope, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
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
            observed: { providers: ["npm"], classified: { managed: ["npm"], deprecated: [], system_only: [], unsupported: [] }, installed: true, preferred_active: true },
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

  it("本机还没保存过配置时出现，并能直接去接管中心", async () => {
    const onGoTakeover = vi.fn();
    renderWithEngine(<AgentsPage onGoTakeover={onGoTakeover} />, routes(null));

    const guide = await screen.findByTestId("first-run-guide");
    expect(guide).toHaveTextContent("三步开始用");
    // 三步里要说清「哪一步才真的改文件」
    expect(guide).toHaveTextContent("点「立即同步」才真正写入");

    await userEvent.click(screen.getByTestId("first-run-goto-takeover"));
    expect(onGoTakeover).toHaveBeenCalledTimes(1);
  });

  it("点过「不再提示」后消失并记住", async () => {
    const { unmount } = renderWithEngine(<AgentsPage />, routes(null));
    await screen.findByTestId("first-run-guide");

    await userEvent.click(screen.getByTestId("first-run-dismiss"));
    expect(screen.queryByTestId("first-run-guide")).toBeNull();

    unmount();
    renderWithEngine(<AgentsPage />, routes(null));
    await waitFor(() => expect(screen.getByTestId("agents-search")).toBeInTheDocument());
    expect(screen.queryByTestId("first-run-guide")).toBeNull();
  });

  it("已经保存过配置的机器上不出现", async () => {
    renderWithEngine(<AgentsPage />, routes({ agents: { "claude-code": { enabled: true } } }));
    await waitFor(() => expect(screen.getByTestId("agents-search")).toBeInTheDocument());
    expect(screen.queryByTestId("first-run-guide")).toBeNull();
  });
});
