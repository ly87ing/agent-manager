import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { McpAssignmentRow, type McpAgentCellView } from "../components/McpAssignmentRow";
import type { EngineElement } from "../lib/contract";
import { makeFreshness } from "./fixtures";

const element: EngineElement = {
  id: "mcp:legacy-svc",
  domain: "mcp",
  spec: { command: "npx" },
  policy: { enabled: true, agents: null },
  observed: {},
  state: "drifted",
  freshness: makeFreshness({ state: "not-applicable" }),
  actions: [],
};

function renderRow(cell: McpAgentCellView) {
  const onAction = vi.fn();
  render(
    <table>
      <tbody>
        <McpAssignmentRow element={element} agent="codex" cell={cell} onOpenDetail={vi.fn()} onAction={onAction} />
      </tbody>
    </table>,
  );
  return onAction;
}

describe("McpAssignmentRow notice", () => {
  it("带 notice 时作为非阻断提示显示，更新按钮仍可用", () => {
    const onAction = renderRow({
      state: "drifted",
      assigned: true,
      enabled: true,
      actions: ["update", "disable", "remove"],
      blocked_reason: null,
      notice: "条目含旧版 Agent Manager 标记，更新即可清理",
    });
    const notice = screen.getByText("条目含旧版 Agent Manager 标记，更新即可清理");
    expect(notice.className).toContain("text-muted-foreground");
    expect(notice.className).not.toContain("text-warning");
    screen.getByTestId("action-update-mcp:legacy-svc-codex").click();
    expect(onAction).toHaveBeenCalledWith(element, "codex", "update");
  });

  it("没有 notice 时不渲染提示", () => {
    renderRow({ state: "managed-ok", assigned: true, enabled: true, actions: ["disable", "remove"], blocked_reason: null });
    expect(screen.queryByText(/旧版/)).not.toBeInTheDocument();
  });
});
