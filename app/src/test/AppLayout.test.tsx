import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Inbox, Library, Package, Settings, Sparkles, Users } from "lucide-react";
import { AppLayout, type NavItem } from "../components/AppLayout";
import { ThemeProvider } from "../lib/theme";

const NAV: NavItem[] = [
  { key: "inbox", label: "待处理", icon: Inbox, badge: 3 },
  { key: "accounts", label: "账号", icon: Users },
  { key: "skills", label: "技能", icon: Sparkles },
  { key: "sources", label: "来源", icon: Library, badge: 5 },
  { key: "programs", label: "程序", icon: Package, badge: 2 },
];

describe("顶栏放不下时收进「更多」", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("窄窗口：末尾的入口收进菜单，数字合计；当前页在菜单里时按钮显示它的名字；点菜单项跳转", async () => {
    // 每个入口 100px 宽、可用 420px：留出「更多」后只平铺 3 个
    vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(100);
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(420);
    vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
    const onNavigate = vi.fn();
    const { rerender } = render(
      <AppLayout nav={NAV} secondaryNav={[{ key: "settings", label: "设置", icon: Settings, iconOnly: true }]} active="accounts" onNavigate={onNavigate}>
        <div />
      </AppLayout>,
      { wrapper: ThemeProvider },
    );
    const nav = screen.getByRole("navigation");
    expect(within(nav).queryByRole("button", { name: "来源" })).toBeNull();
    const more = within(nav).getByTestId("nav-more");
    expect(more).toHaveTextContent("更多7");
    await userEvent.click(more);
    const menu = screen.getByRole("menu");
    expect(within(menu).getAllByRole("menuitem").map((item) => item.textContent)).toEqual(["来源5", "程序2"]);
    expect(within(menu).getByRole("menuitem", { name: /来源/ })).toHaveFocus();
    await userEvent.keyboard("{End}");
    expect(within(menu).getByRole("menuitem", { name: /程序/ })).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}");
    expect(within(menu).getByRole("menuitem", { name: /来源/ })).toHaveFocus();
    await userEvent.keyboard("{Escape}");
    expect(more).toHaveFocus();
    expect(screen.queryByRole("menu")).toBeNull();
    await userEvent.click(more);
    await userEvent.click(screen.getByRole("menuitem", { name: /程序/ }));
    expect(onNavigate).toHaveBeenCalledWith("programs");
    expect(screen.queryByRole("menu")).toBeNull();

    rerender(
      <AppLayout nav={NAV} active="programs:npm:x" onNavigate={onNavigate}>
        <div />
      </AppLayout>,
    );
    expect(within(nav).getByTestId("nav-more")).toHaveTextContent("程序7");
  });
});
