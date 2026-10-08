import { useState } from "react";
import { CheckCircle2, ChevronRight } from "lucide-react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import { EmptyState } from "../components/EmptyState";
import { PageLoading } from "../components/PageState";
import { STATE_TONE } from "../components/StateBadge";
import { useExecutions } from "../lib/execution";
import { INBOX_STATES, pageLabel, useInbox, type InboxFix, type InboxItem } from "../lib/inbox";
import { ignoreItems, restoreItems } from "../lib/inboxIgnore";
import { mcpFillArgs } from "../lib/mcpFill";
import { engineKeys } from "../lib/queries";
import { rulesFillArgs } from "../lib/rulesFill";
import { useAccountNames } from "../lib/useAccountNames";
import { useBatchApply } from "../lib/useBatchApply";
import { cn } from "../lib/utils";

const OPEN_KEY = "agent-manager:inbox-open";
/** 用户展开过的分类，只存在本机浏览器里；读不到就全部收起。 */
function loadOpen(): Record<string, boolean> {
  try {
    const value = JSON.parse(localStorage.getItem(OPEN_KEY) ?? "{}");
    return typeof value === "object" && value !== null ? value : {};
  } catch {
    return {};
  }
}

function saveOpen(value: Record<string, boolean>) {
  try {
    localStorage.setItem(OPEN_KEY, JSON.stringify(value));
  } catch {
    // 存不下只影响下次打开时的展开状态
  }
}

/** 一行：单个项，或同一原因、同一来源合成的一组。 */
interface Entry {
  key: string;
  page: InboxItem["page"];
  title: string;
  detail: string;
  members: InboxItem[];
  fix: InboxFix;
}

function entriesOf(items: InboxItem[]): Entry[] {
  const entries: Entry[] = [];
  const grouped = new Map<string, Entry>();
  for (const item of items) {
    if (!item.group) {
      entries.push({ key: item.id, page: item.page, title: item.title, detail: item.detail, members: [item], fix: item.fix });
      continue;
    }
    const found = grouped.get(item.group.key);
    if (found) {
      found.members.push(item);
      continue;
    }
    const entry: Entry = { key: `group:${item.group.key}`, page: item.page, title: item.group.title, detail: item.detail, members: [item], fix: item.fix };
    grouped.set(item.group.key, entry);
    entries.push(entry);
  }
  for (const entry of grouped.values()) {
    if (entry.members.length === 1) {
      // 只有一项就按单项显示
      const [only] = entry.members;
      Object.assign(entry, { key: only.id, title: only.title, detail: only.detail, fix: only.fix });
      continue;
    }
    const names = entry.members.map((member) => member.title);
    entry.detail = `${names.length} 项：${names.slice(0, 5).join("、")}${names.length > 5 ? " 等" : ""}`;
    // 组里都能直接做同一种动作就合起来做；否则去负责的页面看
    const first = entry.members[0].fix;
    entry.fix = first.kind === "skills" && entry.members.every((member) => member.fix.kind === "skills" && member.fix.label === first.label)
      ? { kind: "skills", label: first.label, rows: entry.members.flatMap((member) => (member.fix.kind === "skills" ? member.fix.rows : [])) }
      : { kind: "open", label: "去查看", target: entry.members[0].group!.open };
  }
  return entries;
}

/**
 * 待处理：全应用里不是「生效」的项，按统一状态分类（可折叠，默认收起），同一原因、同一来源的合成一行。
 * 这里只做汇总和入口：能直接做的（补齐、修复、换到新版本）就地做，和资源页上是同一个动作；
 * 要看差异或做选择的跳到负责的页面。不想处理的可以忽略：不再计入角标与「只看待处理」，底部可恢复；
 * 状态变了会重新提示。做完回读，处理好的自然消失。
 */
export default function InboxPage({ onOpen }: { onOpen: (target: string) => void }) {
  const nameOf = useAccountNames();
  const { items, ignored, loading } = useInbox(nameOf);
  const batchApply = useBatchApply();
  const { execute } = useExecutions();
  const queryClient = useQueryClient();
  const [running, setRunning] = useState<string | null>(null);
  const [open, setOpenState] = useState<Record<string, boolean>>(loadOpen);
  const [expandedEntries, setExpandedEntries] = useState<Set<string>>(() => new Set());

  if (loading && items.length === 0 && ignored.length === 0) return <PageLoading />;

  const setOpen = (key: string, value: boolean) => {
    const next = { ...open, [key]: value };
    setOpenState(next);
    saveOpen(next);
  };
  const toggleEntry = (key: string) =>
    setExpandedEntries((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

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

  const run = async (key: string, fix: InboxFix) => {
    if (fix.kind === "open") return onOpen(fix.target);
    setRunning(key);
    try {
      await perform(fix);
    } finally {
      setRunning(null);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

  /** 「缺失」一类能一次补齐：按领域合并同类动作，逐条照常核对。 */
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

  const busy = running !== null || batchApply.executing;
  const ignoredEntries = entriesOf(ignored);

  const renderEntry = (entry: Entry, mode: "active" | "ignored") => {
    const expandable = entry.members.length > 1;
    const expanded = expandedEntries.has(entry.key);
    return (
      <li key={entry.key} data-testid={`inbox-item-${entry.key}`} className="px-4 py-2 hover:bg-accent/40">
        <div className="flex items-center gap-3">
          <span className="w-12 shrink-0 text-xs text-muted-foreground">{pageLabel(entry.page)}</span>
          <button
            type="button"
            disabled={!expandable}
            aria-expanded={expandable ? expanded : undefined}
            onClick={() => toggleEntry(entry.key)}
            className={cn("flex min-w-0 flex-1 items-start gap-1 text-left", expandable ? "hover:text-foreground" : "cursor-default")}
          >
            {expandable ? (
              <ChevronRight className={cn("mt-1 h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform", expanded && "rotate-90")} aria-hidden />
            ) : (
              <span className="w-3.5 shrink-0" />
            )}
            <span className="min-w-0">
              <span className="block truncate text-sm">{entry.title}</span>
              <span className="block truncate text-xs text-muted-foreground" title={entry.detail}>{entry.detail}</span>
            </span>
          </button>
          {/* 处理按钮宽度不一：放在固定宽的列里右对齐，「忽略」固定在最右，与分组上的「全部忽略」对齐 */}
          <div className="flex w-36 shrink-0 justify-end">
            {mode === "active" && (
              <button
                type="button"
                data-testid={`inbox-fix-${entry.key}`}
                disabled={busy && entry.fix.kind !== "open"}
                onClick={() => void run(entry.key, entry.fix)}
                className={cn(
                  "max-w-full truncate rounded-md px-2.5 py-1 text-xs disabled:opacity-40",
                  entry.fix.kind === "open" ? "border hover:bg-accent" : "bg-primary text-primary-foreground hover:bg-primary/90",
                )}
              >
                {running === entry.key ? "正在处理…" : entry.fix.label}
              </button>
            )}
          </div>
          {mode === "active" ? (
            <button
              type="button"
              data-testid={`inbox-ignore-${entry.key}`}
              onClick={() => ignoreItems(entry.members)}
              title="不想处理：不再计入待处理，底部「已忽略」里可以恢复；状态变了会重新提示"
              className="w-12 shrink-0 rounded-md py-1 text-center text-xs text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              忽略
            </button>
          ) : (
            <button
              type="button"
              data-testid={`inbox-restore-${entry.key}`}
              onClick={() => restoreItems(entry.members)}
              className="w-12 shrink-0 rounded-md border py-1 text-center text-xs hover:bg-accent"
            >
              恢复
            </button>
          )}
        </div>
        {expandable && expanded && (
          <ul data-testid={`inbox-members-${entry.key}`} className="mt-1 space-y-0.5 pl-[4.75rem] text-xs text-muted-foreground">
            {entry.members.map((member) => (
              <li key={member.id} className="truncate">
                {member.title}
              </li>
            ))}
          </ul>
        )}
      </li>
    );
  };

  return (
    <div data-testid="inbox-page" className="space-y-3">
      <h1 className="text-lg font-semibold tracking-tight">待处理</h1>
      {items.length === 0 && (
        <EmptyState
          icon={CheckCircle2}
          testId="inbox-empty"
          title="没有需要处理的"
          description={ignored.length ? "其余的都已忽略；想再看可以在下方「已忽略」里恢复。" : "各账号的技能、MCP、规则、命令都和预期一致，来源也都是最新。"}
        />
      )}
      {INBOX_STATES.map(({ state, label, hint }) => {
        const group = items.filter((item) => item.state === state);
        if (group.length === 0) return null;
        const entries = entriesOf(group);
        const isOpen = open[state] ?? false;
        const fillable = state === "missing" && group.filter((item) => item.fix.kind !== "open").length > 1;
        return (
          <section key={state} data-testid={`inbox-group-${state}`} aria-label={label} className="overflow-clip rounded-lg border bg-card">
            <header className={cn("flex items-center gap-3 bg-muted px-4 py-2 text-xs text-muted-foreground", isOpen && "border-b")}>
              <button
                type="button"
                data-testid={`inbox-toggle-${state}`}
                aria-expanded={isOpen}
                onClick={() => setOpen(state, !isOpen)}
                className="flex min-w-0 flex-1 items-center gap-3 text-left hover:text-foreground"
              >
                <ChevronRight className={cn("h-4 w-4 shrink-0 transition-transform", isOpen && "rotate-90")} aria-hidden />
                <span className={cn("shrink-0 rounded px-1.5 py-0.5 text-[11px] font-medium", STATE_TONE[state])}>{label}</span>
                <span className="shrink-0 tabular-nums text-foreground">{entries.length}</span>
                <span className="min-w-0 truncate">
                  {isOpen ? hint : entries.slice(0, 3).map((entry) => entry.title).join("、") + (entries.length > 3 ? " 等" : "")}
                </span>
              </button>
              {fillable && (
                <button
                  type="button"
                  data-testid="inbox-fill-all"
                  disabled={busy}
                  onClick={() => void fillAll(group.filter((item) => item.fix.kind !== "open"))}
                  className="shrink-0 rounded-md border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs text-primary hover:bg-primary/15 disabled:opacity-40"
                >
                  全部补齐
                </button>
              )}
              <button
                type="button"
                data-testid={`inbox-ignore-all-${state}`}
                onClick={() => ignoreItems(group)}
                title="这一类都不想处理：全部忽略，底部可以恢复"
                className="shrink-0 rounded-md px-2 py-0.5 text-xs hover:bg-accent hover:text-foreground"
              >
                全部忽略
              </button>
            </header>
            {isOpen && <ul className="divide-y">{entries.map((entry) => renderEntry(entry, "active"))}</ul>}
          </section>
        );
      })}
      {ignoredEntries.length > 0 && (
        <section data-testid="inbox-ignored" aria-label="已忽略" className="overflow-clip rounded-lg border bg-card">
          <header className={cn("flex items-center gap-3 px-4 py-2 text-xs text-muted-foreground", open.ignored && "border-b")}>
            <button
              type="button"
              data-testid="inbox-toggle-ignored"
              aria-expanded={Boolean(open.ignored)}
              onClick={() => setOpen("ignored", !open.ignored)}
              className="flex min-w-0 flex-1 items-center gap-3 text-left hover:text-foreground"
            >
              <ChevronRight className={cn("h-4 w-4 shrink-0 transition-transform", open.ignored && "rotate-90")} aria-hidden />
              已忽略 <span className="tabular-nums">{ignoredEntries.length}</span>
              <span className="min-w-0 truncate">不计入待处理；状态变了会重新提示</span>
            </button>
            <button
              type="button"
              data-testid="inbox-restore-all"
              onClick={() => restoreItems(ignored)}
              className="shrink-0 rounded-md px-2 py-0.5 text-xs hover:bg-accent hover:text-foreground"
            >
              全部恢复
            </button>
          </header>
          {open.ignored && <ul className="divide-y">{ignoredEntries.map((entry) => renderEntry(entry, "ignored"))}</ul>}
        </section>
      )}
    </div>
  );
}
