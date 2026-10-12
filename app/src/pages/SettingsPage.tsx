import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ArrowLeft,
  BookOpen,
  ChevronRight,
  History,
  KeyRound,
  Monitor,
  Moon,
  RefreshCw,
  Save,
  Sun,
} from "lucide-react";
import { toast } from "sonner";
import { PageError, PageLoading } from "../components/PageState";
import { ConfirmModal } from "../components/ConfirmModal";
import { engineErrorMessage } from "../lib/engine";
import type { SettingsData, SettingsPreferences } from "../lib/contract";
import { useSaveSettings, useSettings } from "../lib/settings";
import { useAutostart, useSetAutostart } from "../lib/autostart";
import { useTheme, type Theme } from "../lib/theme";
import { cn } from "../lib/utils";
import { ConfigBackupSection } from "../components/ConfigBackupSection";
import SecretsPage from "./SecretsPage";
import TipsPage from "./TipsPage";

const THEME_OPTIONS: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: "light", label: "浅色", icon: Sun },
  { value: "dark", label: "深色", icon: Moon },
  { value: "system", label: "跟随系统", icon: Monitor },
];

function ThemeControl() {
  const { theme, setTheme } = useTheme();
  return (
    <div className="inline-flex rounded-lg border bg-muted p-0.5" role="group" aria-label="外观主题">
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
              "inline-flex items-center gap-1.5 rounded-md px-3 py-1 text-xs transition-colors",
              active ? "bg-card font-medium text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground",
            )}
          >
            <Icon className="h-3.5 w-3.5" aria-hidden />
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

function SettingsGroup({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="overflow-clip rounded-lg border bg-card" aria-label={title}>
      <h2 className="border-b bg-muted px-4 py-2 text-xs font-medium text-foreground">{title}</h2>
      <div className="divide-y">{children}</div>
    </section>
  );
}

function AutostartControl() {
  const state = useAutostart();
  const change = useSetAutostart();
  const error = state.error ?? change.error;
  return (
    <>
      <div className="flex items-center gap-2 text-xs" role="group" aria-label="旧版登录项">
        {!state.isPending && !state.isError && <span className="text-muted-foreground">{state.data ? "已启用旧登录项" : "未启用开机启动"}</span>}
        <button type="button" disabled={state.isPending || state.isError || change.isPending}
          onClick={() => change.mutate(false)} className="rounded-md border px-3 py-1 hover:bg-accent disabled:opacity-50">
          移除旧登录项
        </button>
      </div>
      {state.isPending && <p role="status" className="text-xs text-muted-foreground">正在读取登录项…</p>}
      {error && <div className="flex w-full flex-wrap items-center gap-2 text-xs text-destructive">
        <p role="alert">开机启动设置失败：{engineErrorMessage(error)}</p>
        <button type="button" onClick={() => { change.reset(); void state.refetch(); }} className="rounded border px-2 py-1">重新读取</button>
      </div>}
    </>
  );
}

/** 一行一项：左边名称与一句说明，右边控件。 */
function SettingsRow({ label, note, children }: { label: string; note?: string; children?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
      <div className="min-w-0 flex-1 basis-48">
        <div className="text-sm">{label}</div>
        {note && <div className="text-xs text-muted-foreground">{note}</div>}
      </div>
      {children}
    </div>
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
    key: "tips",
    title: "更多技巧",
    description: "多账号与终端别名的使用经验，只读分享。",
    icon: BookOpen,
  },
  {
    key: "runs",
    title: "操作记录",
    description: "每次写入的记录：做了什么、成功与否、失败原因。",
    icon: History,
  },
  {
    key: "secrets",
    title: "密钥",
    description: "选择外部密钥文件，填写取值并查看哪些还没配。",
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
    if (!data.updaterConfigured || data.runtimeWarning) return;
    setUpdateState("checking");
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
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
    if (!update || data.runtimeWarning) return;
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
          <span className="text-xs text-muted-foreground">/ 工具 / {currentEntry?.title}</span>
        </div>
        {activeAdvanced === "secrets" && <SecretsPage />}
        {activeAdvanced === "tips" && <TipsPage />}
      </div>
    );
  }

  const openTool = (key: string) =>
    key === "runs" && onNavigate ? onNavigate(key) : setActiveAdvanced(key);

  return (
    <div className="space-y-3">
      <h1 className="text-lg font-semibold tracking-tight">设置</h1>

      <SettingsGroup title="通用">
        <SettingsRow label="外观" note="立即生效，保存在本机">
          <ThemeControl />
        </SettingsRow>
        <SettingsRow label="旧版开机启动" note="当前版本不设置开机启动；可移除以前创建且未被改动的登录项">
          <AutostartControl />
        </SettingsRow>
        <SettingsRow label="操作记录保留" note="0 为不自动清理，最多 3650 天">
          <div className="flex items-center gap-2">
            <input
              aria-label="日志保留天数"
              type="number"
              min={0}
              max={3650}
              step={1}
              value={Number.isNaN(preferences.logRetentionDays) ? "" : preferences.logRetentionDays}
              onChange={(e) => setPreferences({ ...preferences, logRetentionDays: e.target.value === "" ? NaN : Number(e.target.value) })}
              aria-invalid={!isRetentionValid}
              aria-describedby={!isRetentionValid ? "log-retention-error" : undefined}
              className={cn("w-20 rounded-md border border-input bg-background px-2 py-1 text-right text-sm tabular-nums", !isRetentionValid && "border-destructive")}
            />
            <span className="text-sm text-muted-foreground">天</span>
            {changed && (
              <button
                type="button"
                onClick={savePreferences}
                disabled={save.isPending || !isFormValid}
                className="inline-flex items-center gap-1 rounded-md bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              >
                <Save className="h-3.5 w-3.5" aria-hidden />
                保存设置
              </button>
            )}
          </div>
          {!isRetentionValid && (
            <p id="log-retention-error" role="alert" className="w-full text-right text-xs text-destructive">
              请输入 0 到 3650 之间的整数。
            </p>
          )}
          {saveStatus && (
            <p role={saveStatus.startsWith("设置保存失败") ? "alert" : "status"} className={cn("w-full text-right text-xs", saveStatus.startsWith("设置保存失败") ? "text-destructive" : "text-success")}>
              {saveStatus}
            </p>
          )}
        </SettingsRow>
        {data.updaterConfigured && (
          <SettingsRow label="应用更新" note="手动检查，确认后下载并安装">
            <button
              type="button"
              onClick={() => void checkForUpdate()}
              disabled={updateState !== "idle" || Boolean(data.runtimeWarning)}
              className="inline-flex items-center gap-1.5 rounded-md border px-3 py-1 text-sm hover:bg-accent disabled:opacity-50"
            >
              <RefreshCw className={cn("h-3.5 w-3.5", updateState === "checking" && "animate-spin")} aria-hidden />
              检查更新
            </button>
            {data.runtimeWarning && <p role="status" className="w-full text-xs text-muted-foreground">安装到“应用程序”后可检查更新。</p>}
            {!data.runtimeWarning && updateState === "idle" && !update && !updateError && (
              <p role="status" className="w-full text-right text-xs text-muted-foreground">{updateStatus ?? "尚未检查更新。"}</p>
            )}
            {updateError && (
              <div className="flex w-full items-center justify-between gap-2 rounded-md bg-destructive/10 p-2.5 text-sm text-destructive">
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
            {update && (
              <div className="w-full rounded-md border border-primary/20 bg-primary/10 p-3 text-sm">
                <p className="font-medium text-primary">发现新版本 {update.version}</p>
                {update.date && <p className="mt-1 text-xs text-primary">发布时间：{update.date}</p>}
                {update.body && <p className="mt-2 whitespace-pre-wrap text-xs text-primary">{update.body}</p>}
                <button type="button" onClick={() => setConfirmUpdate(true)} disabled={updateState !== "idle"} className="mt-3 inline-flex items-center gap-2 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground disabled:opacity-50">
                  下载并安装
                </button>
              </div>
            )}
          </SettingsRow>
        )}
      </SettingsGroup>

      <SettingsGroup title="工具">
        {ADVANCED_ENTRIES.map((entry) => {
          const Icon = entry.icon;
          return (
            <button
              key={entry.key}
              type="button"
              data-testid={`advanced-entry-${entry.key}`}
              onClick={() => openTool(entry.key)}
              className="flex w-full items-center gap-3 px-4 py-3 text-left hover:bg-accent/40 focus:outline-none focus-visible:bg-accent/60"
            >
              <Icon className="h-4 w-4 shrink-0 text-primary" aria-hidden />
              <span className="min-w-0 flex-1">
                <span className="block text-sm">{entry.title}</span>
                <span className="block truncate text-xs text-muted-foreground">{entry.description}</span>
              </span>
              <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden />
            </button>
          );
        })}
      </SettingsGroup>

      <ConfigBackupSection />

      <SettingsGroup title="关于">
        {[["应用版本", data.appVersion], ["数据目录", data.dataDir], ["资源目录", data.resourceDir]].map(([label, value]) => (
          <SettingsRow key={label} label={label}>
            <span className="min-w-0 max-w-[60%] break-all text-right font-mono text-xs text-muted-foreground">{value}</span>
          </SettingsRow>
        ))}
      </SettingsGroup>
      {confirmUpdate && update && <ConfirmModal title="安装应用更新" description={`将下载并安装版本 ${update.version}，完成后请重启应用。`} risk="network" confirmLabel="确认下载并安装" executing={updateState === "installing"} onConfirm={() => void installUpdate()} onClose={() => setConfirmUpdate(false)} />}
    </div>
  );
}
