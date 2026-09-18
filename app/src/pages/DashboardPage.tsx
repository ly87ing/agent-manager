import { useQueries } from "@tanstack/react-query";
import {
  AlertTriangle,
  CheckCircle2,
  Loader2,
  RefreshCw,
  XCircle,
} from "lucide-react";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageLoading } from "../components/PageState";
import { SyncCenter } from "../components/SyncCenter";
import { runEngine, toDiagnostic } from "../lib/engine";
import type {
  AuditData,
  AuditEntryStatus,
  EngineElement,
  StatusData,
} from "../lib/contract";
import { engineKeys, useEngineQuery, useRefreshUpstream } from "../lib/queries";
import { cn, formatCheckedAt } from "../lib/utils";

/** freshness 聚合统计的域集合（提案 §4.1）。 */
const FRESHNESS_DOMAINS = [
  { key: "agents", label: "Agents" },
  { key: "mcp", label: "MCP" },
  { key: "skills", label: "Skills" },
] as const;

/** 引擎的 sections 固定为七个已接线的域 + runtime + storage（见 native_system 的 WIRED_DOMAINS）。 */
const SECTION_LABELS: Record<string, string> = {
  runtime: "运行时",
  storage: "存储",
  agents: "Agents",
  mcp: "MCP",
  skills: "Skills",
  rules: "Rules",
  hooks: "Hooks",
  aliases: "Aliases",
  secrets: "Secrets",
};

export interface FreshnessAggregate {
  current: number;
  behind: number;
  ahead: number;
  unknown: number;
  lastCheckedAt: string | null;
}

/** 聚合一个域的上游轴计数；not-applicable 不计入任何桶。 */
export function aggregateFreshness(elements: EngineElement[]): FreshnessAggregate {
  const aggregate: FreshnessAggregate = { current: 0, behind: 0, ahead: 0, unknown: 0, lastCheckedAt: null };
  for (const element of elements) {
    const { state, checked_at } = element.freshness;
    if (state === "current" || state === "behind" || state === "ahead" || state === "unknown") {
      aggregate[state] += 1;
    }
    if (checked_at && (aggregate.lastCheckedAt === null || checked_at > aggregate.lastCheckedAt)) {
      aggregate.lastCheckedAt = checked_at;
    }
  }
  return aggregate;
}

const AUDIT_STATUS_META: Record<AuditData["status"], { label: string; icon: typeof CheckCircle2; className: string }> = {
  healthy: { label: "健康", icon: CheckCircle2, className: "text-success" },
  warning: { label: "存在警告", icon: AlertTriangle, className: "text-warning" },
  error: { label: "存在错误", icon: XCircle, className: "text-destructive" },
};

const AUDIT_ENTRY_META: Record<AuditEntryStatus, { label: string; className: string }> = {
  ok: { label: "正常", className: "border-success/30 bg-success/10 text-success" },
  notice: { label: "提示", className: "border-info/30 bg-info/10 text-info" },
  warning: { label: "警告", className: "border-warning/30 bg-warning/10 text-warning" },
  error: { label: "错误", className: "border-destructive/30 bg-destructive/10 text-destructive" },
};

function AuditEntryBadge({ status }: { status: AuditEntryStatus }) {
  const meta = AUDIT_ENTRY_META[status];
  return (
    <span className={cn("inline-flex shrink-0 rounded-full border px-2 py-0.5 text-xs font-medium", meta.className)}>
      {meta.label}
    </span>
  );
}

function Card({ title, children, testId }: { title: string; children: React.ReactNode; testId?: string }) {
  return (
    <section data-testid={testId} className="rounded-lg border bg-card p-5 shadow-sm">
      <h3 className="mb-3 text-sm font-semibold text-foreground">{title}</h3>
      {children}
    </section>
  );
}

export default function DashboardPage({ onNavigate }: { onNavigate?: (page: string) => void } = {}) {
  const auditQuery = useEngineQuery<AuditData>("system:audit");

  const statusResults = useQueries({
    queries: FRESHNESS_DOMAINS.map((domain) => ({
      queryKey: engineKeys.command(`${domain.key}:status`),
      queryFn: () => runEngine<StatusData>(`${domain.key}:status`),
    })),
  });

  const refreshMutation = useRefreshUpstream();

  const offline = auditQuery.data?.mode.offline ?? false;

  if (auditQuery.isPending) {
    return <PageLoading text="正在读取引擎状态…" />;
  }

  if (auditQuery.isError) {
    return (
      <ErrorPanel
        title="system:audit 调用失败"
        errors={[toDiagnostic(auditQuery.error)]}
      />
    );
  }

  const audit = auditQuery.data;
  if (!audit) return null;
  const auditData = audit.data;
  const versionLabel = auditData.version
    ? `${auditData.version.commit}（${auditData.version.date}）`
    : "信息不可用";
  const statusMeta = AUDIT_STATUS_META[auditData.status] ?? AUDIT_STATUS_META.error;
  const StatusIcon = statusMeta.icon;

  const freshnessCards = FRESHNESS_DOMAINS.map((domain, index) => {
    const result = statusResults[index];
    const elements = result.data?.data.elements ?? [];
    return {
      ...domain,
      query: result,
      aggregate: aggregateFreshness(elements),
      elements,
    };
  });

  const behindCount = freshnessCards.reduce((sum, card) => sum + card.aggregate.behind, 0);
  const uncheckedCount = freshnessCards.reduce((sum, card) => sum + card.aggregate.unknown, 0);
  const lastCheckedAt = freshnessCards
    .map((card) => card.aggregate.lastCheckedAt)
    .filter((value): value is string => Boolean(value))
    .sort()
    .pop();
  const lastCheckedLabel = lastCheckedAt ? formatCheckedAt(lastCheckedAt) : null;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold tracking-tight">总览</h2>
          <p className="mt-1 text-xs text-muted-foreground">
            运行 {audit.run_id} · 版本 {versionLabel}
          </p>
        </div>
        <div className="flex flex-col items-end gap-1">
          <button
            type="button"
            data-testid="refresh-upstream-button"
            disabled={refreshMutation.isPending || offline}
            onClick={() => refreshMutation.mutate()}
            className={cn(
              "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-primary-foreground transition-colors",
              refreshMutation.isPending || offline
                ? "cursor-not-allowed bg-muted text-muted-foreground"
                : "bg-primary hover:bg-primary/90",
            )}
          >
            {refreshMutation.isPending ? (
              <Loader2 className="h-4 w-4 animate-spin" aria-hidden />
            ) : (
              <RefreshCw className="h-4 w-4" aria-hidden />
            )}
            检查更新
          </button>
          {offline && <p className="text-xs text-warning">离线模式下不可用（需要联网检查）</p>}
        </div>
      </div>

      {/* 同步状态放在最前：用户进来最先要知道的是「有没有东西没落地」 */}
      <SyncCenter onNavigate={onNavigate} />

      {audit.errors.length > 0 && <ErrorPanel title="system:audit 错误" errors={audit.errors} />}
      {audit.warnings.length > 0 && (
        <ErrorPanel title="system:audit 警告" errors={audit.warnings} variant="warning" />
      )}

      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <Card title="总体状态" testId="audit-status-card">
          <div className="flex items-center gap-3">
            <StatusIcon className={cn("h-8 w-8", statusMeta.className)} aria-hidden />
            <div>
              <p className={cn("text-lg font-semibold", statusMeta.className)}>{statusMeta.label}</p>
              <p className="text-sm text-muted-foreground">
                错误 {auditData.summary.errors} · 警告 {auditData.summary.warnings}
              </p>
            </div>
          </div>
        </Card>

        <Card title="警告分类" testId="warning-categories">
          <ul className="space-y-1.5 text-sm">
            {(
              [
                ["optional_tools", "可选工具未就绪"],
                ["unmaterialized", "应生成的文件还没生成"],
                ["official_gaps", "官方缺口"],
              ] as const
            ).map(([key, label]) => (
              <li key={key} className="flex items-center justify-between">
                <span className="text-muted-foreground">{label}</span>
                <span data-testid={`warning-count-${key}`} className="font-mono text-foreground">
                  {auditData.summary.warning_categories[key]}
                </span>
              </li>
            ))}
          </ul>
        </Card>
      </div>

      {/* 版本对比原来占三张卡加一张表，和更新中心完全重复；这里只留一句结论加入口 */}
      <section data-testid="freshness-summary" className="rounded-lg border bg-card p-5 shadow-sm">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="min-w-0">
            <h3 className="text-sm font-semibold text-foreground">版本</h3>
            <p className="mt-1 text-xs text-muted-foreground">
              {behindCount > 0
                ? `${behindCount} 项可更新${uncheckedCount > 0 ? `，另有 ${uncheckedCount} 项还没检查过` : ""}`
                : uncheckedCount > 0
                  ? `都不落后；${uncheckedCount} 项还没检查过`
                  : "都已是最新"}
              {lastCheckedLabel && ` · 上次检查 ${lastCheckedLabel}`}
            </p>
          </div>
          {onNavigate && (behindCount > 0 || uncheckedCount > 0) && (
            <button
              type="button"
              data-testid="goto-updates"
              onClick={() => onNavigate("settings:updates")}
              className="shrink-0 rounded-md border border-primary/30 bg-primary/10 px-3 py-1.5 text-xs text-primary hover:bg-primary/15"
            >
              去更新中心
            </button>
          )}
        </div>
      </section>

      {/* 明细默认收起：进来先回答「要不要做事」，要刨根问底再展开 */}
      <details data-testid="audit-details" className="rounded-lg border bg-card p-5 shadow-sm">
        <summary className="cursor-pointer text-sm font-semibold text-foreground">
          体检明细（{auditData.summary.errors} 个错误 · {auditData.summary.warnings} 个警告）
        </summary>
        <div className="mt-3 space-y-3">
        {Object.entries(auditData.sections).map(([section, entries]) => (
          <Card key={section} title={SECTION_LABELS[section] ?? section} testId={`audit-section-${section}`}>
            <ul className="space-y-1.5">
              {entries.map((entry) => (
                <li key={entry.name} className="flex min-w-0 items-baseline gap-3 text-sm">
                  <AuditEntryBadge status={entry.status} />
                  <span className="min-w-0 [overflow-wrap:anywhere] font-medium text-foreground">{entry.name}</span>
                  <span className="min-w-0 [overflow-wrap:anywhere] text-muted-foreground">{entry.message}</span>
                </li>
              ))}
            </ul>
          </Card>
        ))}
        </div>
      </details>
    </div>
  );
}
