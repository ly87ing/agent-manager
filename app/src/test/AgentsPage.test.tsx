import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { agentManagementFixture, makeAliasStatus, makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import { editorText, editorView } from "./code-editor";
import type { EngineElement, Envelope } from "../lib/contract";

const BASE = "c".repeat(64);
/** 实际配置对不上任何已保存接入：引擎报「未保存」。 */
const UNSAVED = { profile: null, default: false, last_written: null, unverified: [], drifted_from: null };

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
                    base: BASE,
                    current: UNSAVED,
                  }
                : {
                    state: "ready",
                    file: "settings.json",
                    config: {
                      provider: "anthropic",
                      base_url: "https://claude.example.com",
                      model: "claude-test",
                      credential: { configured: true, kind: "token", variable: null, saved_var: null },
                      runtime: { claude_effort_level: "high" },
                    },
                    base: BASE,
                    current: UNSAVED,
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
    "aliases:status": makeEnvelope(makeAliasStatus({ [bin]: `${bin} --dangerously-skip-permissions` }), {
      command: "aliases.status",
    }),
  };
}

describe("助手页产品边界", () => {
  it("首屏只保留助手切换、Provider 列表与新增入口", async () => {
    renderWithEngine(<AgentsPage />, routes());

    expect(await screen.findByTestId("connection-workbench")).toBeInTheDocument();
    expect(screen.getByTestId("add-connection-button")).toBeEnabled();
    expect(screen.getByTestId("connection-card-claude-code-default")).toHaveTextContent("官方账号");
    expect(screen.getAllByText("默认环境").length).toBeGreaterThan(0);
    // 当前以客户端实际配置为准：状态逐行说清账号、登录、模型与自动化，不报「尚未生效」
    expect(screen.getByTestId("default-env-account")).toHaveTextContent("第三方接入 · https://claude.example.com");
    expect(screen.getByTestId("default-env-account")).toHaveTextContent("和官方账号混在主配置里");
    expect(screen.getByTestId("default-env-login")).toHaveTextContent("现在用的是配置里的第三方令牌，不是官方登录");
    expect(screen.getByTestId("default-env-model")).toHaveTextContent("claude-test");
    expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("当前实际配置（未保存）");
    expect(screen.queryByText(/尚未生效/)).toBeNull();
    expect(screen.getByTestId("workbench-tab-automation")).toHaveTextContent("运行设置");
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual(["接入 Provider", "运行设置", "启动命令"]);
    expect(screen.getByTestId("manage-instances-button")).toHaveTextContent("独立环境");
    await userEvent.click(screen.getByTestId("workbench-tab-launch"));
    const launches = screen.getByTestId("agent-launch-commands-claude-code");
    expect(within(launches).getByTestId("agent-launch-claude-code-claude")).toHaveTextContent(
      "alias claude='claude --dangerously-skip-permissions'",
    );
    expect(within(launches).getByTestId("agent-add-launch-claude-code")).toHaveTextContent("新增别名");
    // 别名以 rc 文件为准、保存即写入：没有单独的「同步到终端」
    expect(within(launches).queryByTestId("agent-sync-launch-claude-code")).toBeNull();
    expect(within(launches).getByTestId("agent-launch-override-claude-code-claude")).toHaveTextContent("覆盖环境自动化");
    await userEvent.click(screen.getByTestId("workbench-tab-providers"));
    expect(screen.getByTestId("connection-card-claude-code-live")).toHaveTextContent("https://claude.example.com");
    expect(screen.queryByText(/当前 Provider 尚未应用/)).toBeNull();
    expect(screen.getByTestId("reset-connection-claude-code-default")).toHaveTextContent("切回官方账号");
    // 实际配置里有第三方接入：明确提示与官方账号混在同一个配置目录
    expect(screen.getByTestId("main-mixed-warning")).toBeInTheDocument();
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

  it("主 Agent 的运行别名可就地审查和编辑，保存即按名写回终端配置", async () => {
    const all = routes();
    all["aliases:apply"] = makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" });
    const { calls } = renderWithEngine(<AgentsPage />, all);

    await userEvent.click(await screen.findByTestId("workbench-tab-launch"));
    await userEvent.click(await screen.findByTestId("agent-launch-edit-claude-code-claude"));
    const dialog = await screen.findByRole("dialog", { name: "编辑启动命令" });
    expect(within(dialog).getByTestId("alias-name-input")).toHaveValue("claude");
    expect(within(dialog).getByTestId("alias-command-input")).toHaveValue("claude");
    expect(within(dialog).getByTestId("alias-extra-args-input")).toHaveValue("--dangerously-skip-permissions");
    await userEvent.clear(within(dialog).getByTestId("alias-extra-args-input"));
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const args = calls.find((call) => call.command === "aliases:apply")!.args;
    expect(args[args.indexOf("--set") + 1]).toBe("claude");
    expect(args[args.indexOf("--value") + 1]).toBe("claude");
    expect(args).toContain("--zshrc-base");
    expect(args).not.toContain("--rename-from");
  });

  it("没有单独的「实际配置」页签：运行设置按实际值预填，文件原文收在底部按需展开", async () => {
    const withFile: Record<string, Envelope> = {
      ...routes(),
      "agents:config-file": makeEnvelope(
        {
          element: "agents:claude-code",
          path: "~/.claude/settings.json",
          format: "json",
          exists: true,
          content: '{\n  "effortLevel": "high"\n}\n',
          base: "d".repeat(64),
          blocked_reason: null,
        },
        { command: "agents.config-file" },
      ),
    };
    const { calls } = renderWithEngine(<AgentsPage />, withFile);

    await userEvent.click(await screen.findByTestId("workbench-tab-automation"));
    expect(screen.queryByTestId("workbench-tab-actual")).toBeNull();
    expect(screen.getByTestId("agent-runtime-claude-effort-level")).toHaveValue("high");
    // 原文默认收起，不读文件
    const toggle = screen.getByTestId("agent-edit-config-file");
    expect(toggle).toHaveTextContent("settings.json");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(calls.some((call) => call.command === "agents:config-file")).toBe(false);
    await userEvent.click(toggle);
    expect(editorText(await editorView("config-file-text"))).toBe('{\n  "effortLevel": "high"\n}\n');
  });

  it("官方账号只指定了模型：说清是官方账号与模型来源，不当成未保存的接入，也不给切回按钮", async () => {
    const official = routes("codex", "codex");
    const status = official["agents:status"] as { data: { elements: EngineElement[] } };
    const config = (status.data.elements[0].observed as any).current_config;
    config.config = {
      provider: "openai",
      base_url: null,
      model: "gpt-6-luna",
      credential: { configured: false, kind: null, variable: null },
      runtime: { automation_mode: "full" },
    };
    config.current = { ...UNSAVED, default: true, official: true };
    renderWithEngine(<AgentsPage />, official);

    expect(await screen.findByTestId("default-env-account")).toHaveTextContent("官方账号");
    expect(screen.getByTestId("default-env-account")).not.toHaveTextContent("混在");
    expect(screen.getByTestId("default-env-login")).toHaveTextContent("登录由 Codex 自己保存在 ~/.codex/auth.json 或系统钥匙串，本工具不读取也不改动");
    expect(screen.getByTestId("default-env-model")).toHaveTextContent("gpt-6-luna · config.toml 里的 model");
    const card = screen.getByTestId("connection-card-codex-default");
    expect(card).toHaveTextContent("当前使用");
    expect(card).toHaveTextContent("正在使用 · 模型 gpt-6-luna");
    // 没有「未保存的实际配置」、没有混用警告、没有会删掉模型的切回按钮
    expect(screen.queryByTestId("connection-card-codex-live")).toBeNull();
    expect(screen.queryByTestId("main-mixed-warning")).toBeNull();
    expect(screen.queryByTestId("reset-connection-codex-default")).toBeNull();
    // 第三方接入单独一节，说明只在独立环境里用
    expect(screen.getByTestId("third-party-section-codex")).toHaveTextContent("第三方接入 · 只在独立环境里用");
    expect(screen.getByTestId("third-party-empty-codex")).toBeInTheDocument();
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
    // 引擎按内容识别出 relay 是当前接入
    ((status.data.elements[0] as unknown as EngineElement).observed as any).current_config.current = {
      ...UNSAVED,
      profile: "relay",
      last_written: "relay",
    };
    renderWithEngine(<AgentsPage />, matched);

    const cards = await screen.findAllByTestId(/^connection-card-claude-code-/);
    expect(cards[0]).toHaveAttribute("data-testid", "connection-card-claude-code-relay");
    expect(within(cards[0]).getByText("当前使用")).toBeInTheDocument();
  });

  it("切回官方账号通过 configure 写入并以状态回读确认", async () => {
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
        base: BASE,
        current: { ...UNSAVED, default: true },
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
      "--base",
      BASE,
      "--use-default",
    ]);
    await waitFor(() => expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("官方账号"));
  });

  it("不再展示 Claude 与 Codex 之外的客户端", async () => {
    renderWithEngine(<AgentsPage />, routes("opencode", "opencode"));

    await waitFor(() => expect(screen.queryByTestId("connection-workbench")).toBeNull());
    expect(screen.queryByText("opencode 接入工作台")).toBeNull();
  });

  it("Codex 主界面不展示认证细节，运行设置按实际值预填", async () => {
    renderWithEngine(<AgentsPage />, routes("codex", "codex"));

    expect(await screen.findByTestId("connection-workbench")).toBeInTheDocument();
    expect(screen.queryByText("auth.json")).toBeNull();

    await userEvent.click(screen.getByTestId("workbench-tab-automation"));
    expect(screen.getByTestId("agent-runtime-codex-reasoning-effort")).toHaveValue("high");
    expect(screen.getByTestId("automation-panel")).not.toHaveTextContent("软件保存值");
    expect(screen.getByTestId("agent-runtime-submit")).toBeDisabled();
  });
});
