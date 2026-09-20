import { describe, expect, it } from "vitest";
import AgentsPage from "../pages/AgentsPage";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

function routes(bin = "claude", id = "claude-code"): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "policy:get": makeEnvelope(makePolicyData(), { command: "policy.get" }),
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: `agents:${id}`,
            domain: "agents",
            spec: {
              display_name: id === "claude-code" ? "Claude Code" : id,
              bin,
              preferred_provider: "native",
              providers: {
                native: {
                  managed: true,
                  install: "curl https://example.invalid/install.sh | sh",
                  upgrade: `${bin} update`,
                  uninstall: "uninstall.sh",
                },
              },
              settings: { statusline: "builtin" },
            },
            policy: { enabled: true },
            observed: { install_source: "native", installed: true },
            state: "managed-ok",
            freshness: makeFreshness({ current: "2.0.0", upstream: "2.1.0", state: "behind" }),
            actions: [
              { id: "upgrade", risk: "network", needs_lock: true, offline_ok: false },
              { id: "uninstall", risk: "install-remove", needs_lock: true, offline_ok: false },
            ],
          }),
        ],
      },
      { command: "agents.status" },
    ),
    "profiles:status": makeEnvelope(
      { domain: "profiles", elements: [] },
      { command: "profiles.status" },
    ),
    "sources:get": makeEnvelope(
      {
        sources: [
          {
            key: "aliases.shell",
            label: "Shell Aliases",
            format: "json",
            revision: "rev-1",
            value: {
              timezone: "America/Los_Angeles",
              lang: "en_US.UTF-8",
              aliases: { [bin]: `${bin} --dangerously-skip-permissions` },
            },
          },
        ],
      },
      { command: "sources.get" },
    ),
  };
}

describe("助手页产品边界", () => {
  it("首屏只保留接入、运行设置与实例，不再展示生命周期动作、脚本或完整 alias", async () => {
    renderWithEngine(<AgentsPage />, routes());

    expect(await screen.findByTestId("connection-workbench")).toBeInTheDocument();
    expect(screen.getByTestId("agent-runtime-button")).toBeEnabled();
    expect(screen.getByText("状态栏：用量状态栏")).toBeInTheDocument();
    expect(screen.getByText("权限确认：客户端默认")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "实例" })).toBeInTheDocument();

    expect(screen.queryByText("安装与维护")).toBeNull();
    expect(screen.queryByRole("button", { name: "管理助手" })).toBeNull();
    expect(screen.queryByTestId("agents-check-updates")).toBeNull();
    expect(screen.queryByTestId("action-upgrade-agents:claude-code")).toBeNull();
    expect(screen.queryByTestId("menu-uninstall-agents:claude-code")).toBeNull();
    expect(screen.queryByText(/curl https:\/\/example\.invalid/)).toBeNull();
    expect(screen.queryByText(/claude --dangerously-skip-permissions/)).toBeNull();
  });

  it("不支持应用内接入的客户端保持只读，新增和运行设置均不可用", async () => {
    renderWithEngine(<AgentsPage />, routes("opencode", "opencode"));

    expect(await screen.findByText(/暂不支持在应用内切换接入或写入运行设置/)).toBeInTheDocument();
    expect(screen.getByTestId("add-connection-button")).toBeDisabled();
    expect(screen.getByTestId("agent-runtime-button")).toBeDisabled();
    expect(screen.queryByLabelText("当前运行设置")).toBeNull();
  });
});
