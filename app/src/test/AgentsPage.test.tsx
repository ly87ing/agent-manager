import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { agentManagementFixture, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import type { EngineElement, Envelope } from "../lib/contract";

function routes(bin = "claude", id = "claude-code"): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "policy:get": makeEnvelope(makePolicyData(), { command: "policy.get" }),
    "agents:configure": makeEnvelope(
      { domain: "agents", action: "configure", element: `agents:${id}` },
      { command: "agents.configure" },
    ),
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
  it("首屏只保留助手切换、Provider 列表与新增入口", async () => {
    renderWithEngine(<AgentsPage />, routes());

    expect(await screen.findByTestId("connection-workbench")).toBeInTheDocument();
    expect(screen.getByTestId("add-connection-button")).toBeEnabled();
    expect(screen.getByTestId("connection-card-claude-code-default")).toHaveTextContent("助手默认账号");
    expect(screen.getByText("默认实例")).toBeInTheDocument();
    expect(screen.getByText(/自动化：手动确认/)).toBeInTheDocument();
    expect(screen.getByTestId("agent-runtime-button")).toHaveTextContent("自动化与运行");
    expect(screen.getByTestId("manage-instances-button")).toHaveTextContent("隔离实例");
    const launches = screen.getByTestId("agent-launch-commands-claude-code");
    expect(within(launches).getByTestId("agent-launch-claude-code-claude")).toHaveTextContent(
      "alias claude='claude --dangerously-skip-permissions'",
    );
    expect(within(launches).getByTestId("agent-add-launch-claude-code")).toHaveTextContent("新增别名");
    expect(within(launches).getByTestId("agent-sync-launch-claude-code")).toHaveTextContent("同步到终端");
    expect(screen.getByText("待应用")).toBeInTheDocument();
    expect(screen.getByTestId("reset-connection-claude-code-default")).toHaveTextContent("重置");
    expect(screen.queryByRole("heading", { name: "实例" })).toBeNull();
    expect(screen.queryByText("本机配置状态")).toBeNull();
    expect(screen.queryByText("状态栏：用量状态栏")).toBeNull();

    expect(screen.queryByText("安装与维护")).toBeNull();
    expect(screen.queryByRole("button", { name: "管理助手" })).toBeNull();
    expect(screen.queryByTestId("agents-check-updates")).toBeNull();
    expect(screen.queryByTestId("action-upgrade-agents:claude-code")).toBeNull();
    expect(screen.queryByTestId("menu-uninstall-agents:claude-code")).toBeNull();
    expect(screen.queryByText(/curl https:\/\/example\.invalid/)).toBeNull();
  });

  it("主 Agent 的运行别名可就地审查和编辑，保存与同步仍是独立操作", async () => {
    renderWithEngine(<AgentsPage />, routes());

    await userEvent.click(await screen.findByTestId("agent-launch-edit-claude-code-claude"));
    const dialog = await screen.findByRole("dialog", { name: "编辑启动命令" });
    expect(within(dialog).getByTestId("alias-name-input")).toHaveValue("claude");
    expect(within(dialog).getByTestId("alias-command-input")).toHaveValue("claude");
    expect(within(dialog).getByTestId("alias-extra-args-input")).toHaveValue("--dangerously-skip-permissions");
  });

  it("当前 Provider 排在首位并直接标记当前使用", async () => {
    const matched = routes();
    const status = matched["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      profiles: {
        relay: {
          label: "Relay",
          base_url: "https://claude.example.com",
          auth: "token",
          token_var: "CLAUDE_RELAY_KEY",
          model: "claude-test",
        },
      },
    };
    renderWithEngine(<AgentsPage />, matched);

    const cards = await screen.findAllByTestId(/^connection-card-claude-code-/);
    expect(cards[0]).toHaveAttribute("data-testid", "connection-card-claude-code-relay");
    expect(within(cards[0]).getByText("当前使用")).toBeInTheDocument();
  });

  it("点击 Provider 后仍通过 configure 写入并以状态回读确认", async () => {
    const repairRoutes: Record<string, Envelope | (() => Promise<Envelope>)> = routes();
    const status = repairRoutes["agents:status"] as { data: { elements: EngineElement[] } };
    (status.data.elements[0].spec as Record<string, any>).settings = {
      active: "relay",
      profiles: { relay: { label: "Relay", base_url: "https://claude.example.com", auth: "token", token_var: "CLAUDE_RELAY_KEY", model: "claude-test" } },
    };
    repairRoutes["agents:configure"] = async () => {
      delete ((status.data.elements[0].spec as Record<string, any>).settings as { active?: string }).active;
      (status.data.elements[0].observed as Record<string, unknown>).current_config = {
        state: "ready",
        file: "settings.json",
        config: {
          provider: "anthropic",
          base_url: null,
          model: null,
          small_fast_model: null,
          model_aliases: { opus: null, sonnet: null, haiku: null },
          timeout_ms: null,
          credential: { configured: false, kind: null, variable: null },
          runtime: {},
        },
      };
      return makeEnvelope(
        { domain: "agents", action: "configure", element: "agents:claude-code" },
        { command: "agents.configure" },
      );
    };
    const { calls } = renderWithEngine(<AgentsPage />, repairRoutes);
    await userEvent.click(await screen.findByTestId("reset-connection-claude-code-default"));
    expect(calls.some((call) => call.command === "agents:configure")).toBe(false);
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.some((call) => call.command === "agents:configure")).toBe(true));
    expect(calls.find((call) => call.command === "agents:configure")?.args).toEqual([
      "--only",
      "agents:claude-code",
      "--use-default",
    ]);
    await waitFor(() => expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("助手默认账号"));
  });

  it("不再展示 Claude 与 Codex 之外的客户端", async () => {
    renderWithEngine(<AgentsPage />, routes("opencode", "opencode"));

    await waitFor(() => expect(screen.queryByTestId("connection-workbench")).toBeNull());
    expect(screen.queryByText("opencode 接入工作台")).toBeNull();
  });

  it("Codex 主界面不展示认证细节，运行偏好仍只读展示脱敏观察值", async () => {
    renderWithEngine(<AgentsPage />, routes("codex", "codex"));

    expect(await screen.findByTestId("connection-workbench")).toBeInTheDocument();
    expect(screen.queryByText("auth.json")).toBeNull();

    await userEvent.click(screen.getByTestId("agent-runtime-button"));
    await userEvent.click(screen.getByTestId("agent-runtime-observed-mode"));
    const observedJson = screen.getByTestId("agent-runtime-observed-json");
    expect(observedJson).toHaveTextContent('"model": "gpt-test"');
    expect(observedJson).toHaveTextContent('"codex_reasoning_effort": "high"');
    expect(observedJson).not.toHaveTextContent("token");
    expect(screen.queryByTestId("agent-runtime-submit")).toBeNull();
  });
});
