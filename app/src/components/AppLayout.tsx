import { Moon, Search, Sun, X, type LucideIcon } from "lucide-react";
import { useTheme } from "../lib/theme";
import type { ReactNode } from "react";
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

/** 应用外壳：顶栏导航 + 内容区。主入口在左，次级入口（用量、设置）在右。 */
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
  const renderItem = (item: NavItem) => {
    const Icon = item.icon;
    const isActive = item.key === active || active.startsWith(`${item.key}:`);
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
        <nav className="flex items-center gap-1 overflow-x-auto px-3 py-2">
          <span className="mr-3 shrink-0 text-sm font-semibold tracking-tight">Agent Manager</span>
          {nav.map(renderItem)}
          <div className="ml-auto flex items-center gap-1">
            {onSearch && (
              <button
                type="button"
                data-testid="open-search"
                aria-label="搜索（⌘K）"
                onClick={onSearch}
                title="搜索账号、技能、MCP、规则和操作"
                className="mr-1 flex shrink-0 items-center gap-2 rounded-md border bg-background px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent hover:text-foreground"
              >
                <Search className="h-3.5 w-3.5" aria-hidden />
                搜索
                <kbd className="rounded border bg-muted px-1 font-sans text-[10px]">⌘K</kbd>
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
