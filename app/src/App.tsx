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
import PolicyPage from "./pages/PolicyPage";
import RulesPage from "./pages/RulesPage";
import RunsPage from "./pages/RunsPage";
import SecretsPage from "./pages/SecretsPage";
import SkillsPage from "./pages/SkillsPage";
import SnapshotsPage from "./pages/SnapshotsPage";
import SyncPage from "./pages/SyncPage";
import UpdatesPage from "./pages/UpdatesPage";
import SettingsPage from "./pages/SettingsPage";
import { useSettings } from "./lib/settings";
import { PageError, PageLoading } from "./components/PageState";

const NAV: NavItem[] = [
  { key: "dashboard", label: "总览", icon: LayoutDashboard },
  { key: "agents", label: "助手", icon: Bot },
  { key: "mcp", label: "MCP 服务", icon: Server },
  { key: "skills", label: "技能", icon: Sparkles },
  { key: "rules", label: "规则", icon: ScrollText },
  { key: "hooks", label: "Hooks", icon: Webhook },
  { key: "tools", label: "Tools", icon: Wrench },
  { key: "aliases", label: "Aliases", icon: Link2 },
  { key: "secrets", label: "密钥状态", icon: KeyRound },
  { key: "updates", label: "更新", icon: RefreshCw },
  { key: "runs", label: "任务记录", icon: Activity },
  { key: "policy", label: "个性化配置", icon: SlidersHorizontal },
  { key: "sync", label: "多设备同步", icon: CloudUpload },
  { key: "snapshots", label: "备份与恢复", icon: DatabaseBackup },
  { key: "settings", label: "设置", icon: Settings },
];

export default function App() {
  // 预加载本机偏好，确保首次打开任意预览时已有默认演练值。
  const settings = useSettings();
  const [activePage, setActivePage] = useState<string>("dashboard");
  if (settings.isPending) return <PageLoading text="正在读取应用设置…" />;
  if (settings.isError) return <PageError title="应用设置加载失败" error={settings.error} />;
  return (
    <AppLayout nav={NAV} active={activePage} onNavigate={setActivePage}>
      {activePage === "dashboard" && <DashboardPage />}
      {activePage === "agents" && <AgentsPage />}
      {activePage === "mcp" && <McpPage />}
      {activePage === "skills" && <SkillsPage />}
      {activePage === "rules" && <RulesPage />}
      {activePage === "hooks" && (
        <DomainApplyPage domain="hooks" title="Hooks" description="助手钩子脚本的生成与写入状态。" />
      )}
      {activePage === "tools" && (
        <DomainApplyPage domain="tools" title="Tools" description="全局命令行工具的安装状态（安装/卸载会先预览并二次确认）。" enableToggles />
      )}
      {activePage === "aliases" && (
        <DomainApplyPage domain="aliases" title="Aliases" description="终端快捷命令（alias）的同步状态，只改受管区块、不动其它内容。" />
      )}
      {activePage === "secrets" && <SecretsPage />}
      {activePage === "updates" && <UpdatesPage />}
      {activePage === "runs" && <RunsPage />}
      {activePage === "policy" && <PolicyPage />}
      {activePage === "sync" && <SyncPage />}
      {activePage === "snapshots" && <SnapshotsPage />}
      {activePage === "settings" && <SettingsPage />}
    </AppLayout>
  );
}
