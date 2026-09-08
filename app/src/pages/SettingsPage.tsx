import { useEffect, useRef, useState } from "react";
import { RefreshCw, Save } from "lucide-react";
import { toast } from "sonner";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { ConfirmModal } from "../components/ConfirmModal";
import { engineErrorMessage } from "../lib/engine";
import type { SettingsData, SettingsPreferences } from "../lib/contract";
import { useSaveSettings, useSettings } from "../lib/settings";
import { cn } from "../lib/utils";
import { check } from "@tauri-apps/plugin-updater";

interface AvailableUpdate {
  version: string;
  date?: string | null;
  body?: string | null;
  downloadAndInstall: () => Promise<void>;
  close: () => Promise<void>;
}

function initialPreferences(data?: SettingsData): SettingsPreferences {
  return data?.preferences ?? { offline: false, defaultDryRun: false, upstreamTtlSec: 86400 };
}

export default function SettingsPage() {
  const settings = useSettings();
  const save = useSaveSettings();
  const [preferences, setPreferences] = useState<SettingsPreferences>(() => initialPreferences());
  const [update, setUpdate] = useState<AvailableUpdate | null>(null);
  const [updateState, setUpdateState] = useState<"idle" | "checking" | "installing">("idle");
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [updateStatus, setUpdateStatus] = useState<string | null>(null);
  const [saveStatus, setSaveStatus] = useState<string | null>(null);
  const [confirmUpdate, setConfirmUpdate] = useState(false);
  const isMountedRef = useRef(true);
  const isInstallingRef = useRef(false);

  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    return () => {
      if (!isInstallingRef.current && update) {
        void update.close().catch(() => {});
      }
    };
  }, [update]);

  useEffect(() => {
    if (settings.data) setPreferences(initialPreferences(settings.data));
  }, [settings.data]);

  if (settings.isPending) return <PageLoading />;
  if (settings.isError) return <PageError title="设置加载失败" error={settings.error} />;
  const data = settings.data;
  const changed = JSON.stringify(preferences) !== JSON.stringify(data.preferences);

  const savePreferences = () => {
    if (!Number.isInteger(preferences.upstreamTtlSec) || preferences.upstreamTtlSec < 0) return;
    setSaveStatus(null);
    save.mutate(preferences, {
      onSuccess: () => {
        setSaveStatus("设置已保存（已从引擎读回）"); toast.success("设置已保存");
      },
      onError: (error) => { setSaveStatus(`设置保存失败：${engineErrorMessage(error)}`); toast.error(`设置保存失败：${engineErrorMessage(error)}`); },
    });
  };

  const checkForUpdate = async () => {
    setUpdateError(null);
    setUpdateStatus(null);
    if (update) {
      void update.close().catch(() => {});
    }
    setUpdate(null);
    if (!data.updaterConfigured) return;
    setUpdateState("checking");
    try {
      const found = await check();
      if (!isMountedRef.current) {
        if (found) {
          void found.close().catch(() => {});
        }
        return;
      }
      if (found) {
        let closed = false;
        const safeClose = async () => {
          if (!closed) {
            closed = true;
            try {
              await found.close();
            } catch {
              // ignore
            }
          }
        };
        setUpdate({
          version: found.version,
          date: found.date,
          body: found.body,
          downloadAndInstall: () => found.downloadAndInstall(undefined, { restartAfterInstall: false }),
          close: safeClose,
        });
      } else {
        setUpdateStatus("当前已是最新版本");
        toast.success("当前已是最新版本");
      }
    } catch (error) {
      if (isMountedRef.current) {
        setUpdateError(`检查更新失败：${engineErrorMessage(error)}`);
      }
    } finally {
      if (isMountedRef.current) {
        setUpdateState("idle");
      }
    }
  };

  const installUpdate = async () => {
    if (!update) return;
    const currentUpdate = update;
    setConfirmUpdate(false);
    setUpdateState("installing");
    setUpdateError(null);
    isInstallingRef.current = true;
    try {
      await currentUpdate.downloadAndInstall();
      toast.success("更新已安装，请重启应用生效");
      void currentUpdate.close().catch(() => {});
      if (isMountedRef.current) {
        setUpdateStatus("更新已安装，请重启应用生效");
        setUpdate(null);
      }
    } catch (error) {
      const msg = `更新安装失败：${engineErrorMessage(error)}`;
      toast.error(msg);
      if (!isMountedRef.current) {
        void currentUpdate.close().catch(() => {});
      } else {
        setUpdateError(msg);
      }
    } finally {
      isInstallingRef.current = false;
      if (isMountedRef.current) {
        setUpdateState("idle");
      }
    }
  };

  return (
    <div className="space-y-4">
      <PageHeader title="设置" description="查看应用契约与运行路径，调整本机运行偏好。" />
      <section className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <h2 className="text-sm font-semibold text-slate-800">应用与运行路径</h2>
        <dl className="mt-3 grid gap-3 text-sm sm:grid-cols-2">
          {[["应用版本", data.appVersion], ["契约版本", data.contractVersion], ["资源目录", data.resourceDir], ["数据目录", data.dataDir], ["日志目录", data.logDir]].map(([label, value]) => (
            <div key={label} className="min-w-0"><dt className="text-xs text-slate-500">{label}</dt><dd className="mt-1 break-all font-mono text-xs text-slate-700">{value}</dd></div>
          ))}
        </dl>
      </section>
      <section className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <h2 className="text-sm font-semibold text-slate-800">运行偏好</h2>
        <div className="mt-4 space-y-4 text-sm">
          <label className="flex items-center gap-2"><input type="checkbox" checked={preferences.offline} onChange={(e) => setPreferences({ ...preferences, offline: e.target.checked })} />离线模式</label>
          <label className="flex items-center gap-2"><input type="checkbox" checked={preferences.defaultDryRun} onChange={(e) => setPreferences({ ...preferences, defaultDryRun: e.target.checked })} />预览弹窗默认使用演练</label>
          <label className="flex items-center gap-3"><span>官方缓存 TTL（秒）</span><input aria-label="官方缓存 TTL（秒）" type="number" min={0} step={1} value={preferences.upstreamTtlSec} onChange={(e) => setPreferences({ ...preferences, upstreamTtlSec: Number(e.target.value) })} className="w-32 rounded border border-slate-200 px-2 py-1" /></label>
          <button type="button" onClick={savePreferences} disabled={!changed || save.isPending || !Number.isInteger(preferences.upstreamTtlSec) || preferences.upstreamTtlSec < 0} className={cn("inline-flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium text-white", !changed || save.isPending ? "bg-slate-300" : "bg-indigo-600 hover:bg-indigo-700")}><Save className="h-4 w-4" aria-hidden />保存设置</button>
          {saveStatus && <p role={saveStatus.startsWith("设置保存失败") ? "alert" : "status"} className={saveStatus.startsWith("设置保存失败") ? "text-sm text-red-600" : "text-sm text-emerald-700"}>{saveStatus}</p>}
        </div>
      </section>
      <section className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <div className="flex items-center justify-between gap-3"><div><h2 className="text-sm font-semibold text-slate-800">应用更新</h2><p className="mt-1 text-xs text-slate-500">手动检查，确认后下载并安装。</p></div><button type="button" onClick={() => void checkForUpdate()} disabled={!data.updaterConfigured || updateState !== "idle"} className="inline-flex items-center gap-2 rounded-md bg-indigo-600 px-3 py-2 text-sm font-medium text-white disabled:bg-slate-300"><RefreshCw className={cn("h-4 w-4", updateState === "checking" && "animate-spin")} aria-hidden />检查更新</button></div>
        {!data.updaterConfigured && <p role="status" className="mt-3 text-sm text-slate-500">更新服务未配置。</p>}
        {data.updaterConfigured && updateState === "idle" && !update && !updateError && <p role="status" className="mt-3 text-sm text-slate-500">{updateStatus ?? "尚未检查更新。"}</p>}
        {updateError && (
          <div className="mt-3 flex items-center justify-between gap-2 rounded-md bg-red-50 p-3 text-sm text-red-700">
            <p role="alert">{updateError}</p>
            <button
              type="button"
              onClick={() => void (update ? installUpdate() : checkForUpdate())}
              disabled={updateState !== "idle"}
              className="rounded border border-red-200 bg-white px-2.5 py-1 text-xs font-medium text-red-700 hover:bg-red-50 disabled:opacity-50"
            >
              重试
            </button>
          </div>
        )}
        {update && <div className="mt-3 rounded-md border border-indigo-100 bg-indigo-50 p-3 text-sm"><p className="font-medium text-indigo-900">发现新版本 {update.version}</p>{update.date && <p className="mt-1 text-xs text-indigo-700">发布时间：{update.date}</p>}{update.body && <p className="mt-2 whitespace-pre-wrap text-xs text-indigo-800">{update.body}</p>}<button type="button" onClick={() => setConfirmUpdate(true)} disabled={updateState !== "idle"} className="mt-3 inline-flex items-center gap-2 rounded-md bg-indigo-600 px-3 py-2 text-sm font-medium text-white disabled:bg-slate-300">下载并安装</button></div>}
      </section>
      {confirmUpdate && update && <ConfirmModal title="安装应用更新" description={`将下载并安装版本 ${update.version}，完成后请重启应用。`} risk="network" confirmLabel="确认下载并安装" executing={updateState === "installing"} onConfirm={() => void installUpdate()} onClose={() => setConfirmUpdate(false)} />}
    </div>
  );
}
