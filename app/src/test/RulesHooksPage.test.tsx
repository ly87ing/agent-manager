import { describe, expect, it } from "vitest";
import RulesHooksPage from "../pages/RulesHooksPage";
import { makeAuditData, makeEnvelope, makeMode } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function routes() {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "rules:status": makeEnvelope(
      { domain: "rules", elements: [] },
      { command: "rules.status" },
    ),
    "hooks:status": makeEnvelope(
      { domain: "hooks", elements: [] },
      { command: "hooks.status" },
    ),
    "policy:get": makeEnvelope(
      { layers: { baseline: null, synced: null, local: null } },
      { command: "policy.get" },
    ),
  };
}

describe("RulesHooksPage 规则与 Hooks 切换容器", () => {
  it("默认渲染规则 Tab，tabIndex 正确，选中状态明确", async () => {
    renderWithEngine(<RulesHooksPage />, routes());

    const rulesTab = screen.getByRole("tab", { name: "规则" });
    const hooksTab = screen.getByRole("tab", { name: "Hooks" });

    expect(rulesTab).toHaveAttribute("aria-selected", "true");
    expect(rulesTab).toHaveAttribute("tabindex", "0");
    expect(hooksTab).toHaveAttribute("aria-selected", "false");
    expect(hooksTab).toHaveAttribute("tabindex", "-1");

    // 渲染规则页内容
    expect(await screen.findByRole("button", { name: "立即同步" })).toBeInTheDocument();
  });

  it("支持鼠标点击切换 Tab 与对应视图内容", async () => {
    renderWithEngine(<RulesHooksPage />, routes());

    const hooksTab = screen.getByRole("tab", { name: "Hooks" });
    await userEvent.click(hooksTab);

    expect(hooksTab).toHaveAttribute("aria-selected", "true");
    expect(hooksTab).toHaveAttribute("tabindex", "0");
    expect(screen.getByRole("tab", { name: "规则" })).toHaveAttribute("aria-selected", "false");
    expect(screen.getByRole("tab", { name: "规则" })).toHaveAttribute("tabindex", "-1");

    // 渲染 Hooks 页内容
    expect(await screen.findByTestId("apply-hooks-button")).toBeInTheDocument();

    // 切换回规则 Tab
    const rulesTab = screen.getByRole("tab", { name: "规则" });
    await userEvent.click(rulesTab);
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("button", { name: "立即同步" })).toBeInTheDocument();
  });

  it("支持标准键盘 ArrowRight / ArrowLeft / End / Home 切换焦点与内容", async () => {
    renderWithEngine(<RulesHooksPage />, routes());

    const rulesTab = screen.getByRole("tab", { name: "规则" });
    const hooksTab = screen.getByRole("tab", { name: "Hooks" });

    // 聚焦初始规则 Tab
    rulesTab.focus();
    expect(rulesTab).toHaveFocus();

    // 按下右方向键 -> 切换至 Hooks Tab
    await userEvent.keyboard("{ArrowRight}");
    expect(hooksTab).toHaveFocus();
    expect(hooksTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByTestId("apply-hooks-button")).toBeInTheDocument();

    // 按下左方向键 -> 切回 规则 Tab
    await userEvent.keyboard("{ArrowLeft}");
    expect(rulesTab).toHaveFocus();
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("button", { name: "立即同步" })).toBeInTheDocument();

    // 按下 End 键 -> 跳至最后一个 Tab (Hooks)
    await userEvent.keyboard("{End}");
    expect(hooksTab).toHaveFocus();
    expect(hooksTab).toHaveAttribute("aria-selected", "true");

    // 按下 Home 键 -> 跳至第一个 Tab (规则)
    await userEvent.keyboard("{Home}");
    expect(rulesTab).toHaveFocus();
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
  });
});
