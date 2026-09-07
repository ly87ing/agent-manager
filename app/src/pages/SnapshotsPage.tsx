import { useState } from "react";
import { RotateCcw } from "lucide-react";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { Diagnostic, SnapshotsData } from "../lib/contract";
import { useExecutions } from "../lib/execution";
import { useEngineQuery } from "../lib/queries";
import { formatCheckedAt } from "../lib/utils";

export default function SnapshotsPage() {
  const query = useEngineQuery<SnapshotsData>("snapshots:list");
  const { execute } = useExecutions();
  const [restoring, setRestoring] = useState<string | null>(null);
  const [executing, setExecuting] = useState(false);
  const [resultErrors, setResultErrors] = useState<Diagnostic[] | null>(null);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="snapshots:list 调用失败" error={query.error} />;

  const snapshots = query.data.data.snapshots ?? [];

  const doRestore = async () => {
    if (!restoring) return;
    setExecuting(true);
    setResultErrors(null);
    const envelope = await execute({
      command: "snapshots:restore",
      args: ["--run", restoring],
      label: `恢复到备份 ${restoring}`,
    });
    setExecuting(false);
    if (envelope && !envelope.ok) {
      setResultErrors(envelope.errors);
      return;
    }
    if (envelope?.ok) {
      const restore = (envelope.data as SnapshotsData).restore;
      if (restore) {
        toast.success(
          `恢复完成:${restore.files_restored} 个文件 / ${restore.links_restored} 个链接已还原，删除 ${restore.files_deleted} 个；当前状态已另存为备份 ${restore.resnapshot_run_id}`,
        );
      }
    }
    setRestoring(null);
    void query.refetch();
  };

  return (
    <div className="space-y-4">
      <PageHeader title="备份与恢复" description="每次实际修改前都会自动备份。恢复前会把当前状态再备份一次，所以恢复本身也能撤销。" />
      {query.data.errors.length > 0 && <ErrorPanel title="snapshots:list 错误" errors={query.data.errors} />}

      <section className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-slate-200 text-xs text-slate-500">
              <th className="py-2 pr-4 font-medium">备份编号</th>
              <th className="py-2 pr-4 font-medium">创建时间</th>
              <th className="py-2 pr-4 font-medium">来源操作</th>
              <th className="py-2 pr-4 font-medium">文件 / 链接数</th>
              <th className="py-2 font-medium">操作</th>
            </tr>
          </thead>
          <tbody>
            {snapshots.map((snapshot) => (
              <tr key={snapshot.run_id} data-testid={`snapshot-row-${snapshot.run_id}`} className="border-b border-slate-100 last:border-0">
                <td className="py-2 pr-4 font-mono text-xs text-slate-700">{snapshot.run_id}</td>
                <td className="py-2 pr-4 text-xs text-slate-500">{formatCheckedAt(snapshot.created_at)}</td>
                <td className="py-2 pr-4 font-mono text-xs text-slate-500">{snapshot.command ?? "—"}</td>
                <td className="py-2 pr-4 text-xs text-slate-500">
                  {snapshot.file_count} / {snapshot.link_count}
                </td>
                <td className="py-2">
                  <button
                    type="button"
                    data-testid={`restore-${snapshot.run_id}`}
                    onClick={() => {
                      setResultErrors(null);
                      setRestoring(snapshot.run_id);
                    }}
                    className="inline-flex items-center gap-1 rounded border border-red-200 px-2 py-0.5 text-xs text-red-600 hover:bg-red-50"
                  >
                    <RotateCcw className="h-3 w-3" aria-hidden /> 恢复
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {snapshots.length === 0 && <p className="py-4 text-center text-sm text-slate-400">还没有备份（首次实际修改前会自动创建）</p>}
      </section>

      {restoring && (
        <ConfirmModal
          title={`恢复到备份 ${restoring}`}
          description="将按备份清单还原文件和链接，并先把当前状态再备份一次（所以恢复本身也能撤销）。注意：对 MCP/Alias 等共享配置文件，恢复将覆盖整个配置文件及其实际生效的全部条目，而非仅还原单个选定元素；备份清单之外的文件不受影响。"
          risk="write-local"
          requireText={restoring}
          confirmLabel="确认恢复"
          executing={executing}
          resultErrors={resultErrors}
          onConfirm={() => void doRestore()}
          onClose={() => setRestoring(null)}
        />
      )}
    </div>
  );
}
