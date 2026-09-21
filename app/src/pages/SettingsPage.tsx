import { useEffect, useRef, useState } from "react";
import {
  Activity,
  ArrowLeft,
  KeyRound,
  Link2,
  Monitor,
  Moon,
  RefreshCw,
  Save,
  Sun,
} from "lucide-react";
import { toast } from "sonner";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { ConfirmModal } from "../components/ConfirmModal";
import { engineErrorMessage } from "../lib/engine";
import type { SettingsData, SettingsPreferences } from "../lib/contract";
import { useSaveSettings, useSettings } from "../lib/settings";
import { useTheme, type Theme } from "../lib/theme";
import { cn } from "../lib/utils";
import { check } from "@tauri-apps/plugin-updater";
import { ConfigBackupSection } from "../components/ConfigBackupSection";
import AliasesPage from "./AliasesPage";
import ClientDiagnosticsPage from "./ClientDiagnosticsPage";
import SecretsPage from "./SecretsPage";

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
    key: "client-diagnostics",
    title: "客户端诊断",
    description: "只读查看 AI CLI 的版本与安装来源。",
    icon: Activity,
  },
  {
    key: "aliases",
    title: "终端快捷命令",
    description: "每条命令的启动方式：时区、语言与固定参数。",
    icon: Link2,
  },
  {
    key: "secrets",
    title: "密钥",
    description: "填写 MCP 服务需要的取值，并查看哪些还没配。",
    icon: KeyRound,
  },
];

function initialPreferences(data?: SettingsData): SettingsPreferences {
  return { logRetentionDays: data?.preferences?.logRetentionDays ?? 30 };
}

export default function SettingsPage({
  initialAdvanced,
  onNavigate,
}: {
  initialAdvanced?: string | null;
  onNavigate?: (page: string) => void;
} = {}) {
  const settings = useSettings();
  const save = useSaveSettings();
  const [preferences, setPreferences] = useState<SettingsPreferences>(() => initialPreferences());
  // 命令面板可以直达二级页（密钥、终端快捷命令），不用先进设置再找一次
  const [activeAdvanced, setActiveAdvanced] = useState<string | null>(initialAdvanced ?? null);
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

  const isRetentionValid =
    Number.isInteger(preferences.logRetentionDays) &&
    preferences.logRetentionDays >= 0 &&
    preferences.logRetentionDays <= 3650;
  const isFormValid = isRetentionValid;

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
        {activeAdvanced === "client-diagnostics" && <ClientDiagnosticsPage />}
        {activeAdvanced === "secrets" && <SecretsPage />}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <PageHeader title="设置" description="外观、配置备份与本机运行路径。" />
      <AppearanceSection />
      <ConfigBackupSection onGoSync={onNavigate ? () => onNavigate("dashboard") : undefined} />
      <section className="rounded-lg border bg-card p-5 shadow-sm">
        <h2 className="text-sm font-semibold text-foreground">应用与运行路径</h2>
        <dl className="mt-3 grid gap-3 text-sm sm:grid-cols-2">
          {[["应用版本", data.appVersion], ["资源目录", data.resourceDir], ["数据目录", data.dataDir]].map(([label, value]) => (
            <div key={label} className="min-w-0"><dt className="text-xs text-muted-foreground">{label}</dt><dd className="mt-1 break-all font-mono text-xs text-foreground">{value}</dd></div>
          ))}
        </dl>
        <div className="mt-4 space-y-3 border-t pt-4 text-sm">
          <label className="flex flex-wrap items-center gap-3">
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
            <span className="text-xs text-muted-foreground">（操作记录保留多久，0 为不自动清理，最大 3650 天）</span>
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
          <p className="mt-1 text-xs text-muted-foreground">不常用的诊断与本机配置入口。</p>
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
    </div>
  );
}
