import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import type { EngineElement, Envelope } from "../lib/contract";

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
    "secrets:set": makeEnvelope(
      { domain: "secrets", variable: "RELAY_KEY", stored: true },
      { command: "secrets.set" },
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
  await userEvent.click(await screen.findByRole("button", { name: "管理助手" }));
  return screen.findByRole("dialog");
}

/** 新建一套接入配置并展开它的编辑区（新建后默认就是展开且生效的那套）。 */
async function addConnection() {
  await userEvent.click(screen.getByTestId("add-connection"));
}

function configureCalls<T extends { command: string }>(calls: T[]): T[] {
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

  it("工作台一键切换接入：一条命令带 --use，切回默认带 --use-default", async () => {
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

    expect(await screen.findByTestId("active-connection-summary")).toHaveTextContent("我的中转");
    expect(within(screen.getByTestId("connection-card-claude-code-relay")).getByText("当前接入")).toBeInTheDocument();

    await userEvent.click(screen.getByTestId("activate-connection-claude-code-official"));
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(configureCalls(calls)[0].args).toEqual([
      "--only",
      "agents:claude-code",
      "--use",
      "official",
    ]);

    await userEvent.click(screen.getByTestId("activate-connection-claude-code-default"));
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(2));
    expect(configureCalls(calls)[1].args).toEqual([
      "--only",
      "agents:claude-code",
      "--use-default",
    ]);
  });

  it("工作台编辑当前接入：预填原值、唯一名称不可改，保存后写回当前配置", async () => {
    const withProfiles = routes();
    const status = withProfiles["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      unknown_runtime_flag: "keep",
      profiles: {
        official: { label: "官方", custom_profile_flag: 7 },
        relay: {
          label: "我的中转",
          base_url: "https://old.example.com",
          token_var: "RELAY_KEY",
          model: "old-model",
          future_option: { mode: "keep" },
        },
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    await userEvent.click(await screen.findByTestId("edit-connection-claude-code-relay"));
    expect(await screen.findByRole("dialog", { name: "编辑 我的中转" })).toBeInTheDocument();
    expect(screen.queryByText("1. 选择接入方式")).toBeNull();
    expect(screen.getByTestId("new-connection-base-url")).toHaveValue("https://old.example.com");
    expect(screen.getByTestId("new-connection-activate")).toBeChecked();
    await userEvent.click(screen.getByTestId("new-connection-advanced"));
    expect(screen.getByTestId("new-connection-name")).toBeDisabled();
    await userEvent.clear(screen.getByTestId("new-connection-label"));
    await userEvent.type(screen.getByTestId("new-connection-label"), "新版中转");
    await userEvent.clear(screen.getByTestId("new-connection-base-url"));
    await userEvent.type(screen.getByTestId("new-connection-base-url"), "https://new.example.com");
    await userEvent.clear(screen.getByTestId("new-connection-model"));
    await userEvent.type(screen.getByTestId("new-connection-model"), "new-model");
    await userEvent.click(screen.getByTestId("connection-form-submit"));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.active).toBe("relay");
    expect(definition.settings.unknown_runtime_flag).toBe("keep");
    expect(definition.settings.profiles.official).toEqual({ label: "官方", custom_profile_flag: 7 });
    expect(definition.settings.profiles.relay).toMatchObject({
      label: "新版中转",
      base_url: "https://new.example.com",
      token_var: "RELAY_KEY",
      model: "new-model",
      future_option: { mode: "keep" },
    });
    expect(configureCalls(calls)[0].args).toEqual(["--only", "agents:claude-code", "--use", "relay"]);
    expect(calls.indexOf(saved)).toBeLessThan(calls.indexOf(configureCalls(calls)[0]));
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
    expect(screen.getByTestId("new-connection-activate")).not.toBeChecked();
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

    await userEvent.click(await screen.findByTestId("duplicate-connection-claude-code-relay"));
    expect(await screen.findByRole("dialog", { name: "复制为 我的中转 副本" })).toBeInTheDocument();
    expect(screen.queryByText("1. 选择接入方式")).toBeNull();
    expect(screen.getByTestId("new-connection-label")).toHaveValue("我的中转 副本");
    expect(screen.getByTestId("new-connection-base-url")).toHaveValue("https://relay.example.com");
    expect(screen.getByTestId("new-connection-token-var")).toHaveValue("RELAY_KEY");
    expect(screen.getByTestId("new-connection-secret")).toHaveValue("");
    expect(screen.getByTestId("new-connection-activate")).not.toBeChecked();
    await userEvent.click(screen.getByTestId("new-connection-advanced"));
    expect(screen.getByTestId("new-connection-name")).toHaveValue("relay-copy");
    expect(screen.getByTestId("new-connection-name")).not.toBeDisabled();
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
    const { calls } = renderWithEngine(<AgentsPage />, withProfiles);

    const remove = await screen.findByTestId("remove-connection-claude-code-relay");
    expect(remove).toBeDisabled();
    expect(remove).toHaveAttribute("title", expect.stringContaining("先切换"));
    expect(calls.some((call) => ["policy:set", "agents:configure", "secrets:set"].includes(call.command))).toBe(false);
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

    await userEvent.click(await screen.findByTestId("check-connection-claude-code-relay"));
    const result = await screen.findByTestId("check-result-claude-code-relay");
    expect(result).toHaveTextContent("RELAY_KEY");
    expect(result).toHaveTextContent("密钥页");
  });

  it("没配接入配置的助手：工作台明确展示默认账号和新增入口", async () => {
    renderWithEngine(<AgentsPage />, routes());
    expect(await screen.findByTestId("connection-card-claude-code-default")).toHaveTextContent("当前接入");
    expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("助手默认账号");
    expect(screen.getByTestId("add-connection-button")).toBeEnabled();
    expect(screen.getByText(/目前只有助手默认账号/)).toBeInTheDocument();
  });

  it("切换执行中只锁接入动作，并在按钮上展示真实进行态", async () => {
    const deferred: { release?: () => void } = {};
    const pendingRoutes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    const status = pendingRoutes["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      profiles: {
        relay: { label: "我的中转", base_url: "https://relay.example.com" },
        official: { label: "官方", base_url: "https://api.anthropic.com" },
      },
    };
    pendingRoutes["agents:configure"] = () =>
      new Promise<Envelope>((resolve) => {
        deferred.release = () => resolve(makeEnvelope(
          { domain: "agents", action: "configure", element: "agents:claude-code" },
          { command: "agents.configure" },
        ));
      });
    renderWithEngine(<AgentsPage />, pendingRoutes);

    const activate = await screen.findByTestId("activate-connection-claude-code-relay");
    await userEvent.click(activate);
    await waitFor(() => expect(activate).toHaveTextContent("正在启用"));
    expect(activate).toBeDisabled();
    expect(screen.getByTestId("activate-connection-claude-code-official")).toBeDisabled();
    expect(screen.getByRole("button", { name: "管理助手" })).toBeEnabled();

    deferred.release?.();
    await waitFor(() => expect(activate).toHaveTextContent("启用"));
  });

  it("从厂商预设新增接入：保留完整定义，凭据单独保存，成功后再启用", async () => {
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
    expect(screen.getByTestId("new-connection-base-url")).toHaveValue("https://api.moonshot.cn/anthropic");
    await userEvent.click(screen.getByTestId("new-connection-advanced"));
    expect(screen.getByTestId("new-connection-alias-opus")).toHaveValue("kimi-k3[1m]");
    await userEvent.clear(screen.getByTestId("new-connection-alias-opus"));
    await userEvent.type(screen.getByTestId("new-connection-alias-opus"), "custom-opus");
    await userEvent.type(screen.getByTestId("new-connection-secret"), "fake-secret-value");
    await userEvent.click(screen.getByTestId("connection-form-submit"));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.custom_meta).toEqual({ owner: "local" });
    expect(definition.settings.yolo).toBe(true);
    expect(definition.settings.unknown_runtime_flag).toBe("keep-me");
    expect(definition.settings.active).toBe("official");
    expect(definition.settings.profiles.official).toEqual({ label: "官方", custom_profile_flag: 7 });
    expect(definition.settings.profiles["kimi-cn"]).toMatchObject({
      label: "Kimi（国内）",
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
    expect(configureCalls(calls)[0].args).toEqual([
      "--only",
      "agents:claude-code",
      "--use",
      "kimi-cn",
    ]);
    expect(calls.indexOf(saved)).toBeLessThan(calls.indexOf(secret));
    expect(calls.indexOf(secret)).toBeLessThan(calls.indexOf(configureCalls(calls)[0]));
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

    expect(await screen.findByRole("alert")).toHaveTextContent("接入已保存，但凭据没有存下");
    expect(screen.getByRole("alert")).toHaveTextContent("系统钥匙串暂时不可用");
    expect(screen.getByTestId("new-connection-secret")).toHaveValue("fake-secret-value");
    expect(screen.getByRole("dialog", { name: "为 Claude Code 添加接入" })).toBeInTheDocument();
    expect(calls.some((call) => call.command === "policy:set")).toBe(true);
    expect(configureCalls(calls)).toHaveLength(0);
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
      observed: { install_source: "native", installed: true },
      state: "managed-ok",
      freshness: makeFreshness(),
      actions: [],
    }));
    renderWithEngine(<AgentsPage />, twoAgents);

    expect(await screen.findByTestId("active-connection-summary")).toHaveTextContent("助手默认账号");
    await userEvent.click(screen.getByTestId("connection-agent-codex"));
    expect(screen.getByRole("heading", { name: "Codex 接入工作台" })).toBeInTheDocument();
    expect(screen.getByTestId("active-connection-summary")).toHaveTextContent("GLM Coding");
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

  it("不支持应用内接入的助手：历史 profiles 只读可见，不露出必失败的启用与测试动作", async () => {
    const unsupported = agentRoutes("opencode", "opencode");
    const status = unsupported["agents:status"] as { data: { elements: { spec: Record<string, unknown> }[] } };
    status.data.elements[0].spec.settings = {
      active: "relay",
      profiles: { relay: { label: "历史接入", base_url: "https://relay.example.com" } },
    };
    renderWithEngine(<AgentsPage />, unsupported);

    expect(await screen.findByText(/暂不支持在应用内切换接入/)).toBeInTheDocument();
    expect(screen.getByTestId("connection-card-opencode-relay")).toHaveTextContent("历史接入");
    expect(screen.queryByTestId("activate-connection-opencode-default")).toBeNull();
    expect(screen.queryByTestId("check-connection-opencode-relay")).toBeNull();
    expect(screen.getByTestId("add-connection-button")).toBeDisabled();
  });
});
