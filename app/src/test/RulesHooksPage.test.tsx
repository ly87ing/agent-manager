import { describe, expect, it } from "vitest";
import RulesHooksPage from "../pages/RulesHooksPage";
import RulesPage from "../pages/RulesPage";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode } from "./fixtures";
import type { Envelope } from "../lib/contract";
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
      { layers: { sqlite: null } },
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

  it("规则详情没有原生动作契约时不发起 rules:plan", async () => {
    const detailRoutes: Record<string, Envelope> = routes();
    detailRoutes["rules:status"] = makeEnvelope({
      domain: "rules",
      elements: [
        makeElement({
          id: "rules:claude-code",
          domain: "rules",
          spec: { name: "Claude Code 规则", source: "/tmp/rules.md" },
          observed: { target: "/tmp/.claude/CLAUDE.md", target_exists: true },
          state: "drifted",
          freshness: makeFreshness(),
        }),
      ],
    }, { command: "rules.status" });
    detailRoutes["sources:get"] = makeEnvelope({ sources: [] }, { command: "sources.get" });
    const { calls } = renderWithEngine(<RulesPage />, detailRoutes);

    await userEvent.click(await screen.findByRole("button", { name: "Claude Code 规则" }));
    await screen.findByTestId("drift-panel");
    expect(calls.some((call) => call.command === "rules:plan")).toBe(false);
  });

  it("rules:status 与 hooks:status 返回 ok:false 且 data:{} 时，两 Tab 均显示 DOMAIN_NOT_IMPLEMENTED 且不抛异常", async () => {
    const notImplementedRoutes = {
      "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
      "rules:status": makeEnvelope(
        {} as unknown as { domain: string; elements: [] },
        {
          command: "rules.status",
          ok: false,
          errors: [{ code: "DOMAIN_NOT_IMPLEMENTED", element: null, message: "Rules 域尚未实现" }],
        },
      ),
      "hooks:status": makeEnvelope(
        {} as unknown as { domain: string; elements: [] },
        {
          command: "hooks.status",
          ok: false,
          errors: [{ code: "DOMAIN_NOT_IMPLEMENTED", element: null, message: "Hooks 域尚未实现" }],
        },
      ),
      "policy:get": makeEnvelope(
        { layers: { sqlite: null } },
        { command: "policy.get" },
      ),
    };

    renderWithEngine(<RulesHooksPage />, notImplementedRoutes);

    // 默认 Rules Tab: 展示 DOMAIN_NOT_IMPLEMENTED 与错误消息，不抛异常
    expect(await screen.findByRole("heading", { name: "规则" })).toBeInTheDocument();
    expect(screen.getByText("DOMAIN_NOT_IMPLEMENTED")).toBeInTheDocument();
    expect(screen.getByText("Rules 域尚未实现")).toBeInTheDocument();

    // 切换到 Hooks Tab: 不抛异常，展示 DOMAIN_NOT_IMPLEMENTED 与对应错误消息
    const hooksTab = screen.getByRole("tab", { name: "Hooks" });
    await userEvent.click(hooksTab);
    expect(hooksTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("heading", { name: "Hooks" })).toBeInTheDocument();
    expect(screen.getByText("DOMAIN_NOT_IMPLEMENTED")).toBeInTheDocument();
    expect(screen.getByText("Hooks 域尚未实现")).toBeInTheDocument();

    // 切回 Rules Tab: 依然正常
    const rulesTab = screen.getByRole("tab", { name: "规则" });
    await userEvent.click(rulesTab);
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("heading", { name: "规则" })).toBeInTheDocument();
    expect(screen.getByText("Rules 域尚未实现")).toBeInTheDocument();
  });
});
