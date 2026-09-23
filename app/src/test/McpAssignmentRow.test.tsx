import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { McpEntryRow } from "../components/McpAssignmentRow";
import type { EngineElement } from "../lib/contract";
import type { McpAgentCell } from "../lib/observed";
import { makeFreshness } from "./fixtures";
import { mcpCell } from "./mcp-fixtures";

const element: EngineElement = {
  id: "mcp:legacy-svc",
  domain: "mcp",
  spec: { name: "legacy-svc", saved: null },
  policy: {},
  observed: {},
  state: "managed-ok",
  freshness: makeFreshness({ state: "not-applicable" }),
  actions: [],
};

function renderRow(cell: McpAgentCell) {
  const onAction = vi.fn();
  render(
    <ul>
      <McpEntryRow element={element} agent="codex" cell={cell} onAction={onAction} />
    </ul>,
  );
  return onAction;
}

describe("McpEntryRow", () => {
  it("非阻断 notice 以提示显示，动作按钮按 cell.actions 渲染并回传整条单元格", () => {
    const cell = mcpCell({ notice: "条目含旧版 Agent Manager 标记，编辑或启停时会顺带清除", legacy_marker: true });
    const onAction = renderRow(cell);
    const notice = screen.getByText(/旧版 Agent Manager 标记/);
    expect(notice.className).toContain("text-muted-foreground");
    screen.getByTestId("action-edit-mcp:legacy-svc-codex").click();
    expect(onAction).toHaveBeenCalledWith(element, "codex", "edit", cell);
    expect(screen.getByTestId("action-disable-mcp:legacy-svc-codex")).toBeInTheDocument();
    expect(screen.queryByTestId("action-enable-mcp:legacy-svc-codex")).toBeNull();
  });

  it("停用快照显示「已停用（已保存）」；未建模字段只列名字；命令找不到时提示", () => {
    renderRow(
      mcpCell({
        state: "disabled",
        source: "saved",
        extra: { cwd: "/work", tools: "<hidden>", type: "stdio" },
        command_available: false,
        actions: ["enable", "copy", "remove"],
      }),
    );
    expect(screen.getByText("已停用（已保存）")).toBeInTheDocument();
    expect(screen.getByText("另有字段原样保留：cwd、tools")).toBeInTheDocument();
    expect(screen.getByTestId("command-missing-mcp:legacy-svc-codex")).toBeInTheDocument();
    expect(screen.getByTestId("action-enable-mcp:legacy-svc-codex")).toBeInTheDocument();
  });

  it("没有动作时显示阻断原因；变量只显示名字与是否已配置", () => {
    renderRow(
      mcpCell({
        actions: [],
        blocked_reason: "助手 'codex' 已在本工具中停用",
        vars: { TOKEN: { placeholder: "${MY_TOKEN}", resolved: false, kind: "env" } },
      }),
    );
    expect(screen.getByText("助手 'codex' 已在本工具中停用")).toBeInTheDocument();
    expect(screen.getByText("${MY_TOKEN}")).toBeInTheDocument();
    expect(screen.getByLabelText("未配置")).toBeInTheDocument();
  });
});
