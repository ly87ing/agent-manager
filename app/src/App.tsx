import {
  Activity,
  Bot,
  ScrollText,
  Server,
  Settings,
  Sparkles,
} from "lucide-react";
import { useState } from "react";
import { AppLayout, type NavItem } from "./components/AppLayout";
import AgentsPage from "./pages/AgentsPage";
import McpPage from "./pages/McpPage";
import RulesHooksPage from "./pages/RulesHooksPage";
import RunsPage from "./pages/RunsPage";
import SkillsPage from "./pages/SkillsPage";
import SettingsPage from "./pages/SettingsPage";
import { useSettings } from "./lib/settings";
import { PageError, PageLoading } from "./components/PageState";

const NAV: NavItem[] = [
  { key: "agents", label: "助手", icon: Bot },
  { key: "skills", label: "技能", icon: Sparkles },
  { key: "mcp", label: "MCP", icon: Server },
  { key: "rules-hooks", label: "规则与 Hooks", icon: ScrollText },
  { key: "runs", label: "操作记录", icon: Activity },
  { key: "settings", label: "设置", icon: Settings },
];

export default function App() {
  // 预加载本机偏好，确保首次打开任意预览时已有默认演练值。
  const settings = useSettings();
  const [activePage, setActivePage] = useState<string>("agents");
  const [selectedSkillAgent, setSelectedSkillAgent] = useState<string | null>(null);

  if (settings.isPending) return <PageLoading text="正在读取应用设置…" />;
  if (settings.isError) return <PageError title="应用设置加载失败" error={settings.error} />;

  const handleNavigate = (page: string) => {
    if (page === "skills") {
      setSelectedSkillAgent(null);
    }
    setActivePage(page);
  };

  return (
    <AppLayout nav={NAV} active={activePage} onNavigate={handleNavigate}>
      {activePage === "agents" && (
        <AgentsPage
          onManageSkills={(agentId) => {
            setSelectedSkillAgent(agentId);
            setActivePage("skills");
          }}
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
      {activePage === "runs" && <RunsPage />}
      {activePage === "settings" && <SettingsPage />}
    </AppLayout>
  );
}
