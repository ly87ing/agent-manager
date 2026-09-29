import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { McpServerFormDialog, isValidRemoteUrl, type McpFormMode } from "../components/McpServerFormDialog";
import { mcpCell, mcpTargets } from "./mcp-fixtures";

/**
 * MCP 条目表单：新增直接写所选助手；编辑只改已建模字段。表单只负责把输入整理成
 * --spec（env 的 <literal> = 保留原值、${VAR} = 变量引用、其余 = 新写的普通取值），
 * 疑似凭据等权威校验在引擎。
 */
function renderForm(mode: McpFormMode) {
  const onSubmit = vi.fn();
  render(<McpServerFormDialog mode={mode} onSubmit={onSubmit} onPreview={async () => []} onClose={vi.fn()} />);
  return { onSubmit, dialog: screen.getByRole("dialog") };
}

describe("McpServerFormDialog", () => {
  it("输入有效后逐助手展示实际文件预览，修改输入时旧结果立即失效", async () => {
    const onPreview = vi.fn(async (payload: { agents: string[]; spec: Record<string, unknown> }) =>
      payload.agents.map((agent) => ({
        agent,
        files: [{ path: `/isolated/${agent}/config`, exists: false, content: JSON.stringify(payload.spec) }],
      })));
    render(<McpServerFormDialog mode={{ kind: "add", targets: mcpTargets() }} onSubmit={vi.fn()} onPreview={onPreview} onClose={vi.fn()} />);
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("server-name-input"), "example");
    // 新服务默认写到全部账号
    expect(within(dialog).getByTestId("server-agent-claude-code")).toBeChecked();
    expect(within(dialog).getByTestId("server-agent-codex")).toBeChecked();
    await userEvent.type(within(dialog).getByTestId("server-command-input"), "npx");
    await waitFor(() => expect(within(dialog).getByTestId("mcp-preview-content-codex")).toHaveTextContent('"command":"npx"'));
    expect(within(dialog).getByTestId("mcp-preview-content-claude-code")).toHaveTextContent('"command":"npx"');
    expect(onPreview).toHaveBeenCalledWith(expect.objectContaining({ agents: ["claude-code", "codex"] }));
    await userEvent.type(within(dialog).getByTestId("server-command-input"), "2");
    expect(within(dialog).queryByTestId("mcp-preview-content-codex")).toBeNull();
    await waitFor(() => expect(within(dialog).getByTestId("mcp-preview-content-codex")).toHaveTextContent('"command":"npx2"'));
  });

  it("粘贴配置：整段贴进来填进表单，密钥改成变量并说明；没保存前不提交", async () => {
    const onSubmit = vi.fn();
    render(<McpServerFormDialog mode={{ kind: "add", targets: mcpTargets() }} onSubmit={onSubmit} onPreview={async () => []} onClose={vi.fn()} />);
    const dialog = screen.getByRole("dialog");
    await userEvent.click(within(dialog).getByTestId("server-paste-open"));
    const input = within(dialog).getByTestId("server-paste-input");
    await userEvent.click(input);
    await userEvent.paste('{"mcpServers": {"github": {"command": "npx", "args": ["-y", "gh-mcp"], "env": {"GITHUB_TOKEN": "ghp_secret", "MODE": "fast"}}}}');
    await userEvent.click(within(dialog).getByTestId("server-paste-fill"));
    expect(within(dialog).getByTestId("server-name-input")).toHaveValue("github");
    expect(within(dialog).getByTestId("server-command-input")).toHaveValue("npx");
    expect(within(dialog).getByTestId("server-args-input")).toHaveValue("-y\ngh-mcp");
    expect(within(dialog).getByTestId("server-env-input")).toHaveValue("GITHUB_TOKEN=${GITHUB_TOKEN}\nMODE=fast");
    expect(within(dialog).getByTestId("server-paste-notes")).toHaveTextContent("GITHUB_TOKEN 的取值没有填进来");
    expect(dialog).not.toHaveTextContent("ghp_secret");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("粘贴读不出来时就地说明，表单不动", async () => {
    render(<McpServerFormDialog mode={{ kind: "add", targets: mcpTargets() }} onSubmit={vi.fn()} onPreview={async () => []} onClose={vi.fn()} />);
    const dialog = screen.getByRole("dialog");
    await userEvent.click(within(dialog).getByTestId("server-paste-open"));
    await userEvent.type(within(dialog).getByTestId("server-paste-input"), "not json");
    await userEvent.click(within(dialog).getByTestId("server-paste-fill"));
    expect(within(dialog).getByRole("alert")).toHaveTextContent("读不出来");
    expect(within(dialog).getByTestId("server-name-input")).toHaveValue("");
  });

  it("预览失败显示目标原因，不把失败结果当作配置文件", async () => {
    render(<McpServerFormDialog mode={{ kind: "add", targets: mcpTargets() }} onSubmit={vi.fn()}
      onPreview={async () => [{ agent: "claude-code", files: [], error: "目标配置已被外部修改" }]} onClose={vi.fn()} />);
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("server-name-input"), "example");
    await userEvent.click(within(dialog).getByTestId("server-agent-claude-code"));
    await userEvent.type(within(dialog).getByTestId("server-command-input"), "npx");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("目标配置已被外部修改");
    expect(within(dialog).queryByTestId("mcp-preview-content-claude-code")).toBeNull();
  });

  it("新增：名称必填；账号默认全选、全取消时要求至少选一个；超时输入只在选中 Codex 时出现", async () => {
    const { onSubmit, dialog } = renderForm({ kind: "add", targets: mcpTargets() });
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    expect(within(dialog).getByRole("alert")).toHaveTextContent("名称只能用");
    await userEvent.type(within(dialog).getByTestId("server-name-input"), "srv");
    await userEvent.click(within(dialog).getByTestId("server-agent-claude-code"));
    await userEvent.click(within(dialog).getByTestId("server-agent-codex"));
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    expect(within(dialog).getByRole("alert")).toHaveTextContent("至少选一个账号");
    await userEvent.click(within(dialog).getByTestId("server-agent-claude-code"));
    expect(within(dialog).queryByTestId("server-timeout-input")).toBeNull();
    await userEvent.click(within(dialog).getByTestId("server-agent-codex"));
    expect(within(dialog).getByTestId("server-timeout-input")).toHaveValue("120");
    await userEvent.type(within(dialog).getByTestId("server-command-input"), "npx");
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    expect(onSubmit).toHaveBeenCalledWith({
      name: "srv",
      agents: ["claude-code", "codex"],
      spec: { command: "npx", args: [], env: {} },
      startupTimeoutSec: 120,
    });
  });

  it("新增远程：地址走白名单；header 只接受变量引用（VAR 简写补成 ${VAR}），字面量拒绝", async () => {
    const { onSubmit, dialog } = renderForm({ kind: "add", targets: mcpTargets() });
    await userEvent.type(within(dialog).getByTestId("server-name-input"), "remote");
    await userEvent.click(within(dialog).getByTestId("server-agent-claude-code"));
    await userEvent.click(within(dialog).getByTestId("server-transport-remote"));
    await userEvent.type(within(dialog).getByTestId("server-url-input"), "http://example.com/mcp");
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    expect(within(dialog).getByRole("alert")).toHaveTextContent("服务地址仅支持");
    const url = within(dialog).getByTestId("server-url-input");
    await userEvent.clear(url);
    await userEvent.type(url, "https://mcp.example.com/mcp");
    const headers = within(dialog).getByTestId("server-headers-input");
    await userEvent.type(headers, "Authorization=Bearer abc");
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    expect(within(dialog).getByRole("alert")).toHaveTextContent("请求头的值只能写");
    await userEvent.clear(headers);
    await userEvent.type(headers, "Authorization=MY_TOKEN");
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({ spec: { url: "https://mcp.example.com/mcp", headers: { Authorization: "${MY_TOKEN}" } } }),
    );
  });

  it("编辑远程：传输方式锁定；地址没改就不再过白名单；<literal> 保留原值", async () => {
    const cell = mcpCell({
      transport: "remote",
      entry: { url: "http://192.168.1.5:8080/mcp", headers: { "X-Static": "<literal>", Authorization: "${TOKEN}" } },
    });
    const { onSubmit, dialog } = renderForm({ kind: "edit", name: "lan", agent: "claude-code", cell, format: "claude-json", targets: [{ agent: "claude-code", cell, format: "claude-json" }], differs: false });
    expect(within(dialog).getByTestId("server-transport-stdio")).toBeDisabled();
    expect(within(dialog).getByTestId("server-headers-input")).toHaveValue("X-Static=（保留原值）\nAuthorization=${TOKEN}");
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    expect(onSubmit).toHaveBeenCalledWith({
      name: "lan",
      agents: ["claude-code"],
      spec: { url: "http://192.168.1.5:8080/mcp", headers: { "X-Static": "<literal>", Authorization: "${TOKEN}" } },
      startupTimeoutSec: null,
    });
  });

  it("编辑 Claude stdio：没有超时输入；env 支持保留原值、变量引用与普通取值；列出原样保留的字段", async () => {
    const cell = mcpCell({
      entry: { command: "npx", args: ["-y", "fs"], env: { SECRET: "<literal>", REF: "${REF_VAR}" } },
      extra: { alwaysLoad: true, type: "stdio" },
    });
    const { onSubmit, dialog } = renderForm({ kind: "edit", name: "fs", agent: "claude-code", cell, format: "claude-json", targets: [{ agent: "claude-code", cell, format: "claude-json" }], differs: false });
    expect(within(dialog).queryByTestId("server-timeout-input")).toBeNull();
    expect(within(dialog).getByTestId("server-preserved-fields")).toHaveTextContent("alwaysLoad");
    const env = within(dialog).getByTestId("server-env-input");
    await userEvent.type(env, "{enter}NODE_ENV=production");
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    expect(onSubmit).toHaveBeenCalledWith({
      name: "fs",
      agents: ["claude-code"],
      spec: { command: "npx", args: ["-y", "fs"], env: { SECRET: "<literal>", REF: "${REF_VAR}", NODE_ENV: "production" } },
      startupTimeoutSec: null,
    });
  });

  it("不可写的助手（文件出错或在本工具中停用）不出现在新增选项里", () => {
    const { dialog } = renderForm({
      kind: "add",
      targets: mcpTargets({ codex: { error: "解析失败" }, "claude-code": { agent_enabled: false } }),
    });
    expect(within(dialog).queryByTestId("server-agent-codex")).toBeNull();
    expect(within(dialog).queryByTestId("server-agent-claude-code")).toBeNull();
    expect(within(dialog).getByText("没有可写入的助手配置")).toBeInTheDocument();
  });

  it("远程地址前端预检与引擎白名单同语义", () => {
    expect(isValidRemoteUrl("https://mcp.example.com/mcp")).toBe(true);
    expect(isValidRemoteUrl("http://localhost:3000/mcp")).toBe(true);
    expect(isValidRemoteUrl("http://127.0.0.1/mcp")).toBe(true);
    expect(isValidRemoteUrl("http://example.com/mcp")).toBe(false);
    expect(isValidRemoteUrl("https://user@host/mcp")).toBe(false);
    expect(isValidRemoteUrl("ftp://host")).toBe(false);
  });
});
