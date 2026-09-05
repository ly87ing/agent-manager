import { useState } from "react";
import { CloudDownload, CloudUpload, WifiOff } from "lucide-react";
import { ConfirmModal } from "../components/ConfirmModal";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { AuditData, Diagnostic, ProfileData, ProfileSyncState } from "../lib/contract";
import { useExecutions } from "../lib/execution";
import { useEngineQuery } from "../lib/queries";
import { cn, formatCheckedAt } from "../lib/utils";

const STATE_META: Record<ProfileSyncState, { label: string; className: string }> = {
  clean: { label: "一致", className: "border-emerald-200 bg-emerald-50 text-emerald-700" },
  ahead: { label: "本地领先", className: "border-sky-200 bg-sky-50 text-sky-700" },
  behind: { label: "远端领先", className: "border-amber-200 bg-amber-50 text-amber-700" },
  diverged: { label: "已分叉", className: "border-red-200 bg-red-50 text-red-700" },
  uninitialized: { label: "远端未初始化", className: "border-zinc-200 bg-zinc-100 text-zinc-500" },
  "local-only": { label: "仅本地（offline）", className: "border-slate-200 bg-slate-100 text-slate-500" },
};

function shortDigest(digest: string | null): string {
  if (!digest) return "—";
  return digest.length > 14 ? `${digest.slice(0, 7)}…${digest.slice(-6)}` : digest;
}

export default function SyncPage() {
  const statusQuery = useEngineQuery<ProfileData>("profile:status");
  const auditQuery = useEngineQuery<AuditData>("system:audit");
  const { execute } = useExecutions();
  const [confirm, setConfirm] = useState<"pull" | "push" | null>(null);
  const [executing, setExecuting] = useState(false);
  const [resultErrors, setResultErrors] = useState<Diagnostic[] | null>(null);

  if (statusQuery.isPending || auditQuery.isPending) return <PageLoading />;
  if (statusQuery.isError) return <PageError title="profile:status 调用失败" error={statusQuery.error} />;

  const offline = auditQuery.data?.mode.offline ?? false;
  const envelope = statusQuery.data;
  const unconfigured = envelope.errors.some((err) => err.code === "PROFILE_BACKEND_UNCONFIGURED");
  const data = envelope.ok ? envelope.data : null;

  const runVerb = async () => {
    if (!confirm) return;
    setExecuting(true);
    setResultErrors(null);
    const result = await execute({
      command: `profile:${confirm}`,
      label: confirm === "pull" ? "拉取共享层（profile:pull）" : "推送共享层（profile:push）",
    });
    setExecuting(false);
    if (result && !result.ok) {
      // 推送门禁（SECRET_GATE_BLOCKED）/ 后端失败等：留在弹窗展示
      setResultErrors(result.errors);
      return;
    }
    setConfirm(null);
    void statusQuery.refetch();
  };

  return (
    <div className="space-y-4">
      <PageHeader title="配置同步" description="个性化配置共享层的 git 后端同步（§7.2）。后端地址只经 env / 仅本机层提供，已脱敏显示。" />
      {envelope.errors.length > 0 && !unconfigured && <ErrorPanel title="profile:status 错误" errors={envelope.errors} />}

      {unconfigured ? (
        <section className="rounded-lg border border-dashed border-slate-300 bg-white p-8 text-center">
          <h3 className="text-sm font-semibold text-slate-700">未配置同步后端</h3>
          <p className="mx-auto mt-2 max-w-lg text-xs text-slate-500">
            通过环境变量 <code className="font-mono">PROFILE_SYNC_REPO_URL</code> 或仅本机层
            <code className="font-mono"> manager.local.json </code>的
            <code className="font-mono">profile_sync.repo_url</code> 键提供 git 后端地址（地址不进主仓库）。
          </p>
        </section>
      ) : data ? (
        <>
          <section data-testid="profile-status-card" className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold text-slate-700">后端状态</h3>
              {data.state && (
                <span data-testid="profile-state" className={cn("rounded-full border px-2 py-0.5 text-xs font-medium", STATE_META[data.state].className)}>
                  {STATE_META[data.state].label}
                </span>
              )}
            </div>
            <dl className="mt-3 grid grid-cols-1 gap-2 text-xs text-slate-600 sm:grid-cols-2">
              <div className="flex gap-2"><dt className="text-slate-400">后端类型</dt><dd className="font-mono">{data.backend.type ?? "—"}</dd></div>
              <div className="flex gap-2"><dt className="text-slate-400">地址（脱敏）</dt><dd className="font-mono break-all">{data.backend.url ?? "—"}</dd></div>
              <div className="flex gap-2"><dt className="text-slate-400">机器 id</dt><dd className="font-mono">{data.machine_id}</dd></div>
              <div className="flex gap-2"><dt className="text-slate-400">最近同步</dt><dd>{formatCheckedAt(data.last_sync)}</dd></div>
              <div className="flex gap-2"><dt className="text-slate-400">本地共享层</dt><dd className="font-mono">{data.local.exists ? `${data.local.key_count} 键` : "不存在"}</dd></div>
              <div className="flex gap-2"><dt className="text-slate-400">三向 base</dt><dd>{data.base.exists ? "存在" : "无"}</dd></div>
              {data.ahead !== null && <div className="flex gap-2"><dt className="text-slate-400">本地未推送</dt><dd>{data.ahead} 键</dd></div>}
              {data.behind !== null && <div className="flex gap-2"><dt className="text-slate-400">远端未拉取</dt><dd>{data.behind} 键</dd></div>}
            </dl>

            <div className="mt-4 flex items-center gap-2 border-t border-slate-100 pt-4">
              <button
                type="button"
                data-testid="profile-pull-button"
                disabled={offline || executing}
                onClick={() => setConfirm("pull")}
                className={cn(
                  "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white",
                  offline ? "cursor-not-allowed bg-slate-300" : "bg-indigo-600 hover:bg-indigo-700",
                )}
              >
                <CloudDownload className="h-4 w-4" aria-hidden /> 拉取（pull）
              </button>
              <button
                type="button"
                data-testid="profile-push-button"
                disabled={offline || executing}
                onClick={() => setConfirm("push")}
                className={cn(
                  "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white",
                  offline ? "cursor-not-allowed bg-slate-300" : "bg-indigo-600 hover:bg-indigo-700",
                )}
              >
                <CloudUpload className="h-4 w-4" aria-hidden /> 推送（push）
              </button>
              {offline && (
                <span className="inline-flex items-center gap-1 text-xs text-amber-700">
                  <WifiOff className="h-3.5 w-3.5" aria-hidden /> offline 模式：network 级动作已禁用
                </span>
              )}
            </div>
          </section>

          {data.conflicts.length > 0 && (
            <section data-testid="profile-conflicts" className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
              <h3 className="mb-1 text-sm font-semibold text-slate-700">冲突（{data.conflicts.length}）</h3>
              <p className="mb-3 text-xs text-slate-500">
                以下键双侧都改过且取值不同，合并已<strong>保留本地值</strong>。
                {/* 引擎 v1 无 --accept-remote：UI 只读展示，「以远端为准」需到策略编辑器手动对齐本地层后 push。 */}
                若要以远端为准，请到策略编辑器把对应键改为远端值后重新推送。
              </p>
              <table className="w-full text-left text-xs">
                <thead>
                  <tr className="border-b border-slate-200 text-slate-500">
                    <th className="py-1.5 pr-3 font-medium">键</th>
                    <th className="py-1.5 pr-3 font-medium">base</th>
                    <th className="py-1.5 pr-3 font-medium">本地（已保留）</th>
                    <th className="py-1.5 font-medium">远端</th>
                  </tr>
                </thead>
                <tbody>
                  {data.conflicts.map((conflict) => (
                    <tr key={conflict.key} data-testid={`conflict-${conflict.key}`} className="border-b border-slate-100 last:border-0">
                      <td className="py-1.5 pr-3 font-mono break-all text-slate-700">{conflict.key}</td>
                      {(["base", "local", "remote"] as const).map((side) => (
                        <td key={side} className="py-1.5 pr-3 font-mono text-slate-500 last:pr-0">
                          {conflict[side].type}
                          {conflict[side].digest && <span className="ml-1 text-slate-400">({shortDigest(conflict[side].digest)})</span>}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
          )}

          {data.gate && (
            <p className="text-xs text-slate-400">
              推送门禁：已{data.gate.scanned ? "执行" : "跳过"}疑似明文扫描{data.gate.scanned ? `，命中 ${data.gate.findings} 项` : ""}。
            </p>
          )}
        </>
      ) : null}

      {confirm && (
        <ConfirmModal
          title={confirm === "pull" ? "拉取共享层" : "推送共享层"}
          description={
            confirm === "pull"
              ? "从同步后端拉取远端共享层，与本地三向合并后写回 synced 层；双侧冲突按键保留本地值并报告。执行前会自动快照。"
              : "把本地共享层发布到同步后端。推送前对内容做疑似明文扫描硬门禁（命中即中止），通过后自动快照并更新远端机器注册表。"
          }
          risk="network"
          confirmLabel={confirm === "pull" ? "确认拉取" : "确认推送"}
          executing={executing}
          resultErrors={resultErrors}
          onConfirm={() => void runVerb()}
          onClose={() => setConfirm(null)}
        />
      )}
    </div>
  );
}
