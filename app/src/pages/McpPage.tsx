import { useEffect, useRef, useState } from "react";
import { Plus, SearchX, Server, Wand2 } from "lucide-react";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { mcpEntrySummary } from "../components/McpAssignmentRow";
import { McpCopyDialog } from "../components/McpCopyDialog";
import { McpServerCard } from "../components/McpServerCard";
import { McpServerFormDialog, type McpFilePreview, type McpFormMode, type McpFormPayload } from "../components/McpServerFormDialog";
import { PageError, PageLoading } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { SecretValueDialog } from "../components/SecretValueDialog";
import type { EngineElement, Envelope, McpTarget, StatusData } from "../lib/contract";
import type { McpAgentCell, McpObserved, McpSavedDefinition, McpSpec } from "../lib/observed";
import { OVERLAY_COPY } from "../lib/ui-copy";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { useEngineQuery } from "../lib/queries";
import { useDirectApply } from "../lib/useDirectApply";
import { useExecutions } from "../lib/execution";
import { mcpFillArgs, mcpFillRows } from "../lib/mcpFill";
import { useAccountNames } from "../lib/useAccountNames";

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
  const { execute } = useExecutions();
  const nameOfAgent = useAccountNames();
  const [search, setSearch] = useState("");
  const [agentFilter, setAgentFilter] = useState<string>("all");
  const [filling, setFilling] = useState(false);
  const [form, setForm] = useState<McpFormMode | null>(null);
  /** 从已保存定义添加时，按助手追加的参数（旧版 extra_args_by_agent）。 */
  const savedExtraArgs = useRef<Record<string, string[]>>({});
  const [copying, setCopying] = useState<{ element: EngineElement; agent: string; cell: McpAgentCell; to?: string } | null>(null);
  const [pendingRemove, setPendingRemove] = useState<{ element: EngineElement; agent: string; cell: McpAgentCell } | null>(null);
  const [removePreview, setRemovePreview] = useState<{
    key: string;
    files: { path: string; exists: boolean; content: string }[];
    warnings: string[];
    error?: string;
  } | null>(null);
  const [pendingForget, setPendingForget] = useState<EngineElement | null>(null);
  const [editingSecret, setEditingSecret] = useState<string | null>(null);
  const removeKey = pendingRemove ? JSON.stringify([pendingRemove.element.id, pendingRemove.agent, pendingRemove.cell.base]) : "";

  useEffect(() => {
    if (!pendingRemove) return;
    let current = true;
    const args = ["--only", pendingRemove.element.id, "--agent", pendingRemove.agent,
      "--action", "remove", "--base", pendingRemove.cell.base];
    void runEngine<{ files: { path: string; exists: boolean; content: string }[] }>("mcp:preview", args)
      .then((envelope) => {
        if (!current) return;
        setRemovePreview(envelope.ok
          ? { key: removeKey, files: envelope.data.files, warnings: envelope.warnings.map((warning) => warning.message) }
          : { key: removeKey, files: [], warnings: [], error: envelope.errors[0]?.message ?? "预览失败" });
      })
      .catch((reason) => {
        if (current) setRemovePreview({ key: removeKey, files: [], warnings: [], error: engineErrorMessage(reason) });
      });
    return () => { current = false; };
  }, [removeKey]);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="MCP 状态读取失败" error={query.error} />;
  if (!query.data.ok) {
    return (
      <div className="space-y-3">
        <h1 className="sr-only">MCP</h1>
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
      setRemovePreview(null);
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

  const writableAgents = targets.filter((target) => target.error === null && target.agent_enabled).map((target) => target.agent);
  const shownAgents = agentFilter === "all" ? writableAgents : writableAgents.filter((agent) => agent === agentFilter);
  const shown = agentFilter === "all" ? filtered : filtered.filter((element) => cellsOf(element)[agentFilter]);
  const pendingFill = mcpFillRows(elements, targets, agentFilter === "all" ? undefined : [agentFilter]);
  /** 补齐：逐条复制，单条失败不影响其余；每条仍由引擎带 base 锁内核对。 */
  const runFill = async () => {
    if (filling) return;
    setFilling(true);
    let ok = 0;
    let failed = 0;
    try {
      for (const row of pendingFill) {
        const envelope = await execute({ command: "mcp:apply", label: `补齐 ${row.name} @ ${nameOfAgent(row.to)}`, args: mcpFillArgs(row), silent: true });
        if (envelope?.ok) ok += 1;
        else failed += 1;
      }
    } finally {
      setFilling(false);
    }
    (failed ? toast.error : toast.success)(`补齐完成：成功 ${ok}，失败 ${failed}${failed ? "，详见任务记录" : ""}`);
  };

  const brokenTargets = targets.filter((target) => target.error !== null);

  return (
    <div className="space-y-3">
      <h1 className="sr-only">MCP</h1>
      {query.data.errors.length > 0 && <ErrorPanel title="MCP 状态读取失败" errors={query.data.errors} />}
      {brokenTargets.map((target) => (
        <div
          key={target.agent}
          data-testid={`mcp-target-error-${target.agent}`}
          className="rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning"
        >
          <span className="font-medium">{nameOfAgent(target.agent)}</span> 的 MCP 配置读不了：{target.error}
          <span className="block break-all font-mono text-[11px] opacity-80">{target.path}</span>
        </div>
      ))}

      <div className="flex flex-wrap items-center gap-2">
        <SearchInput value={search} onChange={setSearch} placeholder="搜索服务…" testId="mcp-search" />
        <select
          aria-label="账号筛选"
          data-testid="filter-agent-select"
          value={agentFilter}
          onChange={(e) => setAgentFilter(e.target.value)}
          className="rounded-md border bg-card px-2 py-1.5 text-sm"
        >
          <option value="all">全部账号</option>
          {agentColumns.map((agent) => (
            <option key={agent} value={agent}>{nameOfAgent(agent)}</option>
          ))}
        </select>
        <span className="text-xs text-muted-foreground" data-testid="mcp-counts">
          共 {elements.length} 个服务{disabledCount > 0 ? `，${disabledCount} 个已停用` : ""}
        </span>
        <div className="ml-auto flex items-center gap-2">
          {pendingFill.length > 0 && (
            <button
              type="button"
              data-testid="mcp-fill"
              disabled={filling || busy}
              title="把在用的服务从同类账号复制到还没有它的账号；账号上停用的不动"
              onClick={() => void runFill()}
              className="inline-flex items-center gap-1 rounded-md border border-primary/30 bg-primary/10 px-3 py-1.5 text-sm text-primary hover:bg-primary/15 disabled:opacity-40"
            >
              <Wand2 className="h-4 w-4" aria-hidden /> 补齐 {pendingFill.length}
            </button>
          )}
          <button
            type="button"
            data-testid="add-server-button"
            onClick={() => {
              savedExtraArgs.current = {};
              setForm({ kind: "add", targets });
            }}
            disabled={targets.every((target) => target.error !== null || !target.agent_enabled)}
            className="inline-flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Plus className="h-4 w-4" aria-hidden /> {OVERLAY_COPY.addServer}
          </button>
        </div>
      </div>

      {elements.length === 0 ? (
        <EmptyState
          icon={Server}
          title="各账号的配置里还没有 MCP 服务"
          description={<p>点击右上角「{OVERLAY_COPY.addServer}」写入第一个服务；在助手里直接加的条目也会出现在这里。</p>}
        />
      ) : (search_ || agentFilter !== "all") && shown.length === 0 ? (
        <EmptyState
          icon={SearchX}
          testId="mcp-search-empty"
          title="无匹配结果"
          description="没有名称、启动命令、地址或账号包含该关键词的服务。"
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
        <ul className="divide-y rounded-lg border bg-card">
          {shown.map((element) => (
            <McpServerCard
              key={element.id}
              element={element}
              agents={shownAgents}
              busy={busy}
              exclusive={agentFilter !== "all"}
              nameOf={nameOfAgent}
              onAddHere={(target, from, to) => {
                const cell = cellsOf(target)[from];
                if (cell) setCopying({ element: target, agent: from, cell, to });
              }}
              onAction={handleAction}
              onAddFromSaved={openAddFromSaved}
              onForget={setPendingForget}
              onConfigureSecrets={setEditingSecret}
            />
          ))}
        </ul>
      )}

      {form && <McpServerFormDialog mode={form} onClose={() => setForm(null)} onSubmit={(payload) => void submitForm(payload)} onPreview={previewForm} />}

      {copying && (
        <McpCopyDialog
          name={nameOf(copying.element)}
          from={copying.agent}
          initialTo={copying.to}
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
          riskLabel={pendingRemove.cell.source === "saved" ? "删除已保存内容" : undefined}
          confirmLabel="确认删除"
          confirmDisabled={removePreview?.key !== removeKey || Boolean(removePreview.error)}
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
        >
          <section className="mt-3 space-y-2 rounded border p-3 text-xs" data-testid="mcp-remove-preview">
            <p className="font-medium">删除后的配置文件</p>
            {removePreview?.key !== removeKey && <p className="text-muted-foreground">正在读取实际文件并计算…</p>}
            {removePreview?.key === removeKey && removePreview.error && <p role="alert" className="text-destructive">{removePreview.error}</p>}
            {removePreview?.key === removeKey && !removePreview.error && <>
              {removePreview.files.length === 0 && <p className="text-muted-foreground">只删除已保存的停用内容，助手配置文件不变。</p>}
              {removePreview.files.map((file) => <div key={file.path} className="space-y-1">
                <p className="break-all font-mono">{file.path}{file.exists ? "（已有文件）" : "（将新建）"}</p>
                <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono" data-testid="mcp-remove-preview-content">{file.content}</pre>
              </div>)}
              {removePreview.warnings.map((warning) => <p key={warning} className="text-warning">{warning}</p>)}
            </>}
            <p className="text-muted-foreground">只读预览；文件原文可能含敏感值，仅在本机查看。</p>
          </section>
        </ConfirmModal>
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
