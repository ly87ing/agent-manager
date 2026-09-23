import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
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
  render(<McpServerFormDialog mode={mode} onSubmit={onSubmit} onClose={vi.fn()} />);
  return { onSubmit, dialog: screen.getByRole("dialog") };
}

describe("McpServerFormDialog", () => {
  it("新增：名称与助手必填；超时输入只在选中 Codex 时出现", async () => {
    const { onSubmit, dialog } = renderForm({ kind: "add", targets: mcpTargets() });
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    expect(within(dialog).getByRole("alert")).toHaveTextContent("名称只能用");
    await userEvent.type(within(dialog).getByTestId("server-name-input"), "srv");
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    expect(within(dialog).getByRole("alert")).toHaveTextContent("至少选一个助手");
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
    expect(within(dialog).getByRole("alert")).toHaveTextContent("header 值只能写");
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
    const { onSubmit, dialog } = renderForm({ kind: "edit", name: "lan", agent: "claude-code", cell, format: "claude-json" });
    expect(within(dialog).getByTestId("server-transport-stdio")).toBeDisabled();
    expect(within(dialog).getByTestId("server-headers-input")).toHaveValue("X-Static=<literal>\nAuthorization=${TOKEN}");
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
    const { onSubmit, dialog } = renderForm({ kind: "edit", name: "fs", agent: "claude-code", cell, format: "claude-json" });
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
