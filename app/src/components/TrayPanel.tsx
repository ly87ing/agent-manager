import { invoke, isTauri } from "@tauri-apps/api/core";
import { Check, Copy, ExternalLink, LogOut, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { buildAccounts } from "../lib/accounts";
import type { StatusData } from "../lib/contract";
import { useEngineQuery } from "../lib/queries";
import { cn } from "../lib/utils";
import { QuotaCells } from "./QuotaCells";
import { accountCommand } from "../lib/accountCommand";
import { defaultAuthShell } from "../lib/profileDefinition";

const REFRESH_MS = 10 * 60_000;

async function hidePanel() {
  if (!isTauri()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().hide();
}

/**
 * 托盘面板：点托盘图标弹出的小窗口，按助手列出全部账号（启动命令 · 额度 · 重置倒计时）。
 *
 * 只读加复制：复制启动命令后到终端里自己运行，面板不切换账号、不写任何配置。
 */
export function TrayPanel() {
  const agents = useEngineQuery<StatusData>("agents:status", [], { refetchInterval: REFRESH_MS });
  const profiles = useEngineQuery<StatusData>("profiles:status", [], { refetchInterval: REFRESH_MS });
  const aliases = useEngineQuery<StatusData>("aliases:status");
  const [now, setNow] = useState(() => new Date());
  const [copied, setCopied] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  const refresh = () => {
    setNow(new Date());
    void agents.refetch();
    void profiles.refetch();
  };

  useEffect(() => {
    // 每次弹出都重读一次本机记录；平时按分钟走倒计时
    const onFocus = () => refresh();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") void hidePanel();
    };
    const timer = window.setInterval(() => setNow(new Date()), 60_000);
    window.addEventListener("focus", onFocus);
    window.addEventListener("keydown", onKey);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("keydown", onKey);
    };
    // refresh 只用到稳定的 refetch，挂一次即可
  }, []);

  const aliasValues = Object.fromEntries(
    (aliases.data?.ok ? aliases.data.data.aliases ?? [] : [])
      .filter((item) => item.value !== null)
      .map((item) => [item.name, item.value as string]),
  );
  const groups = agents.data?.ok
    ? buildAccounts(agents.data.data.elements, profiles.data?.ok ? profiles.data.data.elements : [], aliasValues)
    : [];

  const copy = async (element: string) => {
    try {
      const command = await accountCommand(element, "launch", defaultAuthShell());
      await navigator.clipboard.writeText(command.terminal_command);
      setFailed(false);
      setCopied(element);
      window.setTimeout(() => setCopied((value) => (value === element ? null : value)), 1500);
    } catch {
      setFailed(true);
    }
  };

  const openMain = () => {
    if (isTauri()) void invoke("tray_panel_open_main");
  };

  return (
    <div data-testid="tray-panel" className="flex h-screen flex-col overflow-hidden border bg-background text-foreground">
      <header className="flex items-center gap-2 border-b px-3 py-2">
        <span className="text-sm font-semibold">账号与额度</span>
        <button
          type="button"
          data-testid="tray-panel-refresh"
          title="重新读取本机额度记录"
          onClick={refresh}
          className="ml-auto rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <RefreshCw className={cn("h-3.5 w-3.5", (agents.isFetching || profiles.isFetching) && "animate-spin")} aria-hidden />
          <span className="sr-only">刷新</span>
        </button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {agents.isPending && <p className="px-3 py-6 text-center text-xs text-muted-foreground">读取中…</p>}
        {agents.data && !agents.data.ok && <p className="px-3 py-6 text-center text-xs text-destructive">账号读取失败，打开主界面查看</p>}
        {groups.map((group) => (
          <section key={group.agentId} aria-label={group.label}>
            <h2 className="bg-muted px-3 py-1 text-[11px] font-medium text-muted-foreground">{group.label}</h2>
            <ul className="divide-y">
              {group.accounts.map((account) => (
                <li key={account.elementId} data-testid={`tray-panel-row-${account.elementId}`} className="space-y-1.5 px-3 py-2">
                  <div className="flex items-center gap-2">
                    <span className="min-w-0 truncate text-sm font-medium">{account.isMain ? "主账号" : account.name}</span>
                    {account.command ? (
                      <code className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground" title={account.command}>
                        {account.command}
                      </code>
                    ) : (
                      <span className="flex-1 text-xs text-warning">没有启动命令</span>
                    )}
                    <button
                      type="button"
                      data-testid={`tray-panel-copy-${account.elementId}`}
                      disabled={Boolean(account.blockedReason || account.mainHasThirdParty)}
                      onClick={() => void copy(account.elementId)}
                      className="inline-flex w-16 shrink-0 items-center justify-center gap-1 rounded-md border px-2 py-1 text-xs hover:bg-accent disabled:opacity-40"
                    >
                      {copied === account.elementId ? (
                        <><Check className="h-3.5 w-3.5 text-success" aria-hidden /> 已复制</>
                      ) : (
                        <><Copy className="h-3.5 w-3.5" aria-hidden /> 复制</>
                      )}
                    </button>
                  </div>
                  <QuotaCells usage={account.usage} testId={`tray-panel-quota-${account.elementId}`} now={now} />
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
      {failed && <p role="alert" className="border-t px-3 py-1.5 text-xs text-destructive">复制失败，请打开主界面复制</p>}
      <footer className="flex items-center gap-1 border-t px-2 py-1.5 text-xs">
        <button
          type="button"
          data-testid="tray-panel-open-main"
          onClick={openMain}
          className="inline-flex items-center gap-1 rounded px-2 py-1 hover:bg-accent"
        >
          <ExternalLink className="h-3.5 w-3.5" aria-hidden /> 打开主界面
        </button>
        <button
          type="button"
          data-testid="tray-panel-quit"
          onClick={() => isTauri() && void invoke("tray_panel_quit")}
          className="ml-auto inline-flex items-center gap-1 rounded px-2 py-1 text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <LogOut className="h-3.5 w-3.5" aria-hidden /> 退出
        </button>
      </footer>
    </div>
  );
}
