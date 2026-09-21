import { X, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../lib/utils";

export interface NavItem {
  key: string;
  label: string;
  icon: LucideIcon;
}

/** 应用外壳：桌面侧边栏、窄屏顶部导航与内容区。 */
export function AppLayout({
  nav,
  active,
  onNavigate,
  updateVersion,
  onDismissUpdate,
  children,
}: {
  nav: NavItem[];
  active: string;
  onNavigate: (key: string) => void;
  /** 发现新版本时在「设置」导航项上显示提示点；点击导航项即进入设置页查看。 */
  updateVersion?: string | null;
  /** 按版本记住「不再提示」，直到出现更新版本再提示。 */
  onDismissUpdate?: () => void;
  children: ReactNode;
}) {
  return (
    <div className="flex h-screen flex-col bg-background text-foreground sm:flex-row">
      <aside className="flex w-full shrink-0 flex-col border-b bg-card sm:w-56 sm:border-b-0 sm:border-r">
        <div className="border-b px-4 py-3 sm:px-5 sm:py-4">
          <h1 className="text-base font-semibold tracking-tight">Agent Manager</h1>
        </div>
        <nav className="flex gap-1 overflow-x-auto p-2 sm:block sm:flex-1 sm:space-y-0.5 sm:overflow-y-auto">
          {nav.map((item) => {
            const Icon = item.icon;
            // settings:secrets 这类二级页仍高亮它所属的一级项
            const isActive = item.key === active || active.startsWith(`${item.key}:`);
            const showUpdateBadge = item.key === "settings" && Boolean(updateVersion);
            return (
              <div key={item.key} className="relative shrink-0">
                <button
                  type="button"
                  onClick={() => onNavigate(item.key)}
                  aria-current={isActive ? "page" : undefined}
                  className={cn(
                    "flex items-center gap-2.5 whitespace-nowrap rounded-md px-3 py-2 text-left text-sm transition-colors sm:w-full",
                    isActive ? "bg-primary/10 font-medium text-primary" : "text-muted-foreground hover:bg-accent",
                    showUpdateBadge && "pr-8",
                  )}
                >
                  <Icon className="h-4 w-4 shrink-0" aria-hidden />
                  {item.label}
                  {showUpdateBadge && (
                    <span
                      data-testid="update-badge"
                      title={`发现新版本 ${updateVersion}，点击查看`}
                      className="ml-auto h-2 w-2 shrink-0 rounded-full bg-success"
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
                    className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
                  >
                    <X className="h-3 w-3" aria-hidden />
                  </button>
                )}
              </div>
            );
          })}
        </nav>
        <div className="hidden border-t px-4 py-3 text-xs text-muted-foreground sm:block">
          <p className="font-medium text-foreground">快捷键</p>
          <ul className="mt-1.5 space-y-1">
            <li className="flex items-center gap-1.5">
              <kbd className="rounded border bg-muted px-1 font-mono text-[10px]">⌘K</kbd>搜索与执行
            </li>
            <li className="flex items-center gap-1.5">
              <kbd className="rounded border bg-muted px-1 font-mono text-[10px]">⌘,</kbd>打开设置
            </li>
            <li className="flex items-center gap-1.5">
              <kbd className="rounded border bg-muted px-1 font-mono text-[10px]">⌘F</kbd>聚焦页内搜索
            </li>
          </ul>
        </div>
      </aside>
      <main className="min-h-0 min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-5xl space-y-6 p-4 sm:p-6">
          {children}
        </div>
      </main>
    </div>
  );
}
