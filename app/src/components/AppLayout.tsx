import { ChevronDown, Moon, Search, Sun, X, type LucideIcon } from "lucide-react";
import { useTheme } from "../lib/theme";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { MOD } from "../lib/platform";
import { cn } from "../lib/utils";

export interface NavItem {
  key: string;
  label: string;
  icon: LucideIcon;
  /** 只显示图标（顶栏右侧的次级入口）。 */
  iconOnly?: boolean;
  /** 这一页待处理的项数；0 或不给不显示。 */
  badge?: number;
}

/** 「更多」按钮预留的宽度（含间距）。 */
const MORE_WIDTH = 96;
const ITEM_GAP = 4;

/**
 * 应用外壳：顶栏导航 + 内容区。主入口在左，次级入口（用量、设置）在右。
 * 窗口放不下全部主入口时，从末尾起收进「更多」菜单（不出横向滚动条）；当前页在菜单里时「更多」显示它的名字并高亮。
 */
export function AppLayout({
  nav,
  secondaryNav = [],
  active,
  onNavigate,
  updateVersion,
  onDismissUpdate,
  onSearch,
  children,
}: {
  nav: NavItem[];
  secondaryNav?: NavItem[];
  /** 当前页；`settings:secrets` 这类二级页高亮它所属的一级项。 */
  active: string;
  onNavigate: (key: string) => void;
  /** 发现新版本时在「设置」上显示提示点；点击即进入设置页查看。 */
  updateVersion?: string | null;
  /** 按版本记住「不再提示」，直到出现更新版本再提示。 */
  onDismissUpdate?: () => void;
  /** 打开全局搜索（与 ⌘K 相同）。 */
  onSearch?: () => void;
  children: ReactNode;
}) {
  const { resolvedTheme, setTheme } = useTheme();
  const nextTheme = resolvedTheme === "dark" ? "light" : "dark";
  const itemsRef = useRef<HTMLDivElement>(null);
  const measureRef = useRef<HTMLDivElement>(null);
  const moreRef = useRef<HTMLDivElement>(null);
  const moreButtonRef = useRef<HTMLButtonElement>(null);
  const moreMenuRef = useRef<HTMLDivElement>(null);
  const [visibleCount, setVisibleCount] = useState(nav.length);
  const [moreOpen, setMoreOpen] = useState(false);
  const widthKey = nav.map((item) => `${item.key}:${item.badge ?? 0}`).join("|");

  // 按实际宽度算能平铺几个：量的是隐藏的一排副本，所以收起后再变宽也能放回来
  useLayoutEffect(() => {
    const container = itemsRef.current;
    const measure = measureRef.current;
    if (!container || !measure || typeof ResizeObserver === "undefined") return;
    const update = () => {
      const widths = [...measure.children].map((el) => (el as HTMLElement).offsetWidth + ITEM_GAP);
      const available = container.clientWidth;
      // 还没排版（宽度为 0）时不收起
      if (available === 0 || widths.reduce((sum, width) => sum + width, 0) <= available) return setVisibleCount(nav.length);
      let used = MORE_WIDTH;
      let count = 0;
      for (const width of widths) {
        if (used + width > available) break;
        used += width;
        count += 1;
      }
      setVisibleCount(count);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(container);
    return () => observer.disconnect();
  }, [widthKey]);

  useEffect(() => {
    if (!moreOpen) return;
    moreMenuRef.current?.querySelector<HTMLButtonElement>('[aria-current="page"], [role="menuitem"]')?.focus();
    const close = (event: MouseEvent) => {
      if (!moreRef.current?.contains(event.target as Node)) setMoreOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setMoreOpen(false);
      moreButtonRef.current?.focus();
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", onKey);
    };
  }, [moreOpen]);

  const isActiveKey = (key: string) => key === active || active.startsWith(`${key}:`);
  const shown = nav.slice(0, visibleCount);
  const folded = nav.slice(visibleCount);
  const foldedBadge = folded.reduce((sum, item) => sum + (item.badge ?? 0), 0);
  const foldedActive = folded.find((item) => isActiveKey(item.key));

  const renderItem = (item: NavItem) => {
    const Icon = item.icon;
    const isActive = isActiveKey(item.key);
    const showUpdateBadge = item.key === "settings" && Boolean(updateVersion);
    return (
      <div key={item.key} className="relative shrink-0">
        <button
          type="button"
          onClick={() => onNavigate(item.key)}
          aria-current={isActive ? "page" : undefined}
          aria-label={item.iconOnly ? item.label : undefined}
          title={item.iconOnly ? item.label : undefined}
          className={cn(
            "flex items-center gap-1.5 whitespace-nowrap rounded-md px-3 py-1.5 text-sm transition-colors",
            isActive ? "bg-primary/10 font-medium text-primary" : "text-muted-foreground hover:bg-accent hover:text-foreground",
            item.iconOnly && "px-2",
          )}
        >
          <Icon className="h-4 w-4 shrink-0" aria-hidden />
          {!item.iconOnly && item.label}
          {Boolean(item.badge) && (
            <span
              data-testid={`nav-badge-${item.key}`}
              title={`${item.badge} 项需要处理`}
              aria-hidden
              className="min-w-[1.125rem] rounded-full bg-warning/20 px-1 text-center text-[11px] font-medium leading-[1.125rem] text-warning"
            >
              {item.badge}
            </span>
          )}
          {showUpdateBadge && (
            <span
              data-testid="update-badge"
              title={`发现新版本 ${updateVersion}，点击查看`}
              className="h-2 w-2 shrink-0 rounded-full bg-success"
            />
          )}
        </button>
        {showUpdateBadge && (
          <button
            type="button"
            data-testid="update-badge-dismiss"
            aria-label={`不再提示版本 ${updateVersion} 的更新`}
            title={`不再提示版本 ${updateVersion} 的更新`}
            onClick={onDismissUpdate}
            className="absolute -right-1 -top-1 rounded-full bg-card p-0.5 text-muted-foreground hover:text-foreground"
          >
            <X className="h-3 w-3" aria-hidden />
          </button>
        )}
      </div>
    );
  };

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <header className="shrink-0 border-b bg-card">
        <nav className="relative flex items-center gap-1 px-3 py-2">
          {/* 应用名在窗口标题栏里已有，顶栏只放入口 */}
          <div ref={itemsRef} className="flex min-w-0 flex-1 items-center gap-1">
            {shown.map(renderItem)}
            {folded.length > 0 && (
              <div ref={moreRef} className="relative shrink-0">
                <button
                  type="button"
                  ref={moreButtonRef}
                  data-testid="nav-more"
                  aria-haspopup="menu"
                  aria-expanded={moreOpen}
                  onClick={() => setMoreOpen((open) => !open)}
                  className={cn(
                    "flex items-center gap-1 whitespace-nowrap rounded-md px-3 py-1.5 text-sm transition-colors",
                    foldedActive ? "bg-primary/10 font-medium text-primary" : "text-muted-foreground hover:bg-accent hover:text-foreground",
                  )}
                >
                  {foldedActive ? foldedActive.label : "更多"}
                  {foldedBadge > 0 && (
                    <span className="min-w-[1.125rem] rounded-full bg-warning/20 px-1 text-center text-[11px] font-medium leading-[1.125rem] text-warning">
                      {foldedBadge}
                    </span>
                  )}
                  <ChevronDown className="h-3.5 w-3.5" aria-hidden />
                </button>
                {moreOpen && (
                  <div
                    ref={moreMenuRef}
                    role="menu"
                    aria-label="更多页面"
                    onKeyDown={(event) => {
                      if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
                      event.preventDefault();
                      const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
                      const current = items.indexOf(document.activeElement as HTMLButtonElement);
                      const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : (current + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
                      items[next]?.focus();
                    }}
                    className="absolute left-0 top-full z-50 mt-1 min-w-36 rounded-md border bg-card p-1 shadow-lg"
                  >
                    {folded.map((item) => {
                      const Icon = item.icon;
                      const isActive = isActiveKey(item.key);
                      return (
                        <button
                          key={item.key}
                          type="button"
                          role="menuitem"
                          onClick={() => {
                            setMoreOpen(false);
                            onNavigate(item.key);
                          }}
                          aria-current={isActive ? "page" : undefined}
                          className={cn(
                            "flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-sm",
                            isActive ? "bg-primary/10 font-medium text-primary" : "hover:bg-accent",
                          )}
                        >
                          <Icon className="h-4 w-4 shrink-0" aria-hidden />
                          <span className="flex-1 text-left">{item.label}</span>
                          {Boolean(item.badge) && <span className="text-[11px] font-medium text-warning">{item.badge}</span>}
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            )}
          </div>
          {/* 量宽度用的隐藏副本：与入口同样的字号与内边距，裁在零尺寸框里，不可交互、不进无障碍树 */}
          <div aria-hidden className="pointer-events-none invisible absolute left-0 top-0 h-0 w-0 overflow-hidden">
            <div ref={measureRef} className="flex w-max">
              {nav.map((item) => (
                <span key={item.key} className="flex items-center gap-1.5 whitespace-nowrap px-3 py-1.5 text-sm font-medium">
                  <span className="h-4 w-4" />
                  {item.label}
                  {Boolean(item.badge) && <span className="min-w-[1.125rem] px-1 text-[11px]">{item.badge}</span>}
                </span>
              ))}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            {onSearch && (
              <button
                type="button"
                data-testid="open-search"
                aria-label={`搜索（${MOD}K）`}
                onClick={onSearch}
                title={`搜索账号、技能、MCP、规则和操作（${MOD}K）`}
                className="mr-1 flex shrink-0 items-center gap-2 rounded-md border bg-background px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent hover:text-foreground"
              >
                <Search className="h-3.5 w-3.5" aria-hidden />
                <span className="hidden xl:inline">搜索</span>
                <kbd className="hidden rounded border bg-muted px-1 font-sans text-[10px] xl:inline">{MOD}K</kbd>
              </button>
            )}
            <button
              type="button"
              data-testid="theme-toggle"
              aria-label={nextTheme === "light" ? "切换到浅色" : "切换到深色"}
              title={nextTheme === "light" ? "切换到浅色" : "切换到深色"}
              onClick={() => setTheme(nextTheme)}
              className="rounded-md p-2 text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              {resolvedTheme === "dark" ? <Sun className="h-4 w-4" aria-hidden /> : <Moon className="h-4 w-4" aria-hidden />}
            </button>
            {secondaryNav.map(renderItem)}
          </div>
        </nav>
      </header>
      <main className="min-h-0 min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-6xl space-y-4 px-4 py-4">{children}</div>
      </main>
    </div>
  );
}
