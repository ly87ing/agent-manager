import { useRef, useState, type KeyboardEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Plus } from "lucide-react";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { ErrorPanel } from "../components/ErrorPanel";
import { McpAssignmentRow, type McpAgentCellView } from "../components/McpAssignmentRow";
import { McpImportCandidates } from "../components/McpImportCandidates";
import { McpServerCard, McpServerDetail } from "../components/McpServerCard";
import { McpServerFormDialog } from "../components/McpServerFormDialog";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { Diagnostic, EngineElement, McpImportCandidate, PolicyData, PolicyLayers, StatusData } from "../lib/contract";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { candidateToDefinition } from "../lib/mcpImport";
import { agentCellsOf, type McpObserved, type McpSpec } from "../lib/observed";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { OVERLAY_COPY } from "../lib/ui-copy";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { useExecuteApply, usePlanPreview } from "../lib/usePlanPreview";
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

  const preview = usePlanPreview();
  const executeApply = useExecuteApply();
  const definitionOps = useDefinitionOps({ section: "mcp", domain: "mcp" });

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

  /** 所有写操作统一走 plan 预览 → 确认 apply；取消零 policy:set、零 apply。 */
  const openAction = (
    title: string,
    subtitle: string,
    args: string[],
    options?: { confirmLabel?: string; requireConfirmText?: string; takeoverEligible?: boolean },
  ) => {
    // 显式接管：逐 agent add/update 遇「非受管同名条目」拒绝时，错误区可带标注重试；
    // 已带 flag 的请求不再重复提供重试
    const canTakeover = options?.takeoverEligible === true && !args.includes("--allow-takeover");
    preview.open({
      title,
      subtitle,
      planCommands: [{ command: "mcp:plan", args }],
      confirmLabel: options?.confirmLabel ?? "立即执行",
      requireConfirmText: options?.requireConfirmText,
      execute: executeApply({ command: "mcp:apply", label: title, extraArgs: args }),
      takeoverRetry: canTakeover
        ? () => openAction(title, subtitle, [...args, "--allow-takeover"], options)
        : undefined,
    });
  };

  /** 库级操作：仅按 observed.actions 提供的 enable/disable/remove，无 agent 参数。 */
  const handleServerAction = (element: EngineElement, action: "enable" | "disable" | "remove") => {
    const name = element.id.replace(/^mcp:/, "");
    const args = ["--only", element.id, "--action", action];
    if (action === "enable") {
      openAction(`启用服务 ${name}`, "仅切换服务库开关；保留各助手分配与配置文件，不恢复或重写助手配置。", args);
    } else if (action === "disable") {
      openAction(`停用服务 ${name}`, "仅切换服务库开关；保留各助手分配与配置文件，不撤下助手配置。", args);
    } else {
      const affected = Object.values(agentCellsOf(element)).filter(
        (cell) => (cell as McpAgentCellView).assigned === true,
      ).length;
      openAction(
        `删除服务 ${name}`,
        `从配置中移除服务，并清理 ${affected} 条受影响的助手配置；不会卸载已安装的软件包。`,
        args,
        { confirmLabel: "立即删除", requireConfirmText: name },
      );
    }
  };

  /** 服务 × 助手 操作：按钮来自 cell.actions，plan/apply 携带精确 --only/--agent/--action。 */
  const handleCellAction = (element: EngineElement, agent: string, action: string) => {
    const args = ["--only", element.id, "--agent", agent, "--action", action];
    const labelMap: Record<string, string> = {
      add: "添加",
      update: "更新",
      enable: "启用",
      disable: "禁用",
      remove: "从此助手删除",
    };
    openAction(`${labelMap[action] ?? action} ${element.id} @ ${agent}`, "仅同步此服务在对应助手中的配置。", args, {
      takeoverEligible: action === "add" || action === "update",
    });
  };

  /** 冲突行的显式接管入口：直接以 --allow-takeover 打开预览（预览可见 takeover 标注）。 */
  const handleTakeover = (element: EngineElement, agent: string, action: string) => {
    const args = ["--only", element.id, "--agent", agent, "--action", action, "--allow-takeover"];
    openAction(
      `接管原条目 ${element.id} @ ${agent}`,
      "以本工具的标准配置替换目标文件中的同名外部条目；其余条目与字段保持不变。",
      args,
      { takeoverEligible: true },
    );
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

  return (
    <div className="space-y-6">
      <PageHeader title="MCP" description="服务库管理 MCP 服务定义；「助手中的服务」按助手查看并操作每个服务的配置。" />

      {query.data.errors.length > 0 && <ErrorPanel title="mcp:status 错误" errors={query.data.errors} />}

      <div role="tablist" aria-label="MCP 管理视图" className="flex border-b border-slate-200">
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
              "border-b-2 px-4 py-2 text-sm font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-500",
              activeTab === tab
                ? "border-indigo-600 font-semibold text-indigo-600"
                : "border-transparent text-slate-500 hover:border-slate-300 hover:text-slate-700",
            )}
          >
            {tab === "library" ? "服务库" : "助手中的服务"}
          </button>
        ))}
      </div>

      {activeTab === "library" && (
        <div id="panel-library" role="tabpanel" aria-labelledby="tab-library" className="space-y-4">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs text-slate-500">共 {elements.length} 个服务</span>
            <button
              type="button"
              data-testid="add-server-button"
              onClick={() => setFormState("new")}
              className="inline-flex items-center gap-1.5 rounded-md bg-indigo-600 px-3.5 py-1.5 text-xs font-medium text-white hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2"
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
          {elements.length === 0 && <p className="py-4 text-center text-sm text-slate-400">无 MCP server 定义</p>}

          {selected ? (
            <McpServerDetail element={selected} />
          ) : (
            elements.length > 0 && (
              <p className="text-xs text-slate-400">点击服务名展开详情（启动命令 / 密钥变量 / 写入位置 / 同步详情）。</p>
            )
          )}

          <McpImportCandidates candidates={importCandidates} existingNames={existingNames} onImport={setImportTarget} />
        </div>
      )}

      {activeTab === "agents" && (
        <div id="panel-agents" role="tabpanel" aria-labelledby="tab-agents" className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-slate-200 bg-white p-4 shadow-sm">
            <div className="flex flex-wrap items-center gap-4">
              <label className="flex items-center gap-2 text-xs font-medium text-slate-700">
                <span>助手筛选:</span>
                <select
                  aria-label="助手筛选"
                  data-testid="filter-agent-select"
                  value={agentFilter}
                  onChange={(e) => setAgentFilter(e.target.value)}
                  className="rounded border border-slate-300 bg-white px-2 py-1 font-mono text-xs text-slate-700"
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
                  className="inline-flex items-center gap-1 rounded border border-slate-200 px-2 py-1 text-xs text-slate-600 hover:bg-slate-50"
                >
                  <ArrowLeft className="h-3 w-3" aria-hidden /> 清除筛选
                </button>
              )}
            </div>

            <span className="text-xs text-slate-500">当前展示 {assignmentRows.length} 条配置记录</span>
          </div>

          <div className="overflow-x-auto rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
            <table className="w-full text-left text-sm">
              <thead>
                <tr className="border-b border-slate-200 text-xs text-slate-500">
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
              <p className="py-8 text-center text-sm text-slate-400">无符合筛选条件的服务配置</p>
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
            void definitionOps.saveAndApply({ name, definition, isNew, elementId: `mcp:${name}`, complete });
          }}
        />
      )}

      {preview.modal}
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
    </div>
  );
}
