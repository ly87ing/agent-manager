import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/**
 * 助手运行参数：保存即写进助手自己的配置文件。
 *
 * 两处必须先问过用户，引擎侧强制、界面负责问：降安全等级（YOLO）、目标被外部改过。
 */

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
            observed: { install_source: "native", installed: true },
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
    "sources:get": makeEnvelope(
      {
        sources: [
          {
            key: "aliases.shell",
            label: "Shell Aliases",
            format: "json",
            revision: "rev-1",
            value: { timezone: "America/Los_Angeles", lang: "en_US.UTF-8", aliases: {} },
          },
        ],
      },
      { command: "sources.get" },
    ),
    "sources:set": makeEnvelope({ key: "aliases.shell", revision: "rev-2" }, { command: "sources.set" }),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
    ...overrides,
  };
}

async function openEditor() {
  await userEvent.click(await screen.findByRole("button", { name: "编辑命令" }));
  return screen.findByRole("dialog");
}

/** 新建一套接入配置并展开它的编辑区（新建后默认就是展开且生效的那套）。 */
async function addConnection() {
  await userEvent.click(screen.getByTestId("add-connection"));
}

function configureCalls(calls: { command: string; args?: string[] }[]) {
  return calls.filter((call) => call.command === "agents:configure");
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
              observed: { install_source: "native", installed: true },
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

  it("填了接口地址保存：先存定义，再写进助手配置文件", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await addConnection();
    await userEvent.type(screen.getByTestId("connection-base-url-input"), "https://api.example.com");
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    // 定义先落库，参数才写文件
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.profiles.relay.base_url).toBe("https://api.example.com");
    expect(definition.settings.active).toBe("relay");
    expect(configureCalls(calls)[0].args).toEqual(["--only", "agents:claude-code"]);
  });

  it("一项都没填：只存定义，不碰助手的配置文件", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    expect(configureCalls(calls)).toHaveLength(0);
  });

  it("令牌只填变量名：定义里存的是名字，不是取值", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await addConnection();
    await userEvent.type(screen.getByTestId("connection-token-var-input"), "MY_GATEWAY_TOKEN");
    await userEvent.selectOptions(screen.getByTestId("connection-auth-select"), "api_key");
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.profiles.relay.token_var).toBe("MY_GATEWAY_TOKEN");
    expect(definition.settings.profiles.relay.auth).toBe("api_key");
    expect(JSON.stringify(definition)).not.toContain("sk-");
  });

  it("勾了跳过权限确认：先弹确认，取消就不写文件", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await userEvent.click(screen.getByTestId("agent-yolo-checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await screen.findByRole("dialog", { name: "跳过权限确认" });
    expect(configureCalls(calls)).toHaveLength(0);

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(configureCalls(calls)).toHaveLength(0);
  });

  it("勾了跳过权限确认并确认：带 --allow-yolo 写入", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await userEvent.click(screen.getByTestId("agent-yolo-checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await screen.findByRole("dialog", { name: "跳过权限确认" });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(configureCalls(calls)[0].args).toEqual([
      "--only",
      "agents:claude-code",
      "--allow-yolo",
    ]);
  });

  it("配置文件被外部改过：先问再覆盖，确认后带 --allow-takeover 重试", async () => {
    let allowed = false;
    const base = routes();
    const withRefusal: Record<string, Envelope | (() => Promise<Envelope>)> = {
      ...base,
      "agents:configure": async () => {
        if (allowed) {
          return makeEnvelope(
            { domain: "agents", action: "configure", element: "agents:claude-code" },
            { command: "agents.configure" },
          );
        }
        allowed = true;
        return makeEnvelope(
          {},
          {
            command: "agents.configure",
            ok: false,
            errors: [
              {
                code: "AGENT_ERROR",
                element: null,
                message: ".claude/settings.json 里这几项被本工具以外改过，拒绝覆盖；确认要用这里的内容替换请选择接管",
              },
            ],
          },
        );
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withRefusal);
    await openEditor();
    await addConnection();
    await userEvent.type(screen.getByTestId("connection-base-url-input"), "https://api.example.com");
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    // 第一次被拒 → 弹接管确认
    await screen.findByRole("dialog", { name: "覆盖被改过的配置" });
    expect(configureCalls(calls)).toHaveLength(1);

    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(2));
    expect(configureCalls(calls)[1].args).toContain("--allow-takeover");
  });

  it("按厂商添加一套：地址与模型名填好，只剩令牌要自己配", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await userEvent.selectOptions(screen.getByTestId("add-connection-preset"), "deepseek-claude");
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    const entry = definition.settings.profiles.deepseek;
    expect(entry.base_url).toBe("https://api.deepseek.com/anthropic");
    expect(entry.model).toBe("deepseek-flash[1m]");
    expect(entry.model_aliases.haiku).toBe("deepseek-flash");
    // 预设只带变量名，取值仍要自己在密钥页填
    expect(entry.token_var).toBe("DEEPSEEK_API_KEY");
    expect(definition.settings.active).toBe("deepseek");
  });

  it("Codex 的厂商清单是另一套端点，不会拿 Claude 那条去填", async () => {
    renderWithEngine(<AgentsPage />, agentRoutes("codex", "codex"));
    await openEditor();
    const picker = screen.getByTestId("add-connection-preset");
    // 选项只给 Codex 那一套
    await userEvent.selectOptions(picker, "deepseek-codex");
    expect(screen.getByTestId("connection-base-url-input")).toHaveValue("https://api.deepseek.com/");
    expect(screen.getByTestId("connection-model-input")).toHaveValue("deepseek-flash");
  });

  it("状态栏选用本工具的：定义里记下来，随保存一起写入", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await userEvent.selectOptions(screen.getByTestId("statusline-select"), "builtin");
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.statusline).toBe("builtin");
  });

  it("多套接入配置：只有选中的那套是生效的", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await addConnection();
    await userEvent.type(screen.getByTestId("connection-base-url-input"), "https://a.example.com");
    await addConnection();
    // 第二套新建后自动展开，填它的地址
    await userEvent.type(screen.getByTestId("connection-base-url-input"), "https://b.example.com");
    // 把生效切回第一套
    await userEvent.click(screen.getByTestId("use-connection-relay"));
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(Object.keys(definition.settings.profiles).sort()).toEqual(["relay", "relay2"]);
    expect(definition.settings.active).toBe("relay");
    expect(definition.settings.profiles.relay.base_url).toBe("https://a.example.com");
    expect(definition.settings.profiles.relay2.base_url).toBe("https://b.example.com");
  });

  it("卡片上直接切换接入配置：一条命令带 --use，切回默认带 --use-default", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      profiles: {
        official: { label: "官方" },
        relay: { label: "我的中转", base_url: "https://relay.example.com" },
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    const picker = await screen.findByTestId("switch-connection-agents:claude-code");
    expect(picker).toHaveValue("relay");

    await userEvent.selectOptions(picker, "official");
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(configureCalls(calls)[0].args).toEqual([
      "--only",
      "agents:claude-code",
      "--use",
      "official",
    ]);

    await userEvent.selectOptions(picker, "");
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(2));
    expect(configureCalls(calls)[1].args).toEqual([
      "--only",
      "agents:claude-code",
      "--use-default",
    ]);
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

    await userEvent.click(await screen.findByTestId("check-connection-agents:claude-code"));
    const result = await screen.findByTestId("check-result-agents:claude-code");
    expect(result).toHaveTextContent("通了");
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

    await userEvent.click(await screen.findByTestId("check-connection-agents:claude-code"));
    const result = await screen.findByTestId("check-result-agents:claude-code");
    expect(result).toHaveTextContent("RELAY_KEY");
    expect(result).toHaveTextContent("密钥页");
  });

  it("没配接入配置的助手：卡片上不出现切换器", async () => {
    renderWithEngine(<AgentsPage />, routes());
    await screen.findByRole("button", { name: "编辑命令" });
    expect(screen.queryByTestId("switch-connection-agents:claude-code")).toBeNull();
  });

  it("Codex 也能配接入配置，但没有认证方式与状态栏那两项", async () => {
    renderWithEngine(<AgentsPage />, agentRoutes("codex", "codex"));
    await openEditor();
    await addConnection();
    // 地址、令牌变量名、模型都有
    expect(screen.getByTestId("connection-base-url-input")).toBeInTheDocument();
    expect(screen.getByTestId("connection-token-var-input")).toBeInTheDocument();
    expect(screen.getByTestId("connection-model-input")).toBeInTheDocument();
    // Codex 只认环境变量名，没有「令牌怎么发」可选；也没有状态栏机制
    expect(screen.queryByTestId("connection-auth-select")).toBeNull();
    expect(screen.queryByTestId("statusline-select")).toBeNull();
    // 跳过权限确认两家都有
    expect(screen.getByTestId("agent-yolo-checkbox")).toBeInTheDocument();
  });

  it("既不是 Claude 也不是 Codex 的助手：不显示运行参数这一段", async () => {
    renderWithEngine(<AgentsPage />, agentRoutes("opencode", "opencode"));
    await openEditor();
    expect(screen.queryByTestId("add-connection")).toBeNull();
    expect(screen.queryByTestId("agent-yolo-checkbox")).toBeNull();
  });
});
