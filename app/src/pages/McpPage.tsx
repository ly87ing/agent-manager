import { useRef, useState, type KeyboardEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, LayoutGrid, Plus, RefreshCw, SearchX, Server } from "lucide-react";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { McpAssignmentRow, type McpAgentCellView } from "../components/McpAssignmentRow";
import { ResourceAgentMatrix, type MatrixRow, type MatrixRowCell } from "../components/ResourceAgentMatrix";
import { McpImportCandidates } from "../components/McpImportCandidates";
import { McpServerCard } from "../components/McpServerCard";
import { McpServerFormDialog } from "../components/McpServerFormDialog";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { SecretValueDialog } from "../components/SecretValueDialog";
import type { Diagnostic, EngineElement, McpImportCandidate, PolicyData, PolicyLayers, StatusData } from "../lib/contract";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { candidateToDefinition, isMcpTakeoverBlocked } from "../lib/mcpImport";
import { buildMcpGlobalSyncPlan, buildMcpSyncPlan, type McpSyncPlan } from "../lib/mcpSync";
import { agentCellsOf, type McpObserved, type McpSpec } from "../lib/observed";
import { useBatchApply } from "../lib/useBatchApply";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { useDirectApply } from "../lib/useDirectApply";
import { useTakeover, type TakeoverTask } from "../lib/useTakeover";
import { OVERLAY_COPY } from "../lib/ui-copy";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { cn } from "../lib/utils";
import { supportedAgentKindOf } from "../lib/agentManagement";

/** 从 policy:get 的 sqlite 有效定义中取服务完整定义；无 DB 旧存储返回 null（调用方走迁移后取完整定义）。 */
function fullMcpDefinition(layers: PolicyLayers | null, name: string): Record<string, unknown> | null {
  const definitions = (layers?.sqlite as { mcp?: { definitions?: Record<string, unknown> } } | null | undefined)?.mcp?.definitions;
  const definition = definitions?.[name];
  return typeof definition === "object" && definition !== null ? (definition as Record<string, unknown>) : null;
}

export default function McpPage() {
  const query = useEngineQuery<StatusData>("mcp:status");
  const agentsQuery = useEngineQuery<StatusData>("agents:status");
  const profilesQuery = useEngineQuery<StatusData>("profiles:status");
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = useState<"library" | "agents">("library");
  const [agentFilter, setAgentFilter] = useState<string>("all");
  const [search, setSearch] = useState("");
  /** 展开中的服务行：矩阵默认只看状态，点开才出逐助手操作；可同时展开多行。 */
  const [expandedMcpIds, setExpandedMcpIds] = useState<Set<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [formState, setFormState] = useState<{ editing: EngineElement } | "new" | null>(null);
  const [editingSecret, setEditingSecret] = useState<string | null>(null);
  const [importTarget, setImportTarget] = useState<McpImportCandidate | null>(null);
  const [importExecuting, setImportExecuting] = useState(false);
  const [importErrors, setImportErrors] = useState<Diagnostic[] | null>(null);
  const [pendingSync, setPendingSync] = useState<
    | {
        title: string;
        serviceCount: number;
        actionCount: number;
        takeoverCount: number;
        removeCount: number;
        importCount: number;
        secretImportCount: number;
        execution: { kind: "plan"; plan: McpSyncPlan } | { kind: "all"; tasks: TakeoverTask[]; blockedCount: number };
      }
    | null
  >(null);

  const directApply = useDirectApply();
  const batchApply = useBatchApply();
  const takeover = useTakeover();
  const definitionOps = useDefinitionOps({ section: "mcp", domain: "mcp" });

  const [pendingRemove, setPendingRemove] = useState<
    | { kind: "server"; element: EngineElement }
    | { kind: "cell"; element: EngineElement; agent: string }
    | null
  >(null);

  const tabRefs = useRef<Record<"library" | "agents", HTMLButtonElement | null>>({ library: null, agents: null });

  const handleTabKeyDown = (e: KeyboardEvent<HTMLButtonElement>, tab: "library" | "agents") => {
    const tabs: ("library" | "agents")[] = ["library", "agents"];
    const index = tabs.indexOf(tab);
    let nextIndex = -1;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      nextIndex = (index + 1) % tabs.length;
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      nextIndex = (index - 1 + tabs.length) % tabs.length;
    } else if (e.key === "Home") {
      e.preventDefault();
      nextIndex = 0;
    } else if (e.key === "End") {
      e.preventDefault();
      nextIndex = tabs.length - 1;
    }
    if (nextIndex >= 0) {
      setActiveTab(tabs[nextIndex]);
      tabRefs.current[tabs[nextIndex]]?.focus();
    }
  };

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="mcp:status 调用失败" error={query.error} />;
  if (!query.data.ok) {
    return (
      <div className="space-y-6">
        <PageHeader title="MCP" description="服务库管理 MCP 服务定义；「助手中的服务」按助手查看并操作每个服务的配置。" />
        <ErrorPanel title="mcp:status 错误" errors={query.data.errors} />
      </div>
    );
  }

  const elements = query.data.data.elements;
  const layers = policyQuery.data?.data.layers ?? null;
  const selected = elements.find((element) => element.id === selectedId) ?? null;

  // 助手列来源：agents:status、服务声明的 agents、已报告的 cell。
  // 不取 policy 层的 agents 结构——其中混有 definitions 等保留字段，不能当助手 ID 列表。
  const supportedAgents = new Set([
    "claude-code",
    "codex",
    ...(agentsQuery.data?.data?.elements ?? [])
      .filter((element) => supportedAgentKindOf(element) !== null)
      .map((element) => element.id.replace(/^agents:/, "")),
    ...(profilesQuery.data?.ok ? profilesQuery.data.data.elements : [])
      .filter((element) => {
        const kind = (element.observed as { kind?: unknown }).kind;
        return kind === "claude" || kind === "codex";
      })
      .map((element) => element.id.replace(/^profiles:/, "")),
  ]);
  const statusAgents = Array.from(supportedAgents);
  const specAgents = elements.flatMap((element) => (element.spec as McpSpec).agents ?? []);
  const cellAgents = elements.flatMap((element) => Object.keys(agentCellsOf(element)));
  const agentColumns = Array.from(new Set([...statusAgents, ...specAgents, ...cellAgents]))
    .filter((agent) => Boolean(agent) && supportedAgents.has(agent))
    .sort();

  /** 冲突行的显式接管入口：一键直接以精确范围 + --allow-takeover 执行 apply。 */
  const handleTakeover = (element: EngineElement, agent: string, action: string) => {
    void directApply.apply({
      command: "mcp:apply",
      label: `接管原条目 ${element.id} @ ${agent}`,
      args: ["--only", element.id, "--agent", agent, "--action", action, "--allow-takeover"],
    });
  };

  /** 库级操作：仅按 observed.actions 提供的 enable/disable/remove；remove 需先经独立 ConfirmModal。 */
  const handleServerAction = (element: EngineElement, action: "enable" | "disable" | "remove" | "upgrade") => {
    if (action === "remove") {
      setPendingRemove({ kind: "server", element });
      return;
    }
    const name = element.id.replace(/^mcp:/, "");
    const titleMap: Record<"enable" | "disable" | "remove" | "upgrade", string> = {
      enable: `启用服务 ${name}`,
      disable: `停用服务 ${name}`,
      remove: `删除服务 ${name}`,
      upgrade: `把 ${name} 换到新版本`,
    };
    void directApply
      .apply({ command: "mcp:apply", label: titleMap[action], args: ["--only", element.id, "--action", action] })
      .then((envelope) => {
        // 换完版本号顺手重查一次，否则卡片还挂着旧读数。各助手那边要再点一次
        // 「更新」才会真正换过去——那条路上的外部配置保护不能绕。
        if (action !== "upgrade" || !envelope?.ok) return;
        toast.info("定义已换到新版本。已经接管的助手要再点一次「更新」才会真正换过去。");
        return runEngine("system:refresh-upstream", ["--only", element.id])
          .catch(() => undefined)
          .finally(() => void queryClient.invalidateQueries({ queryKey: engineKeys.all }));
      });
  };

  /** 服务 × 助手 操作：按钮来自 cell.actions，apply 携带精确 --only/--agent/--action；remove 需先经独立 ConfirmModal。
   * add/update 因同名外部条目 fail-closed 时，失败 toast 外再以动作按钮提供显式接管直执。 */
  const handleCellAction = (element: EngineElement, agent: string, action: string) => {
    if (action === "remove") {
      setPendingRemove({ kind: "cell", element, agent });
      return;
    }
    const args = ["--only", element.id, "--agent", agent, "--action", action];
    const labelMap: Record<string, string> = {
      add: "添加",
      update: "更新",
      enable: "启用",
      disable: "禁用",
      remove: "从此助手删除",
    };
    const label = `${labelMap[action] ?? action} ${element.id} @ ${agent}`;
    void directApply.apply({ command: "mcp:apply", label, args }).then((envelope) => {
      if (
        (action === "add" || action === "update") &&
        envelope &&
        !envelope.ok &&
        envelope.errors.some((diag) => isMcpTakeoverBlocked(diag.message))
      ) {
        toast.error(
          "目标文件中的这个服务条目含有不能静默覆盖的内容。确认其内容可弃后，可显式接管：以服务库定义替换该条目（其他服务与顶层配置保持不变）。",
          { action: { label: "接管原条目并重试", onClick: () => handleTakeover(element, agent, action) } },
        );
      }
    });
  };

  const executeSync = async (plan: McpSyncPlan) => {
    await batchApply.applyBatch({
      command: "mcp:apply",
      actionLabel: "同步",
      rows: plan.rows,
    });
    if (plan.blocked.length > 0) {
      toast.warning(`仍有 ${plan.blocked.length} 个助手需要先处理阻塞原因`);
    }
  };

  const requestSync = (title: string, serviceCount: number, plan: McpSyncPlan) => {
    if (plan.rows.length === 0) {
      if (plan.blocked.length > 0) toast.warning("当前没有可自动执行的同步项，请先处理服务卡片中的阻塞原因");
      return;
    }
    if (plan.takeoverCount > 0 || plan.removeCount > 0) {
      setPendingSync({
        title,
        serviceCount,
        actionCount: plan.rows.length,
        takeoverCount: plan.takeoverCount,
        removeCount: plan.removeCount,
        importCount: 0,
        secretImportCount: 0,
        execution: { kind: "plan", plan },
      });
      return;
    }
    void executeSync(plan);
  };

  /** 服务库的一键同步：自动选出每个助手需要的动作；接管或清理残留只做一次汇总确认。 */
  const handleSync = (element: EngineElement) => {
    const plan = buildMcpSyncPlan(element);
    requestSync(`同步 ${element.id.replace(/^mcp:/, "")}`, 1, plan);
  };

  // 未导入候选（引擎已掩码，前端拿不到明文）与同名冲突视图（元素 + policy 定义双来源）
  const importCandidates = query.data.data.import_candidates ?? [];
  const policyDefinitions = (layers?.sqlite as { mcp?: { definitions?: Record<string, unknown> } } | null | undefined)?.mcp?.definitions ?? {};
  const existingNames = new Set<string>([
    ...elements.map((element) => element.id.replace(/^mcp:/, "")),
    ...Object.keys(policyDefinitions),
  ]);
  const globalSync = buildMcpGlobalSyncPlan(elements, importCandidates, existingNames);

  const executeSyncAll = async (tasks: TakeoverTask[], blockedCount: number) => {
    await takeover.run(tasks, "纳管并同步");
    if (blockedCount > 0) toast.warning(`仍有 ${blockedCount} 项需要先处理服务卡片中的阻塞原因`);
  };

  /** 页面级动作覆盖完整服务库及全部可导入候选，不受当前搜索结果影响。 */
  const handleSyncAll = () => {
    if (globalSync.tasks.length === 0) {
      if (globalSync.blockedCount > 0) toast.warning("当前没有可自动执行的项目，请先处理服务卡片中的阻塞原因");
      return;
    }
    const request = {
      title: "全部纳管并同步",
      serviceCount: globalSync.serviceCount,
      actionCount: globalSync.actionCount,
      takeoverCount: globalSync.takeoverCount,
      removeCount: globalSync.removeCount,
      importCount: globalSync.importCount,
      secretImportCount: globalSync.secretImportCount,
      execution: { kind: "all" as const, tasks: globalSync.tasks, blockedCount: globalSync.blockedCount },
    };
    if (request.importCount > 0 || request.takeoverCount > 0 || request.removeCount > 0) {
      setPendingSync(request);
      return;
    }
    void executeSyncAll(globalSync.tasks, globalSync.blockedCount);
  };

  /** 导入 = policy:set 保存定义（不执行动作、不改动助手配置）；转换只依赖掩码 spec。 */
  const confirmImport = async () => {
    if (!importTarget) return;
    setImportExecuting(true);
    setImportErrors(null);
    const definition = candidateToDefinition(importTarget);
    try {
      const envelope = await runEngine<PolicyData>("policy:set", [
        "--layer",
        "local",
        "--key",
        `mcp.definitions.${importTarget.name}`,
        "--value",
        JSON.stringify({ ...definition, _delete: false, _replace: true }),
      ]);
      if (!envelope.ok) {
        setImportErrors(envelope.errors);
        setImportExecuting(false);
        return;
      }
    } catch (err) {
      setImportErrors([{ code: "BRIDGE_ERROR", element: null, message: engineErrorMessage(err) }]);
      setImportExecuting(false);
      return;
    }
    setImportExecuting(false);
    setImportTarget(null);
    void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    toast.success("已保存到库，按需执行添加/更新动作；应用时如遇同名冲突可选择接管");
  };

  const relevantAgents = agentFilter === "all" ? agentColumns : [agentFilter];
  /** 服务库搜索：名称/启动命令/地址。 */
  const filteredElements = elements.filter((element) => {
    const spec = element.spec as McpSpec;
    return matchesKeyword(search, [element.id, spec.command, spec.url, ...(spec.args ?? [])]);
  });
  /** 搜索与助手筛选叠加（AND）；行级额外匹配助手名列。 */
  /** 矩阵行：一行一个服务，每个助手一格；展开那一行才出逐助手操作。 */
  const mcpMatrixRows: MatrixRow[] = elements
    .filter((element) => matchesKeyword(search, [element.id, (element.spec as McpSpec).command]))
    .map((element) => {
      const cells = (element.observed as McpObserved)?.agents ?? {};
      const rowCells: Record<string, MatrixRowCell | null> = {};
      for (const agent of relevantAgents) {
        const cell = cells[agent] as McpAgentCellView | undefined;
        if (!cell) {
          rowCells[agent] = null;
          continue;
        }
        rowCells[agent] = {
          state: cell.state,
          // 版本与阻塞原因放悬停：单开两列「最新版本 / 当前配置版本」多半整列同一个值，
          // 而且「当前配置版本=未配置」与「状态=未配置」并排会让人分不清在说什么
          title: [
            `${agent}: ${cell.state}`,
            cell.version ? `当前 ${cell.version}` : null,
            cell.blocked_reason ?? null,
          ]
            .filter(Boolean)
            .join(" · "),
          hasActions: (cell.actions?.length ?? 0) > 0,
        };
      }
      return { key: element.id, label: element.id.replace(/^mcp:/, ""), cells: rowCells };
    });

  const assignmentRows = elements.flatMap((element) => {
    const cells = (element.observed as McpObserved)?.agents ?? {};
    // cell 缺失 = 引擎未报告此助手的配置路径：不伪造可执行 cell，行内明确显示未配置
    return relevantAgents.map((agent) => ({
      key: `${element.id}-${agent}`,
      element,
      agent,
      cell: (cells[agent] as McpAgentCellView | undefined) ?? null,
    }));
  }).filter(({ element, agent }) => {
    const spec = element.spec as McpSpec;
    return matchesKeyword(search, [element.id, spec.command, spec.url, agent]);
  });

  const confirmRemove = () => {
    if (!pendingRemove) return;
    if (pendingRemove.kind === "server") {
      const name = pendingRemove.element.id.replace(/^mcp:/, "");
      void directApply.apply({
        command: "mcp:apply",
        label: `删除服务 ${name}`,
        args: ["--only", pendingRemove.element.id, "--action", "remove"],
      });
    } else {
      void directApply.apply({
        command: "mcp:apply",
        label: `从此助手删除 ${pendingRemove.element.id} @ ${pendingRemove.agent}`,
        args: ["--only", pendingRemove.element.id, "--agent", pendingRemove.agent, "--action", "remove"],
      });
    }
    setPendingRemove(null);
  };

  return (
    <div className="space-y-6">
      <PageHeader title="MCP" description="服务库管理 MCP 服务定义；「助手中的服务」按助手查看并操作每个服务的配置。" />

      {query.data.errors.length > 0 && <ErrorPanel title="mcp:status 错误" errors={query.data.errors} />}

      <div role="tablist" aria-label="MCP 管理视图" className="flex border-b">
        {(["library", "agents"] as const).map((tab) => (
          <button
            key={tab}
            ref={(el) => {
              tabRefs.current[tab] = el;
            }}
            type="button"
            role="tab"
            id={`tab-${tab}`}
            aria-controls={`panel-${tab}`}
            aria-selected={activeTab === tab}
            tabIndex={activeTab === tab ? 0 : -1}
            onClick={() => setActiveTab(tab)}
            onKeyDown={(e) => handleTabKeyDown(e, tab)}
            className={cn(
              "border-b-2 px-4 py-2 text-sm font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-ring",
              activeTab === tab
                ? "border-primary font-semibold text-primary"
                : "border-transparent text-muted-foreground hover:border-input hover:text-foreground",
            )}
          >
            {tab === "library" ? "服务库" : "助手中的服务"}
          </button>
        ))}
      </div>

      {activeTab === "library" && (
        <div id="panel-library" role="tabpanel" aria-labelledby="tab-library" className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex flex-wrap items-center gap-3">
              <span className="text-xs text-muted-foreground">共 {elements.length} 个服务</span>
              <SearchInput value={search} onChange={setSearch} placeholder="搜索服务…" testId="mcp-search" />
            </div>
            <div className="flex items-center gap-2">
              <button
                type="button"
                data-testid="sync-all-mcp"
                onClick={handleSyncAll}
                disabled={
                  batchApply.executing ||
                  takeover.executing ||
                  directApply.executing ||
                  (globalSync.tasks.length === 0 && globalSync.blockedCount === 0)
                }
                title="按服务库定义同步全部服务到全部适用助手；不受搜索结果影响"
                className="inline-flex items-center gap-1.5 rounded-md border border-input bg-card px-3.5 py-1.5 text-xs font-medium text-foreground hover:bg-accent focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
              >
                <RefreshCw className={cn("h-4 w-4", takeover.executing && "animate-spin")} aria-hidden />
                {takeover.executing && takeover.progress
                  ? `正在处理 ${takeover.progress.done + 1}/${takeover.progress.total}`
                  : "全部纳管并同步"}
              </button>
              <button
                type="button"
                data-testid="add-server-button"
                onClick={() => setFormState("new")}
                className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3.5 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
              >
                <Plus className="h-4 w-4" aria-hidden /> {OVERLAY_COPY.addServer}
              </button>
            </div>
          </div>

          {search.trim() && filteredElements.length === 0 ? (
            <EmptyState
              icon={SearchX}
              testId="mcp-search-empty"
              title="无匹配结果"
              description="没有名称、启动命令或地址包含该关键词的服务。"
              actions={
                <button
                  type="button"
                  onClick={() => setSearch("")}
                  className="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent"
                >
                  清除搜索关键词
                </button>
              }
            />
          ) : (
            <div className="grid grid-cols-1 gap-4">
              {filteredElements.map((element) => (
                <McpServerCard
                  key={element.id}
                  element={element}
                  selected={selectedId === element.id}
                  onSelect={setSelectedId}
                  onEdit={(el) => setFormState({ editing: el })}
                  onAction={handleServerAction}
                  onSync={handleSync}
                  syncing={batchApply.executing || takeover.executing || directApply.executing}
                  onConfigureSecrets={setEditingSecret}
                />
              ))}
            </div>
          )}
          {elements.length === 0 && (
            <EmptyState
              icon={Server}
              title="暂无 MCP 服务定义"
              description={
                <div className="space-y-1">
                  <p>点击右上角「{OVERLAY_COPY.addServer}」创建第一个服务</p>
                  {importCandidates.length > 0 && (
                    <p>检测到 {importCandidates.length} 个可导入的现有配置，也可在下方「发现未导入的服务器」中导入</p>
                  )}
                </div>
              }
            />
          )}

          {!selected && filteredElements.length > 0 && (
            <p className="text-xs text-muted-foreground">点击服务名展开完整参数、变量状态、写入位置和同步详情。</p>
          )}

          <McpImportCandidates candidates={importCandidates} existingNames={existingNames} onImport={setImportTarget} />
        </div>
      )}

      {activeTab === "agents" && (
        <div id="panel-agents" role="tabpanel" aria-labelledby="tab-agents" className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border bg-card p-4 shadow-sm">
            <div className="flex flex-wrap items-center gap-4">
              <SearchInput value={search} onChange={setSearch} placeholder="搜索服务…" testId="mcp-search" />
              <label className="flex items-center gap-2 text-xs font-medium text-foreground">
                <span>助手筛选:</span>
                <select
                  aria-label="助手筛选"
                  data-testid="filter-agent-select"
                  value={agentFilter}
                  onChange={(e) => setAgentFilter(e.target.value)}
                  className="rounded border border-input bg-card px-2 py-1 font-mono text-xs text-foreground"
                >
                  <option value="all">全部助手 ({agentColumns.length})</option>
                  {agentColumns.map((ag) => (
                    <option key={ag} value={ag}>
                      {ag}
                    </option>
                  ))}
                </select>
              </label>

              {agentFilter !== "all" && (
                <button
                  type="button"
                  onClick={() => setAgentFilter("all")}
                  className="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent"
                >
                  <ArrowLeft className="h-3 w-3" aria-hidden /> 清除筛选
                </button>
              )}
            </div>

            <span className="text-xs text-muted-foreground">当前展示 {assignmentRows.length} 条配置记录</span>
          </div>

          <ResourceAgentMatrix
            rows={mcpMatrixRows}
            agents={relevantAgents}
            kind="mcp"
            resourceHeader="服务"
            expandedKeys={expandedMcpIds}
            onToggleExpand={(key) =>
              setExpandedMcpIds((prev) => {
                const next = new Set(prev);
                if (next.has(key)) next.delete(key);
                else next.add(key);
                return next;
              })
            }
            renderExpanded={(row) => (
              <table className="w-full text-left text-sm">
                <thead>
                  <tr className="border-b text-[11px] text-muted-foreground">
                    <th className="py-1.5 pr-4 font-medium">服务</th>
                    <th className="py-1.5 pr-4 font-medium">助手</th>
                    <th className="py-1.5 pr-4 font-medium">状态</th>
                    <th className="py-1.5 font-medium">操作</th>
                  </tr>
                </thead>
                <tbody>
                  {assignmentRows
                    .filter((item) => item.element.id === row.key)
                    .map(({ key, element, agent, cell }) => (
                      <McpAssignmentRow
                        key={key}
                        element={element}
                        agent={agent}
                        cell={cell}
                        onOpenDetail={(id) => {
                          setSelectedId(id);
                          setActiveTab("library");
                        }}
                        onAction={handleCellAction}
                        onTakeover={handleTakeover}
                      />
                    ))}
                </tbody>
              </table>
            )}
            emptyHint={
              search.trim() ? (
                <EmptyState
                  compact
                  icon={SearchX}
                  title="无匹配结果"
                  description="没有名称、启动命令或助手包含该关键词的服务配置。"
                  actions={
                    <button
                      type="button"
                      onClick={() => setSearch("")}
                      className="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent"
                    >
                      清除搜索关键词
                    </button>
                  }
                />
              ) : (
                <EmptyState
                  compact
                  icon={agentFilter !== "all" ? SearchX : LayoutGrid}
                  title={agentFilter !== "all" ? "无符合筛选条件的服务配置" : "暂无可展示的服务配置"}
                  description={agentFilter !== "all" ? "点击上方「清除筛选」后查看全部助手" : "请先在「服务库」新增服务"}
                />
              )
            }
          />
        </div>
      )}

      {formState && (
        <McpServerFormDialog
          agentIds={agentColumns}
          editing={formState === "new" ? null : formState.editing}
          layers={layers}
          fullDefinition={formState === "new" || !formState.editing ? null : fullMcpDefinition(layers, formState.editing.id.replace(/^mcp:/, ""))}
          onClose={() => setFormState(null)}
          onSubmit={(name, definition, isNew, complete) => {
            setFormState(null);
            void definitionOps.saveDefinition({ name, definition, isNew, complete });
          }}
        />
      )}

      {editingSecret && (
        <SecretValueDialog
          variable={editingSecret}
          stored={false}
          busy={directApply.executing}
          onClose={() => setEditingSecret(null)}
          onSubmit={(value) => {
            void directApply
              .apply({
                command: "secrets:set",
                label: `写入外部密钥 ${editingSecret}`,
                args: ["--var", editingSecret, "--value", value],
              })
              .then((envelope) => {
                if (envelope?.ok) setEditingSecret(null);
              });
          }}
        />
      )}

      {definitionOps.modals}

      {importTarget && (
        <ConfirmModal
          title={`导入 ${importTarget.name}`}
          description={`将 ${importTarget.agent} 现有配置中的「${importTarget.name}」保存为服务库定义；扫描结果只带变量名，不带密钥取值。仅保存定义，不执行添加/更新动作，也不改动助手配置。`}
          risk="write-local"
          confirmLabel="确认导入"
          executing={importExecuting}
          resultErrors={importErrors}
          onConfirm={() => void confirmImport()}
          onClose={() => {
            setImportTarget(null);
            setImportErrors(null);
          }}
        />
      )}

      {pendingRemove && (
        <ConfirmModal
          title={
            pendingRemove.kind === "server"
              ? `删除服务 ${pendingRemove.element.id.replace(/^mcp:/, "")}`
              : `从此助手删除 ${pendingRemove.element.id} @ ${pendingRemove.agent}`
          }
          description={
            pendingRemove.kind === "server"
              ? "删除服务会清理各助手中的配置，但不会卸载软件。确认后执行。"
              : "只清理该助手的配置。确认后执行。"
          }
          risk={pendingRemove.kind === "server" ? "install-remove" : "write-local"}
          requireText={pendingRemove.kind === "server" ? pendingRemove.element.id.replace(/^mcp:/, "") : undefined}
          confirmLabel="确认删除"
          executing={directApply.executing}
          onConfirm={confirmRemove}
          onClose={() => setPendingRemove(null)}
        />
      )}

      {pendingSync && (
        <ConfirmModal
          title={pendingSync.title}
          description={`将执行 ${pendingSync.actionCount} 项助手配置同步，涉及 ${pendingSync.serviceCount} 个服务${pendingSync.importCount > 0 ? `；其中 ${pendingSync.importCount} 个未纳管服务会先导入服务库` : ""}${pendingSync.secretImportCount > 0 ? `，${pendingSync.secretImportCount} 个服务会把原配置里的变量取值转存到“密钥”页选择的外部文件（界面不回显）` : ""}${pendingSync.takeoverCount > 0 ? `，${pendingSync.takeoverCount} 项会用服务库定义接管同名条目` : ""}${pendingSync.removeCount > 0 ? `，并清理 ${pendingSync.removeCount} 项由本工具写入的旧配置` : ""}。未列入计划的配置和顶层配置保持不变。`}
          risk="write-local"
          confirmLabel="确认同步"
          executing={batchApply.executing || takeover.executing}
          onConfirm={() => {
            const execution = pendingSync.execution;
            const promise =
              execution.kind === "plan"
                ? executeSync(execution.plan)
                : executeSyncAll(execution.tasks, execution.blockedCount);
            void promise.then(() => setPendingSync(null));
          }}
          onClose={() => setPendingSync(null)}
        />
      )}
    </div>
  );
}
