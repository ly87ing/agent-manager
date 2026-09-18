import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  KeyRound,
  Link2,
  Monitor,
  Moon,
  RefreshCw,
  Save,
  SlidersHorizontal,
  Sun,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { ConfirmModal } from "../components/ConfirmModal";
import { engineErrorMessage, runEngine } from "../lib/engine";
import type { LogsClearData, SettingsData, SettingsPreferences } from "../lib/contract";
import { useSaveSettings, useSettings } from "../lib/settings";
import { useTheme, type Theme } from "../lib/theme";
import { cn } from "../lib/utils";
import { check } from "@tauri-apps/plugin-updater";
import { ConfigBackupSection } from "../components/ConfigBackupSection";
import AliasesPage from "./AliasesPage";
import SecretsPage from "./SecretsPage";
import UpdatesPage from "./UpdatesPage";
import PolicyPage from "./PolicyPage";

const THEME_OPTIONS: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: "light", label: "浅色", icon: Sun },
  { value: "dark", label: "深色", icon: Moon },
  { value: "system", label: "跟随系统", icon: Monitor },
];

function AppearanceSection() {
  const { theme, setTheme } = useTheme();
  return (
    <section className="rounded-lg border bg-card p-5 shadow-sm">
      <h2 className="text-sm font-semibold text-foreground">外观</h2>
      <p className="mt-1 text-xs text-muted-foreground">选择界面主题，立即生效并保存在本机。</p>
      <div className="mt-3 inline-flex rounded-lg border bg-muted p-0.5" role="group" aria-label="外观主题">
        {THEME_OPTIONS.map((option) => {
          const Icon = option.icon;
          const active = theme === option.value;
          return (
            <button
              key={option.value}
              type="button"
              aria-pressed={active}
              data-testid={`theme-option-${option.value}`}
              onClick={() => setTheme(option.value)}
              className={cn(
                "inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs transition-colors",
                active ? "bg-card font-medium text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground",
              )}
            >
              <Icon className="h-3.5 w-3.5" aria-hidden />
              {option.label}
            </button>
          );
        })}
      </div>
    </section>
  );
}

interface AvailableUpdate {
  version: string;
  date?: string | null;
  body?: string | null;
  downloadAndInstall: () => Promise<void>;
  close: () => Promise<void>;
}

const ADVANCED_ENTRIES = [
  {
    key: "aliases",
    title: "终端快捷命令",
    description: "每条命令的启动方式：时区、语言与固定参数。",
    icon: Link2,
  },
  {
    key: "secrets",
    title: "密钥状态",
    description: "环境变量与密钥配置状态检测。",
    icon: KeyRound,
  },
  {
    key: "updates",
    title: "更新中心",
    description: "各组件版本检查与更新对比。",
    icon: RefreshCw,
  },
  {
    key: "policy",
    title: "个性化配置",
    description: "SQLite 本机配置原文查看与编辑。",
    icon: SlidersHorizontal,
  },
];

function initialPreferences(data?: SettingsData): SettingsPreferences {
  const p = data?.preferences;
  return {
    offline: p?.offline ?? false,
    upstreamTtlSec: p?.upstreamTtlSec ?? 86400,
    logRetentionDays: p?.logRetentionDays ?? 30,
  };
}

export default function SettingsPage() {
  const settings = useSettings();
  const save = useSaveSettings();
  const [preferences, setPreferences] = useState<SettingsPreferences>(() => initialPreferences());
  const [activeAdvanced, setActiveAdvanced] = useState<string | null>(null);
  const [update, setUpdate] = useState<AvailableUpdate | null>(null);
  const [updateState, setUpdateState] = useState<"idle" | "checking" | "installing">("idle");
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [updateStatus, setUpdateStatus] = useState<string | null>(null);
  const [saveStatus, setSaveStatus] = useState<string | null>(null);
  const [confirmUpdate, setConfirmUpdate] = useState(false);
  const [showClearLogsConfirm, setShowClearLogsConfirm] = useState(false);
  const [isClearingLogs, setIsClearingLogs] = useState(false);
  const [clearLogsStatus, setClearLogsStatus] = useState<string | null>(null);
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

  const isTtlValid = Number.isInteger(preferences.upstreamTtlSec) && preferences.upstreamTtlSec >= 0;
  const isRetentionValid =
    Number.isInteger(preferences.logRetentionDays) &&
    preferences.logRetentionDays >= 0 &&
    preferences.logRetentionDays <= 3650;
  const isFormValid = isTtlValid && isRetentionValid;

  const savePreferences = () => {
    if (!isFormValid) return;
    setSaveStatus(null);
    save.mutate(preferences, {
      onSuccess: () => {
        setSaveStatus("设置已保存（已从引擎读回）"); toast.success("设置已保存");
      },
      onError: (error) => { setSaveStatus(`设置保存失败：${engineErrorMessage(error)}`); toast.error(`设置保存失败：${engineErrorMessage(error)}`); },
    });
  };

  const handleClearLogs = async () => {
    setIsClearingLogs(true);
    setClearLogsStatus(null);
    try {
      const res = await runEngine<LogsClearData>("logs:clear");
      setShowClearLogsConfirm(false);
      if (!res.ok) {
        const err = res.errors?.[0];
        const errorMsg = err?.message || "未知错误";
        const codePrefix = err?.code ? `[${err.code}] ` : "";
        const msg = `清理诊断日志失败：${codePrefix}${errorMsg}`;
        setClearLogsStatus(msg);
        toast.error(msg);
        return;
      }
      const count = res.data?.deleted ?? 0;
      const msg = `诊断日志已清理（共删除 ${count} 个日志文件）`;
      setClearLogsStatus(msg);
      toast.success(msg);
    } catch (err) {
      setShowClearLogsConfirm(false);
      const msg = `清理诊断日志失败：${engineErrorMessage(err)}`;
      setClearLogsStatus(msg);
      toast.error(msg);
    } finally {
      setIsClearingLogs(false);
    }
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

  if (activeAdvanced !== null) {
    const currentEntry = ADVANCED_ENTRIES.find((entry) => entry.key === activeAdvanced);
    return (
      <div className="space-y-4">
        <div className="flex items-center gap-3 border-b pb-3">
          <button
            type="button"
            data-testid="back-to-settings-button"
            onClick={() => setActiveAdvanced(null)}
            className="inline-flex items-center gap-1.5 rounded-md border bg-card px-3 py-1.5 text-xs font-medium text-foreground hover:bg-accent focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
          >
            <ArrowLeft className="h-3.5 w-3.5" aria-hidden />
            返回设置
          </button>
          <span className="text-xs text-muted-foreground">/ 高级管理 / {currentEntry?.title}</span>
        </div>
        {activeAdvanced === "aliases" && <AliasesPage />}
        {activeAdvanced === "secrets" && <SecretsPage />}
        {activeAdvanced === "updates" && <UpdatesPage />}
        {activeAdvanced === "policy" && <PolicyPage />}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <PageHeader title="设置" description="查看应用契约与运行路径，调整本机运行偏好。" />
      <AppearanceSection />
      <ConfigBackupSection />
      {data.maintenanceWarning && (
        <div
          role="alert"
          data-testid="maintenance-warning-banner"
          className="rounded-lg border border-warning/30 bg-warning/10 p-4 text-xs text-warning"
        >
          <p className="font-semibold">启动维护提示</p>
          <p className="mt-1">{data.maintenanceWarning}</p>
        </div>
      )}
      <section className="rounded-lg border bg-card p-5 shadow-sm">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-sm font-semibold text-foreground">应用与运行路径</h2>
          <button
            type="button"
            data-testid="clear-logs-button"
            onClick={() => setShowClearLogsConfirm(true)}
            disabled={isClearingLogs}
            className="inline-flex items-center gap-1.5 rounded-md border bg-card px-3 py-1.5 text-xs font-medium text-foreground hover:bg-accent focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 disabled:opacity-50"
          >
            <Trash2 className="h-3.5 w-3.5 text-muted-foreground" aria-hidden />
            清理诊断日志
          </button>
        </div>
        {clearLogsStatus && (
          <p
            role={clearLogsStatus.startsWith("清理诊断日志失败") ? "alert" : "status"}
            className={cn("mt-2 text-xs", clearLogsStatus.startsWith("清理诊断日志失败") ? "text-destructive" : "text-success")}
          >
            {clearLogsStatus}
          </p>
        )}
        <dl className="mt-3 grid gap-3 text-sm sm:grid-cols-2">
          {[["应用版本", data.appVersion], ["契约版本", data.contractVersion], ["资源目录", data.resourceDir], ["数据目录", data.dataDir], ["日志目录", data.logDir]].map(([label, value]) => (
            <div key={label} className="min-w-0"><dt className="text-xs text-muted-foreground">{label}</dt><dd className="mt-1 break-all font-mono text-xs text-foreground">{value}</dd></div>
          ))}
        </dl>
      </section>
      <section className="rounded-lg border bg-card p-5 shadow-sm">
        <h2 className="text-sm font-semibold text-foreground">运行偏好</h2>
        <div className="mt-4 space-y-4 text-sm">
          <label className="flex items-center gap-2"><input type="checkbox" checked={preferences.offline} onChange={(e) => setPreferences({ ...preferences, offline: e.target.checked })} />离线模式</label>
          <label className="flex items-center gap-3"><span>官方缓存 TTL（秒）</span><input aria-label="官方缓存 TTL（秒）" type="number" min={0} step={1} value={Number.isNaN(preferences.upstreamTtlSec) ? "" : preferences.upstreamTtlSec} onChange={(e) => setPreferences({ ...preferences, upstreamTtlSec: e.target.value === "" ? NaN : Number(e.target.value) })} className="w-32 rounded border px-2 py-1" /></label>
          <label className="flex items-center gap-3">
            <span>日志保留天数</span>
            <input
              aria-label="日志保留天数"
              type="number"
              min={0}
              max={3650}
              step={1}
              value={Number.isNaN(preferences.logRetentionDays) ? "" : preferences.logRetentionDays}
              onChange={(e) => setPreferences({ ...preferences, logRetentionDays: e.target.value === "" ? NaN : Number(e.target.value) })}
              className="w-32 rounded border px-2 py-1"
            />
            <span className="text-xs text-muted-foreground">（同时控制任务记录与诊断日志，0 为不自动清理，最大 3650 天）</span>
          </label>
          <button type="button" onClick={savePreferences} disabled={!changed || save.isPending || !isFormValid} className={cn("inline-flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium text-primary-foreground", !changed || save.isPending || !isFormValid ? "bg-muted text-muted-foreground" : "bg-primary hover:bg-primary/90")}><Save className="h-4 w-4" aria-hidden />保存设置</button>
          {saveStatus && <p role={saveStatus.startsWith("设置保存失败") ? "alert" : "status"} className={saveStatus.startsWith("设置保存失败") ? "text-sm text-destructive" : "text-sm text-success"}>{saveStatus}</p>}
        </div>
      </section>
      <section className="rounded-lg border bg-card p-5 shadow-sm">
        <div className="flex items-center justify-between gap-3"><div><h2 className="text-sm font-semibold text-foreground">应用更新</h2><p className="mt-1 text-xs text-muted-foreground">手动检查，确认后下载并安装。</p></div><button type="button" onClick={() => void checkForUpdate()} disabled={!data.updaterConfigured || updateState !== "idle"} className="inline-flex items-center gap-2 rounded-md bg-primary px-3 py-2 text-sm font-medium text-primary-foreground disabled:opacity-50"><RefreshCw className={cn("h-4 w-4", updateState === "checking" && "animate-spin")} aria-hidden />检查更新</button></div>
        {!data.updaterConfigured && <p role="status" className="mt-3 text-sm text-muted-foreground">更新服务未配置。</p>}
        {data.updaterConfigured && updateState === "idle" && !update && !updateError && <p role="status" className="mt-3 text-sm text-muted-foreground">{updateStatus ?? "尚未检查更新。"}</p>}
        {updateError && (
          <div className="mt-3 flex items-center justify-between gap-2 rounded-md bg-destructive/10 p-3 text-sm text-destructive">
            <p role="alert">{updateError}</p>
            <button
              type="button"
              onClick={() => void (update ? installUpdate() : checkForUpdate())}
              disabled={updateState !== "idle"}
              className="rounded border border-destructive/30 bg-card px-2.5 py-1 text-xs font-medium text-destructive hover:bg-destructive/10 disabled:opacity-50"
            >
              重试
            </button>
          </div>
        )}
        {update && <div className="mt-3 rounded-md border border-primary/20 bg-primary/10 p-3 text-sm"><p className="font-medium text-primary">发现新版本 {update.version}</p>{update.date && <p className="mt-1 text-xs text-primary">发布时间：{update.date}</p>}{update.body && <p className="mt-2 whitespace-pre-wrap text-xs text-primary">{update.body}</p>}<button type="button" onClick={() => setConfirmUpdate(true)} disabled={updateState !== "idle"} className="mt-3 inline-flex items-center gap-2 rounded-md bg-primary px-3 py-2 text-sm font-medium text-primary-foreground disabled:opacity-50">下载并安装</button></div>}
      </section>
      <section className="rounded-lg border bg-card p-5 shadow-sm">
        <div>
          <h2 className="text-sm font-semibold text-foreground">高级管理</h2>
          <p className="mt-1 text-xs text-muted-foreground">保留的系统诊断、快捷命令、密钥与更新管理入口。</p>
        </div>
        <div className="mt-4 grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
          {ADVANCED_ENTRIES.map((entry) => {
            const Icon = entry.icon;
            return (
              <button
                key={entry.key}
                type="button"
                data-testid={`advanced-entry-${entry.key}`}
                onClick={() => setActiveAdvanced(entry.key)}
                className="flex flex-col items-start rounded-lg border bg-muted/50 p-3.5 text-left transition-all hover:border-primary/40 hover:bg-primary/10 hover:shadow-xs focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
              >
                <div className="flex w-full items-center justify-between gap-2">
                  <span className="flex items-center gap-2 text-xs font-semibold text-foreground">
                    <Icon className="h-4 w-4 text-primary" aria-hidden />
                    {entry.title}
                  </span>
                </div>
                <p className="mt-1.5 text-xs text-muted-foreground line-clamp-2">{entry.description}</p>
              </button>
            );
          })}
        </div>
      </section>
      {confirmUpdate && update && <ConfirmModal title="安装应用更新" description={`将下载并安装版本 ${update.version}，完成后请重启应用。`} risk="network" confirmLabel="确认下载并安装" executing={updateState === "installing"} onConfirm={() => void installUpdate()} onClose={() => setConfirmUpdate(false)} />}
      {showClearLogsConfirm && (
        <ConfirmModal
          title="清理诊断日志"
          description="将删除本地所有 agent-manager-*.jsonl 结构化诊断日志文件。任务运行历史（SQLite 记录）与个性化配置不会受到影响。"
          risk="write-local"
          riskLabel="清理诊断日志"
          confirmLabel="确认清理"
          executing={isClearingLogs}
          onConfirm={() => void handleClearLogs()}
          onClose={() => setShowClearLogsConfirm(false)}
        />
      )}
    </div>
  );
}
