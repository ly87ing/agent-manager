import { describe, expect, it } from "vitest";
import McpPage from "../pages/McpPage";
import type { StatusData } from "../lib/contract";
import { candidateToDefinition, headerToVarName } from "../lib/mcpImport";
import { makeElement, makeEnvelope, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

function importRoutes(): Record<string, ReturnType<typeof makeEnvelope>> {
  const status: StatusData = {
    domain: "mcp",
    elements: [
      makeElement({
        id: "mcp:chrome-devtools",
        domain: "mcp",
        spec: { command: "npx", args: ["-y", "chrome-devtools-mcp"], agents: null, env: {} },
        observed: { agents: {}, env: {}, actions: ["disable", "remove"] },
        state: "managed-ok",
      }),
    ],
    import_candidates: [
      {
        name: "plain-stdio",
        agent: "claude-code",
        transport: "stdio",
        spec: { command: "npx", args: ["-y", "plain-mcp"], env: { PLAIN_TOKEN: "<literal>", lower_key: "<literal>" } },
      },
      {
        name: "remote-claude",
        agent: "claude-code",
        transport: "remote",
        spec: { url: "https://api.example.com/mcp", headers: { Authorization: "<literal>", "x-api-key": "<literal>" } },
      },
      {
        name: "codex-remote",
        agent: "codex",
        transport: "remote",
        spec: { url: "https://codex.example.com/mcp", headers: { Authorization: "${REMOTE_TOKEN}", "x-static": "<literal>" } },
      },
    ],
  };
  return {
    "mcp:status": makeEnvelope(status, { command: "mcp.status" }),
    "policy:get": makeEnvelope(
      makePolicyData({
        layers: {
          sqlite: {
            mcp: {
              definitions: { "chrome-devtools": { command: "npx", args: ["-y", "chrome-devtools-mcp"] } },
            },
          },
        },
      }),
      { command: "policy.get" },
    ),
    "policy:set": makeEnvelope(
      makePolicyData({ layers: null, effective: null, attribution: null, layer: "sqlite", key: "mcp.definitions.plain-stdio", value: null, path: "/native-manager.db" }),
      { command: "policy.set" },
    ),
  };
}

describe("candidateToDefinition 纯转换", () => {
  it("env 一律转 ${ENV_KEY}；literal header 转大写下划线化占位符；${VAR} 引用直通；agents 固定来源助手", () => {
    const stdio = candidateToDefinition({
      name: "plain-stdio",
      agent: "claude-code",
      transport: "stdio",
      spec: { command: "npx", args: ["-y", "plain-mcp"], env: { PLAIN_TOKEN: "<literal>", lower_key: "<literal>" } },
    });
    expect(stdio).toEqual({
      command: "npx",
      args: ["-y", "plain-mcp"],
      env: { PLAIN_TOKEN: "${PLAIN_TOKEN}", lower_key: "${lower_key}" },
      agents: ["claude-code"],
    });
    expect(JSON.stringify(stdio)).not.toContain("<literal>");

    const remote = candidateToDefinition({
      name: "remote-claude",
      agent: "claude-code",
      transport: "remote",
      spec: { url: "https://api.example.com/mcp", headers: { Authorization: "<literal>", "x-api-key": "<literal>" } },
    });
    expect(remote).toEqual({
      url: "https://api.example.com/mcp",
      headers: { Authorization: "${AUTHORIZATION}", "x-api-key": "${X_API_KEY}" },
      agents: ["claude-code"],
    });

    const codex = candidateToDefinition({
      name: "codex-remote",
      agent: "codex",
      transport: "remote",
      spec: { url: "https://codex.example.com/mcp", headers: { Authorization: "${REMOTE_TOKEN}", "x-static": "<literal>" } },
    });
    expect(codex).toMatchObject({
      headers: { Authorization: "${REMOTE_TOKEN}", "x-static": "${X_STATIC}" },
      agents: ["codex"],
    });
  });

  it("headerToVarName 大写下划线化并处理数字开头", () => {
    expect(headerToVarName("Authorization")).toBe("AUTHORIZATION");
    expect(headerToVarName("x-api-key")).toBe("X_API_KEY");
    expect(headerToVarName("1st-token")).toBe("_1ST_TOKEN");
  });
});

describe("McpPage 发现未导入的服务器", () => {
  it("渲染候选列表：名称、来源助手、传输形态与掩码键清单", async () => {
    renderWithEngine(<McpPage />, importRoutes());
    const section = await screen.findByTestId("mcp-import-candidates");

    const plain = within(section).getByTestId("import-candidate-plain-stdio");
    expect(plain).toHaveTextContent("plain-stdio");
    expect(plain).toHaveTextContent("claude-code");
    expect(plain).toHaveTextContent("stdio");
    expect(plain).toHaveTextContent("env: PLAIN_TOKEN=<literal>, lower_key=<literal>");

    const remote = within(section).getByTestId("import-candidate-remote-claude");
    expect(remote).toHaveTextContent("远程");
    expect(remote).toHaveTextContent("headers: Authorization=<literal>, x-api-key=<literal>");

    // Codex env_http_headers 引用直通展示
    const codex = within(section).getByTestId("import-candidate-codex-remote");
    expect(codex).toHaveTextContent("headers: Authorization=${REMOTE_TOKEN}, x-static=<literal>");

    expect(within(section).getAllByRole("button", { name: /导入/ })).toHaveLength(3);
  });

  it("导入流程：单级确认后 policy:set 保存定义（value 无明文），toast 提示按需执行动作，不 plan/apply", async () => {
    const { calls } = renderWithEngine(<McpPage />, importRoutes());
    await userEvent.click(await screen.findByTestId("import-button-remote-claude"));

    const dialog = await screen.findByRole("dialog", { name: "导入 remote-claude" });
    expect(dialog).toHaveTextContent(/仅保存定义，不执行添加\/更新动作/);
    await userEvent.click(within(dialog).getByTestId("confirm-modal-confirm"));

    await screen.findByText("已保存到库，按需执行添加/更新动作；应用时如遇同名冲突可选择接管");
    const setCalls = calls.filter((call) => call.command === "policy:set");
    expect(setCalls).toHaveLength(1);
    const write = setCalls[0];
    expect(write.args[write.args.indexOf("--key") + 1]).toBe("mcp.definitions.remote-claude");
    const rawValue = write.args[write.args.indexOf("--value") + 1];
    const value = JSON.parse(rawValue);
    expect(value).toEqual({
      url: "https://api.example.com/mcp",
      headers: { Authorization: "${AUTHORIZATION}", "x-api-key": "${X_API_KEY}" },
      agents: ["claude-code"],
      _delete: false,
      _replace: true,
    });
    // 反例断言：payload 全文不得出现明文哨兵或掩码残留
    expect(rawValue).not.toContain("<literal>");
    expect(rawValue).not.toContain("sentinel");
    // 保存不执行：不触发 plan/apply
    expect(calls.some((call) => call.command === "mcp:plan")).toBe(false);
    expect(calls.some((call) => call.command === "mcp:apply")).toBe(false);
  });

  it("stdio 候选导入：env 键转 ${ENV_KEY} 占位符", async () => {
    const { calls } = renderWithEngine(<McpPage />, importRoutes());
    await userEvent.click(await screen.findByTestId("import-button-plain-stdio"));
    const dialog = await screen.findByRole("dialog", { name: "导入 plain-stdio" });
    await userEvent.click(within(dialog).getByTestId("confirm-modal-confirm"));
    await screen.findByText("已保存到库，按需执行添加/更新动作；应用时如遇同名冲突可选择接管");
    const write = calls.find((call) => call.command === "policy:set");
    const value = JSON.parse(write!.args[write!.args.indexOf("--value") + 1]);
    expect(value).toMatchObject({
      command: "npx",
      args: ["-y", "plain-mcp"],
      env: { PLAIN_TOKEN: "${PLAIN_TOKEN}", lower_key: "${lower_key}" },
      agents: ["claude-code"],
    });
  });

  it("同名冲突或非法名的候选显示原因且按钮禁用", async () => {
    const routes = importRoutes();
    const status = routes["mcp:status"].data as StatusData;
    status.import_candidates = [
      { name: "chrome-devtools", agent: "codex", transport: "stdio", spec: { command: "npx", args: [], env: {} } },
      { name: "bad name", agent: "codex", transport: "stdio", spec: { command: "npx", args: [], env: {} } },
    ];
    renderWithEngine(<McpPage />, routes);
    const section = await screen.findByTestId("mcp-import-candidates");

    const conflictBtn = within(section).getByTestId("import-button-chrome-devtools");
    expect(conflictBtn).toBeDisabled();
    expect(within(section).getByTestId("import-blocked-chrome-devtools")).toHaveTextContent("库中已存在同名定义");

    const illegalBtn = within(section).getByTestId("import-button-bad name");
    expect(illegalBtn).toBeDisabled();
    expect(within(section).getByTestId("import-blocked-bad name")).toHaveTextContent("名称非法");
  });

  it("带旧体系管理标记的候选显示「此前由本工具管理」徽标，纯外部候选不显示", async () => {
    const routes = importRoutes();
    const status = routes["mcp:status"].data as StatusData;
    status.import_candidates = [
      {
        name: "legacy-marked",
        agent: "claude-code",
        transport: "stdio",
        spec: { command: "legacy-mcp", args: [], env: { LEGACY_TOKEN: "<literal>" } },
        previously_managed: true,
      },
      {
        name: "plain-external",
        agent: "codex",
        transport: "stdio",
        spec: { command: "npx", args: [], env: {} },
        previously_managed: false,
      },
    ];
    renderWithEngine(<McpPage />, routes);
    const section = await screen.findByTestId("mcp-import-candidates");

    const legacy = within(section).getByTestId("import-candidate-legacy-marked");
    expect(within(legacy).getByTestId("previously-managed-legacy-marked")).toHaveTextContent("此前由本工具管理");

    const plain = within(section).getByTestId("import-candidate-plain-external");
    expect(within(plain).queryByTestId("previously-managed-plain-external")).not.toBeInTheDocument();
  });
});
