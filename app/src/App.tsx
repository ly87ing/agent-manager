import {
  Bot,
  KeyRound,
  LayoutDashboard,
  RefreshCw,
  ScrollText,
  Server,
  Settings,
  Sparkles,
  Webhook,
} from "lucide-react";
import { useState } from "react";
import { AppLayout, type NavItem } from "./components/AppLayout";
import AgentsPage from "./pages/AgentsPage";
import DashboardPage from "./pages/DashboardPage";
import McpPage from "./pages/McpPage";
import { PlaceholderPage } from "./pages/PlaceholderPage";
import RulesPage from "./pages/RulesPage";
import SecretsPage from "./pages/SecretsPage";
import SkillsPage from "./pages/SkillsPage";
import UpdatesPage from "./pages/UpdatesPage";

const NAV: NavItem[] = [
  { key: "dashboard", label: "Dashboard", icon: LayoutDashboard },
  { key: "agents", label: "Agents", icon: Bot },
  { key: "mcp", label: "MCP", icon: Server },
  { key: "skills", label: "Skills", icon: Sparkles },
  { key: "rules", label: "Rules", icon: ScrollText },
  { key: "hooks", label: "Hooks", icon: Webhook },
  { key: "secrets", label: "Secrets", icon: KeyRound },
  { key: "updates", label: "更新中心", icon: RefreshCw },
  { key: "settings", label: "设置", icon: Settings },
];

export default function App() {
  const [activePage, setActivePage] = useState<string>("dashboard");
  const activeLabel = NAV.find((item) => item.key === activePage)?.label ?? activePage;

  return (
    <AppLayout nav={NAV} active={activePage} onNavigate={setActivePage}>
      {activePage === "dashboard" && <DashboardPage />}
      {activePage === "agents" && <AgentsPage />}
      {activePage === "mcp" && <McpPage />}
      {activePage === "skills" && <SkillsPage />}
      {activePage === "rules" && <RulesPage />}
      {activePage === "secrets" && <SecretsPage />}
      {activePage === "updates" && <UpdatesPage />}
      {(activePage === "hooks" || activePage === "settings") && <PlaceholderPage title={activeLabel} />}
    </AppLayout>
  );
}
