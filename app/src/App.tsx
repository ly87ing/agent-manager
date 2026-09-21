import {
  Activity,
  Bot,
  LayoutDashboard,
  ScrollText,
  Server,
  Settings,
  Sparkles,
  Wallet,
} from "lucide-react";
import { useEffect, useState } from "react";
import { AppLayout, type NavItem } from "./components/AppLayout";
import { CommandPalette } from "./components/CommandPalette";
import AgentsPage from "./pages/AgentsPage";
import DashboardPage from "./pages/DashboardPage";
import McpPage from "./pages/McpPage";
import RulesHooksPage from "./pages/RulesHooksPage";
import RunsPage from "./pages/RunsPage";
import SkillsPage from "./pages/SkillsPage";
import UsagePage from "./pages/UsagePage";
import TakeoverPage from "./pages/TakeoverPage";
import SettingsPage from "./pages/SettingsPage";
import { useSettings } from "./lib/settings";
import { subscribeTrayActions } from "./lib/trayActions";
import { useUpdateCheck } from "./lib/update-check";
import { PageError, PageLoading } from "./components/PageState";

const NAV: NavItem[] = [
  { key: "dashboard", label: "总览", icon: LayoutDashboard },
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
  const update = useUpdateCheck();

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) return;
      if (isTextEditableTarget(event.target)) return;
      if (event.key === ",") {
        event.preventDefault();
        setActivePage("settings");
      } else if (event.key.toLowerCase() === "k") {
        // 命令面板：一个入口搜到资源、页面与常用动作
        event.preventDefault();
        setPaletteOpen((prev) => !prev);
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
  }, []);

  useEffect(
    () =>
      subscribeTrayActions((action) => {
        if (action === "search") setPaletteOpen(true);
        else if (action === "takeover") setActivePage("takeover");
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
      <div key={activePage} className="animate-page-in">
        {activePage === "dashboard" && <DashboardPage onNavigate={handleNavigate} />}
        {activePage === "agents" && (
          <AgentsPage
            onManageSkills={(agentId) => {
              setSelectedSkillAgent(agentId);
              setActivePage("skills");
            }}
            onGoTakeover={() => setActivePage("takeover")}
          />
        )}
        {activePage === "skills" && (
          <SkillsPage
            initialAgent={selectedSkillAgent}
            onResetAgent={() => setSelectedSkillAgent(null)}
          />
        )}
        {activePage === "mcp" && <McpPage />}
        {activePage === "rules-hooks" && <RulesHooksPage />}
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
      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
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
