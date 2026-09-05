import { useQueries } from "@tanstack/react-query";
import {
  AlertTriangle,
  CheckCircle2,
  Info,
  Loader2,
  RefreshCw,
  XCircle,
} from "lucide-react";
import { ElementTable } from "../components/ElementTable";
import { ErrorPanel } from "../components/ErrorPanel";
import { engineErrorMessage, runEngine, toDiagnostic } from "../lib/engine";
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
  { key: "tools", label: "工具" },
  { key: "mcp", label: "MCP" },
  { key: "skills", label: "Skills" },
] as const;

const SECTION_LABELS: Record<string, string> = {
  config: "配置文件",
  security: "安全",
  runtime: "运行时",
  storage: "存储",
  agents: "Agents",
  tools: "工具",
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
  healthy: { label: "健康", icon: CheckCircle2, className: "text-emerald-600" },
  warning: { label: "存在警告", icon: AlertTriangle, className: "text-amber-600" },
  error: { label: "存在错误", icon: XCircle, className: "text-red-600" },
};

const AUDIT_ENTRY_META: Record<AuditEntryStatus, { label: string; className: string }> = {
  ok: { label: "正常", className: "border-emerald-200 bg-emerald-50 text-emerald-700" },
  notice: { label: "提示", className: "border-sky-200 bg-sky-50 text-sky-700" },
  warning: { label: "警告", className: "border-amber-200 bg-amber-50 text-amber-700" },
  error: { label: "错误", className: "border-red-200 bg-red-50 text-red-700" },
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
    <section data-testid={testId} className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
      <h3 className="mb-3 text-sm font-semibold text-slate-700">{title}</h3>
      {children}
    </section>
  );
}

export default function DashboardPage() {
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
    return (
      <div className="flex items-center gap-2 py-20 text-sm text-slate-400">
        <Loader2 className="h-4 w-4 animate-spin" aria-hidden />
        正在读取引擎状态…
      </div>
    );
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

  const staleElements = freshnessCards
    .flatMap((card) => card.elements)
    .filter((element) => element.freshness.state === "behind" || element.freshness.state === "unknown");

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold tracking-tight">Dashboard</h2>
          <p className="mt-1 text-xs text-slate-400">
            运行 {audit.run_id} · 版本 {auditData.version.commit}（{auditData.version.date}）
          </p>
        </div>
        <div className="flex flex-col items-end gap-1">
          <button
            type="button"
            data-testid="refresh-upstream-button"
            disabled={refreshMutation.isPending || offline}
            onClick={() => refreshMutation.mutate()}
            className={cn(
              "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white transition-colors",
              refreshMutation.isPending || offline
                ? "cursor-not-allowed bg-slate-300"
                : "bg-indigo-600 hover:bg-indigo-700",
            )}
          >
            {refreshMutation.isPending ? (
              <Loader2 className="h-4 w-4 animate-spin" aria-hidden />
            ) : (
              <RefreshCw className="h-4 w-4" aria-hidden />
            )}
            刷新上游状态
          </button>
          {offline && <p className="text-xs text-amber-600">offline 模式下禁用（network 级探测）</p>}
        </div>
      </div>

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
              <p className="text-sm text-slate-500">
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
                ["unmaterialized", "托管产物尚未生成"],
                ["official_gaps", "官方缺口"],
              ] as const
            ).map(([key, label]) => (
              <li key={key} className="flex items-center justify-between">
                <span className="text-slate-600">{label}</span>
                <span data-testid={`warning-count-${key}`} className="font-mono text-slate-800">
                  {auditData.summary.warning_categories[key]}
                </span>
              </li>
            ))}
          </ul>
        </Card>
      </div>

      <div>
        <h3 className="mb-3 text-sm font-semibold text-slate-700">上游新鲜度（仅读探测缓存，status 零网络）</h3>
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
          {freshnessCards.map((card) => (
            <Card key={card.key} title={card.label} testId={`freshness-card-${card.key}`}>
              {card.query.isError ? (
                <p className="text-sm text-red-600">加载失败：{engineErrorMessage(card.query.error)}</p>
              ) : (
                <>
                  <ul className="space-y-1.5 text-sm">
                    {(
                      [
                        ["current", "已是最新"],
                        ["behind", "落后"],
                        ["ahead", "领先"],
                        ["unknown", "未知"],
                      ] as const
                    ).map(([state, label]) => (
                      <li key={state} className="flex items-center justify-between">
                        <span className="text-slate-600">{label}</span>
                        <span
                          data-testid={`freshness-count-${card.key}-${state}`}
                          className={cn(
                            "font-mono",
                            state === "behind" && card.aggregate.behind > 0 ? "font-semibold text-amber-600" : "text-slate-800",
                            state === "unknown" && card.aggregate.unknown > 0 ? "font-semibold text-slate-500" : "",
                          )}
                        >
                          {card.aggregate[state]}
                        </span>
                      </li>
                    ))}
                  </ul>
                  <p className="mt-3 flex items-center gap-1 border-t border-slate-100 pt-2 text-xs text-slate-400">
                    <Info className="h-3 w-3" aria-hidden />
                    上次检查：{formatCheckedAt(card.aggregate.lastCheckedAt)}
                  </p>
                </>
              )}
            </Card>
          ))}
        </div>
      </div>

      {staleElements.length > 0 && (
        <Card title={`落后 / 未知元素（${staleElements.length}）`} testId="stale-elements">
          <ElementTable elements={staleElements} />
        </Card>
      )}

      <div className="space-y-4">
        <h3 className="text-sm font-semibold text-slate-700">审计明细</h3>
        {Object.entries(auditData.sections).map(([section, entries]) => (
          <Card key={section} title={SECTION_LABELS[section] ?? section} testId={`audit-section-${section}`}>
            <ul className="space-y-1.5">
              {entries.map((entry) => (
                <li key={entry.name} className="flex items-baseline gap-3 text-sm">
                  <AuditEntryBadge status={entry.status} />
                  <span className="shrink-0 font-medium text-slate-700">{entry.name}</span>
                  <span className="text-slate-500">{entry.message}</span>
                </li>
              ))}
            </ul>
          </Card>
        ))}
      </div>
    </div>
  );
}
