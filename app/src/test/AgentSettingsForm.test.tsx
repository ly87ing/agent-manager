import { describe, expect, it, onTestFinished, vi } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { agentManagementFixture, makeAliasStatus, makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import type { EngineElement, Envelope } from "../lib/contract";

/**
 * 助手运行参数与接入：以客户端实际配置为准。表单按实际值预填，写入只发改动项并带上
 * status 给出的写入基准；外部改过就被引擎拒绝，界面刷新后让用户重来。
 *
 * 需要用户点头的，引擎侧强制、界面负责问：降安全等级（YOLO）、放弃切换前存不下的实际配置或令牌。
 */

const BASE = "b".repeat(64);
const DEFAULT_CURRENT = { profile: null, default: true, last_written: null, unverified: [], drifted_from: null };

function routes(overrides: Record<string, Envelope> = {}): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "policy:get": makeEnvelope(makePolicyData(), { command: "policy.get" }),
    "policy:set": makeEnvelope(makePolicyData(), { command: "policy.set" }),
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:claude-code",
            domain: "agents",
            spec: {
              display_name: "Claude Code",
              bin: "claude",
              preferred_provider: "native",
              providers: { native: { managed: true, install: "i.sh", upgrade: "u.sh" } },
            },
            policy: { enabled: true },
            observed: {
              install_source: "native",
              installed: true,
              management: agentManagementFixture("claude"),
              current_config: { state: "missing", file: "settings.json", target: "/Users/demo/.claude/settings.json", config: null, base: BASE, current: DEFAULT_CURRENT },
            },
            state: "managed-ok",
            freshness: makeFreshness(),
            actions: [],
          }),
        ],
      },
      { command: "agents.status" },
    ),
    "agents:apply": makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }),
    "agents:configure": makeEnvelope(
      { domain: "agents", action: "configure", element: "agents:claude-code" },
      { command: "agents.configure" },
    ),
    "secrets:set": makeEnvelope(
      { domain: "secrets", variable: "RELAY_KEY", stored: true },
      { command: "secrets.set" },
    ),
    "aliases:status": makeEnvelope(makeAliasStatus({}), { command: "aliases.status" }),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
    ...overrides,
  };
}

/** 运行设置直接在页签里，不再弹对话框。 */
async function openRuntimeSettings(): Promise<HTMLElement> {
  await userEvent.click(await screen.findByTestId("workbench-tab-automation"));
  return screen.findByTestId("automation-panel");
}

function configureCalls<T extends { command: string }>(calls: T[]): T[] {
  return calls.filter((call) => call.command === "agents:configure");
}

/** 引擎按内容识别出的当前接入：测试里直接指定。 */
function markCurrent(element: EngineElement, profile: string | null): void {
  const observed = element.observed as Record<string, any>;
  observed.current_config = {
    ...(observed.current_config ?? { state: "missing", file: "settings.json", config: null }),
    base: BASE,
    current: { ...DEFAULT_CURRENT, profile, default: profile === null, last_written: profile },
  };
}

/** 主配置里有旧版本留下、还没保存的第三方配置：当前既不是官方账号也对不上已保存接入。 */
function markUnsavedLive(element: EngineElement): void {
  markCurrent(element, null);
  (element.observed as any).current_config.current.default = false;
}

/** 点「官方账号」卡片上的「切回官方账号」并确认。 */
async function switchBackToOfficial(): Promise<void> {
  await userEvent.click(await screen.findByTestId("reset-connection-claude-code-default"));
  const confirm = await screen.findByRole("dialog", { name: "把 Claude Code 的主配置切回官方账号" });
  await userEvent.click(within(confirm).getByTestId("confirm-modal-confirm"));
}

function configureArgsOf(...rest: string[]): string[] {
  return ["--only", "agents:claude-code", "--base", BASE, ...rest];
}

function runtimeOf<T extends { args?: string[] }>(call: T): Record<string, unknown> {
  return JSON.parse(call.args![call.args!.indexOf("--runtime") + 1]);
}

function setClaudeCurrentConfig(element: EngineElement, profile: string | null): void {
  const settings = (element.spec as { settings?: { profiles?: Record<string, Record<string, unknown>> } }).settings;
  const connection = profile ? settings?.profiles?.[profile] : null;
  (element.observed as Record<string, unknown>).current_config = {
    state: "ready",
    file: "settings.json",
    target: "/Users/demo/.claude/settings.json",
    base: BASE,
    current: { ...DEFAULT_CURRENT, profile, default: profile === null, last_written: profile },
    config: {
      provider: "anthropic",
      base_url: connection?.base_url ?? null,
      model: connection?.model ?? null,
      small_fast_model: connection?.small_fast_model ?? null,
      model_aliases: connection?.model_aliases ?? { opus: null, sonnet: null, haiku: null },
      timeout_ms: connection?.timeout_ms ?? null,
      credential: {
        configured: connection !== null,
        kind: connection ? connection.auth ?? "token" : null,
        variable: null,
      },
      runtime: {},
    },
  };
}

describe("助手运行参数", () => {
  function agentRoutes(id: string, bin: string) {
    return routes({
      "agents:status": makeEnvelope(
        {
          domain: "agents",
          elements: [
            makeElement({
              id: `agents:${id}`,
              domain: "agents",
              spec: {
                display_name: id,
                bin,
                preferred_provider: "native",
                providers: { native: { managed: true, install: "i.sh", upgrade: "u.sh" } },
              },
              policy: { enabled: true },
              observed: {
                install_source: "native",
                installed: true,
                management: bin === "claude" || bin === "codex" ? agentManagementFixture(bin) : null,
                current_config: bin === "codex"
                  ? { state: "missing", file: "config.toml", target: "/Users/demo/.codex/config.toml", config: null, base: BASE, current: DEFAULT_CURRENT }
                  : { state: "missing", file: "settings.json", target: "/Users/demo/.claude/settings.json", config: null, base: BASE, current: DEFAULT_CURRENT },
              },
              state: "managed-ok",
              freshness: makeFreshness(),
              actions: [],
            }),
          ],
        },
        { command: "agents.status" },
      ),
    });
  }

  it("运行设置页不展示生命周期脚本，只把改动交给引擎一条命令写入", async () => {
    const withExisting = routes();
    const status = withExisting["agents:status"] as { data: { elements: { spec: Record<string, any> }[] } };
    status.data.elements[0].spec.custom_meta = { owner: "local" };
    status.data.elements[0].spec.providers.native.uninstall = "keep-uninstall.sh";
    status.data.elements[0].spec.settings = {
      active: "relay",
      automation_mode: "manual",
      unknown_runtime_flag: "keep",
      profiles: { relay: { label: "中转", future_option: 7 } },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withExisting);

    const panel = await openRuntimeSettings();
    expect(panel).toHaveTextContent("账号与登录在「账号与登录」里看，第三方接入在下方建成独立环境使用");
    // 没改动时不能写
    expect(screen.getByTestId("agent-runtime-submit")).toBeDisabled();
    expect(screen.queryByLabelText("安装命令")).toBeNull();
    expect(screen.queryByLabelText("升级命令")).toBeNull();
    expect(screen.queryByLabelText("卸载命令")).toBeNull();
    await userEvent.selectOptions(screen.getByTestId("agent-runtime-statusline"), "builtin");
    await userEvent.click(screen.getByTestId("agent-runtime-submit"));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    // 文件与库由引擎在同一事务里写，界面不再先存一份定义（被拒时库也不会先变）
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(configureCalls(calls)[0].args).toEqual(
      configureArgsOf("--runtime", JSON.stringify({ statusline: "builtin" })),
    );
  });

  it("Claude 高级运行参数在写入预览集中显示真实配置键并保存具体值", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    const dialog = await openRuntimeSettings();
    expect(dialog).not.toHaveTextContent("env.ENABLE_TOOL_SEARCH");

    // max 只对单次会话有效，官方不接受持久保存：选项里没有它
    expect(screen.queryByRole("option", { name: "max" })).toBeNull();
    await userEvent.selectOptions(screen.getByTestId("agent-runtime-claude-effort-level"), "xhigh");
    await userEvent.click(screen.getByTestId("agent-runtime-claude-tool-search"));
    await userEvent.click(screen.getByTestId("agent-runtime-claude-agent-teams"));
    await userEvent.click(screen.getByTestId("agent-runtime-claude-disable-autoupdate"));
    await userEvent.click(screen.getByTestId("agent-runtime-claude-disable-artifact"));
    await userEvent.click(screen.getByTestId("agent-runtime-claude-hide-attribution"));
    expect(dialog).toHaveTextContent('"ENABLE_TOOL_SEARCH": "true"');
    expect(dialog).toHaveTextContent('"CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS": "1"');
    // 官方禁用自动更新的写法
    expect(dialog).toHaveTextContent('"DISABLE_AUTOUPDATER": "1"');
    expect(dialog).toHaveTextContent('"commit": ""');
    expect(dialog).toHaveTextContent('"pr": ""');
    // 官方持久键，不再写单次会话覆盖变量
    expect(dialog).toHaveTextContent('"effortLevel": "xhigh"');
    expect(dialog).toHaveTextContent('"enableArtifact": false');
    expect(dialog).not.toHaveTextContent("CLAUDE_CODE_EFFORT_LEVEL");
    expect(dialog).not.toHaveTextContent("CLAUDE_CODE_DISABLE_ARTIFACT");
    await userEvent.click(screen.getByTestId("agent-runtime-submit"));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(runtimeOf(configureCalls(calls)[0])).toEqual({
      claude_effort_level: "xhigh",
      claude_tool_search: true,
      claude_agent_teams: true,
      claude_disable_autoupdate: true,
      claude_disable_artifact: true,
      claude_hide_attribution: true,
    });
  });

  it("写入预览显示 Claude 最终配置值和真实目标文件", async () => {
    renderWithEngine(<AgentsPage />, routes());
    const dialog = await openRuntimeSettings();
    expect(screen.queryByTestId("agent-runtime-preview")).toBeNull();
    await userEvent.click(screen.getByTestId("agent-runtime-automation-mode-full"));
    expect(screen.getByTestId("agent-runtime-target")).toHaveTextContent("/Users/demo/.claude/settings.json");
    expect(screen.getByTestId("agent-runtime-preview")).toHaveTextContent('"defaultMode": "bypassPermissions"');
    expect(dialog).toHaveTextContent("文件里的其他配置保持不变");
    await userEvent.click(screen.getByTestId("agent-runtime-reset"));
    expect(screen.queryByTestId("agent-runtime-preview")).toBeNull();
  });

  it("写入预览显示 Codex TOML 最终配置值和真实目标文件", async () => {
    renderWithEngine(<AgentsPage />, agentRoutes("codex", "codex"));
    await openRuntimeSettings();
    await userEvent.click(screen.getByTestId("agent-runtime-automation-mode-full"));
    await userEvent.selectOptions(screen.getByTestId("agent-runtime-codex-reasoning-effort"), "high");
    expect(screen.getByTestId("agent-runtime-target")).toHaveTextContent("/Users/demo/.codex/config.toml");
    const preview = screen.getByTestId("agent-runtime-preview");
    expect(preview).toHaveTextContent('approval_policy = "never"');
    expect(preview).toHaveTextContent('sandbox_mode = "danger-full-access"');
    expect(preview).toHaveTextContent('model_reasoning_effort = "high"');
  });

  it("默认模型从实际配置预填，预览只列 model 并只提交该字段", async () => {
    const withModel = routes();
    const status = withModel["agents:status"] as { data: { elements: EngineElement[] } };
    const element = status.data.elements[0];
    setClaudeCurrentConfig(element, null);
    (element.observed as any).current_config.config.runtime = { automation_mode: "manual", default_model: "claude-sonnet-5" };
    const { calls } = renderWithEngine(<AgentsPage />, withModel);
    await openRuntimeSettings();
    const input = screen.getByTestId("agent-runtime-default-model");
    expect(input).toHaveValue("claude-sonnet-5");
    await userEvent.clear(input);
    await userEvent.type(input, "claude-opus-5");
    expect(screen.getByTestId("agent-runtime-preview")).toHaveTextContent('"model": "claude-opus-5"');
    await userEvent.click(screen.getByTestId("agent-runtime-submit"));
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(runtimeOf(configureCalls(calls)[0])).toEqual({ default_model: "claude-opus-5" });
    expect(await screen.findByText("写入已经返回成功，但状态回读没有确认生效")).toBeInTheDocument();
  });

  it("Codex 运行参数允许研发填写推理、上下文、压缩与搜索的具体值", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, agentRoutes("codex", "codex"));
    const dialog = await openRuntimeSettings();

    await userEvent.selectOptions(screen.getByTestId("agent-runtime-codex-reasoning-effort"), "high");
    await userEvent.type(screen.getByTestId("agent-runtime-codex-context-window"), "1000000");
    await userEvent.type(screen.getByTestId("agent-runtime-codex-auto-compact-token-limit"), "900000");
    await userEvent.selectOptions(screen.getByTestId("agent-runtime-codex-web-search"), "live");
    expect(dialog).toHaveTextContent("model_reasoning_effort");
    expect(dialog).toHaveTextContent("model_context_window");
    expect(dialog).toHaveTextContent("model_auto_compact_token_limit");
    expect(dialog).toHaveTextContent("web_search");
    await userEvent.click(screen.getByTestId("agent-runtime-submit"));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(runtimeOf(configureCalls(calls)[0])).toEqual({
      codex_reasoning_effort: "high",
      codex_context_window: 1000000,
      codex_auto_compact_token_limit: 900000,
      codex_web_search: "live",
    });
  });

  it("完全放开先二次确认，取消时定义和助手文件都不写", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openRuntimeSettings();
    await userEvent.click(screen.getByTestId("agent-runtime-automation-mode-full"));
    await userEvent.click(screen.getByTestId("agent-runtime-submit"));

    await screen.findByRole("dialog", { name: "完全放开助手权限" });
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(configureCalls(calls)).toHaveLength(0);
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(configureCalls(calls)).toHaveLength(0);
  });

  it("完全放开经确认后带 --allow-yolo 写入", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openRuntimeSettings();
    await userEvent.click(screen.getByTestId("agent-runtime-automation-mode-full"));
    await userEvent.click(screen.getByTestId("agent-runtime-submit"));
    await screen.findByRole("dialog", { name: "完全放开助手权限" });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(configureCalls(calls)[0].args).toEqual(
      configureArgsOf("--runtime", JSON.stringify({ automation_mode: "full" }), "--allow-yolo"),
    );
  });

  it("自动化页与表单都以客户端实际值为准，不拿库里的上次写入记录比对", async () => {
    const withRecord = routes();
    const status = withRecord["agents:status"] as { data: { elements: EngineElement[] } };
    const element = status.data.elements[0];
    // 库里记着上次写的是智能自动，客户端实际已是手动确认
    (element.spec as Record<string, unknown>).settings = { automation_mode: "auto" };
    setClaudeCurrentConfig(element, null);
    const current = (element.observed as any).current_config.config;
    current.runtime = { automation_mode: "manual" };
    const { calls } = renderWithEngine(<AgentsPage />, withRecord);

    const panel = await openRuntimeSettings();
    expect(screen.getByTestId("default-env-automation")).toHaveTextContent("手动确认");
    expect(panel).not.toHaveTextContent("尚未生效");
    expect(screen.queryByTestId("agent-runtime-apply")).toBeNull();
    expect(screen.getByTestId("agent-runtime-automation-mode-manual")).toBeChecked();
    expect(configureCalls(calls)).toHaveLength(0);
  });

  it("实际档位是本工具不提供的值时显示「保持当前」，只提交改动项，回读不误报成功", async () => {
    const withCustom = routes();
    const status = withCustom["agents:status"] as { data: { elements: EngineElement[] } };
    const element = status.data.elements[0];
    setClaudeCurrentConfig(element, null);
    (element.observed as any).current_config.config.runtime = {
      automation_mode: "custom",
      automation_raw: { "permissions.defaultMode": "acceptEdits" },
      claude_effort_level: "low",
    };
    const { calls } = renderWithEngine(<AgentsPage />, withCustom);

    await openRuntimeSettings();
    expect(screen.getByTestId("agent-runtime-automation-mode-custom")).toBeChecked();
    expect(screen.getByTestId("agent-runtime-panel")).toHaveTextContent("acceptEdits");
    expect(screen.getByTestId("agent-runtime-claude-effort-level")).toHaveValue("low");
    await userEvent.selectOptions(screen.getByTestId("agent-runtime-claude-effort-level"), "high");
    await userEvent.click(screen.getByTestId("agent-runtime-submit"));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(runtimeOf(configureCalls(calls)[0])).toEqual({ claude_effort_level: "high" });
    // 引擎说成功，但回读的实际值没变：不能报成功
    expect(await screen.findByText("写入已经返回成功，但状态回读没有确认生效")).toBeInTheDocument();
  });

  it("旧格式状态栏修复后回读仍是旧格式（被外部改回裸路径）时不报成功", async () => {
    const legacy: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    const status = legacy["agents:status"] as { data: { elements: EngineElement[] } };
    const element = status.data.elements[0];
    setClaudeCurrentConfig(element, null);
    (element.observed as any).current_config.config.runtime = { automation_mode: "manual", statusline: "builtin-legacy" };
    let revertedExternally = true;
    legacy["agents:configure"] = async () => {
      // 引擎写成了引用形式，但回读前文件又被外部改回了裸路径
      (element.observed as any).current_config.config.runtime.statusline = revertedExternally ? "builtin-legacy" : "builtin";
      return makeEnvelope({ domain: "agents", action: "configure", statusline: true }, { command: "agents.configure" });
    };
    const { calls } = renderWithEngine(<AgentsPage />, legacy);

    await userEvent.click(await screen.findByTestId("workbench-tab-automation"));
    expect(screen.getByTestId("statusline-legacy-notice")).toBeInTheDocument();
    await userEvent.click(screen.getByTestId("agent-statusline-fix"));
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(configureCalls(calls)[0].args).toEqual(
      configureArgsOf("--runtime", JSON.stringify({ statusline: "builtin" })),
    );
    expect(await screen.findByText("写入已经返回成功，但状态回读没有确认生效")).toBeInTheDocument();
    expect(screen.queryByText("已修复状态栏")).toBeNull();
    expect(screen.getByTestId("statusline-legacy-notice")).toBeInTheDocument();

    // 回读到引用形式才算修好
    revertedExternally = false;
    await userEvent.click(screen.getByTestId("agent-statusline-fix"));
    expect(await screen.findByText("已修复状态栏")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByTestId("statusline-legacy-notice")).toBeNull());
  });

  it("env 里的单次会话覆盖变量如实提示，点「清理覆盖变量」才撤掉；按模型保存的推理强度只读展示", async () => {
    const withOverrides: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    const status = withOverrides["agents:status"] as { data: { elements: EngineElement[] } };
    const element = status.data.elements[0];
    setClaudeCurrentConfig(element, null);
    const config = (element.observed as any).current_config.config;
    config.runtime = { automation_mode: "manual", claude_effort_level: "high" };
    config.claude_model_efforts = { "claude-opus-5": "medium" };
    config.claude_env_overrides = { CLAUDE_CODE_EFFORT_LEVEL: "low" };
    withOverrides["agents:configure"] = async () => {
      config.claude_env_overrides = {};
      return makeEnvelope({ domain: "agents", action: "configure" }, { command: "agents.configure" });
    };
    const { calls } = renderWithEngine(<AgentsPage />, withOverrides);

    await userEvent.click(await screen.findByTestId("workbench-tab-automation"));
    expect(screen.getByTestId("model-efforts")).toHaveTextContent("claude-opus-5 = medium");
    expect(screen.getByTestId("env-overrides-notice")).toHaveTextContent("CLAUDE_CODE_EFFORT_LEVEL=low");
    // 只看不写
    expect(configureCalls(calls)).toHaveLength(0);
    await userEvent.click(screen.getByTestId("agent-env-overrides-clear"));
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(configureCalls(calls)[0].args).toEqual(
      configureArgsOf("--runtime", JSON.stringify({ claude_clear_env_overrides: true })),
    );
    expect(await screen.findByText("已清理 env 里的单次会话覆盖变量")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByTestId("env-overrides-notice")).toBeNull());
  });

  it("配置文件已在外部修改：拒绝后刷新状态并提示重来，不再提供用库覆盖", async () => {
    const withRefusal: Record<string, Envelope | (() => Promise<Envelope>)> = {
      ...routes(),
      "agents:configure": async () => makeEnvelope(
        {},
        {
          command: "agents.configure",
          ok: false,
          errors: [{
            code: "AGENT_CONFIG_CHANGED",
            element: null,
            message: ".claude/settings.json 已在外部修改（与读取时不一致），文件与库都没有改动；请刷新后重试",
          }],
        },
      ),
    };
    const { calls } = renderWithEngine(<AgentsPage />, withRefusal);
    await openRuntimeSettings();
    await userEvent.selectOptions(screen.getByTestId("agent-runtime-statusline"), "builtin");
    const statusCallsBefore = calls.filter((call) => call.command === "agents:status").length;
    await userEvent.click(screen.getByTestId("agent-runtime-submit"));

    expect(await screen.findByText("配置文件已在外部修改，没有写入")).toBeInTheDocument();
    expect(configureCalls(calls)).toHaveLength(1);
    expect(configureCalls(calls)[0].args).not.toContain("--allow-takeover");
    expect(screen.queryByRole("dialog", { name: "选择如何处理配置差异" })).toBeNull();
    expect(calls.filter((call) => call.command === "agents:status").length).toBeGreaterThan(statusCallsBefore);
  });

  it("实际配置对不上已保存接入时显示「当前实际配置（未保存）」，可一键保存", async () => {
    const live: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    const status = live["agents:status"] as { data: { elements: EngineElement[] } };
    (status.data.elements[0].observed as Record<string, unknown>).current_config = {
      state: "ready",
      file: "settings.json",
      target: "/Users/demo/.claude/settings.json",
      base: BASE,
      current: { ...DEFAULT_CURRENT, default: false },
      config: {
        base_url: "https://client.example.com",
        model: "client-model",
        credential: { configured: true, kind: "token", variable: null, saved_var: null },
        runtime: {},
      },
    };
    live["agents:configure"] = async () => {
      (status.data.elements[0].spec as Record<string, unknown>).settings = {
        profiles: { "imported-1": { label: "导入的配置", base_url: "https://client.example.com" } },
      };
      markCurrent(status.data.elements[0], "imported-1");
      return makeEnvelope(
        { domain: "agents", action: "configure", backfill: { name: "imported-1", created: true, label: "导入的配置", token: "adopted", token_var: "IMPORTED_1_TOKEN" } },
        { command: "agents.configure" },
      );
    };
    const { calls } = renderWithEngine(<AgentsPage />, live);

    const card = await screen.findByTestId("connection-card-claude-code-live");
    expect(card).toHaveTextContent("当前实际配置（未保存）");
    expect(card).toHaveTextContent("https://client.example.com");
    expect(card).toHaveTextContent("令牌还没有保存为变量");
    expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("当前实际配置（未保存）");
    expect(within(screen.getByTestId("connection-card-claude-code-default")).queryByText("当前使用")).toBeNull();

    await userEvent.click(screen.getByTestId("save-live-connection-claude-code"));
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(configureCalls(calls)[0].args).toEqual(configureArgsOf("--save-current"));
    expect(await screen.findByText("已把当前实际配置保存为接入配置")).toBeInTheDocument();
    expect(screen.getByText(/令牌写进了密钥文件的变量 IMPORTED_1_TOKEN/)).toBeInTheDocument();
    expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("导入的配置");
  });

  it("切回官方时实际令牌没保存被拒：可去选密钥文件，或确认放弃后带标志重试", async () => {
    const onOpenSecrets = vi.fn();
    const refusing: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    const status = refusing["agents:status"] as { data: { elements: EngineElement[] } };
    (status.data.elements[0].spec as Record<string, unknown>).settings = {
      profiles: { relay: { label: "我的中转", base_url: "https://relay.example.com" } },
    };
    markUnsavedLive(status.data.elements[0]);
    refusing["agents:configure"] = async (args) => {
      if (!args.includes("--discard-live-token")) {
        return makeEnvelope({}, {
          command: "agents.configure",
          ok: false,
          errors: [{ code: "AGENT_LIVE_TOKEN_UNSAVED", element: null, message: "当前实际配置里的令牌还没有保存在任何变量里，切换会丢失它：先到“设置 → 密钥”选择外部密钥文件（令牌会写进该文件，库里只存变量名），或确认放弃这个令牌后带 --discard-live-token 重试" }],
        });
      }
      markCurrent(status.data.elements[0], null);
      return makeEnvelope({ domain: "agents", action: "configure" }, { command: "agents.configure" });
    };
    const { calls } = renderWithEngine(<AgentsPage onOpenSecrets={onOpenSecrets} />, refusing);

    await switchBackToOfficial();
    const dialog = await screen.findByRole("dialog", { name: "当前实际令牌还没有保存" });
    expect(dialog).toHaveTextContent("切换会丢失它");
    expect(dialog).toHaveTextContent("设置 → 密钥");
    // 用界面自己的话，不把给命令行用户的参数提示展示出来
    expect(dialog).not.toHaveTextContent("--discard-live-token");
    expect(within(dialog).getByTestId("confirm-modal-secondary")).toHaveTextContent("去选择密钥文件");
    await userEvent.click(within(dialog).getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(2));
    expect(configureCalls(calls)[1].args).toEqual(configureArgsOf("--use-default", "--discard-live-token"));
    await waitFor(() => expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("官方账号"));
    expect(onOpenSecrets).not.toHaveBeenCalled();
  });

  it("切回官方时实际令牌没保存，可直接去密钥设置，不做任何写入", async () => {
    const onOpenSecrets = vi.fn();
    const refusing: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes({
      "agents:configure": makeEnvelope({}, {
        command: "agents.configure",
        ok: false,
        errors: [{ code: "AGENT_LIVE_TOKEN_UNSAVED", element: null, message: "当前实际配置里的令牌还没有保存在任何变量里" }],
      }),
    });
    const status = refusing["agents:status"] as { data: { elements: EngineElement[] } };
    (status.data.elements[0].spec as Record<string, unknown>).settings = {
      profiles: { relay: { label: "我的中转", base_url: "https://relay.example.com" } },
    };
    markUnsavedLive(status.data.elements[0]);
    const { calls } = renderWithEngine(<AgentsPage onOpenSecrets={onOpenSecrets} />, refusing);

    await switchBackToOfficial();
    const dialog = await screen.findByRole("dialog", { name: "当前实际令牌还没有保存" });
    await userEvent.click(within(dialog).getByTestId("confirm-modal-secondary"));
    expect(onOpenSecrets).toHaveBeenCalledTimes(1);
    expect(configureCalls(calls)).toHaveLength(1);
  });

  it("切回官方时实际配置存不下：列出原因，确认放弃后带 --discard-live 重试", async () => {
    const refusing: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    const status = refusing["agents:status"] as { data: { elements: EngineElement[] } };
    (status.data.elements[0].spec as Record<string, unknown>).settings = {
      profiles: { relay: { label: "我的中转", base_url: "https://relay.example.com" } },
    };
    markUnsavedLive(status.data.elements[0]);
    refusing["agents:configure"] = async (args) => {
      if (!args.includes("--discard-live")) {
        return makeEnvelope({}, {
          command: "agents.configure",
          ok: false,
          errors: [{
            code: "AGENT_LIVE_UNREPRESENTABLE",
            element: null,
            message: "当前实际接入配置无法完整保存，切换会丢失这些内容：同时设置了 ANTHROPIC_AUTH_TOKEN 与 ANTHROPIC_API_KEY，只能保存其中一个。确认放弃当前实际配置后可带 --discard-live 重试",
          }],
        });
      }
      markCurrent(status.data.elements[0], null);
      return makeEnvelope({ domain: "agents", action: "configure" }, { command: "agents.configure" });
    };
    const { calls } = renderWithEngine(<AgentsPage />, refusing);

    await switchBackToOfficial();
    const dialog = await screen.findByRole("dialog", { name: "当前实际配置无法完整保存" });
    expect(dialog).toHaveTextContent("ANTHROPIC_AUTH_TOKEN 与 ANTHROPIC_API_KEY");
    expect(dialog).toHaveTextContent("可以先在配置文件里处理后再切换");
    expect(dialog).not.toHaveTextContent("--discard-live");
    expect(within(dialog).queryByTestId("confirm-modal-secondary")).toBeNull();
    await userEvent.click(within(dialog).getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(2));
    expect(configureCalls(calls)[1].args).toEqual(configureArgsOf("--use-default", "--discard-live"));
  });

  it("主配置里有第三方接入时提示混用、卡片不提供应用；切回官方账号先确认再带 --use-default", async () => {
    const withProfiles: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      profiles: {
        official: { label: "官方" },
        relay: { label: "我的中转", base_url: "https://relay.example.com" },
      },
    };
    setClaudeCurrentConfig(status.data.elements[0] as EngineElement, "relay");
    withProfiles["agents:configure"] = async (args) => {
      if (args.includes("--use-default")) {
        delete (status.data.elements[0].spec.settings as { active?: string }).active;
        setClaudeCurrentConfig(status.data.elements[0] as EngineElement, null);
      }
      return makeEnvelope(
        { domain: "agents", action: "configure", element: "agents:claude-code" },
        { command: "agents.configure" },
      );
    };
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    expect(await screen.findByTestId("active-connection-summary")).toHaveTextContent("我的中转");
    expect(screen.getByTestId("main-mixed-warning")).toHaveTextContent("主配置里有第三方接入");
    expect(within(screen.getByTestId("connection-card-claude-code-relay")).getByText("当前使用")).toBeInTheDocument();
    expect(screen.queryByTestId("activate-connection-claude-code-official")).toBeNull();
    expect(screen.queryByTestId("activate-connection-claude-code-relay")).toBeNull();

    await userEvent.click(screen.getByTestId("reset-connection-claude-code-default"));
    const resetConfirm = await screen.findByRole("dialog", { name: "把 Claude Code 的主配置切回官方账号" });
    expect(configureCalls(calls)).toHaveLength(0);
    // 逐条说清改动范围：撤哪些键、先保存什么、登录与其他设置不动
    const effects = within(resetConfirm).getByTestId("switch-back-effects");
    expect(effects).toHaveTextContent("会撤掉settings.json 的 env 里的接口地址、令牌");
    expect(effects).toHaveTextContent("切之前这份第三方配置先保存成一套接入");
    expect(effects).toHaveTextContent("不会动官方账号的登录（系统钥匙串或 ~/.claude/.credentials.json");
    expect(effects).toHaveTextContent("运行设置（自动化档位、推理强度等）");
    await userEvent.click(within(resetConfirm).getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(configureCalls(calls)[0].args).toEqual(configureArgsOf("--use-default"));
    await waitFor(() => expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("官方账号"));
    expect(screen.queryByTestId("main-mixed-warning")).toBeNull();
  });

  it("切回官方返回成功但实际配置没变时不误报成功，当前项仍按实际显示", async () => {
    const withProfiles: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      profiles: { relay: { label: "我的中转", base_url: "https://relay.example.com" } },
    };
    setClaudeCurrentConfig(status.data.elements[0] as EngineElement, "relay");
    withProfiles["agents:configure"] = async () => {
      delete (status.data.elements[0].spec.settings as { active?: string }).active;
      return makeEnvelope(
        { domain: "agents", action: "configure", element: "agents:claude-code" },
        { command: "agents.configure" },
      );
    };
    renderWithEngine(<AgentsPage />, withProfiles);

    await switchBackToOfficial();
    expect(await screen.findByText("写入已经返回成功，但状态回读没有确认生效")).toBeInTheDocument();
    // 库里的上次写入记录变了也不算：当前项跟着实际配置走
    expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("我的中转");
  });

  it("编辑主配置正在用的那套接入：预填原值、唯一名称不可改，只保存定义不写助手配置", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      unknown_runtime_flag: "keep",
      profiles: {
        official: { label: "官方", custom_profile_flag: 7 },
        relay: {
          label: "我的中转",
          note: "旧备注",
          base_url: "https://old.example.com",
          token_var: "RELAY_KEY",
          model: "old-model",
          future_option: { mode: "keep" },
        },
      },
    };
    markCurrent(status.data.elements[0] as EngineElement, "relay");
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    expect(await screen.findByTestId("connection-note-claude-code-relay")).toHaveTextContent("旧备注");
    await userEvent.click(await screen.findByTestId("edit-connection-claude-code-relay"));
    expect(await screen.findByRole("dialog", { name: "编辑 我的中转" })).toBeInTheDocument();
    expect(screen.getByTestId("new-connection-base-url")).toHaveValue("https://old.example.com");
    expect(screen.getByText(/保存只更新接入定义，不写助手配置/)).toBeInTheDocument();
    expect(screen.getByTestId("connection-form-submit")).toHaveTextContent("保存修改");
    await userEvent.click(screen.getByTestId("new-connection-advanced"));
    expect(screen.getByTestId("new-connection-name")).toBeDisabled();
    await userEvent.clear(screen.getByTestId("new-connection-base-url"));
    await userEvent.type(screen.getByTestId("new-connection-base-url"), "https://new.example.com");
    await userEvent.click(screen.getByTestId("connection-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    expect(configureCalls(calls)).toHaveLength(0);
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.active).toBe("relay");
    expect(definition.settings.unknown_runtime_flag).toBe("keep");
    expect(definition.settings.profiles.relay).toMatchObject({
      base_url: "https://new.example.com",
      token_var: "RELAY_KEY",
      future_option: { mode: "keep" },
    });
  });

  it("小任务模型已被官方弃用：新增接入不再提供，旧接入里的取值可一键挪到 Haiku 别名", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "official",
      profiles: {
        relay: { label: "我的中转", base_url: "https://relay.example.com", token_var: "RELAY_KEY", model: "m", small_fast_model: "relay-small" },
        fresh: { label: "新中转", base_url: "https://fresh.example.com", token_var: "FRESH_KEY", model: "m" },
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    await userEvent.click(await screen.findByTestId("edit-connection-claude-code-fresh"));
    await screen.findByRole("dialog", { name: "编辑 新中转" });
    await userEvent.click(screen.getByTestId("new-connection-advanced"));
    expect(screen.getByTestId("new-connection-alias-haiku")).toBeInTheDocument();
    expect(screen.queryByTestId("new-connection-small-fast-model")).toBeNull();
    await userEvent.keyboard("{Escape}");

    await userEvent.click(await screen.findByTestId("edit-connection-claude-code-relay"));
    await screen.findByRole("dialog", { name: "编辑 我的中转" });
    expect(screen.getByTestId("new-connection-small-fast-model")).toHaveValue("relay-small");
    expect(screen.getByText(/小任务模型（官方已弃用）/)).toBeInTheDocument();
    await userEvent.click(screen.getByTestId("move-small-fast-to-haiku"));
    expect(screen.getByTestId("new-connection-alias-haiku")).toHaveValue("relay-small");
    expect(screen.getByTestId("new-connection-small-fast-model")).toHaveValue("");
    await userEvent.click(screen.getByTestId("connection-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.profiles.relay.small_fast_model).toBeUndefined();
    expect(definition.settings.profiles.relay.model_aliases).toEqual({ haiku: "relay-small" });
  });

  it("Provider 卡片直接提供编辑入口，点击卡片主体也能打开同一个编辑页", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      profiles: {
        relay: { label: "我的中转", base_url: "https://relay.example.com", token_var: "RELAY_KEY", model: "relay-model" },
      },
    };
    renderWithEngine(<AgentsPage />, withProfiles);

    const edit = await screen.findByTestId("edit-connection-claude-code-relay");
    expect(edit).toHaveAccessibleName("编辑 我的中转");
    await userEvent.click(screen.getByTestId("connection-card-claude-code-relay"));
    expect(await screen.findByRole("dialog", { name: "编辑 我的中转" })).toBeInTheDocument();
  });

  it("默认账号是当前项时不提供应用，也不伪装成可编辑或可删除项", async () => {
    renderWithEngine(<AgentsPage />, routes());

    expect(await screen.findByTestId("connection-card-claude-code-default")).toHaveTextContent("当前使用");
    expect(screen.queryByTestId("reset-connection-claude-code-default")).toBeNull();
    expect(screen.queryByTestId("edit-connection-claude-code-default")).toBeNull();
    expect(screen.queryByTestId("remove-connection-claude-code-default")).toBeNull();
  });

  it("工作台编辑非当前接入：默认只保存修改，不偷切当前项", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "official",
      profiles: {
        official: { label: "官方" },
        relay: {
          label: "我的中转",
          base_url: "https://relay.example.com",
          token_var: "RELAY_KEY",
          model: "relay-model",
        },
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    await userEvent.click(await screen.findByTestId("edit-connection-claude-code-relay"));
    expect(await screen.findByRole("dialog", { name: "编辑 我的中转" })).toBeInTheDocument();
    expect(screen.queryByTestId("new-connection-activate")).toBeNull();
    await userEvent.clear(screen.getByTestId("new-connection-label"));
    await userEvent.type(screen.getByTestId("new-connection-label"), "备用中转");
    await userEvent.click(screen.getByTestId("connection-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    expect(configureCalls(calls)).toHaveLength(0);
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.active).toBe("official");
    expect(definition.settings.profiles.relay.label).toBe("备用中转");
  });

  it("工作台复制接入：预填参数并生成新名称，默认只保存且不读取凭据", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "official",
      profiles: {
        official: { label: "官方" },
        relay: {
          label: "我的中转",
          base_url: "https://relay.example.com",
          token_var: "RELAY_KEY",
          model: "relay-model",
          future_option: { mode: "keep" },
        },
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    const manage = await screen.findByLabelText("管理 我的中转");
    await userEvent.click(manage);
    const menu = manage.closest("details")!;
    expect(menu).toHaveAttribute("open");
    await userEvent.click(screen.getByTestId("duplicate-connection-claude-code-relay"));
    expect(menu).not.toHaveAttribute("open");
    const dialog = await screen.findByRole("dialog", { name: "复制 Provider" });
    expect(dialog).toHaveTextContent("原 Provider、当前接入和客户端配置都不会改变");
    expect(screen.getByTestId("new-connection-label")).toHaveValue("我的中转 副本");
    expect(screen.getByTestId("duplicate-connection-summary")).toHaveTextContent("https://relay.example.com");
    expect(screen.getByTestId("duplicate-connection-summary")).toHaveTextContent("relay-model");
    expect(screen.getByTestId("duplicate-connection-summary")).toHaveTextContent("环境变量 RELAY_KEY");
    expect(screen.queryByTestId("new-connection-base-url")).toBeNull();
    expect(screen.queryByTestId("new-connection-token-var")).toBeNull();
    expect(screen.queryByTestId("new-connection-secret")).toBeNull();
    expect(screen.queryByTestId("new-connection-name")).toBeNull();
    expect(screen.getByTestId("connection-form-submit")).toHaveTextContent("创建副本");
    await userEvent.click(screen.getByTestId("connection-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.active).toBe("official");
    expect(definition.settings.profiles.relay.future_option).toEqual({ mode: "keep" });
    expect(definition.settings.profiles["relay-copy"]).toMatchObject({
      label: "我的中转 副本",
      base_url: "https://relay.example.com",
      token_var: "RELAY_KEY",
      model: "relay-model",
      future_option: { mode: "keep" },
    });
    expect(configureCalls(calls)).toHaveLength(0);
    expect(calls.some((call) => call.command === "secrets:set")).toBe(false);
  });

  it("工作台把接入建成独立环境：预填新建表单，保存只写实例定义，不切主配置、不读凭据", async () => {
    // macOS：第三方环境默认让子进程看不到凭据，从接入建成时同样生效
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)");
    onTestFinished(() => { vi.restoreAllMocks(); });
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "official",
      profiles: {
        official: { label: "官方" },
        relay: {
          label: "我的中转",
          base_url: "https://relay.example.com",
          auth: "api_key",
          token_var: "RELAY_KEY",
          model: "relay-model",
          small_fast_model: "relay-small",
        },
      },
    };
    // 新环境的目标文件还不存在：保存后先把配置写进它自己的目录
    withProfiles["profiles:status"] = makeEnvelope(
      { domain: "profiles", elements: [makeElement({ id: "profiles:relay", domain: "profiles", observed: { base: "missing" } })] },
      { command: "profiles.status" },
    );
    withProfiles["profiles:apply"] = makeEnvelope(makeApplyResult({ domain: "profiles" }), { command: "profiles.apply" });
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    await userEvent.click(await screen.findByTestId("make-profile-quick-claude-code-relay"));
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    expect(within(dialog).getByTestId("profile-id-input")).toHaveValue("relay");
    expect(within(dialog).getByTestId("profile-base-url")).toHaveValue("https://relay.example.com");
    expect(within(dialog).getByTestId("profile-token-var")).toHaveValue("RELAY_KEY");
    expect(within(dialog).getByTestId("profile-auth-header")).toHaveValue("api_key");
    // 官方已弃用 ANTHROPIC_SMALL_FAST_MODEL：旧接入的小任务模型按官方替代写成 Haiku 别名
    expect(within(dialog).getByTestId("profile-extra-env")).toHaveTextContent("ANTHROPIC_DEFAULT_HAIKU_MODEL");
    expect(within(dialog).getByTestId("profile-extra-env")).not.toHaveTextContent("ANTHROPIC_SMALL_FAST_MODEL");
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const saved = calls.find((call) => call.command === "policy:set")!;
    expect(saved.args![saved.args!.indexOf("--key") + 1]).toBe("agents.definitions.relay");
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.profile).toMatchObject({
      base_agent: "claude-code",
      config_dir: "~/.claude-relay",
      model: "relay-model",
      env: {
        ANTHROPIC_BASE_URL: "https://relay.example.com",
        ANTHROPIC_API_KEY: "${RELAY_KEY}",
        ANTHROPIC_DEFAULT_HAIKU_MODEL: "relay-small",
        CLAUDE_CODE_SUBPROCESS_ENV_SCRUB: "1",
      },
    });
    expect(configureCalls(calls)).toHaveLength(0);
    expect(calls.some((call) => call.command === "secrets:set")).toBe(false);
    // 只写这个新环境自己的配置文件，且只在它还不存在时写
    await waitFor(() => expect(calls.some((call) => call.command === "profiles:apply")).toBe(true));
    expect(calls.find((call) => call.command === "profiles:apply")!.args).toEqual(["--only", "profiles:relay", "--base", "missing"]);
  });

  it("建成独立环境时查不到实例状态：提示配置没写入，不留未处理的错误", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "official",
      profiles: {
        official: { label: "官方" },
        relay: { label: "我的中转", base_url: "https://relay.example.com", auth: "api_key", token_var: "RELAY_KEY" },
      },
    };
    // 没有注册 profiles:status：模拟引擎调用本身失败
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);
    await userEvent.click(await screen.findByTestId("make-profile-quick-claude-code-relay"));
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));
    expect(await screen.findByText("已保存独立环境 relay，但配置没有写入，启动命令也先不写")).toBeInTheDocument();
    expect(calls.some((call) => call.command === "profiles:apply")).toBe(false);
  });

  it("接入卡片说明已建成哪个独立环境；还没建成的直接给「建成独立环境」", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "official",
      profiles: {
        official: { label: "官方" },
        glm: { label: "智谱", base_url: "https://open.bigmodel.cn/api/anthropic", auth: "token", token_var: "GLM_API_KEY", model: "glm-5.3" },
        kimi: { label: "Kimi", base_url: "https://api.moonshot.cn/anthropic", auth: "token", token_var: "KIMI_API_KEY", model: "kimi-k3" },
      },
    };
    withProfiles["policy:get"] = makeEnvelope(
      makePolicyData({
        effective: {
          agents: {
            definitions: {
              "cc-glm": {
                display_name: "Claude · GLM",
                profile: { base_agent: "claude-code", env: { ANTHROPIC_BASE_URL: "https://open.bigmodel.cn/api/anthropic", ANTHROPIC_AUTH_TOKEN: "${GLM_API_KEY}" } },
              },
            },
          },
        },
      }),
      { command: "policy.get" },
    );
    withProfiles["profiles:status"] = makeEnvelope(
      { domain: "profiles", elements: [makeElement({
        id: "profiles:cc-glm",
        domain: "profiles",
        spec: { display_name: "Claude · GLM", base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-cc-glm", base_url: "https://open.bigmodel.cn/api/anthropic" },
        observed: { kind: "claude", config_dir: "~/.claude-cc-glm", target: "~/.claude-cc-glm/settings.json", fields: [] },
        state: "not-materialized",
      })] },
      { command: "profiles.status" },
    );
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);
    expect(await screen.findByTestId("connection-instances-claude-code-glm")).toHaveTextContent("已建成独立环境：cc-glm");
    expect(screen.getByTestId("connection-instances-claude-code-glm")).not.toHaveTextContent("终端里输入同名命令");
    const profileCard = await screen.findByTestId("profile-card-cc-glm");
    const scrollIntoView = vi.fn();
    Object.defineProperty(profileCard, "scrollIntoView", { configurable: true, value: scrollIntoView });
    await userEvent.click(within(screen.getByTestId("connection-instances-claude-code-glm")).getByRole("button", { name: "cc-glm" }));
    expect(scrollIntoView).toHaveBeenCalledWith({ behavior: "smooth", block: "center" });
    expect(profileCard).toHaveFocus();
    expect(calls.some((call) => call.command === "agents:configure" || call.command === "profiles:apply" || call.command === "aliases:apply")).toBe(false);
    expect(screen.queryByTestId("make-profile-quick-claude-code-glm")).toBeNull();
    await userEvent.click(screen.getByLabelText("管理 智谱"));
    expect(screen.getByTestId("make-profile-claude-code-glm")).toBeInTheDocument();
    expect(screen.queryByText("还没建成独立环境。")).toBeNull();
    expect(screen.getByTestId("make-profile-quick-claude-code-kimi")).toHaveTextContent("建成独立环境");
    expect(screen.queryByTestId("make-profile-claude-code-kimi")).toBeNull();
    await userEvent.click(screen.getByTestId("make-profile-quick-claude-code-kimi"));
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    expect(within(dialog).getByTestId("profile-base-url")).toHaveValue("https://api.moonshot.cn/anthropic");
  });

  it("添加接入只保存 Provider 模板，之后可在卡片上建独立环境", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await userEvent.click(await screen.findByTestId("add-connection-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /DeepSeek/ }));
    expect(within(dialog).queryByTestId("connection-environment-options")).toBeNull();
    await userEvent.click(within(dialog).getByTestId("connection-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    expect(calls.some((call) => call.command === "profiles:apply")).toBe(false);
  });

  it("添加接入时可获取模型列表挑模型；别名可一键都用默认模型；子代理模型随接入保存", async () => {
    const withModels = routes();
    withModels["agents:models"] = makeEnvelope(
      { endpoint: "https://api.deepseek.com/anthropic/v1/models", count: 2, models: ["deepseek-v4-pro", "deepseek-v4-flash"] },
      { command: "agents.models" },
    );
    const rendered = renderWithEngine(<AgentsPage />, withModels);
    await userEvent.click(await screen.findByTestId("add-connection-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /DeepSeek/ }));
    await userEvent.click(within(dialog).getByTestId("connection-fetch-models"));
    expect(await within(dialog).findByTestId("connection-models-message")).toHaveTextContent("已获取 2 个模型");
    const options = [...within(dialog).getByTestId("connection-model-options").querySelectorAll("option")].map((o) => o.getAttribute("value"));
    expect(options).toEqual(["deepseek-v4-pro", "deepseek-v4-flash"]);
    const fetch = rendered.calls.find((call) => call.command === "agents:models")!;
    expect(JSON.parse(fetch.args[fetch.args.indexOf("--draft") + 1])).toEqual({
      kind: "claude",
      base_url: "https://api.deepseek.com/anthropic",
      token_var: "DEEPSEEK_API_KEY",
    });
    const model = within(dialog).getByTestId("new-connection-model");
    await userEvent.clear(model);
    await userEvent.type(model, "deepseek-v4-pro");
    await userEvent.click(within(dialog).getByText("高级字段"));
    await userEvent.click(within(dialog).getByTestId("connection-fill-aliases"));
    for (const tier of ["opus", "sonnet", "haiku", "fable"]) {
      expect(within(dialog).getByTestId(`new-connection-alias-${tier}`)).toHaveValue("deepseek-v4-pro");
    }
    await userEvent.type(within(dialog).getByTestId("new-connection-subagent-model"), "deepseek-v4-flash");
    await userEvent.click(within(dialog).getByTestId("connection-form-submit"));
    await waitFor(() => expect(rendered.calls.some((call) => call.command === "policy:set")).toBe(true));
    const saved = rendered.calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args[saved.args.indexOf("--value") + 1]);
    const entry = Object.values(definition.settings.profiles as Record<string, Record<string, unknown>>).find((item) => item.base_url === "https://api.deepseek.com/anthropic")!;
    expect(entry.subagent_model).toBe("deepseek-v4-flash");
    expect(entry.model_aliases).toEqual({ opus: "deepseek-v4-pro", sonnet: "deepseek-v4-pro", haiku: "deepseek-v4-pro", fable: "deepseek-v4-pro" });
  });

  it("选中厂商预设时给出获取密钥与接入文档的地址（可复制）与出处", async () => {
    renderWithEngine(<AgentsPage />, routes());
    await userEvent.click(await screen.findByTestId("add-connection-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /DeepSeek/ }));
    const links = within(dialog).getByTestId("connection-preset-links");
    expect(links).toHaveTextContent("获取密钥：https://platform.deepseek.com/api_keys");
    expect(links).toHaveTextContent("接入文档：https://api-docs.deepseek.com/quick_start/agent_integrations/claude_code/");
    expect(links).toHaveTextContent("出处：DeepSeek API 文档");
  });

  it("有官方余额接口的接入可查余额，结果就地显示；其他厂商不给这个按钮", async () => {
    const withBalance = routes();
    const status = withBalance["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "official",
      profiles: {
        official: { label: "官方" },
        deepseek: { label: "DeepSeek", base_url: "https://api.deepseek.com/anthropic", auth: "token", token_var: "DEEPSEEK_API_KEY", model: "deepseek-flash" },
        glm: { label: "智谱", base_url: "https://open.bigmodel.cn/api/anthropic", auth: "token", token_var: "GLM_API_KEY", model: "glm-5.3" },
      },
    };
    withBalance["agents:balance"] = makeEnvelope(
      { vendor: "deepseek", items: [{ label: "余额", amount: "110.00", currency: "CNY" }] },
      { command: "agents.balance" },
    );
    const { calls } = renderWithEngine(<AgentsPage />, withBalance);
    await userEvent.click(await screen.findByTestId("connection-balance-button-claude-code-deepseek"));
    expect(await screen.findByTestId("connection-balance-claude-code-deepseek")).toHaveTextContent("余额 110.00 CNY");
    expect(calls.find((call) => call.command === "agents:balance")!.args).toEqual(["--only", "agents:claude-code", "--use", "deepseek"]);
    expect(screen.queryByTestId("connection-balance-button-claude-code-glm")).toBeNull();
  });

  it("工作台导入接入：先预览，只保存定义且不启用、不写凭据", async () => {
    const withProfiles = routes();
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    const importButton = await screen.findByTestId("import-connection-button");
    expect(importButton).toBeVisible();
    expect(screen.queryByLabelText("更多设置")).toBeNull();
    await userEvent.click(importButton);
    const dialog = await screen.findByRole("dialog", { name: "导入接入" });
    expect(dialog).toHaveTextContent("不导入凭据取值、不启用接入");
    fireEvent.change(screen.getByTestId("connection-import-text"), { target: { value: JSON.stringify({
      format: "agent-manager/provider/v1",
      kind: "claude",
      connection: {
        name: "shared-relay",
        label: "分享线路",
        baseUrl: "https://shared.example.com",
        auth: "token",
        tokenVar: "SHARED_KEY",
        model: "shared-model",
        smallFastModel: "",
        aliasOpus: "",
        aliasSonnet: "",
        aliasHaiku: "",
        timeoutMs: "",
      },
    }) } });
    await userEvent.click(screen.getByTestId("connection-import-inspect"));
    expect(screen.getByTestId("connection-import-preview")).toHaveTextContent("分享线路");
    await userEvent.click(screen.getByTestId("connection-import-confirm"));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.profiles["shared-relay"]).toEqual({
      label: "分享线路",
      base_url: "https://shared.example.com",
      token_var: "SHARED_KEY",
      model: "shared-model",
    });
    expect(definition.settings.active).toBeUndefined();
    expect(configureCalls(calls)).toHaveLength(0);
    expect(calls.some((call) => call.command === "secrets:set")).toBe(false);
  });

  it("工作台删除非当前接入：确认后只删目标定义，不切换也不删除凭据", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "official",
      profiles: {
        official: { label: "官方" },
        relay: { label: "我的中转", base_url: "https://relay.example.com" },
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    await userEvent.click(await screen.findByTestId("remove-connection-claude-code-relay"));
    const confirm = await screen.findByRole("dialog", { name: "删除接入 我的中转" });
    expect(confirm).toHaveTextContent("不会删除凭据变量");
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(confirm).not.toBeInTheDocument());
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.active).toBe("official");
    expect(definition.settings.profiles).toEqual({ official: { label: "官方" } });
    expect(configureCalls(calls)).toHaveLength(0);
    expect(calls.some((call) => call.command === "secrets:set")).toBe(false);
  });

  it("当前接入不能删除，按钮说明先切换且不会触发任何写入", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      profiles: {
        relay: { label: "我的中转", base_url: "https://relay.example.com" },
      },
    };
    markCurrent(status.data.elements[0] as EngineElement, "relay");
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    const remove = await screen.findByTestId("remove-connection-claude-code-relay");
    expect(remove).toBeDisabled();
    expect(remove).toHaveAttribute("title", expect.stringContaining("先切换"));
    expect(calls.some((call) => ["policy:set", "agents:configure", "secrets:set"].includes(call.command))).toBe(false);
  });

  it("只有一套接入时保留单项测试，不显示重复的批量检查和排序", async () => {
    const oneConnection = routes({
      "agents:check": makeEnvelope(
        { ok: true, detail: "连接正常", latency_ms: 72 },
        { command: "agents.check" },
      ),
    });
    const status = oneConnection["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "official",
      profiles: { relay: { label: "Relay", base_url: "https://relay.example.com" } },
    };
    renderWithEngine(<AgentsPage />, oneConnection);

    expect(await screen.findByTestId("third-party-section-claude-code")).toBeInTheDocument();
    expect(screen.queryByTestId("check-all-connections-button")).toBeNull();
    const testConnection = screen.getByTestId("check-connection-claude-code-relay");
    expect(testConnection).toBeVisible();
    await userEvent.click(testConnection);
    expect(await screen.findByTestId("check-result-claude-code-relay")).toHaveTextContent("72ms");
    expect(screen.queryByTestId("connection-check-summary")).toBeNull();
    await userEvent.click(screen.getByLabelText("管理 Relay"));
    expect(screen.queryByRole("button", { name: "上移 Relay" })).toBeNull();
    expect(screen.queryByRole("button", { name: "下移 Relay" })).toBeNull();
    expect(within(screen.getByLabelText("管理 Relay").closest("details")!).queryByTestId("check-connection-claude-code-relay")).toBeNull();
  });

  it("接入卡片可调整顺序，只保存定义且不触碰配置或认证", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "beta",
      profiles: {
        alpha: { label: "Alpha", base_url: "https://a.example.com", future: 1 },
        beta: { label: "Beta", base_url: "https://b.example.com", future: 2 },
        gamma: { label: "Gamma", base_url: "https://c.example.com", future: 3 },
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    expect(await screen.findByRole("button", { name: "上移 Alpha" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "上移 Beta" }));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(Object.keys(definition.settings.profiles)).toEqual(["beta", "alpha", "gamma"]);
    expect(definition.settings.active).toBe("beta");
    expect(definition.settings.profiles.beta.future).toBe(2);
    expect(configureCalls(calls)).toHaveLength(0);
    expect(calls.some((call) => call.command === "secrets:set")).toBe(false);
  });

  it("测一下：只读地问一次，把结果就地说清楚", async () => {
    const withProfiles = routes({
      "agents:check": makeEnvelope(
        {
          domain: "agents",
          action: "check",
          element: "agents:claude-code",
          profile: "relay",
          ok: true,
          http_status: 200,
          latency_ms: 432,
          detail: "通了：地址、令牌与模型都没问题",
        },
        { command: "agents.check" },
      ),
    });
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      profiles: { relay: { label: "我的中转", base_url: "https://relay.example.com" } },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    await userEvent.click(await screen.findByTestId("check-connection-claude-code-relay"));
    const result = await screen.findByTestId("check-result-claude-code-relay");
    expect(result).toHaveTextContent("432ms");

    const checks = calls.filter((call) => call.command === "agents:check");
    expect(checks).toHaveLength(1);
    expect(checks[0].args).toEqual(["--only", "agents:claude-code", "--use", "relay"]);
    // 只读：不该顺手写配置
    expect(calls.some((call) => call.command === "agents:configure")).toBe(false);
  });

  it("测一下失败：把引擎给的原因原样摆出来", async () => {
    const withProfiles = routes({
      "agents:check": makeEnvelope(
        {},
        {
          command: "agents.check",
          ok: false,
          errors: [
            {
              code: "AGENT_ERROR",
              element: null,
              message: "环境变量 RELAY_KEY 还没有取值：先在密钥页填上，或在启动环境里 export 它",
            },
          ],
        },
      ),
    });
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      profiles: { relay: { label: "我的中转", base_url: "https://relay.example.com" } },
    };
    renderWithEngine(<AgentsPage />, withProfiles);

    await userEvent.click(await screen.findByTestId("check-connection-claude-code-relay"));
    const result = await screen.findByTestId("check-result-claude-code-relay");
    expect(result).toHaveTextContent("RELAY_KEY");
    expect(result).toHaveTextContent("密钥页");
  });

  it("一键检查全部接入：并发回填成功与失败，汇总最快线路且不重复提交，不写主配置", async () => {
    const batchRoutes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    const status = batchRoutes["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      profiles: {
        relay: { label: "Relay", base_url: "https://relay.example.com" },
        broken: { label: "Broken", base_url: "https://broken.example.com" },
      },
    };
    setClaudeCurrentConfig(status.data.elements[0] as EngineElement, null);
    const releases = new Map<string, () => void>();
    batchRoutes["agents:check"] = (args) => new Promise<Envelope>((resolve) => {
      const profile = args[args.indexOf("--use") + 1];
      releases.set(profile, () => resolve(profile === "relay"
        ? makeEnvelope(
          { ok: true, detail: "连接正常", latency_ms: 128 },
          { command: "agents.check" },
        )
        : makeEnvelope(
          {},
          {
            command: "agents.check",
            ok: false,
            errors: [{ code: "AGENT_ERROR", element: null, message: "模型不可用" }],
          },
        )));
    });
    batchRoutes["agents:configure"] = async (args) => {
      const settings = status.data.elements[0].spec.settings as { active?: string };
      settings.active = args[args.indexOf("--use") + 1];
      setClaudeCurrentConfig(status.data.elements[0] as EngineElement, settings.active);
      return makeEnvelope(
        { domain: "agents", action: "configure", element: "agents:claude-code" },
        { command: "agents.configure" },
      );
    };
    const { calls } = renderWithEngine(<AgentsPage />, batchRoutes);

    const checkAll = await screen.findByTestId("check-all-connections-button");
    await userEvent.click(checkAll);
    await waitFor(() => expect(calls.filter((call) => call.command === "agents:check")).toHaveLength(2));
    expect(checkAll).toBeDisabled();
    expect(checkAll.querySelector(".animate-spin")).not.toBeNull();
    await userEvent.click(checkAll);
    expect(calls.filter((call) => call.command === "agents:check")).toHaveLength(2);

    releases.get("relay")?.();
    releases.get("broken")?.();
    const summary = await screen.findByTestId("connection-check-summary");
    await waitFor(() => expect(summary).toHaveTextContent("已完成 2/2"));
    expect(summary).toHaveTextContent("1 个可用");
    expect(summary).toHaveTextContent("最快：Relay 128ms");
    expect(screen.getByTestId("check-result-claude-code-relay")).toHaveTextContent("128ms");
    expect(screen.getByTestId("check-result-claude-code-broken")).toHaveTextContent("模型不可用");
    expect(checkAll).toBeEnabled();
    // 主配置只放官方账号：检查只报告结果，不提供把最快线路写进主配置
    expect(screen.queryByTestId("activate-fastest-connection")).toBeNull();
    expect(configureCalls(calls)).toHaveLength(0);
  });

  it("外部设置会压过主配置的官方账号时逐条提示来源与影响，不提供修改", async () => {
    const withOverrides = routes();
    const status = withOverrides["agents:status"] as { data: { elements: EngineElement[] } };
    (status.data.elements[0].observed as Record<string, unknown>).credential_overrides = [
      { source: "~/.zshrc", name: "ANTHROPIC_BASE_URL" },
      { source: "~/.claude/settings.json", name: "apiKeyHelper" },
      { source: "/etc/claude-code/managed-settings.json", name: "forceLoginMethod" },
      { source: "~/.config/anthropic/configs/wif.json", name: "oidc_federation" },
    ];
    const { calls } = renderWithEngine(<AgentsPage />, withOverrides);
    const warning = await screen.findByTestId("main-credential-overrides");
    expect(warning).toHaveTextContent("~/.zshrc 里的 ANTHROPIC_BASE_URL：Claude 会把请求连同官方账号的登录凭据发往这个地址");
    expect(warning).toHaveTextContent("~/.claude/settings.json 里的 apiKeyHelper");
    expect(warning).toHaveTextContent("/etc/claude-code/managed-settings.json 里的 forceLoginMethod：组织统一下发的设置限定了登录方式；用第三方凭据的独立环境会在启动时被拦下");
    expect(warning).toHaveTextContent("~/.config/anthropic/configs/wif.json 里的 oidc_federation：活动的 Anthropic 配置档案是联合身份");
    expect(warning).toHaveTextContent("只提示，不会改这些文件");
    expect(within(warning).queryByRole("button")).toBeNull();
    expect(configureCalls(calls)).toHaveLength(0);
  });

  it("官方账号卡片可按需检查登录，登录与退出命令只复制不执行", async () => {
    const withAuth = routes({
      "agents:auth-status": makeEnvelope(
        { element: "agents:claude-code", state: "logged-in", logged_in: true, method: "console", third_party: false, mixed: false, reason: null },
        { command: "agents.auth-status" },
      ),
    });
    const written: string[] = [];
    Object.assign(navigator, {
      clipboard: { writeText: async (text: string) => void written.push(text) },
    });
    const { calls } = renderWithEngine(<AgentsPage />, withAuth);

    const panel = await screen.findByTestId("main-auth-status-claude-code");
    expect(within(panel).getByTestId("main-auth-text-claude-code")).toHaveTextContent("未检查");
    // 刷新页面时不调用官方命令，只在点「检查登录」时查一次
    expect(calls.some((call) => call.command === "agents:auth-status")).toBe(false);
    await userEvent.click(within(panel).getByTestId("main-auth-check-claude-code"));
    await waitFor(() => expect(within(panel).getByTestId("main-auth-text-claude-code")).toHaveTextContent("已登录（Claude Console 账号）"));
    expect(calls.find((call) => call.command === "agents:auth-status")?.args).toEqual(["--only", "agents:claude-code"]);

    await userEvent.click(within(panel).getByTestId("main-claude-code-auth-login"));
    await userEvent.click(within(panel).getByTestId("main-claude-code-auth-logout"));
    await waitFor(() => expect(written).toEqual(["claude auth login", "claude auth logout"]));
    // 主配置的命令不带配置目录变量，也不触发任何写入
    expect(written.join("\n")).not.toContain("CLAUDE_CONFIG_DIR");
    expect(configureCalls(calls)).toHaveLength(0);
  });

  it("没配接入配置的助手：工作台明确展示官方账号和新增入口", async () => {
    renderWithEngine(<AgentsPage />, routes());
    expect(await screen.findByTestId("connection-card-claude-code-default")).toHaveTextContent("当前使用");
    expect(screen.queryByTestId("reset-connection-claude-code-default")).toBeNull();
    expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("官方账号");
    expect(screen.queryByTestId("main-mixed-warning")).toBeNull();
    expect(screen.queryByTestId("main-credential-overrides")).toBeNull();
    expect(screen.getByTestId("add-connection-button")).toBeEnabled();
  });

  it("切回官方执行中只锁接入动作，并在按钮上展示真实进行态", async () => {
    const deferred: { release?: () => void } = {};
    const pendingRoutes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    const status = pendingRoutes["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      profiles: { relay: { label: "我的中转", base_url: "https://relay.example.com" } },
    };
    setClaudeCurrentConfig(status.data.elements[0] as EngineElement, "relay");
    pendingRoutes["agents:configure"] = () =>
      new Promise<Envelope>((resolve) => {
        deferred.release = () => resolve(makeEnvelope(
          { domain: "agents", action: "configure", element: "agents:claude-code" },
          { command: "agents.configure" },
        ));
      });
    renderWithEngine(<AgentsPage />, pendingRoutes);

    await switchBackToOfficial();
    const reset = screen.getByTestId("reset-connection-claude-code-default");
    await waitFor(() => expect(reset).toHaveTextContent("正在切回"));
    expect(reset).toBeDisabled();
    expect(screen.getByTestId("edit-connection-claude-code-relay")).toBeDisabled();
    expect(screen.getByTestId("workbench-tab-automation")).toBeEnabled();

    deferred.release?.();
    await waitFor(() => expect(reset).toHaveTextContent("切回官方账号"));
  });

  it("从厂商预设新增接入：保留完整定义并单独保存凭据，但不自动应用", async () => {
    const withExisting = routes();
    const status = withExisting["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.custom_meta = { owner: "local" };
    status.data.elements[0].spec.settings = {
      active: "official",
      yolo: true,
      unknown_runtime_flag: "keep-me",
      profiles: { official: { label: "官方", custom_profile_flag: 7 } },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withExisting);

    await userEvent.click(await screen.findByTestId("add-connection-button"));
    await screen.findByRole("dialog", { name: "为 Claude Code 添加接入" });
    await userEvent.click(screen.getByTestId("connection-preset-kimi-cn-claude"));
    expect(screen.getByTestId("selected-connection-preset")).toHaveTextContent("已选择：Kimi 开放平台（国内）");
    expect(screen.getByRole("heading", { name: "Provider 配置" })).toBeInTheDocument();
    expect(screen.getByTestId("new-connection-base-url")).toHaveValue("https://api.moonshot.cn/anthropic");
    expect(screen.getByTestId("new-connection-model")).toHaveValue("kimi-k3[1m]");
    await userEvent.click(screen.getByTestId("connection-preset-deepseek-claude"));
    expect(screen.getByTestId("selected-connection-preset")).toHaveTextContent("已选择：DeepSeek");
    expect(screen.getByTestId("new-connection-base-url")).toHaveValue("https://api.deepseek.com/anthropic");
    expect(screen.getByTestId("new-connection-model")).toHaveValue("deepseek-flash[1m]");
    await userEvent.click(screen.getByTestId("connection-preset-kimi-cn-claude"));
    await userEvent.click(screen.getByTestId("new-connection-advanced"));
    expect(screen.getByTestId("new-connection-alias-opus")).toHaveValue("kimi-k3[1m]");
    await userEvent.clear(screen.getByTestId("new-connection-alias-opus"));
    await userEvent.type(screen.getByTestId("new-connection-alias-opus"), "custom-opus");
    await userEvent.type(screen.getByTestId("new-connection-secret"), "fake-secret-value");
    await userEvent.click(screen.getByTestId("connection-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "secrets:set")).toBe(true));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.custom_meta).toEqual({ owner: "local" });
    expect(definition.settings.yolo).toBe(true);
    expect(definition.settings.unknown_runtime_flag).toBe("keep-me");
    expect(definition.settings.active).toBe("official");
    expect(definition.settings.profiles.official).toEqual({ label: "官方", custom_profile_flag: 7 });
    expect(definition.settings.profiles["kimi-cn"]).toMatchObject({
      label: "Kimi 开放平台（国内）",
      base_url: "https://api.moonshot.cn/anthropic",
      token_var: "KIMI_API_KEY",
      model: "kimi-k3[1m]",
      model_aliases: {
        opus: "custom-opus",
        sonnet: "kimi-k3[1m]",
        haiku: "kimi-k2.7-code",
      },
    });
    const secret = calls.find((call) => call.command === "secrets:set")!;
    expect(secret.args).toEqual(["--var", "KIMI_API_KEY", "--value", "fake-secret-value"]);
    expect(configureCalls(calls)).toHaveLength(0);
    expect(calls.indexOf(saved)).toBeLessThan(calls.indexOf(secret));
  });

  it("第一次填 API Key 还没有密钥文件：表单先说明会新建到哪，保存时先新建（仅本人可读）再写入", async () => {
    const secretsStatus = (source: Record<string, unknown>) =>
      makeEnvelope(
        {
          domain: "secrets",
          elements: [
            makeElement({ id: "secrets:mcp-env", domain: "secrets", spec: {}, policy: {}, observed: { variables: [], source }, state: "managed-ok", actions: [] }),
          ],
        },
        { command: "secrets.status" },
      );
    const { calls } = renderWithEngine(<AgentsPage />, routes({
      "secrets:status": secretsStatus({ configured: false, ready: false, path: null, error: null, names: [] }),
      "secrets:source": makeEnvelope({ domain: "secrets", configured: true, created: true }, { command: "secrets.source" }),
    }));

    await userEvent.click(await screen.findByTestId("add-connection-button"));
    await userEvent.click(screen.getByTestId("connection-preset-kimi-cn-claude"));
    expect(screen.getByTestId("new-connection-key-status")).toHaveTextContent(
      "还没有密钥文件：保存时会新建 ~/.config/agent-manager/secrets.env（仅本人可读），API Key 只写进这个文件",
    );
    await userEvent.type(screen.getByTestId("new-connection-secret"), "fake-secret-value");
    await userEvent.click(screen.getByTestId("connection-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "secrets:set")).toBe(true));
    const source = calls.find((call) => call.command === "secrets:source")!;
    expect(source.args).toEqual(["--path", "~/.config/agent-manager/secrets.env", "--create"]);
    const secret = calls.find((call) => call.command === "secrets:set")!;
    expect(calls.indexOf(source)).toBeLessThan(calls.indexOf(secret));
    expect(await screen.findByText("已新建密钥文件 ~/.config/agent-manager/secrets.env")).toBeInTheDocument();
  });

  it("密钥文件里已有这个变量：说明留空保持不变；没有时说明要填上才能用", async () => {
    const withSource = (names: string[]) =>
      makeEnvelope(
        {
          domain: "secrets",
          elements: [
            makeElement({
              id: "secrets:mcp-env",
              domain: "secrets",
              spec: {},
              policy: {},
              observed: { variables: [], source: { configured: true, ready: true, path: "~/private/secrets.env", error: null, names } },
              state: "managed-ok",
              actions: [],
            }),
          ],
        },
        { command: "secrets.status" },
      );
    const { calls } = renderWithEngine(<AgentsPage />, routes({ "secrets:status": withSource(["KIMI_API_KEY"]) }));
    await userEvent.click(await screen.findByTestId("add-connection-button"));
    await userEvent.click(screen.getByTestId("connection-preset-kimi-cn-claude"));
    expect(screen.getByTestId("new-connection-key-status")).toHaveTextContent(
      "~/private/secrets.env 里已有 KIMI_API_KEY 的值：留空保持不变，填了就替换",
    );
    await userEvent.clear(screen.getByTestId("new-connection-token-var"));
    await userEvent.type(screen.getByTestId("new-connection-token-var"), "OTHER_KEY");
    expect(screen.getByTestId("new-connection-key-status")).toHaveTextContent("~/private/secrets.env 里还没有 OTHER_KEY：填上才能用");
    // 已有密钥文件时不再新建
    expect(calls.some((call) => call.command === "secrets:source")).toBe(false);
  });

  it("凭据保存被拒绝：保留弹窗和输入，说明已保存到哪一步且不继续启用", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes({
      "secrets:set": makeEnvelope(
        {},
        {
          command: "secrets.set",
          ok: false,
          errors: [{ code: "SECRET_ERROR", element: null, message: "系统钥匙串暂时不可用" }],
        },
      ),
    }));

    await userEvent.click(await screen.findByTestId("add-connection-button"));
    await userEvent.click(screen.getByTestId("connection-preset-kimi-cn-claude"));
    await userEvent.type(screen.getByTestId("new-connection-secret"), "fake-secret-value");
    await userEvent.click(screen.getByTestId("connection-form-submit"));

    expect(await screen.findByRole("alert")).toHaveTextContent("接入已保存，但凭据没有写入外部文件");
    expect(screen.getByRole("alert")).toHaveTextContent("系统钥匙串暂时不可用");
    expect(screen.getByTestId("new-connection-secret")).toHaveValue("fake-secret-value");
    expect(screen.getByRole("dialog", { name: "为 Claude Code 添加接入" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "打开密钥设置" })).toBeNull();
    expect(calls.some((call) => call.command === "policy:set")).toBe(true);
    expect(configureCalls(calls)).toHaveLength(0);
  });

  it("凭据保存失败时可从错误处直接打开密钥设置", async () => {
    const onOpenSecrets = vi.fn();
    renderWithEngine(<AgentsPage onOpenSecrets={onOpenSecrets} />, routes({
      "secrets:set": makeEnvelope({}, {
        command: "secrets.set",
        ok: false,
        errors: [{ code: "SECRET_ERROR", element: null, message: "外部密钥文件尚未选择" }],
      }),
    }));

    await userEvent.click(await screen.findByTestId("add-connection-button"));
    await userEvent.click(screen.getByTestId("connection-preset-kimi-cn-claude"));
    await userEvent.type(screen.getByTestId("new-connection-secret"), "fake-secret-value");
    await userEvent.click(screen.getByTestId("connection-form-submit"));
    await userEvent.click(await screen.findByRole("button", { name: "打开密钥设置" }));

    expect(onOpenSecrets).toHaveBeenCalledTimes(1);
  });

  it("新增接入取消：关闭弹窗并恢复焦点，零定义、凭据和配置写入", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    const add = await screen.findByTestId("add-connection-button");
    await userEvent.click(add);
    await screen.findByRole("dialog", { name: "为 Claude Code 添加接入" });
    await userEvent.click(screen.getByTestId("connection-preset-deepseek-claude"));
    await userEvent.keyboard("{Escape}");

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "为 Claude Code 添加接入" })).toBeNull());
    expect(document.activeElement).toBe(add);
    expect(calls.some((call) => ["policy:set", "secrets:set", "agents:configure"].includes(call.command))).toBe(false);
  });

  it("自定义接入把校验错误留在弹窗里说明", async () => {
    renderWithEngine(<AgentsPage />, routes());
    await userEvent.click(await screen.findByTestId("add-connection-button"));
    await userEvent.click(screen.getByTestId("connection-preset-custom"));
    await userEvent.click(screen.getByTestId("connection-form-submit"));

    expect(await screen.findByRole("alert")).toHaveTextContent("接口地址必须是 https://");
    expect(screen.getByRole("dialog", { name: "为 Claude Code 添加接入" })).toBeInTheDocument();
  });

  it("工作台可切换助手，并展示所选助手自己的当前接入", async () => {
    const twoAgents = routes();
    const status = twoAgents["agents:status"] as { data: { elements: EngineElement[] } };
    status.data.elements.push(makeElement({
      id: "agents:codex",
      domain: "agents",
      spec: {
        display_name: "Codex",
        bin: "codex",
        providers: { native: { managed: true } },
        settings: {
          active: "glm",
          profiles: { glm: { label: "GLM Coding", base_url: "https://open.bigmodel.cn/api/v1" } },
        },
      },
      policy: { enabled: true },
      observed: {
        install_source: "native",
        installed: true,
        management: agentManagementFixture("codex"),
        current_config: {
          state: "ready",
          file: "config.toml",
          config: { provider: "glm", base_url: "https://open.bigmodel.cn/api/v1", runtime: {} },
          base: BASE,
          current: { ...DEFAULT_CURRENT, profile: "glm", default: false },
        },
      },
      state: "managed-ok",
      freshness: makeFreshness(),
      actions: [],
    }));
    renderWithEngine(<AgentsPage />, twoAgents);

    expect(await screen.findByTestId("active-connection-summary")).toHaveTextContent("官方账号");
    await userEvent.click(screen.getByTestId("connection-agent-codex"));
    expect(screen.getByTestId("connection-agent-codex")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("GLM Coding");
  });

  it("Codex 配置里有会让它拒绝加载的写法时显著提示；default_permissions 接管沙箱时说明档位只改审批", async () => {
    const withIssues = agentRoutes("codex", "codex");
    const status = withIssues["agents:status"] as { data: { elements: EngineElement[] } };
    (status.data.elements[0].observed as Record<string, any>).current_config = {
      state: "ready",
      file: "config.toml",
      target: "/Users/demo/.codex/config.toml",
      config: {
        provider: "openai",
        runtime: { automation_mode: "manual" },
        fatal_issues: ["approval_policy = \"untrusted\" 已被 Codex 移除，请删掉这一行或改用 on-request"],
        default_permissions: ":workspace",
      },
      base: BASE,
      current: DEFAULT_CURRENT,
    };
    renderWithEngine(<AgentsPage />, withIssues);

    const alert = await screen.findByTestId("codex-fatal-issues");
    expect(alert).toHaveTextContent("Codex 会拒绝加载这份配置");
    expect(alert).toHaveTextContent("approval_policy = \"untrusted\" 已被 Codex 移除");
    await userEvent.click(screen.getByTestId("workbench-tab-automation"));
    expect(screen.getByTestId("codex-default-permissions")).toHaveTextContent("default_permissions = :workspace");
  });

  it("Codex 也能配接入配置，但没有认证方式与状态栏那两项", async () => {
    renderWithEngine(<AgentsPage />, agentRoutes("codex", "codex"));
    await userEvent.click(await screen.findByTestId("add-connection-button"));
    await userEvent.click(screen.getByTestId("connection-preset-custom"));
    // 地址、令牌变量名、模型都有；取值和 Claude 一样可选填，只进外部密钥文件
    expect(screen.getByTestId("new-connection-base-url")).toBeInTheDocument();
    expect(screen.getByTestId("new-connection-token-var")).toBeInTheDocument();
    expect(screen.getByTestId("new-connection-model")).toBeInTheDocument();
    expect(screen.getByTestId("new-connection-secret")).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");

    await openRuntimeSettings();
    expect(await screen.findByTestId("agent-runtime-panel")).toBeInTheDocument();
    expect(screen.queryByTestId("agent-runtime-statusline")).toBeNull();
    expect(screen.getByTestId("agent-runtime-automation-mode")).toBeInTheDocument();
  });

  it("既不是 Claude 也不是 Codex 的助手：不进入接入工作台", async () => {
    renderWithEngine(<AgentsPage />, agentRoutes("opencode", "opencode"));
    await waitFor(() => expect(screen.queryByTestId("connection-workbench")).toBeNull());
  });

  it("不支持的历史助手定义不再露出接入卡片", async () => {
    const unsupported = agentRoutes("opencode", "opencode");
    const status = unsupported["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      profiles: { relay: { label: "历史接入", base_url: "https://relay.example.com" } },
    };
    renderWithEngine(<AgentsPage />, unsupported);

    await waitFor(() => expect(screen.queryByTestId("connection-workbench")).toBeNull());
    expect(screen.queryByTestId("connection-card-opencode-relay")).toBeNull();
    expect(screen.queryByTestId("connection-workbench")).toBeNull();
  });
});
