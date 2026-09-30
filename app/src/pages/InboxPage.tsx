import { useState } from "react";
import { CheckCircle2 } from "lucide-react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import { EmptyState } from "../components/EmptyState";
import { PageLoading } from "../components/PageState";
import { STATE_TONE } from "../components/StateBadge";
import { useExecutions } from "../lib/execution";
import { INBOX_STATES, pageLabel, useInbox, type InboxFix, type InboxItem } from "../lib/inbox";
import { mcpFillArgs } from "../lib/mcpFill";
import { engineKeys } from "../lib/queries";
import { rulesFillArgs } from "../lib/rulesFill";
import { useAccountNames } from "../lib/useAccountNames";
import { useBatchApply } from "../lib/useBatchApply";
import { cn } from "../lib/utils";

/**
 * 待处理：全应用里不是「生效」的项按统一状态分组列出，每条一个处理按钮——和资源页上是同一个动作。
 * 能直接做的（补齐、修复、换到新版本）在这里做，要看差异或做选择的跳到负责的页面并定位到那一项。
 * 做完回读，项目自然消失；没有了就是全部生效。
 */
export default function InboxPage({ onOpen }: { onOpen: (target: string) => void }) {
  const nameOf = useAccountNames();
  const { items, loading } = useInbox(nameOf);
  const batchApply = useBatchApply();
  const { execute } = useExecutions();
  const queryClient = useQueryClient();
  const [running, setRunning] = useState<string | null>(null);

  if (loading && items.length === 0) return <PageLoading />;

  /** 执行一个直接能做的动作；刷新交给调用方。 */
  const perform = async (fix: Exclude<InboxFix, { kind: "open" }>) => {
    if (fix.kind === "skills") {
      await batchApply.applyBatch({ actionLabel: fix.label, rows: fix.rows });
    } else if (fix.kind === "mcp-fill") {
      let failed = 0;
      for (const row of fix.rows) {
        const envelope = await execute({ command: "mcp:apply", label: `补齐 ${row.name} @ ${nameOf(row.to)}`, args: mcpFillArgs(row), silent: true });
        if (!envelope?.ok) failed += 1;
      }
      (failed ? toast.error : toast.success)(`补齐 ${fix.rows.length - failed} 处${failed ? `，${failed} 处失败（详见操作记录）` : ""}`);
    } else {
      let failed = 0;
      for (const row of fix.rows) {
        const envelope = await execute({ command: "rules:apply", label: `补齐 ${row.name}`, args: rulesFillArgs(row), silent: true });
        if (!envelope?.ok) failed += 1;
      }
      (failed ? toast.error : toast.success)(`补齐 ${fix.rows.length - failed} 个规则文件${failed ? `，${failed} 个失败（详见操作记录）` : ""}`);
    }
  };

  const run = async (item: InboxItem, fix: InboxFix) => {
    if (fix.kind === "open") return onOpen(fix.target);
    setRunning(item.id);
    try {
      await perform(fix);
    } finally {
      setRunning(null);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

  /** 「缺失」一组能一次补齐：按领域合并同类动作，逐条照常核对。 */
  const fillAll = async (group: InboxItem[]) => {
    setRunning("all");
    try {
      const skills = group.flatMap((item) => (item.fix.kind === "skills" ? item.fix.rows : []));
      if (skills.length) await batchApply.applyBatch({ actionLabel: "补齐", rows: skills });
      const mcp = group.flatMap((item) => (item.fix.kind === "mcp-fill" ? item.fix.rows : []));
      if (mcp.length) await perform({ kind: "mcp-fill", label: "补齐", rows: mcp });
      const rules = group.flatMap((item) => (item.fix.kind === "rules-fill" ? item.fix.rows : []));
      if (rules.length) await perform({ kind: "rules-fill", label: "补齐", rows: rules });
    } finally {
      setRunning(null);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

  if (items.length === 0) {
    return (
      <EmptyState
        icon={CheckCircle2}
        testId="inbox-empty"
        title="没有需要处理的"
        description="各账号的技能、MCP、规则、命令都和预期一致，来源也都是最新。"
      />
    );
  }

  const busy = running !== null || batchApply.executing;
  return (
    <div data-testid="inbox-page" className="space-y-3">
      <h1 className="sr-only">待处理</h1>
      {INBOX_STATES.map(({ state, label, hint }) => {
        const group = items.filter((item) => item.state === state);
        if (group.length === 0) return null;
        const fillable = state === "missing" && group.filter((item) => item.fix.kind !== "open").length > 1;
        return (
          <section key={state} data-testid={`inbox-group-${state}`} aria-label={label} className="overflow-clip rounded-lg border bg-card">
            <header className="flex items-center gap-3 border-b bg-muted px-4 py-2 text-xs text-muted-foreground">
              <span className={cn("rounded px-1.5 py-0.5 text-[11px] font-medium", STATE_TONE[state])}>{label}</span>
              <span className="min-w-0 flex-1">{hint}</span>
              <span className="tabular-nums">{group.length}</span>
              {fillable && (
                <button
                  type="button"
                  data-testid="inbox-fill-all"
                  disabled={busy}
                  onClick={() => void fillAll(group.filter((item) => item.fix.kind !== "open"))}
                  className="rounded-md border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs text-primary hover:bg-primary/15 disabled:opacity-40"
                >
                  全部补齐
                </button>
              )}
            </header>
            <ul className="divide-y">
              {group.map((item) => (
                <li key={item.id} data-testid={`inbox-item-${item.id}`} className="flex items-center gap-3 px-4 py-2 hover:bg-accent/40">
                  <span className="w-12 shrink-0 text-xs text-muted-foreground">{pageLabel(item.page)}</span>
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm">{item.title}</div>
                    <div className="truncate text-xs text-muted-foreground" title={item.detail}>{item.detail}</div>
                  </div>
                  <button
                    type="button"
                    data-testid={`inbox-fix-${item.id}`}
                    disabled={busy && item.fix.kind !== "open"}
                    onClick={() => void run(item, item.fix)}
                    className={cn(
                      "shrink-0 rounded-md px-2.5 py-1 text-xs disabled:opacity-40",
                      item.fix.kind === "open" ? "border hover:bg-accent" : "bg-primary text-primary-foreground hover:bg-primary/90",
                    )}
                  >
                    {running === item.id ? "正在处理…" : item.fix.label}
                  </button>
                </li>
              ))}
            </ul>
          </section>
        );
      })}
      <p className="px-1 text-xs text-muted-foreground">
        每条的按钮和对应页面上的是同一个动作：直接做的会先照常核对实际文件，做完重新读取，处理好的项会从这里消失；需要看差异或做选择的会跳到负责的页面。
      </p>
    </div>
  );
}
