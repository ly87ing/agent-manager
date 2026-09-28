import { useRef, useState, type KeyboardEvent } from "react";
import { LayoutGrid, Plus, SearchX, Server } from "lucide-react";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { McpEntryRow, mcpCellKey, mcpEntrySummary } from "../components/McpAssignmentRow";
import { McpCopyDialog } from "../components/McpCopyDialog";
import { ResourceAgentMatrix, type MatrixRow, type MatrixRowCell } from "../components/ResourceAgentMatrix";
import { McpServerCard } from "../components/McpServerCard";
import { McpServerFormDialog, type McpFilePreview, type McpFormMode, type McpFormPayload } from "../components/McpServerFormDialog";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { SecretValueDialog } from "../components/SecretValueDialog";
import type { EngineElement, Envelope, McpTarget, StatusData } from "../lib/contract";
import type { McpAgentCell, McpObserved, McpSavedDefinition, McpSpec } from "../lib/observed";
import { OVERLAY_COPY } from "../lib/ui-copy";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { useEngineQuery } from "../lib/queries";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

function cellsOf(element: EngineElement): Record<string, McpAgentCell> {
  return (element.observed as McpObserved).agents ?? {};
}

function nameOf(element: EngineElement): string {
  return element.id.replace(/^mcp:/, "");
}

/** 引擎附带的提示（转存了哪些变量、复制时哪些字段没带过去）：只含名字，不含取值。 */
function showNotices(envelope: Envelope | null) {
  for (const warning of envelope?.warnings ?? []) toast.info(warning.message);
}

/**
 * MCP：各助手配置文件里的实际条目就是当前配置。
 *
 * 同一个服务名在不同助手里可以内容不同，逐条编辑、启停、复制、删除；每次写入都带上
 * 打开时该条目的摘要，外部改过就拒绝并刷新。停用：Codex/OpenCode 写官方 enabled=false，
 * Claude Code 把条目移出配置、保存在本工具里（启用时写回）。
 */
export default function McpPage() {
  const query = useEngineQuery<StatusData>("mcp:status");
  const directApply = useDirectApply();
  const [activeTab, setActiveTab] = useState<"services" | "agents">("services");
  const [search, setSearch] = useState("");
  const [agentFilter, setAgentFilter] = useState<string>("all");
  const [expandedMcpIds, setExpandedMcpIds] = useState<Set<string>>(new Set());
  const [form, setForm] = useState<McpFormMode | null>(null);
  /** 从已保存定义添加时，按助手追加的参数（旧版 extra_args_by_agent）。 */
  const savedExtraArgs = useRef<Record<string, string[]>>({});
  const [copying, setCopying] = useState<{ element: EngineElement; agent: string; cell: McpAgentCell } | null>(null);
  const [pendingRemove, setPendingRemove] = useState<{ element: EngineElement; agent: string; cell: McpAgentCell } | null>(null);
  const [pendingForget, setPendingForget] = useState<EngineElement | null>(null);
  const [editingSecret, setEditingSecret] = useState<string | null>(null);
  const tabRefs = useRef<Record<"services" | "agents", HTMLButtonElement | null>>({ services: null, agents: null });

  const handleTabKeyDown = (e: KeyboardEvent<HTMLButtonElement>, tab: "services" | "agents") => {
    const tabs: ("services" | "agents")[] = ["services", "agents"];
    const index = tabs.indexOf(tab);
    let next = -1;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") next = (index + 1) % tabs.length;
    else if (e.key === "ArrowLeft" || e.key === "ArrowUp") next = (index - 1 + tabs.length) % tabs.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = tabs.length - 1;
    if (next >= 0) {
      e.preventDefault();
      setActiveTab(tabs[next]);
      tabRefs.current[tabs[next]]?.focus();
    }
  };

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="MCP 状态读取失败" error={query.error} />;
  if (!query.data.ok) {
    return (
      <div className="space-y-6">
        <PageHeader title="MCP" description="各助手配置文件里的 MCP 条目。" />
        <ErrorPanel title="MCP 状态读取失败" errors={query.data.errors} />
      </div>
    );
  }

  const elements = query.data.data.elements;
  const targets: McpTarget[] = query.data.data.targets ?? [];
  const agentColumns = targets.map((target) => target.agent);
  const targetOf = (agent: string) => targets.find((target) => target.agent === agent);
  const busy = directApply.executing;

  const apply = (label: string, args: string[]) =>
    directApply.apply({ command: "mcp:apply", label, args }).then((envelope) => {
      showNotices(envelope);
      return envelope;
    });

  const handleAction = (element: EngineElement, agent: string, action: string, cell: McpAgentCell) => {
    const name = nameOf(element);
    if (action === "edit") {
      setForm({ kind: "edit", name, agent, cell, format: targetOf(agent)?.format ?? "" });
      return;
    }
    if (action === "copy") {
      setCopying({ element, agent, cell });
      return;
    }
    if (action === "remove") {
      setPendingRemove({ element, agent, cell });
      return;
    }
    const label = `${action === "enable" ? "启用" : "停用"} ${name} @ ${agent}`;
    void apply(label, ["--only", element.id, "--agent", agent, "--action", action, "--base", cell.base]);
  };

  const specFor = (payload: McpFormPayload, agent: string): Record<string, unknown> => {
    const spec: Record<string, unknown> = { ...payload.spec };
    const extra = savedExtraArgs.current[agent];
    if (extra && Array.isArray(spec.args)) spec.args = [...(spec.args as string[]), ...extra];
    if (payload.startupTimeoutSec !== null && targetOf(agent)?.format === "codex-toml") {
      spec.startup_timeout_sec = payload.startupTimeoutSec;
    }
    return spec;
  };

  const previewForm = async (payload: McpFormPayload): Promise<McpFilePreview[]> => {
    return Promise.all(payload.agents.map(async (agent): Promise<McpFilePreview> => {
      const args = [
        "--only", `mcp:${payload.name}`, "--agent", agent,
        "--action", form?.kind === "edit" ? "edit" : "add",
        ...(form?.kind === "edit" ? ["--base", form.cell.base] : []),
        "--spec", JSON.stringify(specFor(payload, agent)),
      ];
      try {
        const envelope = await runEngine<{ files: McpFilePreview["files"] }>("mcp:preview", args);
        return envelope.ok
          ? { agent, files: envelope.data.files }
          : { agent, files: [], error: envelope.errors[0]?.message ?? "预览失败" };
      } catch (reason) {
        return { agent, files: [], error: engineErrorMessage(reason) };
      }
    }));
  };

  const copyArgs = (element: EngineElement, from: string, base: string, to: string, replaceBase: string | null) => [
    "--only", element.id, "--agent", from, "--to", to, "--action", "copy", "--base", base,
    ...(replaceBase ? ["--replace", "--to-base", replaceBase] : []),
  ];

  const submitForm = async (payload: McpFormPayload) => {
    const mode = form;
    setForm(null);
    if (!mode) return;
    if (mode.kind === "edit") {
      void apply(`编辑 ${payload.name} @ ${mode.agent}`, [
        "--only",
        `mcp:${payload.name}`,
        "--agent",
        mode.agent,
        "--action",
        "edit",
        "--base",
        mode.cell.base,
        "--spec",
        JSON.stringify(specFor(payload, mode.agent)),
      ]);
      return;
    }
    // 各助手逐个写入，互不影响：一个失败不拦其他
    for (const agent of payload.agents) {
      await apply(`新增 ${payload.name} @ ${agent}`, [
        "--only",
        `mcp:${payload.name}`,
        "--agent",
        agent,
        "--action",
        "add",
        "--spec",
        JSON.stringify(specFor(payload, agent)),
      ]);
    }
    savedExtraArgs.current = {};
  };

  const openAddFromSaved = (element: EngineElement) => {
    const saved = (element.spec as unknown as McpSpec).saved as McpSavedDefinition;
    savedExtraArgs.current = saved.definition.extra_args_by_agent ?? {};
    setForm({ kind: "add", targets, initialName: nameOf(element), initial: saved.definition });
  };

  const search_ = search.trim();
  const matches = (element: EngineElement) =>
    matchesKeyword(search_, [
      element.id,
      ...Object.values(cellsOf(element)).map((cell) => mcpEntrySummary(cell)),
      ...Object.keys(cellsOf(element)),
    ]);
  const filtered = elements.filter(matches);
  const disabledCount = elements.filter((element) => element.state === "disabled").length;

  const relevantAgents = agentFilter === "all" ? agentColumns : agentColumns.filter((agent) => agent === agentFilter);
  const matrixRows: MatrixRow[] = filtered.map((element) => {
    const cells = cellsOf(element);
    const rowCells: Record<string, MatrixRowCell | null> = {};
    for (const agent of relevantAgents) {
      const cell = cells[agent];
      rowCells[agent] = cell
        ? {
            state: mcpCellKey(cell),
            title: [`${agent}: ${mcpCellKey(cell)}`, mcpEntrySummary(cell), cell.blocked_reason ?? null, cell.notice ?? null]
              .filter(Boolean)
              .join(" · "),
            hasActions: cell.actions.length > 0,
          }
        : null;
    }
    return { key: element.id, label: nameOf(element), cells: rowCells };
  });

  const brokenTargets = targets.filter((target) => target.error !== null);

  return (
    <div className="space-y-6">
      <PageHeader
        title="MCP"
        description="直接查看和编辑各助手配置文件里的 MCP 条目；同名服务在不同助手里可以各不相同。"
      />

      {query.data.errors.length > 0 && <ErrorPanel title="MCP 状态读取失败" errors={query.data.errors} />}
      {brokenTargets.map((target) => (
        <div
          key={target.agent}
          data-testid={`mcp-target-error-${target.agent}`}
          className="rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning"
        >
          <span className="font-mono font-medium">{target.agent}</span> 的 MCP 配置读不了：{target.error}
          <span className="block break-all font-mono text-[11px] opacity-80">{target.path}</span>
        </div>
      ))}

      <div className="flex flex-wrap items-center justify-between gap-2">
        <div role="tablist" aria-label="MCP 视图" className="flex border-b">
          {(["services", "agents"] as const).map((tab) => (
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
              {tab === "services" ? "服务" : "按助手查看"}
            </button>
          ))}
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <span className="text-xs text-muted-foreground" data-testid="mcp-counts">
            共 {elements.length} 个服务{disabledCount > 0 ? `，${disabledCount} 个已停用` : ""}
          </span>
          <SearchInput value={search} onChange={setSearch} placeholder="搜索服务…" testId="mcp-search" />
          <button
            type="button"
            data-testid="add-server-button"
            onClick={() => {
              savedExtraArgs.current = {};
              setForm({ kind: "add", targets });
            }}
            disabled={targets.every((target) => target.error !== null || !target.agent_enabled)}
            className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3.5 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Plus className="h-4 w-4" aria-hidden /> {OVERLAY_COPY.addServer}
          </button>
        </div>
      </div>

      {elements.length === 0 ? (
        <EmptyState
          icon={Server}
          title="各助手的配置里还没有 MCP 服务"
          description={<p>点击右上角「{OVERLAY_COPY.addServer}」写入第一个服务；在助手里直接加的条目也会出现在这里。</p>}
        />
      ) : search_ && filtered.length === 0 ? (
        <EmptyState
          icon={SearchX}
          testId="mcp-search-empty"
          title="无匹配结果"
          description="没有名称、启动命令、地址或助手包含该关键词的服务。"
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
      ) : activeTab === "services" ? (
        <div id="panel-services" role="tabpanel" aria-labelledby="tab-services" className="grid grid-cols-1 gap-4">
          {filtered.map((element) => (
            <McpServerCard
              key={element.id}
              element={element}
              agents={agentColumns}
              busy={busy}
              onAction={handleAction}
              onAddFromSaved={openAddFromSaved}
              onForget={setPendingForget}
              onConfigureSecrets={setEditingSecret}
            />
          ))}
        </div>
      ) : (
        <div id="panel-agents" role="tabpanel" aria-labelledby="tab-agents" className="space-y-3">
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
              {agentColumns.map((agent) => (
                <option key={agent} value={agent}>
                  {agent}
                </option>
              ))}
            </select>
          </label>
          <ResourceAgentMatrix
            rows={matrixRows}
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
            renderExpanded={(row) => {
              const element = filtered.find((item) => item.id === row.key);
              if (!element) return null;
              const cells = cellsOf(element);
              return (
                <ul className="divide-y divide-border/70">
                  {relevantAgents
                    .filter((agent) => cells[agent])
                    .map((agent) => (
                      <McpEntryRow
                        key={agent}
                        element={element}
                        agent={agent}
                        cell={cells[agent]}
                        disabled={busy}
                        onAction={handleAction}
                      />
                    ))}
                </ul>
              );
            }}
            emptyHint={<EmptyState compact icon={LayoutGrid} title="暂无可展示的服务" />}
          />
        </div>
      )}

      {form && <McpServerFormDialog mode={form} onClose={() => setForm(null)} onSubmit={(payload) => void submitForm(payload)} onPreview={previewForm} />}

      {copying && (
        <McpCopyDialog
          name={nameOf(copying.element)}
          from={copying.agent}
          targets={targets}
          cells={cellsOf(copying.element)}
          busy={busy}
          onPreview={async ({ to, replaceBase }) => {
            const args = copyArgs(copying.element, copying.agent, copying.cell.base, to, replaceBase);
            try {
              const envelope = await runEngine<{ files: { path: string; exists: boolean; content: string }[] }>("mcp:preview", args);
              return envelope.ok
                ? { files: envelope.data.files, warnings: envelope.warnings.map((warning) => warning.message) }
                : envelope.errors[0]?.message ?? "预览失败";
            } catch (reason) {
              return engineErrorMessage(reason);
            }
          }}
          onClose={() => setCopying(null)}
          onSubmit={({ to, replaceBase }) => {
            const { element, agent, cell } = copying;
            setCopying(null);
            void apply(`复制 ${nameOf(element)}：${agent} → ${to}`, copyArgs(element, agent, cell.base, to, replaceBase));
          }}
        />
      )}

      {pendingRemove && (
        <ConfirmModal
          title={`从 ${pendingRemove.agent} 删除 ${nameOf(pendingRemove.element)}`}
          description={
            pendingRemove.cell.source === "saved"
              ? "删除本工具里保存的这份停用内容，之后无法再启用。确认后执行。"
              : `从 ${pendingRemove.agent} 的配置文件里删掉这一条${pendingRemove.cell.has_saved ? "，连同本工具保存的停用内容" : ""}，其他条目与配置保持不变。确认后执行。`
          }
          risk="write-local"
          confirmLabel="确认删除"
          executing={busy}
          onConfirm={() => {
            const { element, agent, cell } = pendingRemove;
            setPendingRemove(null);
            void apply(`从 ${agent} 删除 ${nameOf(element)}`, [
              "--only",
              element.id,
              "--agent",
              agent,
              "--action",
              "remove",
              "--base",
              cell.base,
            ]);
          }}
          onClose={() => setPendingRemove(null)}
        />
      )}

      {pendingForget && (
        <ConfirmModal
          title={`删除已保存的定义 ${nameOf(pendingForget)}`}
          description="只删除本工具里保存、还没写进任何助手的这份定义，各助手的配置文件不受影响。"
          risk="write-local"
          confirmLabel="确认删除"
          executing={busy}
          onConfirm={() => {
            const element = pendingForget;
            setPendingForget(null);
            void apply(`删除已保存的定义 ${nameOf(element)}`, ["--only", element.id, "--action", "forget"]);
          }}
          onClose={() => setPendingForget(null)}
        />
      )}

      {editingSecret && (
        <SecretValueDialog
          variable={editingSecret}
          stored={false}
          busy={busy}
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
    </div>
  );
}
