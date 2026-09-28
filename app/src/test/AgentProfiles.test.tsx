import { describe, expect, it, onTestFinished, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import type { Envelope } from "../lib/contract";
import { agentManagementFixture, makeAliasStatus, makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

function profileElement(overrides: Record<string, unknown> = {}) {
  return makeElement({
    id: "profiles:cc-ly",
    domain: "profiles",
    spec: {
      display_name: "Claude Code · ly",
      base_agent: "claude-code",
      kind: "claude",
      config_dir: "~/.claude-cc-ly",
      base_url: "https://api.example.com",
      model: null,
    },
    policy: { enabled: true },
    observed: {
      base_agent: "claude-code",
      kind: "claude",
      config_env: "CLAUDE_CONFIG_DIR",
      config_dir: "~/.claude-cc-ly",
      config_dir_exists: false,
      target: "~/.claude-cc-ly/settings.json",
      target_exists: false,
      env_refs: [{ name: "ANTHROPIC_AUTH_TOKEN", kind: "reference", variable: "MY_TOKEN", resolved: true }],
      launch_value: 'CLAUDE_CONFIG_DIR="~/.claude-cc-ly" claude --dangerously-skip-permissions',
      usage: {
        source: "/home/ly/.claude-cc-ly/.claude.json",
        checked_at: "2026-09-18T03:00:00Z",
        windows: [{ window_minutes: 300, used_percent: 7, resets_at: null }],
      },
      base: "missing",
      fields: [],
      blocked_reason: null,
    },
    state: "not-materialized",
    freshness: makeFreshness({ state: "not-applicable" }),
    actions: [{ id: "apply", risk: "write-local", needs_lock: true, offline_ok: true }],
    ...overrides,
  });
}

function routes(profiles: ReturnType<typeof profileElement>[] = [profileElement()]): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "policy:get": makeEnvelope(
      makePolicyData({
        layers: {
          sqlite: {
            agents: {
              definitions: {
                "cc-ly": {
                  display_name: "Claude Code · ly",
                  bin: "claude",
                  profile: {
                    base_agent: "claude-code",
                    kind: "claude",
                    config_dir: "~/.claude-cc-ly",
                    env: { ANTHROPIC_BASE_URL: "https://api.example.com", ANTHROPIC_AUTH_TOKEN: "${MY_TOKEN}" },
                  },
                },
              },
            },
          },
        },
      }),
      { command: "policy.get" },
    ),
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:claude-code",
            domain: "agents",
            spec: { display_name: "Claude Code", bin: "claude", preferred_provider: "native", providers: { native: { managed: true } } },
            policy: { enabled: true },
            observed: { install_source: "native", installed: true, management: agentManagementFixture("claude") },
            state: "managed-ok",
            freshness: makeFreshness(),
            actions: [],
          }),
          makeElement({
            id: "agents:cc-ly",
            domain: "agents",
            spec: { display_name: "Claude Code · ly", bin: "claude" },
            policy: { enabled: true },
            observed: { install_source: null, installed: true },
            state: "unmanaged-detected",
            freshness: makeFreshness(),
            actions: [],
          }),
        ],
      },
      { command: "agents.status" },
    ),
    "profiles:status": makeEnvelope({ domain: "profiles", elements: profiles }, { command: "profiles.status" }),
    "profiles:apply": makeEnvelope(makeApplyResult({ domain: "profiles" }), { command: "profiles.apply" }),
    "profiles:adopt": makeEnvelope(makeApplyResult({ domain: "profiles" }), { command: "profiles.adopt" }),
    "profiles:seed": makeEnvelope(
      {
        domain: "profiles",
        action: "seed",
        element: "profiles:cc-work",
        results: [
          { item: "permissions", state: "copied", detail: "已带上权限规则" },
          { item: "hooks", state: "copied", detail: "已带上 1 条 hooks" },
          { item: "rules", state: "absent", detail: "默认环境没有 CLAUDE.md" },
        ],
      },
      { command: "profiles.seed" },
    ),
    "agents:apply": makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
    "policy:set": makeEnvelope(makePolicyData(), { command: "policy.set" }),
    "aliases:status": makeEnvelope(
      makeAliasStatus({
        // 指向该实例配置目录的两条命令：卡片按配置目录反查，应两条都列出
        "cc-ly": 'CLAUDE_CONFIG_DIR="$HOME/.claude-cc-ly" claude',
        "cc-ly-yolo": 'CLAUDE_CONFIG_DIR="$HOME/.claude-cc-ly" claude --dangerously-skip-permissions',
        // 与该实例无关的一条：不该出现在实例卡片里
        claude: "claude",
      }),
      { command: "aliases.status" },
    ),
  };
}

describe("助手实例", () => {
  async function openInstanceManager() {
    // 独立环境与默认环境在同一页、同一个助手下列出，不用再切页签
    return screen.findByTestId("independent-environments");
  }

  async function openNewProfileForm() {
    await userEvent.click(await screen.findByTestId("new-environment-button"));
    await userEvent.click(screen.getByTestId("new-environment-manual"));
  }

  it("环境表单按当前输入预览将写入的配置，修改后不展示过期结果", async () => {
    const all: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    all["profiles:preview"] = async (args) => {
      const definition = JSON.parse(args[args.indexOf("--definition") + 1]);
      return makeEnvelope({
        path: `${definition.profile.config_dir}/settings.json`,
        exists: false,
        content: JSON.stringify({ env: definition.profile.env }),
      }, { command: "profiles.preview" });
    };
    const { calls } = renderWithEngine(<AgentsPage />, all);
    await openNewProfileForm();
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    expect(calls.some((call) => call.command === "profiles:preview")).toBe(false);
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "demo");
    await userEvent.type(within(dialog).getByTestId("profile-base-url"), "https://api.example.com");
    expect(calls.some((call) => call.command === "profiles:preview")).toBe(false);
    await userEvent.type(within(dialog).getByTestId("profile-token-var"), "DEMO_KEY");
    const preview = await within(dialog).findByTestId("profile-draft-preview-content");
    expect(preview).toHaveTextContent("https://api.example.com");
    expect(preview).toHaveTextContent("${DEMO_KEY}");
    expect(calls.find((call) => call.command === "profiles:preview")?.args.slice(0, 2)).toEqual(["--only", "profiles:demo"]);
    expect(calls.some((call) => ["policy:set", "profiles:apply", "aliases:apply"].includes(call.command))).toBe(false);
    await userEvent.type(within(dialog).getByTestId("profile-base-url"), "/v1");
    expect(within(dialog).queryByTestId("profile-draft-preview-content")).toBeNull();
    expect(within(dialog).getByText("表单已更改，正在更新预览。")).toBeInTheDocument();
    expect(await within(dialog).findByTestId("profile-draft-preview-content")).toHaveTextContent("https://api.example.com/v1");
  });

  for (const saveTemplate of [false, true]) {
    it(`第三方预设直接创建环境${saveTemplate ? "并可选保存 Provider 模板" : "且默认不保存重复模板"}`, async () => {
      let definitionSaved = false;
      const activeRoutes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
      activeRoutes["policy:set"] = async () => {
        definitionSaved = true;
        return makeEnvelope(makePolicyData(), { command: "policy.set" });
      };
      activeRoutes["profiles:status"] = async () => makeEnvelope({
        domain: "profiles",
        elements: definitionSaved
          ? [profileElement(), profileElement({ id: "profiles:deepseek" })]
          : [profileElement()],
      }, { command: "profiles.status" });
      const { calls } = renderWithEngine(<AgentsPage />, activeRoutes);
      await userEvent.click(await screen.findByTestId("new-environment-button"));
      await userEvent.click(screen.getByTestId("new-environment-third-party"));
      const dialog = await screen.findByRole("dialog", { name: "为 Claude Code 新建第三方环境" });
      await userEvent.click(within(dialog).getByTestId("connection-preset-deepseek-claude"));
      expect(within(dialog).getByTestId("connection-environment-id")).toHaveValue("deepseek");
      expect(within(dialog).getByTestId("connection-save-template")).not.toBeChecked();
      if (saveTemplate) await userEvent.click(within(dialog).getByTestId("connection-save-template"));
      await userEvent.click(within(dialog).getByTestId("connection-form-submit"));
      await waitFor(() => expect(calls.some((call) => call.command === "profiles:apply")).toBe(true));
      const writes = calls.filter((call) => call.command === "policy:set");
      expect(writes.map((call) => call.args[call.args.indexOf("--key") + 1])).toEqual(saveTemplate
        ? ["agents.definitions.claude-code", "agents.definitions.deepseek"]
        : ["agents.definitions.deepseek"]);
      const lastWrite = writes[writes.length - 1];
      const definition = JSON.parse(lastWrite.args[lastWrite.args.indexOf("--value") + 1]);
      expect(definition.profile.source_connection).toBe(saveTemplate ? "deepseek" : undefined);
      expect(definition.profile.env.ANTHROPIC_BASE_URL).toBe("https://api.deepseek.com/anthropic");
      expect(definition.profile.env.ANTHROPIC_AUTH_TOKEN).toBe("${DEEPSEEK_API_KEY}");
      expect(calls.find((call) => call.command === "profiles:apply")!.args).toEqual(["--only", "profiles:deepseek", "--base", "missing"]);
      expect(calls.some((call) => call.command === "agents:configure")).toBe(false);
      const card = await screen.findByTestId("profile-card-deepseek");
      await waitFor(() => expect(card).toHaveFocus());
      expect(within(card).getByTestId("profile-config-file-toggle-deepseek")).toHaveAttribute("aria-expanded", "false");
    });
  }

  it("Provider 表单按当前新环境草稿预览文件和启动命令，输入变化后隐藏旧结果", async () => {
    const all: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    all["profiles:preview"] = async (args) => {
      const definition = JSON.parse(args[args.indexOf("--definition") + 1]);
      return makeEnvelope({
        path: `${definition.profile.config_dir}/settings.json`,
        exists: false,
        content: JSON.stringify({ env: definition.profile.env }),
      }, { command: "profiles.preview" });
    };
    const { calls } = renderWithEngine(<AgentsPage />, all);
    await userEvent.click(await screen.findByTestId("new-environment-button"));
    await userEvent.click(screen.getByTestId("new-environment-third-party"));
    const dialog = await screen.findByRole("dialog", { name: "为 Claude Code 新建第三方环境" });
    await userEvent.click(within(dialog).getByTestId("connection-preset-deepseek-claude"));
    await userEvent.clear(within(dialog).getByTestId("connection-environment-id"));
    expect(within(dialog).getByText("选定接入并填写有效的环境名后，可预览客户端配置文件和启动命令。")).toBeInTheDocument();
    await userEvent.type(within(dialog).getByTestId("connection-environment-id"), "deepseek");
    expect(within(dialog).getByTestId("connection-environment-launch-preview")).toHaveTextContent("alias deepseek=");
    const content = await within(dialog).findByTestId("connection-preview-content-deepseek");
    expect(content).toHaveTextContent("https://api.deepseek.com/anthropic");
    expect(content).toHaveTextContent("${DEEPSEEK_API_KEY}");
    expect(calls.find((call) => call.command === "profiles:preview")?.args.slice(0, 2)).toEqual(["--only", "profiles:deepseek"]);
    expect(calls.some((call) => ["policy:set", "profiles:apply", "aliases:apply"].includes(call.command))).toBe(false);
    await userEvent.click(within(dialog).getByTestId("connection-save-template"));
    expect(within(dialog).queryByTestId("connection-preview-content-deepseek")).toBeNull();
    expect(within(dialog).getByText("表单或勾选目标已变化，正在更新预览。")).toBeInTheDocument();
    await waitFor(() => expect(calls.filter((call) => call.command === "profiles:preview")).toHaveLength(2));
    const updated = calls.filter((call) => call.command === "profiles:preview")[1];
    expect(JSON.parse(updated.args[updated.args.indexOf("--definition") + 1]).profile.source_connection).toBe("deepseek");
  });

  it("第三方环境的凭据写入失败时保留表单，不保存环境或 Provider", async () => {
    const activeRoutes = routes();
    activeRoutes["secrets:set"] = makeEnvelope({}, {
      command: "secrets.set",
      ok: false,
      errors: [{ code: "SECRET_ERROR", element: null, message: "测试凭据文件不可写" }],
    });
    const { calls } = renderWithEngine(<AgentsPage />, activeRoutes);
    await userEvent.click(await screen.findByTestId("new-environment-button"));
    await userEvent.click(screen.getByTestId("new-environment-third-party"));
    const dialog = await screen.findByRole("dialog", { name: "为 Claude Code 新建第三方环境" });
    await userEvent.click(within(dialog).getByTestId("connection-preset-deepseek-claude"));
    await userEvent.type(within(dialog).getByTestId("new-connection-secret"), "fake-secret-value");
    await userEvent.click(within(dialog).getByTestId("connection-form-submit"));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("环境尚未创建");
    expect(within(dialog).getByTestId("new-connection-secret")).toHaveValue("fake-secret-value");
    expect(calls.some((call) => ["policy:set", "profiles:apply", "aliases:apply"].includes(call.command))).toBe(false);
  });

  it("创建后仍读不到新环境状态时不写配置或启动命令，并提示部分完成", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await userEvent.click(await screen.findByTestId("new-environment-button"));
    await userEvent.click(screen.getByTestId("new-environment-third-party"));
    const dialog = await screen.findByRole("dialog", { name: "为 Claude Code 新建第三方环境" });
    await userEvent.click(within(dialog).getByTestId("connection-preset-deepseek-claude"));
    await userEvent.click(within(dialog).getByTestId("connection-form-submit"));
    expect(await screen.findByText("已保存独立环境 deepseek，但配置没有写入，启动命令也先不写")).toBeInTheDocument();
    expect(calls.some((call) => call.command === "profiles:apply" || call.command === "aliases:apply")).toBe(false);
  });

  it("实例单独成卡，展示配置目录与启动方式，不混进主助手列表", async () => {
    renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    expect(within(card).getByText("Claude Code · ly")).toBeInTheDocument();
    expect(card).toHaveTextContent("~/.claude-cc-ly");
    // 卡片头部写明是什么账号、终端里输入什么启动
    expect(within(card).getByTestId("profile-summary-cc-ly")).toHaveTextContent("终端输入 cc-ly");
    // 一个实例可以挂多条启动命令，按配置目录反查后都列出来
    expect(within(card).getByTestId("profile-launch-cc-ly-cc-ly")).toHaveTextContent(
      `alias cc-ly='CLAUDE_CONFIG_DIR="$HOME/.claude-cc-ly" claude'`,
    );
    expect(within(card).getByTestId("profile-launch-cc-ly-cc-ly-yolo")).toHaveTextContent(
      "--dangerously-skip-permissions",
    );
    // 与该实例无关的命令不混进来
    expect(within(card).queryByTestId("profile-launch-cc-ly-claude")).toBeNull();
    // 启动名可直接复制；复制一条会创建命令定义的副本
    expect(within(card).getByTestId("profile-launch-name-cc-ly-cc-ly")).toHaveTextContent("复制启动名");
    expect(within(card).getByTestId("profile-launch-edit-cc-ly-cc-ly")).toBeInTheDocument();
    expect(within(card).getByTestId("profile-launch-copy-cc-ly-cc-ly")).toHaveTextContent("复制一条");
    expect(within(card).getByTestId("profile-launch-remove-cc-ly-cc-ly")).toBeInTheDocument();
    // 实例多半是另一个账号，额度读它自己的配置目录，不是默认那份
    expect(within(card).getByTestId("profile-usage-cc-ly")).toHaveTextContent("7%");
    expect(within(card).getByTestId("profile-usage-cc-ly")).toHaveTextContent("5 小时");
    // 实例不再作为主助手卡片重复出现
    expect(screen.queryByTestId("manage-skills-agents:cc-ly")).toBeNull();
    expect(within(card).getByTestId("profile-skills-cc-ly")).toBeInTheDocument();
  });

  it("实例文件按需读实际原文，刷新后展示外部修改", async () => {
    const all: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    let content = '{"model":"one"}\n';
    all["profiles:config-file"] = async (args) => {
      const selected = args[3] ?? "main";
      return makeEnvelope(
        {
          element: "profiles:cc-ly",
          path: `~/.claude-cc-ly/${selected === "mcp" ? ".claude.json" : selected === "rules" ? "CLAUDE.md" : "settings.json"}`,
          format: selected === "rules" ? "text" : "json",
          exists: true,
          content: selected === "mcp" ? '{"mcpServers":{"demo":{}}}\n' : selected === "rules" ? "# 环境规则\n" : content,
          blocked_reason: null,
        },
        { command: "profiles.config-file" },
      );
    };
    const { calls } = renderWithEngine(<AgentsPage />, all);
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    const toggle = within(card).getByTestId("profile-config-file-toggle-cc-ly");
    expect(toggle).toHaveTextContent("~/.claude-cc-ly/settings.json");
    expect(calls.some((call) => call.command === "profiles:config-file")).toBe(false);
    await userEvent.click(toggle);
    expect(await within(card).findByTestId("profile-config-file-content-cc-ly")).toHaveTextContent("one");
    expect(calls.find((call) => call.command === "profiles:config-file")?.args).toEqual(["--only", "profiles:cc-ly"]);
    content = '{"model":"two"}\n';
    await userEvent.click(within(card).getByRole("button", { name: "刷新" }));
    await waitFor(() => expect(within(card).getByTestId("profile-config-file-content-cc-ly")).toHaveTextContent("two"));
    expect(calls.filter((call) => call.command === "profiles:config-file")).toHaveLength(2);
    await userEvent.click(within(card).getByRole("button", { name: "MCP" }));
    await waitFor(() => expect(within(card).getByTestId("profile-config-file-content-cc-ly")).toHaveTextContent("mcpServers"));
    const fileCalls = calls.filter((call) => call.command === "profiles:config-file");
    expect(fileCalls[fileCalls.length - 1]?.args).toEqual(["--only", "profiles:cc-ly", "--file", "mcp"]);
    await userEvent.click(within(card).getByRole("button", { name: "规则" }));
    await waitFor(() => expect(within(card).getByTestId("profile-config-file-content-cc-ly")).toHaveTextContent("环境规则"));
  });

  for (const state of ["managed-ok", "not-materialized"] as const) {
    it(`没有启动命令时，${state} 状态不误称环境可用同名命令启动`, async () => {
      const withNoLaunch = routes([profileElement({ state })]);
      withNoLaunch["aliases:status"] = makeEnvelope(makeAliasStatus({}), { command: "aliases.status" });
      renderWithEngine(<AgentsPage />, withNoLaunch);
      const card = await screen.findByTestId("profile-card-cc-ly");
      expect(within(card).getByTestId("profile-summary-cc-ly")).toHaveTextContent("还没有启动命令");
      expect(card).not.toHaveTextContent("用它的名字即可启动");
      if (state === "managed-ok") expect(card).not.toHaveTextContent("配置已写入，终端里");
      else expect(card).toHaveTextContent("还没写入这套配置，点「写入配置」。");
    });
  }

  it("复制启动名只写剪贴板，不修改实际别名或环境", async () => {
    const writeText = vi.fn(async () => {});
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    onTestFinished(() => { Object.defineProperty(navigator, "clipboard", { configurable: true, value: originalClipboard }); });
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    const card = await screen.findByTestId("profile-card-cc-ly");
    await userEvent.click(within(card).getByTestId("profile-launch-name-cc-ly-cc-ly-yolo"));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("cc-ly-yolo"));
    expect(calls.some((call) => call.command.endsWith(":apply") || call.command === "policy:set")).toBe(false);
  });

  it("复制实例：预填内容但名称与配置目录可改，且默认换了目录", async () => {
    renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    await userEvent.click(within(card).getByTestId("profile-duplicate-cc-ly"));
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    expect(within(dialog).getByTestId("profile-id-input")).toHaveValue("cc-ly-copy");
    // 名称必须可改：锁死就没法复制
    expect(within(dialog).getByTestId("profile-id-input")).not.toBeDisabled();
    // 两个实例不能指同一套配置，目录默认换掉
    expect(within(dialog).getByTestId("profile-config-dir")).toHaveValue("~/.claude-cc-ly-copy");
  });

  it("新建第三方环境：保存定义，先写好实例配置，再把启动命令按名写进终端配置", async () => {
    const { calls } = renderWithEngine(
      <AgentsPage />,
      routes([profileElement(), profileElement({ id: "profiles:cc-work" })]),
    );
    await openInstanceManager();
    await openNewProfileForm();
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-work");
    await userEvent.type(within(dialog).getByTestId("profile-base-url"), "https://api.example.com");
    await userEvent.type(within(dialog).getByTestId("profile-token-var"), "WORK_TOKEN");
    await userEvent.type(within(dialog).getByTestId("profile-extra-args"), "--dangerously-skip-permissions");
    expect(within(dialog).getByTestId("profile-launch-preview")).toHaveTextContent(
      `alias cc-work='CLAUDE_CONFIG_DIR="$HOME/.claude-cc-work" claude --dangerously-skip-permissions'`,
    );
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const defWrite = calls.find((call) => call.command === "policy:set")!;
    expect(defWrite.args[defWrite.args.indexOf("--key") + 1]).toBe("agents.definitions.cc-work");
    const definition = JSON.parse(defWrite.args[defWrite.args.indexOf("--value") + 1]);
    expect(definition.bin).toBe("claude");
    expect(definition.skills_dir).toBe("~/.claude-cc-work/skills");
    expect(definition.mcp_config).toEqual({ path: "~/.claude-cc-work/.claude.json", format: "json" });
    // 鉴权只存变量名
    expect(definition.profile.env.ANTHROPIC_AUTH_TOKEN).toBe("${WORK_TOKEN}");
    // 启动命令以终端配置为准：保存即按名写进 rc 文件的受管区块（先写定义再写别名）
    const aliasWrite = calls.find((call) => call.command === "aliases:apply")!;
    expect(calls.indexOf(aliasWrite)).toBeGreaterThan(calls.indexOf(defWrite));
    expect(aliasWrite.args[aliasWrite.args.indexOf("--set") + 1]).toBe("cc-work");
    expect(aliasWrite.args[aliasWrite.args.indexOf("--value") + 1]).toBe(
      'CLAUDE_CONFIG_DIR="$HOME/.claude-cc-work" claude --dangerously-skip-permissions',
    );
    // 目标文件还不存在：先写配置，免得启动命令在空目录里引导登录官方账号
    const configWrite = calls.find((call) => call.command === "profiles:apply")!;
    expect(configWrite.args).toEqual(["--only", "profiles:cc-work", "--base", "missing"]);
    expect(calls.indexOf(configWrite)).toBeGreaterThan(calls.indexOf(defWrite));
    expect(calls.indexOf(configWrite)).toBeLessThan(calls.indexOf(aliasWrite));
  });

  it("新建 Claude 官方账号环境：也先写好置空外部凭据变量的配置，再写启动命令", async () => {
    const { calls } = renderWithEngine(
      <AgentsPage />,
      routes([profileElement(), profileElement({ id: "profiles:cc-home" })]),
    );
    await openInstanceManager();
    await openNewProfileForm();
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-home");
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const defWrite = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(defWrite.args[defWrite.args.indexOf("--value") + 1]);
    expect(definition.profile.env).toBeUndefined();
    const configWrite = calls.find((call) => call.command === "profiles:apply")!;
    expect(configWrite.args).toEqual(["--only", "profiles:cc-home", "--base", "missing"]);
    const aliasWrite = calls.find((call) => call.command === "aliases:apply")!;
    expect(calls.indexOf(configWrite)).toBeLessThan(calls.indexOf(aliasWrite));
  });

  it("新建环境默认从默认环境带上全部设置；取消勾选的不带，且在写好配置与启动命令之后才带", async () => {
    const { calls } = renderWithEngine(
      <AgentsPage />,
      routes([profileElement(), profileElement({ id: "profiles:cc-work" })]),
    );
    await openInstanceManager();
    await openNewProfileForm();
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    const section = within(dialog).getByTestId("profile-seed-section");
    for (const item of ["permissions", "statusline", "hooks", "rules", "mcp"]) {
      expect(within(section).getByTestId(`profile-seed-${item}`)).toBeChecked();
    }
    // 说清楚来源与不带什么
    expect(section).toHaveTextContent("CLAUDE.md");
    expect(section).toHaveTextContent("凭据（env、apiKeyHelper）不带");
    await userEvent.click(within(section).getByTestId("profile-seed-statusline"));
    await userEvent.click(within(section).getByTestId("profile-seed-mcp"));
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-work");
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "profiles:seed")).toBe(true));
    const seedCall = calls.find((call) => call.command === "profiles:seed")!;
    expect(seedCall.args).toEqual(["--only", "profiles:cc-work", "--items", "permissions,hooks,rules"]);
    const aliasWrite = calls.find((call) => call.command === "aliases:apply")!;
    const configWrite = calls.find((call) => call.command === "profiles:apply")!;
    expect(calls.indexOf(seedCall)).toBeGreaterThan(calls.indexOf(configWrite));
    expect(calls.indexOf(seedCall)).toBeGreaterThan(calls.indexOf(aliasWrite));
    expect(await screen.findByText("已从默认环境带上：权限规则、Hooks")).toBeInTheDocument();
  });

  it("新建时全都不勾就不带；编辑已有环境不在表单里带", async () => {
    const { calls } = renderWithEngine(
      <AgentsPage />,
      routes([profileElement(), profileElement({ id: "profiles:cc-work" })]),
    );
    await openInstanceManager();
    await openNewProfileForm();
    let dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    for (const item of ["permissions", "statusline", "hooks", "rules", "mcp"]) {
      await userEvent.click(within(dialog).getByTestId(`profile-seed-${item}`));
    }
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-work");
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    expect(calls.some((call) => call.command === "profiles:seed")).toBe(false);

    await userEvent.click(await screen.findByTestId("profile-edit-cc-ly"));
    dialog = await screen.findByRole("dialog", { name: "编辑独立环境" });
    expect(within(dialog).queryByTestId("profile-seed-section")).toBeNull();
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "编辑独立环境" })).toBeNull());
    expect(calls.some((call) => call.command === "profiles:seed")).toBe(false);
  });

  it("共用设置写入被拒时保留差异，说明原因并可刷新", async () => {
    const routeMap = {
      ...routes(),
      "profiles:shared-status": makeEnvelope({ domain: "profiles", items: [{
        item: "rules", state: "different", source: "已设置", target: "已设置",
        changed_fields: ["正文"],
        field_diffs: [{ field: "正文", source: "3 行（内容隐藏）", target: "2 行（内容隐藏）" }],
        source_base: "a".repeat(64), target_base: "b".repeat(64), blocked_reason: null,
      }] }, { command: "profiles.shared-status" }),
      "profiles:shared-apply": makeEnvelope(null, {
        command: "profiles.shared-apply", ok: false,
        errors: [{ code: "PROFILE_ERROR", element: null, message: "来源或环境配置已变化，请刷新差异后重试" }],
      }),
    };
    const { calls } = renderWithEngine(<AgentsPage />, routeMap);
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    await userEvent.click(within(card).getByTestId("profile-shared-cc-ly"));
    const panel = within(card).getByTestId("profile-shared-panel-cc-ly");
    await userEvent.click(within(await within(panel).findByTestId("profile-shared-row-cc-ly-rules")).getByRole("checkbox"));
    await userEvent.click(within(panel).getByTestId("profile-shared-run-cc-ly"));
    expect(await within(panel).findByText(/来源或环境配置已变化/)).toBeInTheDocument();
    expect(within(panel).getByTestId("profile-shared-row-cc-ly-rules")).toHaveTextContent("两边不同");
    expect(calls.some((call) => call.command === "profiles:shared-apply")).toBe(true);
    await userEvent.click(within(panel).getByTestId("profile-shared-refresh-cc-ly"));
    expect(within(panel).getByTestId("profile-shared-run-cc-ly")).toBeDisabled();
  });

  it("已有环境先看共用设置差异，默认不更新；逐项确认后回读", async () => {
    let applied = false;
    const previewItem = (state: "different" | "same") => ({
      item: "permissions", state, source: "包含 allow", target: state === "same" ? "包含 allow" : "包含 deny",
      changed_fields: state === "same" ? [] : ["allow", "deny"],
      field_diffs: state === "same" ? [] : [
        { field: "allow", source: "1 项（内容隐藏）", target: "未设置" },
        { field: "deny", source: "未设置", target: "1 项（内容隐藏）" },
      ],
      source_base: "a".repeat(64), target_base: "b".repeat(64), blocked_reason: null,
    });
    const routeMap = {
      ...routes(),
      "profiles:shared-status": async () => makeEnvelope({
        domain: "profiles", action: "shared-status", element: "profiles:cc-ly",
        items: [previewItem(applied ? "same" : "different"), {
          item: "mcp:private", state: "blocked", source: "包含 command、env", target: "未设置",
          changed_fields: ["command", "env"],
          field_diffs: [{ field: "env", source: "1 个字段（内容隐藏）", target: "未设置" }],
          source_base: "c".repeat(64), target_base: "missing", blocked_reason: "来源条目含明文凭据",
        }],
      }, { command: "profiles.shared-status" }),
      "profiles:shared-apply": async () => {
        applied = true;
        return makeEnvelope({ domain: "profiles", action: "shared-apply", item: "permissions", state: "updated" }, { command: "profiles.shared-apply" });
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, routeMap);
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    await userEvent.click(within(card).getByTestId("profile-shared-cc-ly"));
    const panel = within(card).getByTestId("profile-shared-panel-cc-ly");
    const permission = await within(panel).findByTestId("profile-shared-row-cc-ly-permissions");
    expect(permission).toHaveTextContent("默认环境：包含 allow；此环境：包含 deny");
    expect(permission).toHaveTextContent("allow：默认环境 1 项（内容隐藏）；此环境 未设置");
    expect(within(permission).getByRole("checkbox")).not.toBeChecked();
    expect(within(panel).getByTestId("profile-shared-run-cc-ly")).toBeDisabled();
    expect(within(within(panel).getByTestId("profile-shared-row-cc-ly-mcp:private")).getByRole("checkbox")).toBeDisabled();
    expect(calls.some((call) => call.command === "profiles:shared-apply")).toBe(false);

    await userEvent.click(within(permission).getByRole("checkbox"));
    await userEvent.click(within(panel).getByTestId("profile-shared-run-cc-ly"));
    await waitFor(() => expect(within(panel).getByText("权限规则：已更新并回读一致")).toBeInTheDocument());
    const update = calls.find((call) => call.command === "profiles:shared-apply")!;
    expect(update.args).toEqual(["--only", "profiles:cc-ly", "--item", "permissions", "--source-base", "a".repeat(64), "--target-base", "b".repeat(64)]);
    expect(within(panel).getByTestId("profile-shared-run-cc-ly")).toBeDisabled();
  });

  it("多项共用设置按顺序更新，一项失败仍回读后续成功项", async () => {
    let statuslineUpdated = false;
    const row = (item: string, state: "different" | "same") => ({
      item, state, source: "已设置", target: "已设置", changed_fields: state === "same" ? [] : ["value"],
      field_diffs: state === "same" ? [] : [{ field: "value", source: "已设置（内容隐藏）", target: "已设置（内容隐藏）" }],
      source_base: "a".repeat(64), target_base: "b".repeat(64), blocked_reason: null,
    });
    const routeMap = {
      ...routes(),
      "profiles:shared-status": async () => makeEnvelope({
        domain: "profiles", items: [row("permissions", "different"), row("statusline", statuslineUpdated ? "same" : "different")],
      }, { command: "profiles.shared-status" }),
      "profiles:shared-apply": async (args: string[]) => {
        if (args[3] === "permissions") return makeEnvelope(null, {
          command: "profiles.shared-apply", ok: false,
          errors: [{ code: "PROFILE_ERROR", element: null, message: "目标已被外部修改" }],
        });
        statuslineUpdated = true;
        return makeEnvelope({ domain: "profiles", action: "shared-apply", item: "statusline", state: "updated" }, { command: "profiles.shared-apply" });
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, routeMap);
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    await userEvent.click(within(card).getByTestId("profile-shared-cc-ly"));
    const panel = within(card).getByTestId("profile-shared-panel-cc-ly");
    for (const item of ["permissions", "statusline"]) {
      await userEvent.click(within(await within(panel).findByTestId(`profile-shared-row-cc-ly-${item}`)).getByRole("checkbox"));
    }
    await userEvent.click(within(panel).getByTestId("profile-shared-run-cc-ly"));
    expect(await within(panel).findByText("权限规则：目标已被外部修改")).toBeInTheDocument();
    expect(within(panel).getByText("状态栏：已更新并回读一致")).toBeInTheDocument();
    expect(calls.filter((call) => call.command === "profiles:shared-apply").map((call) => call.args[3])).toEqual(["permissions", "statusline"]);
    expect(within(panel).getByTestId("profile-shared-row-cc-ly-permissions")).toHaveTextContent("两边不同");
    expect(within(panel).getByTestId("profile-shared-row-cc-ly-statusline")).toHaveTextContent("两边一致");
  });

  it("实例卡片把还没置空的外部凭据变量合并成一句说明，不逐条铺开", async () => {
    const isolation = (name: string) => ({ field: `env.${name}`, state: "missing", definition: "", live: null, purpose: "isolation" });
    renderWithEngine(
      <AgentsPage />,
      routes([
        profileElement({
          state: "drifted",
          observed: {
            ...(profileElement().observed as Record<string, unknown>),
            target_exists: true,
            base: "b".repeat(64),
            fields: [
              { field: "model", state: "differs", definition: "opus", live: "sonnet" },
              isolation("CLAUDE_CODE_USE_BEDROCK"),
              isolation("ANTHROPIC_CUSTOM_HEADERS"),
              // 实例文件自己设了、定义没声明：不是 shell 的问题，写入会把它置空
              { field: "env.CLAUDE_CODE_USE_VERTEX", state: "differs", definition: "", live: "1", purpose: "isolation" },
            ],
          },
        }),
      ]),
    );
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    const gaps = within(card).getByTestId("profile-isolation-gaps-cc-ly");
    expect(gaps).toHaveTextContent("2 个凭据相关变量");
    expect(gaps).toHaveTextContent("CLAUDE_CODE_USE_BEDROCK、ANTHROPIC_CUSTOM_HEADERS");
    expect(gaps).not.toHaveTextContent("CLAUDE_CODE_USE_VERTEX");
    const set = within(card).getByTestId("profile-isolation-set-cc-ly");
    expect(set).toHaveTextContent("配置文件自己设了定义里没有的 CLAUDE_CODE_USE_VERTEX：写入配置会把它们置空");
    const differs = within(card).getByTestId("profile-differs-cc-ly");
    expect(differs).toHaveTextContent("model");
    expect(differs).not.toHaveTextContent("CLAUDE_CODE_USE_BEDROCK");
    // 覆盖前的确认点名会被置空的变量，不再笼统说「其他设置保留」了事
    await userEvent.click(within(card).getByTestId("profile-apply-cc-ly"));
    expect(await screen.findByText(/配置文件里自己设的 CLAUDE_CODE_USE_VERTEX 定义里没有，会被置空/)).toBeInTheDocument();
  });

  it("实例卡片列出挡不住的系统级设置：第三方环境被限定登录时说明会被拦下，没有时不显示", async () => {
    const managed = "/etc/claude-code/managed-settings.json";
    renderWithEngine(
      <AgentsPage />,
      routes([
        profileElement({
          observed: {
            ...(profileElement().observed as Record<string, unknown>),
            third_party: true,
            system_overrides: [
              { source: managed, name: "ANTHROPIC_BASE_URL" },
              { source: managed, name: "forceLoginMethod" },
            ],
          },
        }),
        profileElement({
          id: "profiles:cc-home",
          observed: {
            ...(profileElement().observed as Record<string, unknown>),
            third_party: false,
            system_overrides: [{ source: "~/.config/anthropic/configs/default.json", name: "oidc_federation" }],
          },
        }),
        profileElement({ id: "profiles:cc-plain" }),
      ]),
    );
    await openInstanceManager();
    const relay = within(await screen.findByTestId("profile-card-cc-ly")).getByTestId("profile-system-overrides-cc-ly");
    expect(relay).toHaveTextContent("这个环境挡不住");
    expect(relay).toHaveTextContent(`${managed} 里的 ANTHROPIC_BASE_URL：请求改发往这个地址，不用这个环境自己的`);
    expect(relay).toHaveTextContent("forceLoginMethod：限定了登录方式：用第三方凭据的这个环境启动时会被拦下");
    const official = within(screen.getByTestId("profile-card-cc-home")).getByTestId("profile-system-overrides-cc-home");
    expect(official).toHaveTextContent("活动档案是联合身份，会优先于这个环境里的登录");
    expect(within(screen.getByTestId("profile-card-cc-plain")).queryByTestId("profile-system-overrides-cc-plain")).toBeNull();
  });

  it("新建第三方 Claude 环境默认让子进程看不到凭据；官方账号环境默认不开，可手动切换", async () => {
    // macOS：官方实现不需要 bubblewrap，第三方环境默认开
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)");
    onTestFinished(() => { vi.restoreAllMocks(); });
    const { calls } = renderWithEngine(
      <AgentsPage />,
      routes([profileElement(), profileElement({ id: "profiles:cc-lan" }), profileElement({ id: "profiles:cc-plain" })]),
    );
    await openInstanceManager();
    await openNewProfileForm();
    let dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-lan");
    expect(within(dialog).getByTestId("profile-scrub-subprocess-env")).not.toBeChecked();
    // 局域网 http 中转与接入配置同一口径：能保存，但提示明文传输
    await userEvent.type(within(dialog).getByTestId("profile-base-url"), "http://10.0.0.5:8080");
    await userEvent.type(within(dialog).getByTestId("profile-token-var"), "LAN_TOKEN");
    expect(within(dialog).getByTestId("profile-plain-http")).toHaveTextContent("令牌将明文传输");
    expect(within(dialog).getByTestId("profile-scrub-subprocess-env")).toBeChecked();
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    let saved = calls.filter((call) => call.command === "policy:set").slice(-1)[0];
    let definition = JSON.parse(saved.args[saved.args.indexOf("--value") + 1]);
    expect(definition.profile.env).toMatchObject({
      ANTHROPIC_BASE_URL: "http://10.0.0.5:8080",
      ANTHROPIC_AUTH_TOKEN: "${LAN_TOKEN}",
      CLAUDE_CODE_SUBPROCESS_ENV_SCRUB: "1",
    });

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "新建独立环境" })).toBeNull());
    await openNewProfileForm();
    dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-plain");
    await userEvent.click(within(dialog).getByTestId("profile-scrub-subprocess-env"));
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));
    await waitFor(() => expect(calls.filter((call) => call.command === "policy:set").length).toBe(2));
    saved = calls.filter((call) => call.command === "policy:set").slice(-1)[0];
    definition = JSON.parse(saved.args[saved.args.indexOf("--value") + 1]);
    expect(definition.profile.env).toEqual({ CLAUDE_CODE_SUBPROCESS_ENV_SCRUB: "1" });
  });

  it("Linux 上第三方环境默认不开子进程隔离并说明要装 bubblewrap；已有环境保留原写法", async () => {
    // 在 macOS runner 上也显式模拟 Linux，确保断言验证的是该平台分支。
    const originalPlatform = navigator.platform;
    const originalUserAgent = navigator.userAgent;
    Object.defineProperty(navigator, "platform", { configurable: true, value: "Linux x86_64" });
    Object.defineProperty(navigator, "userAgent", { configurable: true, value: "Linux" });
    onTestFinished(() => {
      Object.defineProperty(navigator, "platform", { configurable: true, value: originalPlatform });
      Object.defineProperty(navigator, "userAgent", { configurable: true, value: originalUserAgent });
    });
    const withScrub = profileElement({ id: "profiles:cc-true" });
    const routeMap = routes([profileElement(), withScrub]);
    const policy = routeMap["policy:get"] as { data: { layers: { sqlite: { agents: { definitions: Record<string, any> } } } } };
    policy.data.layers.sqlite.agents.definitions["cc-true"] = {
      display_name: "cc-true",
      bin: "claude",
      profile: {
        base_agent: "claude-code",
        kind: "claude",
        config_dir: "~/.claude-cc-true",
        env: { ANTHROPIC_BASE_URL: "https://relay.example.com", ANTHROPIC_AUTH_TOKEN: "${RELAY}", CLAUDE_CODE_SUBPROCESS_ENV_SCRUB: "true" },
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, routeMap);
    await openInstanceManager();
    await openNewProfileForm();
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    await userEvent.type(within(dialog).getByTestId("profile-base-url"), "https://relay.example.com");
    await userEvent.type(within(dialog).getByTestId("profile-token-var"), "RELAY");
    expect(within(dialog).getByTestId("profile-scrub-subprocess-env")).not.toBeChecked();
    expect(within(dialog).getByTestId("profile-scrub-linux-bwrap")).toHaveTextContent("bubblewrap");
    await userEvent.keyboard("{Escape}");

    // 已有环境写的是 "true"（官方同样认作打开）：显示为开，保存时原样保留
    await userEvent.click(await screen.findByTestId("profile-edit-cc-true"));
    const edit = await screen.findByRole("dialog", { name: "编辑独立环境" });
    expect(within(edit).getByTestId("profile-scrub-subprocess-env")).toBeChecked();
    await userEvent.click(within(edit).getByTestId("profile-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const saved = calls.filter((call) => call.command === "policy:set").slice(-1)[0];
    const definition = JSON.parse(saved.args[saved.args.indexOf("--value") + 1]);
    expect(definition.profile.env.CLAUDE_CODE_SUBPROCESS_ENV_SCRUB).toBe("true");
  });

  it("新建第三方环境时配置写不进去：启动命令也先不写，并说明原因", async () => {
    const refused = {
      ...routes([profileElement(), profileElement({ id: "profiles:cc-work" })]),
      "profiles:apply": async () => makeEnvelope(
        {},
        {
          command: "profiles.apply",
          ok: false,
          errors: [{ code: "ENGINE_ERROR", element: null, message: "[cc-work] 环境变量 WORK_TOKEN 在当前环境里没有取值，拒绝写入不完整的配置" }],
        },
      ),
    };
    const { calls } = renderWithEngine(<AgentsPage />, refused);
    await openInstanceManager();
    await openNewProfileForm();
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-work");
    await userEvent.type(within(dialog).getByTestId("profile-base-url"), "https://api.example.com");
    await userEvent.type(within(dialog).getByTestId("profile-token-var"), "WORK_TOKEN");
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "profiles:apply")).toBe(true));
    expect(await screen.findByText(/配置没有写入，启动命令也先不写/)).toBeInTheDocument();
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
  });

  it("Claude 环境填了接口地址却没填鉴权变量时不保存，免得官方登录被发往第三方地址", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    await openNewProfileForm();
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-relay");
    await userEvent.type(within(dialog).getByTestId("profile-base-url"), "https://api.example.com");
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));
    expect(within(dialog).getByTestId("profile-form-error")).toHaveTextContent("官方账号登录");
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("Codex 环境填了鉴权变量却没填接口地址时不保存，免得接入方式指向不存在的接入表", async () => {
    const codexRoutes = routes();
    (codexRoutes["agents:status"] as { data: { elements: unknown[] } }).data.elements.push(
      makeElement({
        id: "agents:codex",
        domain: "agents",
        spec: { display_name: "Codex", bin: "codex", preferred_provider: "native", providers: { native: { managed: true } } },
        policy: { enabled: true },
        observed: { install_source: "native", installed: true, management: agentManagementFixture("codex") },
        state: "managed-ok",
        freshness: makeFreshness(),
        actions: [],
      }),
    );
    const { calls } = renderWithEngine(<AgentsPage />, codexRoutes);
    await openInstanceManager();
    await openNewProfileForm();
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    await userEvent.click(within(within(dialog).getByTestId("profile-base-agent")).getByRole("button", { name: /Codex/ }));
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cx-relay");
    expect(within(dialog).getByTestId("profile-draft-preview-button")).toBeDisabled();
    await userEvent.type(within(dialog).getByTestId("profile-token-var"), "RELAY_KEY");
    expect(within(dialog).getByTestId("profile-draft-preview-button")).toBeDisabled();
    expect(calls.some((call) => call.command === "profiles:preview")).toBe(false);
    expect(within(dialog).getByTestId("profile-launch-preview")).toHaveTextContent(
      `alias cx-relay='CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME="$HOME/.codex-cx-relay" codex'`,
    );
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));
    expect(within(dialog).getByTestId("profile-form-error")).toHaveTextContent("接口地址");
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("新建实例使用整页分组编辑器，并实时列出全部落地位置与启动值", async () => {
    renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    await openNewProfileForm();
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });

    expect(within(dialog).getByText("1. 选择归属助手")).toBeInTheDocument();
    expect(within(dialog).getByText("2. 命名与存放位置")).toBeInTheDocument();
    expect(within(dialog).getByText("3. 接口、账号与模型")).toBeInTheDocument();
    expect(within(dialog).getByText("4. 运行参数")).toBeInTheDocument();
    expect(within(dialog).getByLabelText("独立环境结果预览")).toBeInTheDocument();

    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-work");
    expect(within(dialog).getByText("CLAUDE_CONFIG_DIR=~/.claude-cc-work")).toBeInTheDocument();
    expect(within(dialog).getByText("~/.claude-cc-work/settings.json")).toBeInTheDocument();
    expect(within(dialog).getByText("~/.claude-cc-work/.claude.json")).toBeInTheDocument();
    expect(within(dialog).getByText("~/.claude-cc-work/skills")).toBeInTheDocument();
    expect(within(dialog).getByTestId("profile-extra-args")).toHaveAttribute("rows", "4");
    expect(within(dialog).getByRole("button", { name: "创建独立环境" })).toBeInTheDocument();
  });

  it("实例表单同样给出该助手的常用参数", async () => {
    renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    await openNewProfileForm();
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    // 归属助手是 Claude Code，因此给 Claude 的预设
    await userEvent.click(within(dialog).getByTestId("profile-preset-claude-yolo"));
    expect(within(dialog).getByTestId("profile-extra-args")).toHaveValue("--dangerously-skip-permissions");
    expect(within(dialog).getByTestId("profile-preset-warning")).toBeInTheDocument();
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-auto");
    expect(within(dialog).getByTestId("profile-launch-preview")).toHaveTextContent("--dangerously-skip-permissions");
  });

  it("写入配置触发 profiles:apply，限定在该实例并带实际内容的基准", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    await userEvent.click(await screen.findByTestId("profile-apply-cc-ly"));
    await waitFor(() => expect(calls.some((call) => call.command === "profiles:apply")).toBe(true));
    const apply = calls.find((call) => call.command === "profiles:apply")!;
    expect(apply.args).toEqual(["--only", "profiles:cc-ly", "--base", "missing"]);
    expect(apply.args).not.toContain("--allow-takeover");
  });

  it("实际配置与定义不同时列出差异；覆盖要先确认，也可以以实际配置更新定义", async () => {
    const base = "c".repeat(64);
    const drifted = profileElement({
      state: "drifted",
      observed: {
        install_source: null,
        ...profileElement().observed,
        target_exists: true,
        config_dir_exists: true,
        base,
        fields: [
          { field: "env.ANTHROPIC_BASE_URL", state: "differs", definition: "https://api.example.com", live: "http://10.0.0.8:8080" },
          { field: "env.ANTHROPIC_AUTH_TOKEN", state: "same", definition: "${MY_TOKEN}", live: "<hidden>" },
        ],
      },
      actions: [
        { id: "apply", risk: "write-local", needs_lock: true, offline_ok: true },
        { id: "adopt", risk: "write-local", needs_lock: false, offline_ok: true },
      ],
    });
    const { calls } = renderWithEngine(<AgentsPage />, routes([drifted]));
    await openInstanceManager();
    const differs = await screen.findByTestId("profile-differs-cc-ly");
    expect(differs).toHaveTextContent("env.ANTHROPIC_BASE_URL");
    expect(differs).toHaveTextContent("http://10.0.0.8:8080");
    expect(differs).not.toHaveTextContent("ANTHROPIC_AUTH_TOKEN");

    // 覆盖：先确认，确认后带基准写入，不带接管旗标
    const apply = screen.getByTestId("profile-apply-cc-ly");
    expect(apply).toHaveTextContent("用定义覆盖实际配置");
    await userEvent.click(apply);
    expect(calls.some((call) => call.command === "profiles:apply")).toBe(false);
    await userEvent.click(await screen.findByRole("button", { name: "确认覆盖" }));
    await waitFor(() => expect(calls.some((call) => call.command === "profiles:apply")).toBe(true));
    expect(calls.find((call) => call.command === "profiles:apply")!.args).toEqual(["--only", "profiles:cc-ly", "--base", base]);

    // 采纳：以实际配置更新定义
    await userEvent.click(screen.getByTestId("profile-adopt-cc-ly"));
    await waitFor(() => expect(calls.some((call) => call.command === "profiles:adopt")).toBe(true));
    expect(calls.find((call) => call.command === "profiles:adopt")!.args).toEqual(["--only", "profiles:cc-ly", "--base", base]);
  });

  it("环境变量取不到值时先给出提示", async () => {
    const missing = profileElement({
      observed: { install_source: null, ...profileElement().observed,
        env_refs: [{ name: "ANTHROPIC_AUTH_TOKEN", kind: "reference", variable: "MY_TOKEN", resolved: false }] },
    });
    renderWithEngine(<AgentsPage />, routes([missing]));
    await openInstanceManager();
    expect(await screen.findByTestId("profile-missing-vars-cc-ly")).toHaveTextContent("MY_TOKEN");
  });

  it("第三方环境不给登录命令；检查登录走官方命令，查到官方登录时标出混用", async () => {
    const withThirdParty = routes([
      profileElement({ observed: { ...(profileElement().observed as Record<string, unknown>), third_party: true } }),
    ]);
    withThirdParty["agents:auth-status"] = makeEnvelope(
      { element: "profiles:cc-ly", state: "logged-in", logged_in: true, method: "claude.ai", third_party: true, mixed: true, reason: null },
      { command: "agents.auth-status" },
    );
    const { calls } = renderWithEngine(<AgentsPage />, withThirdParty);
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    expect(within(card).queryByTestId("profile-cc-ly-auth-login")).toBeNull();
    expect(within(card).getByTestId("profile-cc-ly-auth-logout")).toBeInTheDocument();
    expect(within(card).getByTestId("profile-auth-text-cc-ly")).toHaveTextContent("未检查");
    // 状态轮询不跑官方命令：只有点了才查
    expect(calls.some((call) => call.command === "agents:auth-status")).toBe(false);

    await userEvent.click(within(card).getByTestId("profile-cc-ly-auth-check"));
    await waitFor(() => expect(within(card).getByTestId("profile-auth-text-cc-ly")).toHaveTextContent("已登录（Claude 订阅账号）"));
    expect(calls.find((call) => call.command === "agents:auth-status")?.args).toEqual(["--only", "profiles:cc-ly"]);
    expect(within(card).getByTestId("profile-auth-mixed-cc-ly")).toHaveTextContent("两种凭据混在同一个配置目录");
  });

  it("编辑时换了配置目录提示登录不会跟过去；选 x-api-key 时说明首次启动要确认", async () => {
    renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    await userEvent.click(within(card).getByTestId("profile-edit-cc-ly"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).queryByTestId("profile-config-dir-moved")).toBeNull();
    const dir = within(dialog).getByTestId("profile-config-dir");
    await userEvent.clear(dir);
    await userEvent.type(dir, "~/.claude-other");
    expect(within(dialog).getByTestId("profile-config-dir-moved")).toHaveTextContent("原目录里的登录、会话与技能不会跟过来");
    expect(within(dialog).queryByTestId("profile-api-key-approval")).toBeNull();
    await userEvent.selectOptions(within(dialog).getByTestId("profile-auth-header"), "api_key");
    expect(within(dialog).getByTestId("profile-api-key-approval")).toHaveTextContent("第一次启动会询问是否使用这个 API key");
  });

  it("官方账号环境可以复制登录命令；查不到登录时如实说未登录", async () => {
    const official = routes([
      profileElement({ observed: { ...(profileElement().observed as Record<string, unknown>), third_party: false, env_refs: [] } }),
    ]);
    official["agents:auth-status"] = makeEnvelope(
      { element: "profiles:cc-ly", state: "logged-out", logged_in: false, method: null, third_party: false, mixed: false, reason: null },
      { command: "agents.auth-status" },
    );
    renderWithEngine(<AgentsPage />, official);
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    expect(within(card).getByTestId("profile-cc-ly-auth-login")).toBeInTheDocument();
    await userEvent.click(within(card).getByTestId("profile-cc-ly-auth-check"));
    await waitFor(() => expect(within(card).getByTestId("profile-auth-text-cc-ly")).toHaveTextContent("未登录"));
    expect(within(card).queryByTestId("profile-auth-mixed-cc-ly")).toBeNull();
  });

  it("第三方 Codex 环境的旧启动命令没清空官方凭据变量时提示，一键订正只补前缀", async () => {
    const codexRoutes = routes([
      profileElement({
        id: "profiles:cx-w",
        spec: { display_name: "Codex · w", base_agent: "codex", kind: "codex", config_dir: "~/.codex-w", base_url: "https://relay.example.com/v1", model: null },
        observed: {
          ...(profileElement().observed as Record<string, unknown>),
          kind: "codex",
          config_env: "CODEX_HOME",
          config_dir: "~/.codex-w",
          third_party: true,
          env_refs: [{ name: "RELAY_KEY", kind: "reference", variable: "RELAY_KEY", resolved: true }],
        },
      }),
    ]);
    codexRoutes["aliases:status"] = makeEnvelope(
      makeAliasStatus({ "cx-w": 'CODEX_HOME="$HOME/.codex-w" codex --search' }),
      { command: "aliases.status" },
    );
    (codexRoutes["agents:status"] as { data: { elements: unknown[] } }).data.elements.push(
      makeElement({
        id: "agents:codex",
        domain: "agents",
        spec: { display_name: "Codex", bin: "codex", preferred_provider: "native", providers: { native: { managed: true } } },
        policy: { enabled: true },
        observed: { install_source: "native", installed: true, management: agentManagementFixture("codex") },
        state: "managed-ok",
        freshness: makeFreshness(),
        actions: [],
      }),
    );
    const { calls } = renderWithEngine(<AgentsPage />, codexRoutes);
    await userEvent.click(await screen.findByTestId("connection-agent-codex"));
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cx-w");
    const warning = await within(card).findByTestId("profile-launch-clear-cx-w-cx-w");
    expect(warning).toHaveTextContent("CODEX_API_KEY / CODEX_ACCESS_TOKEN");
    await userEvent.click(within(warning).getByRole("button", { name: /订正/ }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const write = calls.find((call) => call.command === "aliases:apply")!;
    expect(write.args[write.args.indexOf("--set") + 1]).toBe("cx-w");
    expect(write.args[write.args.indexOf("--value") + 1]).toBe(
      'CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME="$HOME/.codex-w" codex --search',
    );
  });

  it("按现在的规则解析不了的旧定义照样列出：说明原因，可编辑修正或移除", async () => {
    const invalid = makeElement({
      id: "profiles:cc-ly",
      domain: "profiles",
      spec: { display_name: "Claude Code · ly", base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-cc-ly" },
      policy: { enabled: false },
      observed: { invalid: true, blocked_reason: "第三方接入的实例不能再打开云厂商开关" },
      state: "unsupported",
      freshness: makeFreshness({ state: "not-applicable" }),
      actions: [],
    });
    const { calls } = renderWithEngine(<AgentsPage />, routes([invalid]));
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    expect(within(card).getByTestId("profile-invalid-cc-ly")).toHaveTextContent("第三方接入的实例不能再打开云厂商开关");
    expect(within(card).queryByTestId("profile-auth-status-cc-ly")).toBeNull();
    await userEvent.click(within(card).getByTestId("profile-edit-cc-ly"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByTestId("profile-id-input")).toHaveValue("cc-ly");
    await userEvent.keyboard("{Escape}");
    await userEvent.click(within(card).getByTestId("profile-remove-cc-ly"));
    await userEvent.click(await screen.findByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const write = calls.find((call) => call.command === "policy:set")!;
    expect(write.args[write.args.indexOf("--key") + 1]).toBe("agents.definitions.cc-ly");
  });

  it("第三方独立环境可测一下：按实例检查，结果就地显示；官方账号环境不给这个按钮", async () => {
    const routeMap = {
      ...routes([
        profileElement({ observed: { ...(profileElement().observed as Record<string, unknown>), third_party: true } }),
        profileElement({ id: "profiles:cc-home", observed: { ...(profileElement().observed as Record<string, unknown>), third_party: false } }),
      ]),
      "agents:check": makeEnvelope(
        { element: "profiles:cc-ly", ok: true, detail: "通了：地址、令牌与模型都没问题", latency_ms: 321 },
        { command: "agents.check" },
      ),
    };
    const { calls } = renderWithEngine(<AgentsPage />, routeMap);
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    await userEvent.click(within(card).getByTestId("profile-check-cc-ly"));
    expect(await within(card).findByTestId("profile-check-result-cc-ly")).toHaveTextContent("通了：地址、令牌与模型都没问题（321ms）");
    const call = calls.find((item) => item.command === "agents:check")!;
    expect(call.args).toEqual(["--only", "profiles:cc-ly"]);
    expect(within(screen.getByTestId("profile-card-cc-home")).queryByTestId("profile-check-cc-home")).toBeNull();
    expect(calls.some((item) => item.command === "profiles:apply")).toBe(false);
  });

  it("新建独立环境时可获取模型列表：失败时就地说明原因", async () => {
    const routeMap = {
      ...routes([profileElement()]),
      "agents:models": makeEnvelope(null, { ok: false, command: "agents.models", errors: [{ code: "AGENT_ERROR", message: "环境变量 RELAY_KEY 还没有取值", element: null }] }),
    };
    const { calls } = renderWithEngine(<AgentsPage />, routeMap);
    await openInstanceManager();
    await openNewProfileForm();
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    expect(within(dialog).getByTestId("profile-fetch-models")).toBeDisabled();
    await userEvent.type(within(dialog).getByTestId("profile-base-url"), "https://relay.example.com");
    await userEvent.type(within(dialog).getByTestId("profile-token-var"), "RELAY_KEY");
    await userEvent.click(within(dialog).getByTestId("profile-fetch-models"));
    expect(await within(dialog).findByTestId("profile-models-message")).toHaveTextContent("RELAY_KEY 还没有取值");
    const call = calls.find((item) => item.command === "agents:models")!;
    expect(JSON.parse(call.args[1])).toEqual({ kind: "claude", base_url: "https://relay.example.com", token_var: "RELAY_KEY" });
  });

  it("密钥换了还没写进环境：卡片单独说明，按钮是「写入新密钥」，直接写不弹覆盖确认", async () => {
    const base = profileElement();
    const stale = profileElement({
      state: "drifted",
      observed: {
        ...(base.observed as Record<string, unknown>),
        third_party: true,
        target_exists: true,
        base: "b".repeat(64),
        fields: [
          { field: "env.ANTHROPIC_AUTH_TOKEN", state: "differs", definition: "${MY_TOKEN}", live: "<hidden>", purpose: "credential", variable: "MY_TOKEN" },
          { field: "env.ANTHROPIC_BASE_URL", state: "same", definition: "https://api.example.com", live: "https://api.example.com" },
        ],
      },
    });
    const { calls } = renderWithEngine(<AgentsPage />, routes([stale]));
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    expect(card).toHaveTextContent("密钥换过了，这个环境还在用旧的");
    expect(within(card).getByTestId("profile-credential-cc-ly")).toHaveTextContent(
      "凭据：密钥文件里的 MY_TOKEN，写入时把值写进 ~/.claude-cc-ly/settings.json（仅本人可读）",
    );
    // 凭据不混进一般差异，也不出现「用定义覆盖」
    expect(within(card).queryByTestId("profile-differs-cc-ly")).toBeNull();
    const write = within(card).getByTestId("profile-apply-cc-ly");
    expect(write).toHaveTextContent("写入新密钥");
    await userEvent.click(write);
    await waitFor(() => expect(calls.some((call) => call.command === "profiles:apply")).toBe(true));
    expect(screen.queryByRole("dialog")).toBeNull();
    const apply = calls.find((call) => call.command === "profiles:apply")!;
    expect(apply.args).toEqual(["--only", "profiles:cc-ly", "--base", "b".repeat(64)]);
  });

  it("移除实例需确认，且只停止管理、不动配置目录", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    await userEvent.click(await screen.findByTestId("profile-remove-cc-ly"));
    expect(await screen.findByText(/配置目录、登录状态与里面的内容都原样保留/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const write = calls.find((call) => call.command === "policy:set")!;
    expect(write.args[write.args.indexOf("--key") + 1]).toBe("agents.definitions.cc-ly");
    expect(JSON.parse(write.args[write.args.indexOf("--value") + 1])).toEqual({ _delete: true });
    expect(calls.some((call) => call.command === "profiles:apply")).toBe(false);
  });

  it("编辑 Provider 先预览环境差异，只有勾选后才按两份基准更新并回读", async () => {
    const oldDefinition = {
      display_name: "Claude Code · ly", bin: "claude",
      profile: {
        base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-cc-ly",
        source_connection: "relay",
        env: { ANTHROPIC_BASE_URL: "https://api.example.com", ANTHROPIC_AUTH_TOKEN: "${MY_TOKEN}" },
      },
    };
    const base = "a".repeat(64);
    const definitionBase = "b".repeat(64);
    let appliedCandidate: Record<string, unknown> | null = null;
    const all = routes([profileElement({
      state: "managed-ok",
      observed: {
        ...(profileElement().observed as Record<string, unknown>),
        base, definition_base: definitionBase, target_exists: true,
        fields: [{ field: "env.ANTHROPIC_BASE_URL", state: "same", definition: "https://api.example.com", live: "https://api.example.com" }],
      },
    })]) as Record<string, Envelope | ((args: string[]) => Promise<Envelope>)>;
    const agents = all["agents:status"] as Envelope<{ elements: { spec: Record<string, unknown> }[] }>;
    (agents.data.elements[0] as any).observed.current_config = { state: "missing", base: "missing", config: null, current: null };
    agents.data.elements[0].spec.settings = {
      profiles: { relay: { label: "Relay", base_url: "https://api.example.com", auth: "token", token_var: "MY_TOKEN", model: "" } },
    };
    all["policy:get"] = async () => makeEnvelope(makePolicyData({
      effective: {
        agents: { definitions: { "cc-ly": appliedCandidate
          ? { ...oldDefinition, profile: { ...oldDefinition.profile, ...appliedCandidate } }
          : oldDefinition } },
      },
    }), { command: "policy.get" });
    all["profiles:status"] = async () => makeEnvelope({
      domain: "profiles",
      elements: [profileElement({
        state: "managed-ok",
        observed: {
          ...(profileElement().observed as Record<string, unknown>),
          base: appliedCandidate ? "c".repeat(64) : base,
          definition_base: appliedCandidate ? "d".repeat(64) : definitionBase,
          target_exists: true,
          fields: [{
            field: "env.ANTHROPIC_BASE_URL", state: "same",
            definition: appliedCandidate ? "https://new.example.com" : "https://api.example.com",
            live: appliedCandidate ? "https://new.example.com" : "https://api.example.com",
          }, {
            field: "env.ANTHROPIC_AUTH_TOKEN", state: "same", definition: "${MY_TOKEN}", live: "<hidden>", purpose: "credential",
          }],
        },
      })],
    }, { command: "profiles.status" });
    all["profiles:apply-definition"] = async (args) => {
      appliedCandidate = JSON.parse(args[args.indexOf("--candidate") + 1]);
      return makeEnvelope(makeApplyResult({ domain: "profiles" }), { command: "profiles.apply-definition" });
    };
    all["profiles:preview"] = async (args) => makeEnvelope({
      path: "~/.claude-cc-ly/settings.json",
      exists: true,
      content: JSON.stringify({ env: JSON.parse(args[args.indexOf("--candidate") + 1]).env }),
    }, { command: "profiles.preview" });
    const { calls } = renderWithEngine(<AgentsPage />, all);
    await userEvent.click(await screen.findByTestId("edit-connection-claude-code-relay"));
    const dialog = await screen.findByRole("dialog", { name: "编辑 Relay" });
    await userEvent.clear(within(dialog).getByTestId("new-connection-base-url"));
    await userEvent.type(within(dialog).getByTestId("new-connection-base-url"), "https://new.example.com");
    const impact = within(dialog).getByTestId("connection-affected-cc-ly");
    expect(impact).toHaveTextContent("API 地址：https://api.example.com → https://new.example.com");
    expect(within(impact).getByRole("checkbox")).not.toBeChecked();
    expect(within(dialog).getByText("只保存 Provider 模板时，客户端配置文件和启动命令不变。")).toBeInTheDocument();
    expect(calls.some((call) => call.command === "profiles:apply-definition")).toBe(false);
    await userEvent.click(within(impact).getByRole("checkbox"));
    expect(await within(dialog).findByTestId("connection-preview-content-cc-ly")).toHaveTextContent("https://new.example.com");
    const previewArgs = calls.find((call) => call.command === "profiles:preview")!.args;
    expect(previewArgs).toContain(base);
    expect(previewArgs).toContain(definitionBase);
    expect(calls.some((call) => call.command === "profiles:apply-definition")).toBe(false);
    await userEvent.click(within(dialog).getByTestId("connection-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "profiles:apply-definition")).toBe(true));
    const update = calls.find((call) => call.command === "profiles:apply-definition")!;
    expect(previewArgs).toEqual(update.args);
    expect(update.args).toContain(base);
    expect(update.args).toContain(definitionBase);
    expect(appliedCandidate).toMatchObject({
      source_connection: "relay",
      env: { ANTHROPIC_BASE_URL: "https://new.example.com", ANTHROPIC_AUTH_TOKEN: "${MY_TOKEN}" },
    });
    expect(calls.find((call) => call.command === "policy:set")!.args).toContain("agents.definitions.claude-code");
    expect(await screen.findByText("Provider 已保存，1 个独立环境已更新并回读")).toBeInTheDocument();
    const card = screen.getByTestId("profile-card-cc-ly");
    await waitFor(() => expect(card).toHaveFocus());
    expect(within(card).getByTestId("profile-config-file-toggle-cc-ly")).toHaveAttribute("aria-expanded", "false");
  });

  it("Provider 上次保存时未同步的环境，再打开编辑可逐字段补同步", async () => {
    const definition = {
      display_name: "Claude Code · ly",
      profile: {
        base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-cc-ly", source_connection: "relay",
        env: { ANTHROPIC_BASE_URL: "https://old.example.com", ANTHROPIC_AUTH_TOKEN: "${MY_TOKEN}" },
      },
    };
    let candidate: Record<string, unknown> | null = null;
    const all = routes([profileElement({
      state: "managed-ok",
      observed: {
        ...(profileElement().observed as Record<string, unknown>),
        base: "a".repeat(64), definition_base: "b".repeat(64), target_exists: true,
      },
    })]) as Record<string, Envelope | ((args: string[]) => Promise<Envelope>)>;
    const agent = (all["agents:status"] as Envelope<{ elements: { spec: Record<string, unknown> }[] }>).data.elements[0];
    (agent as any).observed.current_config = { state: "missing", base: "missing", config: null, current: null };
    agent.spec.settings = {
      profiles: { relay: { label: "Relay", base_url: "https://new.example.com", auth: "token", token_var: "MY_TOKEN", model: "" } },
    };
    all["policy:get"] = async () => makeEnvelope(makePolicyData({ effective: {
      agents: { definitions: { "cc-ly": candidate
        ? { ...definition, profile: { ...definition.profile, ...candidate } }
        : definition } },
    } }), { command: "policy.get" });
    all["profiles:status"] = async () => makeEnvelope({ domain: "profiles", elements: [profileElement({
      state: "managed-ok",
      observed: {
        ...(profileElement().observed as Record<string, unknown>),
        base: candidate ? "c".repeat(64) : "a".repeat(64),
        definition_base: candidate ? "d".repeat(64) : "b".repeat(64),
        target_exists: true,
        fields: [
          { field: "env.ANTHROPIC_BASE_URL", state: "same", definition: candidate ? "https://new.example.com" : "https://old.example.com" },
          { field: "env.ANTHROPIC_AUTH_TOKEN", state: "same", definition: "${MY_TOKEN}" },
        ],
      },
    })] }, { command: "profiles.status" });
    all["profiles:apply-definition"] = async (args) => {
      candidate = JSON.parse(args[args.indexOf("--candidate") + 1]);
      return makeEnvelope(makeApplyResult({ domain: "profiles" }), { command: "profiles.apply-definition" });
    };
    const { calls } = renderWithEngine(<AgentsPage />, all);
    await userEvent.click(await screen.findByTestId("edit-connection-claude-code-relay"));
    const dialog = await screen.findByRole("dialog", { name: "编辑 Relay" });
    const impact = within(dialog).getByTestId("connection-affected-cc-ly");
    expect(impact).toHaveTextContent("API 地址：https://old.example.com → https://new.example.com");
    expect(within(impact).getByTestId("connection-update-cc-ly")).toBeDisabled();
    await userEvent.click(within(impact).getByTestId("connection-override-cc-ly-baseUrl"));
    expect(within(impact).getByTestId("connection-update-cc-ly")).toBeChecked();
    await userEvent.click(within(dialog).getByTestId("connection-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "profiles:apply-definition")).toBe(true));
    expect(candidate).toMatchObject({ env: { ANTHROPIC_BASE_URL: "https://new.example.com", ANTHROPIC_AUTH_TOKEN: "${MY_TOKEN}" } });
    expect(await screen.findByText("Provider 已保存，1 个独立环境已更新并回读")).toBeInTheDocument();
  });

  it("实际环境已偏离定义时不能从 Provider 编辑直接覆盖", async () => {
    const all = routes([profileElement({
      state: "drifted",
      observed: { ...(profileElement().observed as Record<string, unknown>), base: "a".repeat(64), definition_base: "b".repeat(64) },
    })]);
    const agent = (all["agents:status"].data as { elements: { spec: Record<string, unknown> }[] }).elements[0];
    (agent as any).observed.current_config = { state: "missing", base: "missing", config: null, current: null };
    agent.spec.settings = { profiles: { relay: { label: "Relay", base_url: "https://api.example.com", auth: "token", token_var: "MY_TOKEN" } } };
    (all["policy:get"].data as { effective: Record<string, unknown> }).effective = {
      agents: { definitions: { "cc-ly": {
        display_name: "Claude Code · ly",
        profile: { base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-cc-ly", source_connection: "relay", env: { ANTHROPIC_BASE_URL: "https://api.example.com", ANTHROPIC_AUTH_TOKEN: "${MY_TOKEN}" } },
      } } },
    };
    const { calls } = renderWithEngine(<AgentsPage />, all);
    await userEvent.click(await screen.findByTestId("edit-connection-claude-code-relay"));
    const dialog = await screen.findByRole("dialog", { name: "编辑 Relay" });
    await userEvent.clear(within(dialog).getByTestId("new-connection-base-url"));
    await userEvent.type(within(dialog).getByTestId("new-connection-base-url"), "https://new.example.com");
    const impact = within(dialog).getByTestId("connection-affected-cc-ly");
    expect(impact).toHaveTextContent("实际配置与定义不同");
    expect(within(impact).getByRole("checkbox")).toBeDisabled();
    await userEvent.click(within(dialog).getByTestId("connection-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    expect(calls.some((call) => call.command === "profiles:apply-definition")).toBe(false);
  });
});
