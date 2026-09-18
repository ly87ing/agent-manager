import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import type { Envelope } from "../lib/contract";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
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
            observed: { providers: ["native"], classified: { managed: ["native"], deprecated: [], system_only: [], unsupported: [] }, installed: true, preferred_active: true },
            state: "managed-ok",
            freshness: makeFreshness(),
            actions: [],
          }),
          makeElement({
            id: "agents:cc-ly",
            domain: "agents",
            spec: { display_name: "Claude Code · ly", bin: "claude" },
            policy: { enabled: true },
            observed: { providers: [], classified: {}, installed: true },
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
    "agents:apply": makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
    "policy:set": makeEnvelope(makePolicyData(), { command: "policy.set" }),
    "sources:get": makeEnvelope(
      {
        sources: [
          {
            key: "aliases.shell",
            label: "Shell aliases",
            format: "json",
            revision: "rev-1",
            value: { timezone: "America/Los_Angeles", lang: "en_US.UTF-8", aliases: {} },
          },
        ],
      },
      { command: "sources.get" },
    ),
    "sources:set": makeEnvelope({ key: "aliases.shell", revision: "rev-2" }, { command: "sources.set" }),
  };
}

describe("助手实例", () => {
  it("实例单独成卡，展示配置目录与启动方式，不混进主助手列表", async () => {
    renderWithEngine(<AgentsPage />, routes());
    const card = await screen.findByTestId("profile-card-cc-ly");
    expect(within(card).getByText("Claude Code · ly")).toBeInTheDocument();
    expect(card).toHaveTextContent("~/.claude-cc-ly");
    expect(within(card).getByTestId("profile-launch-cc-ly")).toHaveTextContent(
      `alias cc-ly='CLAUDE_CONFIG_DIR="~/.claude-cc-ly" claude --dangerously-skip-permissions'`,
    );
    // 实例不再作为主助手卡片重复出现
    expect(screen.queryByTestId("manage-skills-agents:cc-ly")).toBeNull();
    expect(screen.getByTestId("manage-skills-agents:claude-code")).toBeInTheDocument();
  });

  it("新建实例：保存定义并把启动命令一并存成同名快捷命令，不落地任何配置", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await userEvent.click(await screen.findByTestId("add-profile-button"));
    const dialog = await screen.findByRole("dialog", { name: "新建实例" });
    await userEvent.type(within(dialog).getByTestId("profile-id-input"), "cc-work");
    await userEvent.type(within(dialog).getByTestId("profile-base-url"), "https://api.example.com");
    await userEvent.type(within(dialog).getByTestId("profile-token-var"), "WORK_TOKEN");
    await userEvent.type(within(dialog).getByTestId("profile-extra-args"), "--dangerously-skip-permissions");
    expect(within(dialog).getByTestId("profile-launch-preview")).toHaveTextContent(
      `alias cc-work='CLAUDE_CONFIG_DIR="~/.claude-cc-work" claude --dangerously-skip-permissions'`,
    );
    await userEvent.click(within(dialog).getByTestId("profile-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "sources:set")).toBe(true));
    const defWrite = calls.find((call) => call.command === "policy:set")!;
    expect(defWrite.args[defWrite.args.indexOf("--key") + 1]).toBe("agents.definitions.cc-work");
    const definition = JSON.parse(defWrite.args[defWrite.args.indexOf("--value") + 1]);
    expect(definition.bin).toBe("claude");
    expect(definition.skills_dir).toBe("~/.claude-cc-work/skills");
    expect(definition.mcp_config).toEqual({ path: "~/.claude-cc-work/.claude.json", format: "json" });
    // 鉴权只存变量名
    expect(definition.profile.env.ANTHROPIC_AUTH_TOKEN).toBe("${WORK_TOKEN}");
    // 启动命令同时进快捷命令来源
    const aliasWrite = calls.find((call) => call.command === "sources:set")!;
    const payload = JSON.parse(aliasWrite.args[aliasWrite.args.indexOf("--value") + 1]);
    expect(payload.value.aliases["cc-work"]).toBe(
      'CLAUDE_CONFIG_DIR="~/.claude-cc-work" claude --dangerously-skip-permissions',
    );
    // 保存阶段不写任何配置文件
    expect(calls.some((call) => call.command === "profiles:apply")).toBe(false);
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
  });

  it("写入配置触发 profiles:apply，限定在该实例", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await userEvent.click(await screen.findByTestId("profile-apply-cc-ly"));
    await waitFor(() => expect(calls.some((call) => call.command === "profiles:apply")).toBe(true));
    const apply = calls.find((call) => call.command === "profiles:apply")!;
    expect(apply.args).toEqual(["--only", "profiles:cc-ly"]);
    expect(apply.args).not.toContain("--allow-takeover");
  });

  it("配置被外部改过时按钮转为接管，并显式带接管旗标", async () => {
    const drifted = profileElement({ state: "drifted", observed: { ...profileElement().observed, target_exists: true, config_dir_exists: true } });
    const { calls } = renderWithEngine(<AgentsPage />, routes([drifted]));
    const button = await screen.findByTestId("profile-apply-cc-ly");
    expect(button).toHaveTextContent("接管并写入");
    await userEvent.click(button);
    await waitFor(() => expect(calls.some((call) => call.command === "profiles:apply")).toBe(true));
    expect(calls.find((call) => call.command === "profiles:apply")!.args).toEqual([
      "--only",
      "profiles:cc-ly",
      "--allow-takeover",
    ]);
  });

  it("环境变量取不到值时先给出提示", async () => {
    const missing = profileElement({
      observed: {
        ...profileElement().observed,
        env_refs: [{ name: "ANTHROPIC_AUTH_TOKEN", kind: "reference", variable: "MY_TOKEN", resolved: false }],
      },
    });
    renderWithEngine(<AgentsPage />, routes([missing]));
    expect(await screen.findByTestId("profile-missing-vars-cc-ly")).toHaveTextContent("MY_TOKEN");
  });

  it("移除实例需确认，且只停止管理、不动配置目录", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
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
