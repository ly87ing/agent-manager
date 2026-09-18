import { ErrorPanel } from "../components/ErrorPanel";
import { PageLoading } from "../components/PageState";
import { OverviewList } from "../components/OverviewList";
import { toDiagnostic } from "../lib/engine";
import type { AuditData, AuditEntryStatus } from "../lib/contract";
import { useEngineQuery } from "../lib/queries";
import { cn } from "../lib/utils";

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
  const offline = auditQuery.data?.mode.offline ?? false;

  if (auditQuery.isPending) {
    return <PageLoading text="正在读取引擎状态…" />;
  }
  if (auditQuery.isError) {
    return <ErrorPanel title="system:audit 调用失败" errors={[toDiagnostic(auditQuery.error)]} />;
  }

  const audit = auditQuery.data;
  if (!audit) return null;
  const auditData = audit.data;
  const versionLabel = auditData.version
    ? `${auditData.version.commit}（${auditData.version.date}）`
    : "信息不可用";

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-xl font-semibold tracking-tight">总览</h2>
        <p className="mt-1 text-xs text-muted-foreground">
          本机有哪些东西、什么版本、要不要更新、同步到位没有。
          {offline && <span className="ml-1 text-warning">离线模式下不能检查更新。</span>}
        </p>
      </div>

      {/* 清单即总览：不同步的一键做掉；细节在各自页面看 */}
      <OverviewList onNavigate={onNavigate} />

      {audit.errors.length > 0 && <ErrorPanel title="体检读取出错" errors={audit.errors} />}

      {/* 明细默认收起：进来先回答「要不要做事」，要刨根问底再展开 */}
      <details data-testid="audit-details" className="rounded-lg border bg-card p-5 shadow-sm">
        <summary className="cursor-pointer text-sm font-semibold text-foreground">
          体检明细（{auditData.summary.errors} 个错误 · {auditData.summary.warnings} 个警告）
        </summary>
        <p className="mt-2 text-[11px] text-muted-foreground">运行 {audit.run_id} · 版本 {versionLabel}</p>
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
