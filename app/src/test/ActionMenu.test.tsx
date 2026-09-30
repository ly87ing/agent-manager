import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ActionMenu } from "../components/AccountGrid";

/**
 * 回归：行尾「⋯」菜单放在 overflow 裁切的表格里时整个被裁掉看不见（2026-09-30 真机截图）。
 * 菜单要渲染在页面最外层，不能是被裁切容器的后代。
 */
describe("ActionMenu", () => {
  it("菜单渲染在 overflow 容器之外，点菜单项执行并关闭", async () => {
    const onSelect = vi.fn();
    render(
      <div data-testid="clipper" style={{ overflow: "hidden", height: 20 }}>
        <ActionMenu label="更多操作" items={[{ label: "编辑", testId: "edit", onSelect }]} />
      </div>,
    );
    await userEvent.click(screen.getByRole("button", { name: "更多操作" }));
    const menu = screen.getByRole("menu", { name: "更多操作" });
    expect(screen.getByTestId("clipper").contains(menu)).toBe(false);
    expect(menu.parentElement).toBe(document.body);
    expect(menu.style.position).toBe("fixed");
    await userEvent.click(within(menu).getByRole("menuitem", { name: "编辑" }));
    expect(onSelect).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("点外面、按 Esc 或滚动时关闭", async () => {
    render(
      <div>
        <button type="button">外面</button>
        <ActionMenu label="更多操作" items={[{ label: "编辑", testId: "edit", onSelect: () => undefined }]} />
      </div>,
    );
    const trigger = screen.getByRole("button", { name: "更多操作" });
    await userEvent.click(trigger);
    await userEvent.click(screen.getByRole("button", { name: "外面" }));
    expect(screen.queryByRole("menu")).toBeNull();
    await userEvent.click(trigger);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(trigger).toHaveFocus();
    await userEvent.click(trigger);
    window.dispatchEvent(new Event("scroll"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByRole("menu")).toBeNull();
  });
});
