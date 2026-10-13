import { Bot, Library, Package, Rocket, ScrollText, Server, Settings, Sparkles } from "lucide-react";
import { lazy, Suspense, useEffect, useState } from "react";
import { AppLayout, type NavItem } from "./components/AppLayout";
import { CommandPalette } from "./components/CommandPalette";
import { ImportLinkDialog } from "./components/ImportLinkDialog";
import { subscribeImportLinks, type SharePayload } from "./lib/shareLink";
import { useSettings } from "./lib/settings";
import { PageError, PageLoading } from "./components/PageState";
import { t, useLanguage  } from "./lib/i18n";

const McpPage = lazy(() => import("./pages/McpPage"));
const RulesPage = lazy(() => import("./pages/RulesPage"));
const RunsPage = lazy(() => import("./pages/RunsPage"));
const LaunchPage = lazy(() => import("./pages/LaunchPage"));
const SkillsPage = lazy(() => import("./pages/SkillsPage"));
const SettingsPage = lazy(() => import("./pages/SettingsPage"));
const SourcesPage = lazy(() => import("./pages/SourcesPage"));
const ProgramsPage = lazy(() => import("./pages/ProgramsPage"));
const AgentsPage = lazy(() => import("./pages/AgentsPage"));

// Keep resource queries in their pages; navigation does not collect global status.
const NAV: NavItem[] = [
  { key: "launch", get label() { return t("启动"); }, icon: Rocket },
  { key: "skills", get label() { return t("技能"); }, icon: Sparkles },
  { key: "mcp", label: "MCP", icon: Server },
  { key: "rules", get label() { return t("规则"); }, icon: ScrollText },
  { key: "sources", get label() { return t("来源"); }, icon: Library },
  { key: "programs", get label() { return t("依赖程序"); }, icon: Package },
  { key: "agents", get label() { return t("助手"); }, icon: Bot },
];
const SECONDARY_NAV: NavItem[] = [
  { key: "settings", get label() { return t("设置"); }, icon: Settings, iconOnly: true },
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
  useLanguage();
  // 预加载本机偏好，确保首次打开任意预览时已有默认演练值。
  const settings = useSettings();
  const [activePage, setActivePage] = useState<string>("launch");
  const [selectedSkillAgent, setSelectedSkillAgent] = useState<string | null>(null);
  /** 「skills:<名称>」跳转带进来的技能搜索词：直接定位到那一行。 */
  const [skillSearch, setSkillSearch] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteScope, setPaletteScope] = useState<"all" | "quick">("all");
  const [paletteTarget, setPaletteTarget] = useState<{ keyword: string; itemId: string } | null>(null);
  /** 跳到来源页时定位的来源（名称；`unmanaged` 为未纳管）。 */
  const [sourceFocus, setSourceFocus] = useState<string | null>(null);
  /** 跳到 MCP 页时直接搜到的服务。 */
  const [mcpSearch, setMcpSearch] = useState<string | null>(null);
  /** 跳到程序页时定位的程序（程序标识，如 npm:@wecom/cli）。 */
  const [programFocus, setProgramFocus] = useState<string | null>(null);
  /** 从分享链接导入：对话框里的链接文字（null = 没打开），以及确认后交给页面预填的内容。 */
  const [importText, setImportText] = useState<string | null>(null);
  const [importPayload, setImportPayload] = useState<SharePayload | null>(null);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) return;
      if (isTextEditableTarget(event.target)) return;
      if (event.key === ",") {
        event.preventDefault();
        setActivePage("settings");
      } else if (event.key.toLowerCase() === "k") {
        // Both shortcuts search extension resources.
        event.preventDefault();
        const nextScope = "all";
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

  // 系统里点开 agentmanager://import 链接：打开导入对话框并填好链接，由用户检查确认
  useEffect(() => subscribeImportLinks((url) => setImportText(url)), []);

  if (settings.isPending) return <PageLoading text={t("正在读取应用设置…")} />;
  if (settings.isError) return <PageError title={t("应用设置加载失败")} error={settings.error} />;

  const handleNavigate = (page: string) => {
    if (page === "import-link") {
      setImportText("");
      return;
    }
    if (page === "settings:aliases") {
      setActivePage("skills");
      return;
    }
    if (page === "settings:launch") {
      setActivePage("launch");
      return;
    }
    if (page.startsWith("skills:")) {
      setSelectedSkillAgent(null);
      setSkillSearch(page.slice("skills:".length));
      setActivePage("skills");
      return;
    }
    if (page.startsWith("sources:")) {
      setSourceFocus(page.slice("sources:".length));
      setActivePage("sources");
      return;
    }
    if (page.startsWith("programs:")) {
      setProgramFocus(page.slice("programs:".length));
      setActivePage("programs");
      return;
    }
    if (page.startsWith("mcp:")) {
      setMcpSearch(page.slice("mcp:".length));
      setActivePage("mcp");
      return;
    }
    if (page === "skills") {
      setSelectedSkillAgent(null);
      setSkillSearch(null);
    }
    if (page === "sources") setSourceFocus(null);
    if (page === "mcp") setMcpSearch(null);
    if (page === "programs") setProgramFocus(null);
    setActivePage(page === "accounts" || page === "commands" ? "skills" : page);
  };

  return (
    <AppLayout
      nav={NAV}
      secondaryNav={SECONDARY_NAV}
      active={activePage === "runs" ? "settings" : activePage}
      onNavigate={handleNavigate}
      onSearch={() => {
        setPaletteScope("all");
        setPaletteTarget(null);
        setPaletteOpen(true);
      }}
    >
      {settings.data?.runtimeWarning && (
        <p role="alert" className="mb-3 rounded-lg border border-warning/30 bg-warning/10 px-4 py-3 text-sm text-foreground">
          {settings.data.runtimeWarning}
        </p>
      )}
      {/* key 驱动重挂载，让页面切换时重放一次轻量进场动画 */}
      <Suspense fallback={<PageLoading text={t("正在打开页面…")} />}>
        <div key={`${activePage}:${activePage === "sources" ? sourceFocus ?? "" : activePage === "mcp" ? mcpSearch ?? "" : activePage === "programs" ? programFocus ?? "" : ""}`} className="animate-page-in">
          {activePage === "launch" && <LaunchPage />}
          {activePage === "programs" && <ProgramsPage focus={programFocus} onOpen={handleNavigate} />}
          {activePage === "agents" && <AgentsPage />}
          {activePage === "skills" && (
            <SkillsPage
              onOpenSource={(name) => handleNavigate(name ? `sources:${name}` : "sources")}
              onOpenProgram={(program) => handleNavigate(`programs:${program}`)}
              initialAgent={selectedSkillAgent}
              initialSearch={skillSearch}
              onResetAgent={() => setSelectedSkillAgent(null)}
            />
          )}
          {activePage === "mcp" && (
            <McpPage importPayload={importPayload?.kind === "mcp" ? importPayload : null} onImportConsumed={() => setImportPayload(null)} initialSearch={mcpSearch} onOpenProgram={(program) => handleNavigate(`programs:${program}`)} />
          )}
          {activePage === "sources" && (
            <SourcesPage
              focus={sourceFocus}
              onImportLink={() => setImportText("")}
              importPayload={importPayload?.kind === "skills" ? importPayload : null}
              onImportConsumed={() => setImportPayload(null)}
            />
          )}
          {activePage === "rules" && <RulesPage onOpenSource={(name) => handleNavigate(name ? `sources:${name}` : "sources")} />}
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
        onNavigate={handleNavigate}
        onOpenAgentSkills={(agentId) => {
          setSelectedSkillAgent(agentId);
          setSkillSearch(null);
          setActivePage("skills");
        }}
      />
      {importText !== null && (
        <ImportLinkDialog
          key={importText}
          initialText={importText}
          onClose={() => setImportText(null)}
          onContinue={(payload) => {
            setImportText(null);
            setImportPayload(payload);
            setActivePage(payload.kind === "mcp" ? "mcp" : "sources");
          }}
        />
      )}
    </AppLayout>
  );
}
