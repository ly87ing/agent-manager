import {
  Activity,
  Bot,
  CloudUpload,
  DatabaseBackup,
  KeyRound,
  LayoutDashboard,
  Link2,
  RefreshCw,
  ScrollText,
  Server,
  Settings,
  SlidersHorizontal,
  Sparkles,
  Webhook,
  Wrench,
} from "lucide-react";
import { useState } from "react";
import { AppLayout, type NavItem } from "./components/AppLayout";
import AgentsPage from "./pages/AgentsPage";
import DashboardPage from "./pages/DashboardPage";
import DomainApplyPage from "./pages/DomainApplyPage";
import McpPage from "./pages/McpPage";
import { PlaceholderPage } from "./pages/PlaceholderPage";
import PolicyPage from "./pages/PolicyPage";
import RulesPage from "./pages/RulesPage";
import RunsPage from "./pages/RunsPage";
import SecretsPage from "./pages/SecretsPage";
import SkillsPage from "./pages/SkillsPage";
import SnapshotsPage from "./pages/SnapshotsPage";
import SyncPage from "./pages/SyncPage";
import UpdatesPage from "./pages/UpdatesPage";

const NAV: NavItem[] = [
  { key: "dashboard", label: "Dashboard", icon: LayoutDashboard },
  { key: "agents", label: "Agents", icon: Bot },
  { key: "mcp", label: "MCP", icon: Server },
  { key: "skills", label: "Skills", icon: Sparkles },
  { key: "rules", label: "Rules", icon: ScrollText },
  { key: "hooks", label: "Hooks", icon: Webhook },
  { key: "tools", label: "Tools", icon: Wrench },
  { key: "aliases", label: "Aliases", icon: Link2 },
  { key: "secrets", label: "Secrets", icon: KeyRound },
  { key: "updates", label: "更新中心", icon: RefreshCw },
  { key: "runs", label: "执行中心", icon: Activity },
  { key: "policy", label: "策略编辑器", icon: SlidersHorizontal },
  { key: "sync", label: "配置同步", icon: CloudUpload },
  { key: "snapshots", label: "备份回滚", icon: DatabaseBackup },
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
      {activePage === "hooks" && (
        <DomainApplyPage domain="hooks" title="Hooks" description="claude-hooks.json / codex-hooks.json 渲染结果（hooks 无上游轴；收敛 = 渲染到仓级配置）。" />
      )}
      {activePage === "tools" && (
        <DomainApplyPage domain="tools" title="Tools" description="全局工具链（安装/升级属 install-remove 级，预览含此类动作时须二次确认）。" />
      )}
      {activePage === "aliases" && (
        <DomainApplyPage domain="aliases" title="Aliases" description="受管 shell alias 区块（收敛 = 写入 zshrc/zprofile 受管区块，区块外不动）。" />
      )}
      {activePage === "secrets" && <SecretsPage />}
      {activePage === "updates" && <UpdatesPage />}
      {activePage === "runs" && <RunsPage />}
      {activePage === "policy" && <PolicyPage />}
      {activePage === "sync" && <SyncPage />}
      {activePage === "snapshots" && <SnapshotsPage />}
      {activePage === "settings" && <PlaceholderPage title={activeLabel} />}
    </AppLayout>
  );
}
