import { describe, expect, it } from "vitest";
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
    await userEvent.click(await screen.findByTestId("manage-instances-button"));
    return screen.findByTestId("independent-environments");
  }

  it("实例单独成卡，展示配置目录与启动方式，不混进主助手列表", async () => {
    renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    const card = await screen.findByTestId("profile-card-cc-ly");
    expect(within(card).getByText("Claude Code · ly")).toBeInTheDocument();
    expect(card).toHaveTextContent("~/.claude-cc-ly");
    // 一个实例可以挂多条启动命令，按配置目录反查后都列出来
    expect(within(card).getByTestId("profile-launch-cc-ly-cc-ly")).toHaveTextContent(
      `alias cc-ly='CLAUDE_CONFIG_DIR="$HOME/.claude-cc-ly" claude'`,
    );
    expect(within(card).getByTestId("profile-launch-cc-ly-cc-ly-yolo")).toHaveTextContent(
      "--dangerously-skip-permissions",
    );
    // 与该实例无关的命令不混进来
    expect(within(card).queryByTestId("profile-launch-cc-ly-claude")).toBeNull();
    // 每条都能编辑、复制、删除
    expect(within(card).getByTestId("profile-launch-edit-cc-ly-cc-ly")).toBeInTheDocument();
    expect(within(card).getByTestId("profile-launch-copy-cc-ly-cc-ly")).toBeInTheDocument();
    expect(within(card).getByTestId("profile-launch-remove-cc-ly-cc-ly")).toBeInTheDocument();
    // 实例多半是另一个账号，额度读它自己的配置目录，不是默认那份
    expect(within(card).getByTestId("profile-usage-cc-ly")).toHaveTextContent("7%");
    expect(within(card).getByTestId("profile-usage-cc-ly")).toHaveTextContent("5 小时");
    // 实例不再作为主助手卡片重复出现
    expect(screen.queryByTestId("manage-skills-agents:cc-ly")).toBeNull();
    expect(within(card).getByTestId("profile-skills-cc-ly")).toBeInTheDocument();
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

  it("新建实例：保存定义，并把启动命令按名直接写进终端配置；实例配置目录不落地", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    await userEvent.click(await screen.findByTestId("add-profile-button"));
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
    // 实例配置目录仍要单独「写入配置」才落地
    expect(calls.some((call) => call.command === "profiles:apply")).toBe(false);
  });

  it("Claude 环境填了接口地址却没填鉴权变量时不保存，免得官方登录被发往第三方地址", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    await userEvent.click(await screen.findByTestId("add-profile-button"));
    const dialog = await screen.findByRole("dialog", { name: "新建独立环境" });
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-relay");
    await userEvent.type(within(dialog).getByTestId("profile-base-url"), "https://api.example.com");
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));
    expect(within(dialog).getByTestId("profile-form-error")).toHaveTextContent("官方账号登录");
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("新建实例使用整页分组编辑器，并实时列出全部落地位置与启动值", async () => {
    renderWithEngine(<AgentsPage />, routes());
    await openInstanceManager();
    await userEvent.click(await screen.findByTestId("add-profile-button"));
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
    await userEvent.click(await screen.findByTestId("add-profile-button"));
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
});
