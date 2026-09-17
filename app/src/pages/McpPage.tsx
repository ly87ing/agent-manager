import { useRef, useState, type KeyboardEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, LayoutGrid, Plus, SearchX, Server } from "lucide-react";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { McpAssignmentRow, type McpAgentCellView } from "../components/McpAssignmentRow";
import { McpImportCandidates } from "../components/McpImportCandidates";
import { McpServerCard, McpServerDetail } from "../components/McpServerCard";
import { McpServerFormDialog } from "../components/McpServerFormDialog";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { Diagnostic, EngineElement, McpImportCandidate, PolicyData, PolicyLayers, StatusData } from "../lib/contract";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { candidateToDefinition, MCP_UNMANAGED_CONFLICT_MARKER } from "../lib/mcpImport";
import { agentCellsOf, type McpObserved, type McpSpec } from "../lib/observed";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { useDirectApply } from "../lib/useDirectApply";
import { OVERLAY_COPY } from "../lib/ui-copy";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { cn } from "../lib/utils";

/** 从 policy:get 的 sqlite 有效定义中取服务完整定义；无 DB 旧存储返回 null（调用方走迁移后取完整定义）。 */
function fullMcpDefinition(layers: PolicyLayers | null, name: string): Record<string, unknown> | null {
  const definitions = (layers?.sqlite as { mcp?: { definitions?: Record<string, unknown> } } | null | undefined)?.mcp?.definitions;
  const definition = definitions?.[name];
  return typeof definition === "object" && definition !== null ? (definition as Record<string, unknown>) : null;
}

export default function McpPage() {
  const query = useEngineQuery<StatusData>("mcp:status");
  const agentsQuery = useEngineQuery<StatusData>("agents:status");
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = useState<"library" | "agents">("library");
  const [agentFilter, setAgentFilter] = useState<string>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [formState, setFormState] = useState<{ editing: EngineElement } | "new" | null>(null);
  const [importTarget, setImportTarget] = useState<McpImportCandidate | null>(null);
  const [importExecuting, setImportExecuting] = useState(false);
  const [importErrors, setImportErrors] = useState<Diagnostic[] | null>(null);

  const directApply = useDirectApply();
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
  const statusAgents = (agentsQuery.data?.data?.elements ?? []).map((el) => el.id.replace(/^agents:/, ""));
  const specAgents = elements.flatMap((element) => (element.spec as McpSpec).agents ?? []);
  const cellAgents = elements.flatMap((element) => Object.keys(agentCellsOf(element)));
  const agentColumns = Array.from(new Set([...statusAgents, ...specAgents, ...cellAgents]))
    .filter(Boolean)
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
  const handleServerAction = (element: EngineElement, action: "enable" | "disable" | "remove") => {
    if (action === "remove") {
      setPendingRemove({ kind: "server", element });
      return;
    }
    const name = element.id.replace(/^mcp:/, "");
    const titleMap: Record<"enable" | "disable" | "remove", string> = {
      enable: `启用服务 ${name}`,
      disable: `停用服务 ${name}`,
      remove: `删除服务 ${name}`,
    };
    void directApply.apply({ command: "mcp:apply", label: titleMap[action], args: ["--only", element.id, "--action", action] });
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
        envelope.errors.some((diag) => diag.message.includes(MCP_UNMANAGED_CONFLICT_MARKER))
      ) {
        toast.error(
          "目标文件中已存在同名的外部条目，默认拒绝覆盖。确认其内容可弃后，可显式接管：以本工具的标准配置替换该条目（其余条目与字段保持不变）。",
          { action: { label: "接管原条目并重试", onClick: () => handleTakeover(element, agent, action) } },
        );
      }
    });
  };

  // 未导入候选（引擎已掩码，前端拿不到明文）与同名冲突视图（元素 + policy 定义双来源）
  const importCandidates = query.data.data.import_candidates ?? [];
  const policyDefinitions = (layers?.sqlite as { mcp?: { definitions?: Record<string, unknown> } } | null | undefined)?.mcp?.definitions ?? {};
  const existingNames = new Set<string>([
    ...elements.map((element) => element.id.replace(/^mcp:/, "")),
    ...Object.keys(policyDefinitions),
  ]);

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
  const assignmentRows = elements.flatMap((element) => {
    const cells = (element.observed as McpObserved)?.agents ?? {};
    // cell 缺失 = 引擎未报告此助手的配置路径：不伪造可执行 cell，行内明确显示未配置
    return relevantAgents.map((agent) => ({
      key: `${element.id}-${agent}`,
      element,
      agent,
      cell: (cells[agent] as McpAgentCellView | undefined) ?? null,
    }));
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
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs text-muted-foreground">共 {elements.length} 个服务</span>
            <button
              type="button"
              data-testid="add-server-button"
              onClick={() => setFormState("new")}
              className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3.5 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
            >
              <Plus className="h-4 w-4" aria-hidden /> {OVERLAY_COPY.addServer}
            </button>
          </div>

          <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
            {elements.map((element) => (
              <McpServerCard
                key={element.id}
                element={element}
                selected={selectedId === element.id}
                onSelect={setSelectedId}
                onEdit={(el) => setFormState({ editing: el })}
                onAction={handleServerAction}
              />
            ))}
          </div>
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

          {selected ? (
            <McpServerDetail element={selected} />
          ) : (
            elements.length > 0 && (
              <p className="text-xs text-muted-foreground">点击服务名展开详情（启动命令 / 密钥变量 / 写入位置 / 同步详情）。</p>
            )
          )}

          <McpImportCandidates candidates={importCandidates} existingNames={existingNames} onImport={setImportTarget} />
        </div>
      )}

      {activeTab === "agents" && (
        <div id="panel-agents" role="tabpanel" aria-labelledby="tab-agents" className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border bg-card p-4 shadow-sm">
            <div className="flex flex-wrap items-center gap-4">
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

          <div className="overflow-x-auto rounded-lg border bg-card p-5 shadow-sm">
            <table className="w-full text-left text-sm">
              <thead>
                <tr className="border-b text-xs text-muted-foreground">
                  <th className="py-2.5 pr-4 font-medium">服务</th>
                  <th className="py-2.5 pr-4 font-medium">助手</th>
                  <th className="py-2.5 pr-4 font-medium">最新版本</th>
                  <th className="py-2.5 pr-4 font-medium">当前配置版本</th>
                  <th className="py-2.5 pr-4 font-medium">状态</th>
                  <th className="py-2.5 font-medium">操作</th>
                </tr>
              </thead>
              <tbody>
                {assignmentRows.map(({ key, element, agent, cell }) => (
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
            {assignmentRows.length === 0 && (
              <EmptyState
                compact
                icon={agentFilter !== "all" ? SearchX : LayoutGrid}
                title={agentFilter !== "all" ? "无符合筛选条件的服务配置" : "暂无可展示的服务配置"}
                description={agentFilter !== "all" ? "点击上方「清除筛选」后查看全部助手" : "请先在「服务库」新增服务"}
              />
            )}
          </div>
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

      {definitionOps.modals}

      {importTarget && (
        <ConfirmModal
          title={`导入 ${importTarget.name}`}
          description={`将 ${importTarget.agent} 现有配置中的「${importTarget.name}」保存为服务库定义；密钥以变量引用形式入库（本工具自始至终拿不到明文）。仅保存定义，不执行添加/更新动作，不改动助手配置。`}
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
    </div>
  );
}
