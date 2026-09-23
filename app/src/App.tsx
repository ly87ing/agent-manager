import {
  Activity,
  Bot,
  LayoutDashboard,
  PackageCheck,
  ScrollText,
  Server,
  Settings,
  Sparkles,
  Wallet,
} from "lucide-react";
import { lazy, Suspense, useEffect, useState } from "react";
import { AppLayout, type NavItem } from "./components/AppLayout";
import { CommandPalette } from "./components/CommandPalette";
import AgentsPage from "./pages/AgentsPage";
import { useSettings } from "./lib/settings";
import { subscribeTrayActions } from "./lib/trayActions";
import { useUpdateCheck } from "./lib/update-check";
import { PageError, PageLoading } from "./components/PageState";

const DashboardPage = lazy(() => import("./pages/DashboardPage"));
const McpPage = lazy(() => import("./pages/McpPage"));
const RulesHooksPage = lazy(() => import("./pages/RulesHooksPage"));
const RunsPage = lazy(() => import("./pages/RunsPage"));
const SkillsPage = lazy(() => import("./pages/SkillsPage"));
const UsagePage = lazy(() => import("./pages/UsagePage"));
const TakeoverPage = lazy(() => import("./pages/TakeoverPage"));
const SettingsPage = lazy(() => import("./pages/SettingsPage"));

const NAV: NavItem[] = [
  { key: "dashboard", label: "总览", icon: LayoutDashboard },
  { key: "takeover", label: "导入现有配置", icon: PackageCheck },
  { key: "agents", label: "助手", icon: Bot },
  { key: "skills", label: "技能", icon: Sparkles },
  { key: "mcp", label: "MCP", icon: Server },
  { key: "rules-hooks", label: "规则与 Hooks", icon: ScrollText },
  { key: "usage", label: "用量", icon: Wallet },
  { key: "runs", label: "操作记录", icon: Activity },
  { key: "settings", label: "设置", icon: Settings },
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
  const [activePage, setActivePage] = useState<string>("agents");
  const [selectedSkillAgent, setSelectedSkillAgent] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteScope, setPaletteScope] = useState<"all" | "connections">("all");
  const [paletteConnectionTarget, setPaletteConnectionTarget] = useState<{ keyword: string; itemId: string } | null>(null);
  const update = useUpdateCheck();

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) return;
      if (isTextEditableTarget(event.target)) return;
      if (event.key === ",") {
        event.preventDefault();
        setActivePage("settings");
      } else if (event.key.toLowerCase() === "k") {
        // ⌘K 搜全部；⌘⇧K 只看接入，减少高频切换前的输入。
        event.preventDefault();
        const nextScope = event.shiftKey ? "connections" : "all";
        setPaletteConnectionTarget(null);
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
          setPaletteConnectionTarget(null);
          setPaletteOpen(true);
        }
        else if (action === "takeover") setActivePage("takeover");
        else {
          setPaletteScope("connections");
          setPaletteConnectionTarget({
            keyword: `${action.agentName} · ${action.profileLabel}`,
            itemId: `connection:${action.agentId}:${action.profile || "default"}`,
          });
          setPaletteOpen(true);
        }
      }),
    [],
  );

  if (settings.isPending) return <PageLoading text="正在读取应用设置…" />;
  if (settings.isError) return <PageError title="应用设置加载失败" error={settings.error} />;

  const handleNavigate = (page: string) => {
    if (page === "skills") {
      setSelectedSkillAgent(null);
    }
    setActivePage(page);
  };

  return (
    <AppLayout
      nav={NAV}
      active={activePage}
      onNavigate={handleNavigate}
      updateVersion={update.availableVersion}
      onDismissUpdate={update.dismiss}
    >
      {/* key 驱动重挂载，让页面切换时重放一次轻量进场动画 */}
      <Suspense fallback={<PageLoading text="正在打开页面…" />}>
        <div key={activePage} className="animate-page-in">
          {activePage === "dashboard" && <DashboardPage onNavigate={handleNavigate} />}
          {activePage === "agents" && (
            <AgentsPage
              onManageSkills={(agentId) => {
                setSelectedSkillAgent(agentId);
                setActivePage("skills");
              }}
              onOpenSecrets={() => setActivePage("settings:secrets")}
            />
          )}
          {activePage === "skills" && (
            <SkillsPage
              initialAgent={selectedSkillAgent}
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
        initialKeyword={paletteConnectionTarget?.keyword}
        initialItemId={paletteConnectionTarget?.itemId}
        onClose={() => {
          setPaletteOpen(false);
          setPaletteScope("all");
          setPaletteConnectionTarget(null);
        }}
        onNavigate={(page) => {
          if (page === "skills") setSelectedSkillAgent(null);
          setActivePage(page);
        }}
        onOpenAgentSkills={(agentId) => {
          setSelectedSkillAgent(agentId);
          setActivePage("skills");
        }}
      />
    </AppLayout>
  );
}
