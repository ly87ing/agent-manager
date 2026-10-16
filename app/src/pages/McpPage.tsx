import { t } from "../lib/i18n";
import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import { Info, Plus, SearchX, Server } from "lucide-react";
import { notifyError, toast } from "../lib/notifications";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { mcpEntrySummary } from "../components/McpAssignmentRow";
import { AccountHeader } from "../components/AccountGrid";
import { McpRemoveDialog, type McpRemovePreview } from "../components/McpRemoveDialog";
import { McpCheckDialog, type McpCheckResult } from "../components/McpCheckDialog";
import { McpMarkerCleanupDialog } from "../components/McpMarkerCleanupDialog";
import { McpServerCard } from "../components/McpServerCard";
import { McpServerFormDialog, type McpFilePreview, type McpFormMode, type McpFormPayload } from "../components/McpServerFormDialog";
import { PageError, PageLoading, ReadAgain } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { SecretValueDialog } from "../components/SecretValueDialog";
import type { EngineElement, Envelope, McpTarget, StatusData } from "../lib/contract";
import type { McpAgentCell, McpObserved, McpSavedDefinition, McpSpec } from "../lib/observed";
import { OVERLAY_COPY } from "../lib/ui-copy";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { useEngineQuery } from "../lib/queries";
import { useDirectApply } from "../lib/useDirectApply";
import { useExecutions } from "../lib/execution";
import { effectHint } from "../lib/effectHints";
import { mcpFillArgs, mcpFillRows } from "../lib/mcpFill";
import { mcpItems } from "../lib/inbox";
import { AttentionOnlyToggle } from "../components/StateBadge";
import { dependenciesOf } from "../components/DependencyChips";
import type { ProgramsData } from "../lib/programs";
import { useAccountGroups, useAccountNames } from "../lib/useAccountNames";
import { encodeShareLink, type SharePayload } from "../lib/shareLink";
import { Select } from "../components/Select";

function cellsOf(element: EngineElement): Record<string, McpAgentCell> {
  return (element.observed as McpObserved).agents ?? {};
}

function nameOf(element: EngineElement): string {
  return element.id.replace(/^mcp:/, "");
}

/**
 * MCP：各助手配置文件里的实际条目就是当前配置。
 *
 * 同一个服务名在不同助手里可以内容不同，逐条编辑、启停、复制、删除；每次写入都带上
 * 打开时该条目的摘要，外部改过就拒绝并刷新。停用：Codex/OpenCode 写官方 enabled=false，
 * Claude Code 把条目移出配置、保存在本工具里（启用时写回）。
 */
export default function McpPage({
  importPayload,
  onImportConsumed,
  initialSearch,
  onOpenProgram,
}: {
  /** 从待处理进来时直接搜到那个服务。 */
  initialSearch?: string | null;
  /** 依赖标签点过去：程序页的那一行（程序标识）。 */
  onOpenProgram?: (program: string) => void;
  /** 从分享链接导入的服务：打开预填好的「新增」表单，用户检查后保存。 */
  importPayload?: Extract<SharePayload, { kind: "mcp" }> | null;
  onImportConsumed?: () => void;
} = {}) {
  const query = useEngineQuery<StatusData>("mcp:status");
  const directApply = useDirectApply();
  const { execute } = useExecutions();
  const nameOfAgent = useAccountNames();
  const groupsOf = useAccountGroups();
  const [search, setSearch] = useState(initialSearch ?? "");
  // Derive attention from this page's status without querying other domains.
  const attentionSubjects = new Set(mcpItems(
    query.data?.ok ? query.data.data.elements ?? [] : [],
    query.data?.ok ? query.data.data.targets ?? [] : [],
    nameOfAgent,
  ).map((item) => item.subject));
  const [attentionOnly, setAttentionOnly] = useState(false);
  const programsQuery = useEngineQuery<ProgramsData>("programs:status");
  const programs = programsQuery.data?.ok ? programsQuery.data.data.elements : [];
  const [agentFilter, setAgentFilter] = useState<string>("all");
  const [filling, setFilling] = useState(false);
  const [form, setForm] = useState<McpFormMode | null>(null);
  /** 从已保存定义添加时，按助手追加的参数（旧版 extra_args_by_agent）。 */
  const savedExtraArgs = useRef<Record<string, string[]>>({});
  const [removing, setRemoving] = useState<EngineElement | null>(null);
  const [pendingForget, setPendingForget] = useState<EngineElement | null>(null);
  const [editingSecret, setEditingSecret] = useState<string | null>(null);
  const [checking, setChecking] = useState<{ element: EngineElement; agent?: string } | null>(null);
  const [checks, setChecks] = useState<Record<string, Record<string, McpCheckResult>>>({});
  const [cleaning, setCleaning] = useState<{ element: EngineElement; agent: string; cell: McpAgentCell } | null>(null);

  useEffect(() => {
    if (!importPayload || !query.data?.ok) return;
    savedExtraArgs.current = {};
    setForm({ kind: "add", targets: query.data.data.targets ?? [], initialName: importPayload.name, initial: importPayload.spec });
    onImportConsumed?.();
  }, [importPayload, query.data]);
  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title={t("MCP 状态读取失败")} error={query.error} onRetry={() => void query.refetch()} retrying={query.isFetching} />;
  if (!query.data.ok) {
    return (
      <div className="space-y-3">
        <h1 className="text-lg font-semibold tracking-tight">MCP</h1>
        <PageError title={t("MCP 状态读取失败")} errors={query.data.errors} onRetry={() => void query.refetch()} retrying={query.isFetching} />
      </div>
    );
  }

  const elements = query.data.data.elements;
  const allTargets: McpTarget[] = query.data.data.targets ?? [];
  /** 配置格式本工具还不支持的助手（如 Kimi Code）：不占列，只在表头说明里提一句。 */
  const unsupported = allTargets.filter((target) => target.error?.startsWith("不支持的 MCP 配置格式"));
  const targets = allTargets.filter((target) => !unsupported.includes(target));
  const agentColumns = targets.map((target) => target.agent);
  /** 助手名单里没有的助手（如 kimi-code）写成 Kimi Code。 */
  const readableName = (agent: string) =>
    nameOfAgent(agent) !== agent ? nameOfAgent(agent) : agent.split("-").map((word) => word.charAt(0).toUpperCase() + word.slice(1)).join(" ");
  const targetOf = (agent: string) => targets.find((target) => target.agent === agent);
  const busy = directApply.executing;

  const apply = (label: string, args: string[], silent = false) =>
    directApply.apply({ command: "mcp:apply", label, args, silent });

  /**
   * 撤销：写入后条目的基准变了，先重读状态拿这个助手条目的新基准再做反向操作；
   * 期间被外部改过就由引擎照常拒绝。
   */
  const undoWith = (element: EngineElement, agent: string, label: string, action: string) => () =>
    void runEngine<StatusData>("mcp:status", []).then((envelope) => {
      const cell = envelope.ok ? cellsOf(envelope.data.elements.find((item) => item.id === element.id) ?? element)[agent] : undefined;
      if (!cell) {
        toast.error(t("{0}：这个助手上已经找不到这一条", label));
        return;
      }
      void apply(label, ["--only", element.id, "--agent", agent, "--action", action, "--base", cell.base]);
    });

  /** 助手开关：启用 / 停用这一个助手上的条目，可撤销。 */
  const handleAction = (element: EngineElement, agent: string, action: string, cell: McpAgentCell) => {
    const verb = action === "enable" ? t("启用") : t("停用");
    const label = `${verb} ${nameOf(element)} @ ${nameOfAgent(agent)}`;
    void directApply
      .apply({
        command: "mcp:apply",
        label,
        args: ["--only", element.id, "--agent", agent, "--action", action, "--base", cell.base],
        undo: undoWith(element, agent, t("撤销：{0}", label), action === "enable" ? "disable" : "enable"),
      });
  };

  /**
   * 复制分享链接：取一份能读出内容的条目；明文取值（界面上是 <literal>）不放进链接，
   * 换成同名的 ${变量} 引用，对方要在自己机器上配这个变量。
   */
  const shareServer = async (element: EngineElement) => {
    const cell = Object.values(cellsOf(element)).find((item) => item.source === "file" && (item.entry?.command || item.entry?.url));
    if (!cell?.entry) return;
    const replaced: string[] = [];
    const asVars = (map: Record<string, string> | undefined) =>
      Object.fromEntries(
        Object.entries(map ?? {}).map(([key, value]) => {
          if (/^\$\{[A-Za-z_][A-Za-z0-9_]*\}$/.test(value)) return [key, value];
          replaced.push(key);
          return [key, `\${${key.replace(/[^A-Za-z0-9_]/g, "_").toUpperCase()}}`];
        }),
      );
    const entry = cell.entry;
    const spec = entry.url ? { url: entry.url, headers: asVars(entry.headers) } : { command: entry.command, args: entry.args ?? [], env: asVars(entry.env) };
    try {
      await navigator.clipboard.writeText(encodeShareLink({ kind: "mcp", name: nameOf(element), spec }));
      toast.success(t("已复制 {0} 的分享链接", nameOf(element)), {
        description: replaced.length ? t("这些取值没放进链接，改成了同名变量：{0}；对方要自己配。", replaced.join("、")) : undefined,
      });
    } catch (reason) {
      notifyError(t("复制失败"), reason);
    }
  };

  /** 编辑：一张表单写到所有有这个服务的助手；各助手内容不同时先以 base 那份预填。 */
  const openEdit = (element: EngineElement, base?: string) => {
    const editable = Object.entries(cellsOf(element)).filter(([, cell]) => cell.actions.includes("edit"));
    if (editable.length === 0) return;
    const [agent, cell] = editable.find(([key]) => key === base) ?? editable[0];
    setForm({
      kind: "edit",
      name: nameOf(element),
      agent,
      cell,
      format: targetOf(agent)?.format ?? "",
      targets: editable.map(([key, item]) => ({ agent: key, cell: item, format: targetOf(key)?.format ?? "" })),
      differs: new Set(editable.map(([, item]) => mcpEntrySummary(item))).size > 1,
    });
  };

  const removePreview = async (element: EngineElement, agent: string, cell: McpAgentCell): Promise<McpRemovePreview> => {
    const args = ["--only", element.id, "--agent", agent, "--action", "remove", "--base", cell.base];
    try {
      const envelope = await runEngine<{ files: { path: string; exists: boolean; content: string; original?: string | null }[] }>("mcp:preview", args);
      return envelope.ok
        ? { files: envelope.data.files, warnings: envelope.warnings.map((warning) => warning.message) }
        : envelope.errors[0]?.message ?? t("预览失败");
    } catch (reason) {
      return engineErrorMessage(reason);
    }
  };

  /** 删除：逐个助手执行，各自带写入基准；一个失败不拦其他。 */
  const runRemove = async (element: EngineElement, agents: string[]) => {
    setRemoving(null);
    for (const agent of agents) {
      const cell = cellsOf(element)[agent];
      if (!cell) continue;
      await apply(t("从 {0} 删除 {1}", nameOfAgent(agent), nameOf(element)), ["--only", element.id, "--agent", agent, "--action", "remove", "--base", cell.base]);
    }
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

  /** 编辑时每个助手用它自己条目的写入基准。 */
  const baseOf = (agent: string) => (form?.kind === "edit" ? form.targets.find((target) => target.agent === agent)?.cell.base : undefined);
  const previewForm = async (payload: McpFormPayload): Promise<McpFilePreview[]> => {
    return Promise.all(payload.agents.map(async (agent): Promise<McpFilePreview> => {
      const base = baseOf(agent);
      const args = [
        "--only", `mcp:${payload.name}`, "--agent", agent,
        "--action", form?.kind === "edit" ? "edit" : "add",
        ...(base ? ["--base", base] : []),
        "--spec", JSON.stringify(specFor(payload, agent)),
      ];
      try {
        const envelope = await runEngine<{ files: McpFilePreview["files"] }>("mcp:preview", args);
        return envelope.ok
          ? { agent, files: envelope.data.files }
          : { agent, files: [], error: envelope.errors[0]?.message ?? t("预览失败") };
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
    const results: (Envelope | null)[] = [];
    if (mode.kind === "edit") {
      // 各助手逐个写入，各自核对是否被外部改过：一个失败不拦其他
      for (const target of mode.targets.filter((target) => payload.agents.includes(target.agent))) {
        results.push(await apply(t("编辑 {0} @ {1}", payload.name, nameOfAgent(target.agent)), [
          "--only",
          `mcp:${payload.name}`,
          "--agent",
          target.agent,
          "--action",
          "edit",
          "--base",
          target.cell.base,
          "--spec",
          JSON.stringify(specFor(payload, target.agent)),
        ], true));
      }
    } else {
      // 各助手逐个写入，互不影响：一个失败不拦其他
      for (const agent of payload.agents) {
        results.push(await apply(t("新增 {0} @ {1}", payload.name, nameOfAgent(agent)), [
          "--only",
          `mcp:${payload.name}`,
          "--agent",
          agent,
          "--action",
          "add",
          "--spec",
          JSON.stringify(specFor(payload, agent)),
        ], true));
      }
      savedExtraArgs.current = {};
    }
    const succeeded = results.filter((result) => result?.ok).length;
    const failed = results.length - succeeded;
    const details = results.flatMap((result) => result ? [...result.errors, ...result.warnings].map((item) => item.message) : [t("详见任务记录")]);
    toast[failed === results.length ? "error" : failed || details.length ? "warning" : "success"](t("MCP 配置：成功 {0}，失败 {1}", succeeded, failed), {
      description: [payload.name, succeeded ? effectHint("mcp:apply") : null, ...details].filter(Boolean).join("\n"),
    });
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
  const groups = groupsOf(agentFilter === "all" ? agentColumns : agentColumns.filter((agent) => agent === agentFilter));
  const enabledOn = (agent: string) => elements.filter((element) => cellsOf(element)[agent]?.state === "enabled").length;
  const shown = (agentFilter === "all" ? filtered : filtered.filter((element) => cellsOf(element)[agentFilter])).filter(
    (element) => !attentionOnly || attentionSubjects.has(element.id),
  );
  /** 补齐一个助手：把在用的服务从同类助手逐条复制过来，单条失败不影响其余；每条仍由引擎带 base 锁内核对。 */
  /** 不给助手时补齐所有助手。 */
  const runFill = async (agent?: string) => {
    const pendingFill = mcpFillRows(elements, targets, agent ? [agent] : undefined);
    if (filling || pendingFill.length === 0) return;
    setFilling(true);
    let ok = 0;
    let failed = 0;
    try {
      for (const row of pendingFill) {
        const envelope = await execute({ command: "mcp:apply", label: t("补齐 {0} @ {1}", row.name, nameOfAgent(row.to)), args: mcpFillArgs(row), silent: true });
        if (envelope?.ok) ok += 1;
        else failed += 1;
      }
    } finally {
      setFilling(false);
    }
    toast[failed ? ok ? "warning" : "error" : "success"](t("补齐完成：成功 {0}，失败 {1}{2}", ok, failed, failed ? t("，详见任务记录") : ""));
  };

  const brokenTargets = targets.filter((target) => target.error !== null);
  const fillAllCount = agentFilter === "all" ? mcpFillRows(elements, targets).length : 0;

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h1 className="text-lg font-semibold tracking-tight">MCP</h1>
        <ReadAgain queries={[query, programsQuery]} disabled={busy || filling} />
      </div>
      <p className="text-xs text-muted-foreground">{t("开关表示配置是否启用；连接结果来自手动检查，不代表助手已经加载。")}</p>
      {query.data.errors.length > 0 && <ErrorPanel title={t("MCP 状态读取失败")} errors={query.data.errors} />}
      {brokenTargets.map((target) => (
        <div
          key={target.agent}
          data-testid={`mcp-target-error-${target.agent}`}
          className="rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning"
        >
          <span className="font-medium">{nameOfAgent(target.agent)}</span>{(" " + t("的 MCP 配置读不了："))}{target.error}
          <span className="block break-all font-mono text-[11px] opacity-80">{target.path}</span>
          <div className="mt-2 flex flex-wrap gap-2">
            <button type="button" onClick={() => void navigator.clipboard.writeText(target.path).then(() => toast.success(t("已复制到剪贴板"))).catch(() => toast.error(t("复制失败，请手动选中文本复制")))} className="rounded border border-warning/40 px-2 py-1 hover:bg-warning/10">{t("复制配置路径")}</button>
            <button type="button" disabled={query.isFetching} onClick={() => void query.refetch()} className="rounded border border-warning/40 px-2 py-1 hover:bg-warning/10 disabled:opacity-40">{query.isFetching ? t("正在读取数据…") : t("重新读取")}</button>
          </div>
        </div>
      ))}

      <div className="flex flex-wrap items-center gap-2">
        <SearchInput value={search} onChange={setSearch} placeholder={t("搜索服务…")} testId="mcp-search" />
        <Select
          label={t("助手筛选")}
          testId="filter-agent-select"
          value={agentFilter}
          onChange={setAgentFilter}
          className="rounded-md border bg-card px-2 py-1.5 text-sm"
          options={[{ value: "all", label: t("全部助手") }, ...agentColumns.map((agent) => ({ value: agent, label: nameOfAgent(agent) }))]}
        />
        <AttentionOnlyToggle checked={attentionOnly} count={attentionSubjects.size} onChange={setAttentionOnly} />
        <div className="ml-auto flex items-center gap-2">
          {fillAllCount > 0 && (
            <button
              type="button"
              data-testid="mcp-fill-all"
              disabled={filling || busy}
              title={t("把在用的服务从同类助手复制到所有还没有它的助手；助手上停用的不动")}
              onClick={() => void runFill()}
              className="inline-flex items-center gap-1 rounded-md border border-primary/30 bg-primary/10 px-3 py-1.5 text-sm text-primary hover:bg-primary/15 disabled:opacity-40"
            >{(t("全部补齐") + " ")}{fillAllCount}
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
          title={t("各助手的配置里还没有 MCP 服务")}
          description={<p>{t("点击右上角「")}{OVERLAY_COPY.addServer}{t("」写入第一个服务；在助手里直接加的条目也会出现在这里。")}</p>}
        />
      ) : (search_ || agentFilter !== "all" || attentionOnly) && shown.length === 0 ? (
        <EmptyState
          icon={SearchX}
          testId="mcp-search-empty"
          title={t("无匹配结果")}
          description={t("没有名称、启动命令、地址或助手包含该关键词的服务。")}
          actions={
            <button
              type="button"
              onClick={() => setSearch("")}
              className="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent"
            >{t("清除搜索关键词")}</button>
          }
        />
      ) : (
        <div role="region" aria-label={t("MCP 助手列表")} tabIndex={0} className="table-scroll rounded-lg border bg-card" style={{ "--table-min-width": `${Math.max(704, 400 + groups.reduce((count, group) => count + group.columns.length * 64 + 16, 0))}px` } as CSSProperties}>
          <AccountHeader
            className="sticky top-0 z-10"
            groups={groups}
            left={
              <span className="flex items-center gap-2">
                <span className="font-medium text-foreground" data-testid="mcp-counts">{(t("MCP 服务 ·") + " ")}{shown.length}
                  {disabledCount > 0 && <span className="font-normal text-muted-foreground">（{disabledCount}{(" " + t("个已停用）"))}</span>}
                </span>
                {unsupported.length > 0 && (
                  <span data-testid="mcp-unsupported" title={t("这些助手的 MCP 配置格式本工具还不支持，没有列出；它们自己的 MCP 不受影响。")}>
                    <Info className="inline h-3.5 w-3.5 align-[-2px]" aria-hidden /> {unsupported.map((target) => readableName(target.agent)).join("、")}{(" " + t("暂不支持"))}</span>
                )}
              </span>
            }
            below={(column) => {
              const missing = writableAgents.includes(column.id) ? mcpFillRows(elements, targets, [column.id]).length : 0;
              return (
                <>
                  <span className="tabular-nums" data-testid={`mcp-count-${column.id}`}>{enabledOn(column.id)}/{elements.length}</span>
                  {missing > 0 && (
                    <button
                      type="button"
                      data-testid={`mcp-fill-${column.id}`}
                      disabled={filling || busy}
                      title={t("把在用的 {0} 个服务复制到 {1}；助手上停用的不动", missing, column.fullName)}
                      onClick={() => void runFill(column.id)}
                      className="text-primary hover:underline disabled:opacity-40"
                    >{(t("补齐") + " ")}{missing}
                    </button>
                  )}
                </>
              );
            }}
          />
          <ul className="divide-y">
            {shown.map((element) => (
              <McpServerCard
                key={element.id}
                element={element}
                dependencies={dependenciesOf(programs, "mcp", nameOf(element))}
                onOpenProgram={onOpenProgram}
                groups={groups}
                writable={writableAgents}
                busy={busy}
                formatOf={(agent) => targetOf(agent)?.format}
                onAddHere={(target, from, to) => {
                  const cell = cellsOf(target)[from];
                  if (!cell) return;
                  const label = t("把 {0} 复制到 {1}", nameOf(target), nameOfAgent(to));
                  void directApply
                    .apply({
                      command: "mcp:apply",
                      label,
                      args: copyArgs(target, from, cell.base, to, null),
                      // 撤销复制 = 从这个助手删掉刚复制过来的那一条
                      undo: undoWith(target, to, t("撤销：{0}", label), "remove"),
                    });
                }}
                onAction={handleAction}
                onEdit={(target) => openEdit(target)}
                onShare={(target) => void shareServer(target)}
                onRemove={setRemoving}
                onAddFromSaved={openAddFromSaved}
                onForget={setPendingForget}
                onConfigureSecrets={setEditingSecret}
                nameOf={nameOfAgent}
                checks={checks[element.id]}
                onCheck={(element, agent) => setChecking({ element, agent })}
                onCleanMarkers={(element, agent, cell) => setCleaning({ element, agent, cell })}
              />
            ))}
          </ul>
        </div>
      )}

      {checking && <McpCheckDialog
        element={checking.element}
        initialAgent={checking.agent}
        initialResult={checking.agent && checks[checking.element.id]?.[checking.agent]?.base === cellsOf(checking.element)[checking.agent]?.base ? checks[checking.element.id][checking.agent] : undefined}
        nameOf={nameOfAgent}
        onResult={(agent, result) => {
          setChecks((current) => {
            const results = { ...current[checking.element.id] };
            if (result) results[agent] = result;
            else delete results[agent];
            return { ...current, [checking.element.id]: results };
          });
          if (!result) void query.refetch();
        }}
        onEdit={(agent) => { openEdit(checking.element, agent); setChecking(null); }}
        onClose={() => setChecking(null)}
      />}

      {cleaning && <McpMarkerCleanupDialog id={cleaning.element.id} agent={cleaning.agent} base={cleaning.cell.base} nameOf={nameOfAgent} busy={busy} onClose={() => setCleaning(null)} onConfirm={(args) => {
        void apply(t("清理旧标记 {0} @ {1}", nameOf(cleaning.element), nameOfAgent(cleaning.agent)), args).then((envelope) => { if (envelope?.ok) setCleaning(null); });
      }} />}

      {form && (
        <McpServerFormDialog
          key={form.kind === "edit" ? `edit:${form.agent}` : "add"}
          mode={form}
          nameOf={nameOfAgent}
          onSwitchBase={(agent) => {
            const element = elements.find((item) => item.id === `mcp:${form.kind === "edit" ? form.name : ""}`);
            if (element) openEdit(element, agent);
          }}
          onClose={() => setForm(null)}
          onSubmit={(payload) => void submitForm(payload)}
          onPreview={previewForm}
        />
      )}

      {removing && (
        <McpRemoveDialog
          name={nameOf(removing)}
          entries={Object.entries(cellsOf(removing))
            .filter(([, cell]) => cell.actions.includes("remove"))
            .map(([agent, cell]) => ({ agent, cell }))}
          busy={busy}
          nameOf={nameOfAgent}
          onPreview={(agent, cell) => removePreview(removing, agent, cell)}
          onConfirm={(agents) => void runRemove(removing, agents)}
          onClose={() => setRemoving(null)}
        />
      )}

      {pendingForget && (
        <ConfirmModal
          title={t("删除存着的配置 {0}", nameOf(pendingForget))}
          description={t("只删除本工具里保存、还没写进任何助手的这份定义，各助手的配置文件不受影响。")}
          risk="write-local"
          confirmLabel={t("确认删除")}
          executing={busy}
          onConfirm={() => {
            const element = pendingForget;
            setPendingForget(null);
            void apply(t("删除存着的配置 {0}", nameOf(element)), ["--only", element.id, "--action", "forget"]);
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
                label: t("写入外部密钥 {0}", editingSecret),
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
