import { useState } from "react";
import { CloudDownload, CloudUpload, UploadCloud, WifiOff } from "lucide-react";
import { ConfirmModal } from "../components/ConfirmModal";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { AuditData, Diagnostic, ProfileData, ProfileDiffEntry } from "../lib/contract";
import { useExecutions } from "../lib/execution";
import { useEngineQuery } from "../lib/queries";
import { PROFILE_CHANGE_COPY, PROFILE_STATE_COPY } from "../lib/ui-copy";
import { cn, formatCheckedAt } from "../lib/utils";

function shortDigest(digest: string | null): string {
  if (!digest) return "—";
  return digest.length > 12 ? `${digest.slice(0, 6)}…${digest.slice(-4)}` : digest;
}

/** 差异对比表：per 顶层键的变化方向与两侧类型+指纹截断；值永不显示。 */
function DiffTable({
  diff,
  offline,
  onAcceptRemote,
}: {
  diff: ProfileDiffEntry[];
  offline: boolean;
  onAcceptRemote: (key: string) => void;
}) {
  return (
    <section data-testid="profile-diff-table" className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
      <h3 className="mb-3 text-sm font-semibold text-slate-700">逐项差异（只显示类型与内容指纹，值永不显示）</h3>
      <table className="w-full text-left text-xs">
        <thead>
          <tr className="border-b border-slate-200 text-slate-500">
            <th className="py-1.5 pr-3 font-medium">配置项</th>
            <th className="py-1.5 pr-3 font-medium">状态</th>
            <th className="py-1.5 pr-3 font-medium">这台电脑</th>
            <th className="py-1.5 pr-3 font-medium">远端</th>
            <th className="py-1.5 font-medium">操作</th>
          </tr>
        </thead>
        <tbody>
          {diff.map((entry) => {
            const meta = PROFILE_CHANGE_COPY[entry.change];
            return (
              <tr key={entry.key} data-testid={`diff-${entry.key}`} className="border-b border-slate-100 last:border-0">
                <td className="py-1.5 pr-3 font-mono text-slate-700">{entry.key}</td>
                <td className="py-1.5 pr-3">
                  <span className={cn("inline-flex rounded-full border px-2 py-0.5 font-medium", meta.className)}>{meta.label}</span>
                </td>
                <td className="py-1.5 pr-3 font-mono text-slate-500">
                  {entry.local_type}
                  {entry.local_digest && <span className="ml-1 text-slate-400">({shortDigest(entry.local_digest)})</span>}
                </td>
                <td className="py-1.5 pr-3 font-mono text-slate-500">
                  {entry.remote_type}
                  {entry.remote_digest && <span className="ml-1 text-slate-400">({shortDigest(entry.remote_digest)})</span>}
                </td>
                <td className="py-1.5">
                  {entry.change === "both" && (
                    <button
                      type="button"
                      data-testid={`accept-remote-${entry.key}`}
                      disabled={offline}
                      onClick={() => onAcceptRemote(entry.key)}
                      className={cn(
                        "rounded border px-2 py-0.5",
                        offline ? "cursor-not-allowed border-slate-200 text-slate-300" : "border-amber-300 text-amber-700 hover:bg-amber-50",
                      )}
                    >
                      以远端为准
                    </button>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </section>
  );
}

type PendingAction = "pull" | "accept-remote" | "push" | "force-push" | { acceptRemoteKey: string };

export default function SyncPage() {
  const statusQuery = useEngineQuery<ProfileData>("profile:status");
  const auditQuery = useEngineQuery<AuditData>("system:audit");
  const { execute } = useExecutions();
  const [pending, setPending] = useState<PendingAction | null>(null);
  const [executing, setExecuting] = useState(false);
  const [resultErrors, setResultErrors] = useState<Diagnostic[] | null>(null);

  if (statusQuery.isPending || auditQuery.isPending) return <PageLoading />;
  if (statusQuery.isError) return <PageError title="profile:status 调用失败" error={statusQuery.error} />;

  const offline = auditQuery.data?.mode.offline ?? false;
  const envelope = statusQuery.data;
  const unconfigured = envelope.errors.some((err) => err.code === "PROFILE_BACKEND_UNCONFIGURED");
  const data = envelope.ok ? envelope.data : null;
  const diverged = data?.state === "diverged";

  const runAction = async () => {
    if (pending === null) return;
    setExecuting(true);
    setResultErrors(null);
    let command = "profile:pull";
    let args: string[] = [];
    let label = "拉取远端改动";
    if (pending === "accept-remote") {
      args = ["--accept-remote"];
      label = "以远端覆盖本地";
    } else if (pending === "push") {
      command = "profile:push";
      label = "推送到远端";
    } else if (pending === "force-push") {
      command = "profile:push";
      args = ["--force"];
      label = "强制推送（以本机覆盖远端）";
    } else if (typeof pending === "object" && "acceptRemoteKey" in pending) {
      args = ["--accept-remote", "--only", pending.acceptRemoteKey];
      label = `以远端为准:${pending.acceptRemoteKey}`;
    }
    const result = await execute({ command, args, label });
    setExecuting(false);
    if (result && !result.ok) {
      // 推送门禁（SECRET_GATE_BLOCKED）/ 后端失败等：留在弹窗展示
      setResultErrors(result.errors);
      return;
    }
    setPending(null);
    void statusQuery.refetch();
  };

  const confirmContent = (action: PendingAction): { title: string; description: string; requireText?: string; confirmLabel: string } => {
    if (action === "pull") {
      return {
        title: "拉取远端改动",
        description: "从同步后端拉取远端改动，与本机内容合并后写回「多设备共享」层；两边都改过的配置项会保留本机的值并逐条报告。执行前会自动备份。",
        confirmLabel: "确认拉取",
      };
    }
    if (action === "accept-remote") {
      return {
        title: "以远端覆盖本地",
        description: "远端内容将完整覆盖本机的「多设备共享」层，这台电脑上未推送的改动会被覆盖掉。执行前会自动备份（可撤销）。确认请输入「覆盖」。",
        requireText: "覆盖",
        confirmLabel: "确认覆盖",
      };
    }
    if (action === "push") {
      return {
        title: "推送到远端",
        description: "把「多设备共享」层发布到同步后端。推送前会做疑似密钥扫描（命中即中止），通过后自动备份并登记这台电脑的同步时间。",
        confirmLabel: "确认推送",
      };
    }
    if (action === "force-push") {
      return {
        title: "强制推送（以本机覆盖远端）",
        description: "两边都有改动的状态下，强制用这台电脑的内容覆盖远端——远端未拉取的改动会丢失。推送前安全检查仍会执行。确认请输入「强制推送」。",
        requireText: "强制推送",
        confirmLabel: "强制推送",
      };
    }
    const key = (action as { acceptRemoteKey: string }).acceptRemoteKey;
    return {
      title: `以远端为准:${key}`,
      description: `只把「${key}」这一项改成远端的值（本机这项的改动将被覆盖）。执行前会自动备份。`,
      confirmLabel: "确认以远端为准",
    };
  };

  return (
    <div className="space-y-4">
      <PageHeader title="多设备同步" description="把「多设备共享」层的个性化配置在多台电脑之间同步。后端地址只从环境变量或仅本机配置读取，已脱敏显示。" />
      {envelope.errors.length > 0 && !unconfigured && <ErrorPanel title="profile:status 错误" errors={envelope.errors} />}

      {unconfigured ? (
        <section className="rounded-lg border border-dashed border-slate-300 bg-white p-8 text-center">
          <h3 className="text-sm font-semibold text-slate-700">还没有配置同步后端</h3>
          <p className="mx-auto mt-2 max-w-lg text-xs text-slate-500">
            通过环境变量 <code className="font-mono">PROFILE_SYNC_REPO_URL</code> 或「个性化配置」仅这台电脑层的
            <code className="font-mono"> profile_sync.repo_url </code> 键提供 git 后端地址（地址不进主仓库）。
          </p>
        </section>
      ) : data ? (
        <>
          <section data-testid="profile-status-card" className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold text-slate-700">同步状态</h3>
              {data.state && (
                <span data-testid="profile-state" className={cn("rounded-full border px-2 py-0.5 text-xs font-medium", PROFILE_STATE_COPY[data.state].className)}>
                  {PROFILE_STATE_COPY[data.state].label}
                </span>
              )}
            </div>
            <dl className="mt-3 grid grid-cols-1 gap-2 text-xs text-slate-600 sm:grid-cols-2">
              <div className="flex gap-2"><dt className="text-slate-400">同步方式</dt><dd className="font-mono">{data.backend.type ?? "—"}</dd></div>
              <div className="flex gap-2"><dt className="text-slate-400">地址（已脱敏）</dt><dd className="font-mono break-all">{data.backend.url ?? "—"}</dd></div>
              <div className="flex gap-2"><dt className="text-slate-400">这台电脑</dt><dd className="font-mono">{data.machine_id}</dd></div>
              <div className="flex gap-2"><dt className="text-slate-400">最近同步</dt><dd>{formatCheckedAt(data.last_sync)}</dd></div>
              <div className="flex gap-2"><dt className="text-slate-400">本机共享层内容</dt><dd className="font-mono">{data.local.exists ? `${data.local.key_count} 键` : "不存在"}</dd></div>
              <div className="flex gap-2"><dt className="text-slate-400">合并基准</dt><dd>{data.base.exists ? "存在" : "无"}</dd></div>
              {data.ahead !== null && <div className="flex gap-2"><dt className="text-slate-400">待推送</dt><dd>{data.ahead} 键</dd></div>}
              {data.behind !== null && <div className="flex gap-2"><dt className="text-slate-400">待拉取</dt><dd>{data.behind} 键</dd></div>}
            </dl>

            <div className="mt-4 flex flex-wrap items-center gap-2 border-t border-slate-100 pt-4">
              <button
                type="button"
                data-testid="profile-pull-button"
                disabled={offline || executing}
                onClick={() => setPending("pull")}
                className={cn("inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white", offline ? "cursor-not-allowed bg-slate-300" : "bg-indigo-600 hover:bg-indigo-700")}
              >
                <CloudDownload className="h-4 w-4" aria-hidden /> 拉取（合并）
              </button>
              <button
                type="button"
                data-testid="profile-accept-remote-button"
                disabled={offline || executing}
                onClick={() => setPending("accept-remote")}
                className={cn("inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white", offline ? "cursor-not-allowed bg-slate-300" : "bg-amber-600 hover:bg-amber-700")}
              >
                <CloudDownload className="h-4 w-4" aria-hidden /> 以远端覆盖本地
              </button>
              <button
                type="button"
                data-testid="profile-push-button"
                disabled={offline || executing}
                onClick={() => setPending("push")}
                className={cn("inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white", offline ? "cursor-not-allowed bg-slate-300" : "bg-indigo-600 hover:bg-indigo-700")}
              >
                <CloudUpload className="h-4 w-4" aria-hidden /> 推送
              </button>
              <button
                type="button"
                data-testid="profile-force-push-button"
                disabled={offline || executing || !diverged}
                title={diverged ? "两边都有改动：强制用本机内容覆盖远端" : "仅在「两边都有改动」时可用"}
                onClick={() => setPending("force-push")}
                className={cn("inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white", offline || !diverged ? "cursor-not-allowed bg-slate-300" : "bg-red-600 hover:bg-red-700")}
              >
                <UploadCloud className="h-4 w-4" aria-hidden /> 强制推送
              </button>
              {offline && (
                <span className="inline-flex items-center gap-1 text-xs text-amber-700">
                  <WifiOff className="h-3.5 w-3.5" aria-hidden /> 离线模式：需要联网的操作已禁用
                </span>
              )}
            </div>
          </section>

          {data.diff && data.diff.length > 0 && (
            <DiffTable diff={data.diff} offline={offline} onAcceptRemote={(key) => setPending({ acceptRemoteKey: key })} />
          )}

          {data.conflicts.length > 0 && (
            <section data-testid="profile-conflicts" className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
              <h3 className="mb-1 text-sm font-semibold text-slate-700">上次合并的冲突（{data.conflicts.length}）</h3>
              <p className="mb-3 text-xs text-slate-500">
                以下配置项在这台电脑和远端都改过且不一样，合并时已<strong>保留这台电脑的值</strong>；想改用远端的值，可用上方差异表（或本页「以远端覆盖本地」）里的「以远端为准」。
              </p>
              <table className="w-full text-left text-xs">
                <thead>
                  <tr className="border-b border-slate-200 text-slate-500">
                    <th className="py-1.5 pr-3 font-medium">配置项</th>
                    <th className="py-1.5 pr-3 font-medium">合并基准</th>
                    <th className="py-1.5 pr-3 font-medium">这台电脑（已保留）</th>
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
              推送前安全检查：已{data.gate.scanned ? "执行" : "跳过"}疑似密钥扫描{data.gate.scanned ? `，命中 ${data.gate.findings} 项` : ""}。
            </p>
          )}
        </>
      ) : null}

      {pending !== null && (
        <ConfirmModal
          title={confirmContent(pending).title}
          description={confirmContent(pending).description}
          risk="network"
          requireText={confirmContent(pending).requireText}
          confirmLabel={confirmContent(pending).confirmLabel}
          executing={executing}
          resultErrors={resultErrors}
          onConfirm={() => void runAction()}
          onClose={() => setPending(null)}
        />
      )}
    </div>
  );
}
