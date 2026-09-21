import { describe, expect, it } from "vitest";
import AgentsPage from "../pages/AgentsPage";
import { agentManagementFixture, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
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
            observed: {
              install_source: "native",
              installed: true,
              management: bin === "claude" || bin === "codex" ? agentManagementFixture(bin) : null,
              auth: bin === "codex"
                ? { state: "ready", authenticated: true, mode: "chatgpt", storage: "file", file: "auth.json" }
                : null,
              current_config: bin === "codex"
                ? {
                    state: "ready",
                    file: "config.toml",
                    config: {
                      provider: "relay",
                      base_url: "https://relay.example.com/v1",
                      model: "gpt-test",
                      credential: { configured: true, kind: "env", variable: "CODEX_RELAY_KEY" },
                      runtime: { codex_reasoning_effort: "high" },
                    },
                  }
                : {
                    state: "ready",
                    file: "settings.json",
                    config: {
                      provider: "anthropic",
                      base_url: "https://claude.example.com",
                      model: "claude-test",
                      credential: { configured: true, kind: "token", variable: null },
                      runtime: { claude_effort_level: "high" },
                    },
                  },
            },
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
    const current = screen.getByTestId("agent-current-config");
    expect(current).toHaveTextContent("本机当前配置");
    expect(current).toHaveTextContent("https://claude.example.com");
    expect(current).toHaveTextContent("凭据：已配置（值已隐藏）");

    expect(screen.queryByText("安装与维护")).toBeNull();
    expect(screen.queryByRole("button", { name: "管理助手" })).toBeNull();
    expect(screen.queryByTestId("agents-check-updates")).toBeNull();
    expect(screen.queryByTestId("action-upgrade-agents:claude-code")).toBeNull();
    expect(screen.queryByTestId("menu-uninstall-agents:claude-code")).toBeNull();
    expect(screen.queryByText(/curl https:\/\/example\.invalid/)).toBeNull();
    expect(screen.queryByText(/claude --dangerously-skip-permissions/)).toBeNull();
  });

  it("不再展示 Claude 与 Codex 之外的客户端", async () => {
    renderWithEngine(<AgentsPage />, routes("opencode", "opencode"));

    expect(await screen.findByRole("heading", { name: "实例" })).toBeInTheDocument();
    expect(screen.queryByTestId("connection-workbench")).toBeNull();
    expect(screen.queryByText("opencode 接入工作台")).toBeNull();
  });

  it("Codex 接入切换明确保留官方认证文件", async () => {
    renderWithEngine(<AgentsPage />, routes("codex", "codex"));

    const boundary = await screen.findByTestId("codex-auth-boundary");
    expect(boundary).toHaveTextContent("切换接入只会更新 config.toml");
    expect(boundary).toHaveTextContent("auth.json 中的 Codex 官方登录会保留");
    expect(boundary).toHaveTextContent("只有单独确认的认证操作才能修改");
    expect(screen.getByTestId("agent-auth-status-codex")).toHaveTextContent("官方登录：已识别（chatgpt）");
    expect(screen.getByTestId("agent-auth-status-codex")).toHaveTextContent("auth.json");
    expect(screen.getByTestId("agent-codex-auth-login")).toHaveTextContent("复制登录命令");
    expect(screen.getByTestId("agent-codex-auth-logout")).toHaveTextContent("复制退出命令");
    expect(screen.getByText("只复制，不执行")).toBeInTheDocument();
    const current = screen.getByTestId("agent-current-config");
    expect(current).toHaveTextContent("config.toml · 只读");
    expect(current).toHaveTextContent("变量 CODEX_RELAY_KEY");

    await userEvent.click(screen.getByTestId("agent-runtime-button"));
    await userEvent.click(screen.getByTestId("agent-runtime-observed-mode"));
    const observedJson = screen.getByTestId("agent-runtime-observed-json");
    expect(observedJson).toHaveTextContent('"model": "gpt-test"');
    expect(observedJson).toHaveTextContent('"codex_reasoning_effort": "high"');
    expect(observedJson).not.toHaveTextContent("token");
    expect(screen.queryByTestId("agent-runtime-submit")).toBeNull();
  });
});
