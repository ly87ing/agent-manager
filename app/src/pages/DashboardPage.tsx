import { ErrorPanel } from "../components/ErrorPanel";
import { PageLoading } from "../components/PageState";
import { OverviewList } from "../components/OverviewList";
import { toDiagnostic } from "../lib/engine";
import type { AuditData } from "../lib/contract";
import { useEngineQuery } from "../lib/queries";

export default function DashboardPage({ onNavigate }: { onNavigate?: (page: string) => void } = {}) {
  const auditQuery = useEngineQuery<AuditData>("system:audit");

  if (auditQuery.isPending) {
    return <PageLoading text="正在读取引擎状态…" />;
  }
  if (auditQuery.isError) {
    return <ErrorPanel title="system:audit 调用失败" errors={[toDiagnostic(auditQuery.error)]} />;
  }

  const audit = auditQuery.data;
  if (!audit) return null;

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-xl font-semibold tracking-tight">总览</h2>
        <p className="mt-1 text-xs text-muted-foreground">
          本机有哪些东西、什么版本、要不要更新、同步到位没有。
        </p>
      </div>

      {/* 清单即总览：不同步的一键做掉；细节在各自页面看 */}
      <OverviewList onNavigate={onNavigate} />

      {audit.errors.length > 0 && <ErrorPanel title="读取出错" errors={audit.errors} />}
    </div>
  );
}
