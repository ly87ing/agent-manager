import { ScrollText, Server, Settings, Sparkles, Users, Wallet } from "lucide-react";
import { lazy, Suspense, useEffect, useState } from "react";
import { AppLayout, type NavItem } from "./components/AppLayout";
import { CommandPalette } from "./components/CommandPalette";
import { useSettings } from "./lib/settings";
import { subscribeTrayActions } from "./lib/trayActions";
import { useUpdateCheck } from "./lib/update-check";
import { PageError, PageLoading } from "./components/PageState";

const AccountsPage = lazy(() => import("./pages/AccountsPage"));
const McpPage = lazy(() => import("./pages/McpPage"));
const RulesHooksPage = lazy(() => import("./pages/RulesHooksPage"));
const RunsPage = lazy(() => import("./pages/RunsPage"));
const SkillsPage = lazy(() => import("./pages/SkillsPage"));
const UsagePage = lazy(() => import("./pages/UsagePage"));
const TakeoverPage = lazy(() => import("./pages/TakeoverPage"));
const SettingsPage = lazy(() => import("./pages/SettingsPage"));

// 多账号设计（docs/MULTI-ACCOUNT-DESIGN.md）：主入口是账号与共享资源；导入现有配置与操作记录从设置进入。
const NAV: NavItem[] = [
  { key: "accounts", label: "账号", icon: Users },
  { key: "skills", label: "技能", icon: Sparkles },
  { key: "mcp", label: "MCP", icon: Server },
  { key: "rules-hooks", label: "规则", icon: ScrollText },
];
const SECONDARY_NAV: NavItem[] = [
  { key: "usage", label: "用量", icon: Wallet, iconOnly: true },
  { key: "settings", label: "设置", icon: Settings, iconOnly: true },
];

/** 事件目标是否处于文本编辑语境（输入框/文本域/下拉/可编辑区）：命中时全局快捷键不抢按键。 */
function isTextEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag === "INPUT") {
    const type = (target as HTMLInputElement).type;
    return !["checkbox", "radio", "button", "submit", "range", "color", "file"].includes(type);
  }
  return false;
}

export default function App() {
  // 预加载本机偏好，确保首次打开任意预览时已有默认演练值。
  const settings = useSettings();
  const [activePage, setActivePage] = useState<string>("accounts");
  const [selectedSkillAgent, setSelectedSkillAgent] = useState<string | null>(null);
  /** 「skills:<名称>」跳转带进来的技能搜索词：直接定位到那一行。 */
  const [skillSearch, setSkillSearch] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteScope, setPaletteScope] = useState<"all" | "quick">("all");
  const [paletteTarget, setPaletteTarget] = useState<{ keyword: string; itemId: string } | null>(null);
  const update = useUpdateCheck();

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) return;
      if (isTextEditableTarget(event.target)) return;
      if (event.key === ",") {
        event.preventDefault();
        setActivePage("settings");
      } else if (event.key.toLowerCase() === "k") {
        // ⌘K 搜全部；⌘⇧K 只看账号。
        event.preventDefault();
        const nextScope = event.shiftKey ? "quick" : "all";
        setPaletteTarget(null);
        setPaletteOpen((prev) => (prev && paletteScope === nextScope ? false : true));
        setPaletteScope(nextScope);
      } else if (event.key.toLowerCase() === "f") {
        // 仅当前页挂了搜索框（data-page-search）时响应；无搜索框的页面不抢占
        const searchInput = document.querySelector<HTMLElement>("[data-page-search]");
        if (searchInput) {
          event.preventDefault();
          searchInput.focus();
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [paletteScope]);

  useEffect(
    () =>
      subscribeTrayActions((action) => {
        if (action === "search") {
          setPaletteScope("all");
          setPaletteTarget(null);
        } else {
          // 托盘点中账号：面板定位到它，按 Enter 复制启动命令
          setPaletteScope("quick");
          setPaletteTarget({ keyword: `${action.agentName} · ${action.accountName}`, itemId: `account:${action.accountId}` });
        }
        setPaletteOpen(true);
      }),
    [],
  );

  if (settings.isPending) return <PageLoading text="正在读取应用设置…" />;
  if (settings.isError) return <PageError title="应用设置加载失败" error={settings.error} />;

  const handleNavigate = (page: string) => {
    if (page.startsWith("skills:")) {
      setSelectedSkillAgent(null);
      setSkillSearch(page.slice("skills:".length));
      setActivePage("skills");
      return;
    }
    if (page === "skills") {
      setSelectedSkillAgent(null);
      setSkillSearch(null);
    }
    setActivePage(page);
  };

  return (
    <AppLayout
      nav={NAV}
      secondaryNav={SECONDARY_NAV}
      active={activePage === "runs" || activePage === "takeover" ? "settings" : activePage}
      onNavigate={handleNavigate}
      updateVersion={update.availableVersion}
      onDismissUpdate={update.dismiss}
    >
      {/* key 驱动重挂载，让页面切换时重放一次轻量进场动画 */}
      <Suspense fallback={<PageLoading text="正在打开页面…" />}>
        <div key={activePage} className="animate-page-in">
          {activePage === "accounts" && (
            <AccountsPage
              onManageSkills={(accountId) => {
                setSelectedSkillAgent(accountId);
                setSkillSearch(null);
                setActivePage("skills");
              }}
            />
          )}
          {activePage === "skills" && (
            <SkillsPage
              initialAgent={selectedSkillAgent}
              initialSearch={skillSearch}
              onResetAgent={() => setSelectedSkillAgent(null)}
            />
          )}
          {activePage === "mcp" && <McpPage />}
          {activePage === "rules-hooks" && <RulesHooksPage onNavigate={handleNavigate} />}
          {activePage === "takeover" && <TakeoverPage />}
          {activePage === "usage" && <UsagePage />}
          {activePage === "runs" && <RunsPage />}
          {activePage.startsWith("settings") && (
            <SettingsPage
              key={activePage}
              initialAdvanced={activePage.startsWith("settings:") ? activePage.slice("settings:".length) : null}
              onNavigate={handleNavigate}
            />
          )}
        </div>
      </Suspense>
      <CommandPalette
        open={paletteOpen}
        scope={paletteScope}
        initialKeyword={paletteTarget?.keyword}
        initialItemId={paletteTarget?.itemId}
        onClose={() => {
          setPaletteOpen(false);
          setPaletteScope("all");
          setPaletteTarget(null);
        }}
        onNavigate={(page) => {
          if (page === "skills") {
            setSelectedSkillAgent(null);
            setSkillSearch(null);
          }
          setActivePage(page);
        }}
        onOpenAgentSkills={(agentId) => {
          setSelectedSkillAgent(agentId);
          setSkillSearch(null);
          setActivePage("skills");
        }}
      />
    </AppLayout>
  );
}
