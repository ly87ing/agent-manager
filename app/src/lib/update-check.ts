import { useCallback, useEffect, useState } from "react";
import { useSettings } from "./settings";

/**
 * 启动后的自动更新检查（对应设置页手动「检查更新」之外的静默通道）：
 * 仅当 updater 已配置时延迟发起；发现新版本时由侧边栏「设置」项上的 badge 呈现，
 * 失败静默（console.warn），不打 toast 骚扰。检查到句柄后立即 close 释放，
 * 实际的下载安装仍在设置页手动完成（那里会重新 check 一次拿到新句柄）。
 */

const DISMISS_KEY = "agent-manager:update:dismissedVersion";

export function getDismissedUpdateVersion(): string | null {
  try {
    return window.localStorage.getItem(DISMISS_KEY);
  } catch {
    return null;
  }
}

function persistDismissedVersion(version: string): void {
  try {
    window.localStorage.setItem(DISMISS_KEY, version);
  } catch {
    // localStorage 不可用时忽略，退化为本次会话内不再提示
  }
}

export function useUpdateCheck({ delayMs = 1500 }: { delayMs?: number } = {}) {
  const settings = useSettings();
  const configured = settings.data?.updaterConfigured ?? false;
  const [availableVersion, setAvailableVersion] = useState<string | null>(null);

  useEffect(() => {
    if (!configured) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      void (async () => {
        try {
          // 浏览器/测试环境无插件时动态 import 直接失败，走 catch 静默
          const { check } = await import("@tauri-apps/plugin-updater");
          const found = await check();
          if (!found) return;
          const close = () => void found.close().catch(() => {});
          if (cancelled) {
            close();
            return;
          }
          close();
          // 已在该版本点过「不再提示」：本次不再展示 badge（设置页手动检查不受此影响）
          if (found.version !== getDismissedUpdateVersion()) {
            setAvailableVersion(found.version);
          }
        } catch (error) {
          console.warn("自动检查更新失败（已忽略）：", error);
        }
      })();
    }, delayMs);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [configured, delayMs]);

  const dismiss = useCallback(() => {
    setAvailableVersion((version) => {
      if (version) persistDismissedVersion(version);
      return null;
    });
  }, []);

  return { availableVersion, dismiss };
}
