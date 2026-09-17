import { invoke } from "@tauri-apps/api/core";
import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { Toaster } from "sonner";

export type Theme = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

export const THEME_STORAGE_KEY = "agent-manager:theme";

const THEME_VALUES: readonly Theme[] = ["light", "dark", "system"];

/** 读取持久化主题；非法值/读取失败回落到 system。 */
export function getStoredTheme(storageKey: string = THEME_STORAGE_KEY): Theme {
  try {
    const stored = window.localStorage.getItem(storageKey);
    if (stored === "light" || stored === "dark" || stored === "system") {
      return stored;
    }
  } catch {
    // localStorage 不可用（隐私模式等）时按默认处理
  }
  return "system";
}

function prefersDark(): boolean {
  return (
    typeof window.matchMedia === "function" && window.matchMedia("(prefers-color-scheme: dark)").matches
  );
}

/** system 解析为操作系统当前亮暗。 */
export function resolveTheme(theme: Theme): ResolvedTheme {
  if (theme === "system") {
    return prefersDark() ? "dark" : "light";
  }
  return theme;
}

/** 把主题应用到 <html>（.dark class），返回解析后的亮暗态。 */
export function applyTheme(theme: Theme): ResolvedTheme {
  const resolved = resolveTheme(theme);
  document.documentElement.classList.toggle("dark", resolved === "dark");
  return resolved;
}

/**
 * 首屏防闪烁：在 React render 之前同步预置 .dark。
 * CSP 为 script-src 'self'（无 unsafe-inline），index.html 内联脚本会被生产环境拦截，
 * 因此预置只能放在 main.tsx 的模块顶层执行。
 */
export function initTheme(storageKey: string = THEME_STORAGE_KEY): Theme {
  const theme = getStoredTheme(storageKey);
  applyTheme(theme);
  return theme;
}

function syncNativeTheme(theme: Theme): void {
  // best-effort 同步原生标题栏；浏览器/测试环境没有该 Tauri 命令，失败吞掉
  void invoke("set_window_theme", { theme }).catch(() => {});
}

interface ThemeContextValue {
  theme: Theme;
  resolvedTheme: ResolvedTheme;
  setTheme: (theme: Theme) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(() => getStoredTheme());
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() => resolveTheme(theme));

  useEffect(() => {
    setResolvedTheme(applyTheme(theme));
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, theme);
    } catch {
      // 持久化失败不阻塞界面
    }
    syncNativeTheme(theme);
  }, [theme]);

  // system 模式下跟随操作系统亮暗变化
  useEffect(() => {
    if (theme !== "system" || typeof window.matchMedia !== "function") {
      return;
    }
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setResolvedTheme(applyTheme("system"));
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, [theme]);

  const value = useMemo<ThemeContextValue>(
    () => ({
      theme,
      resolvedTheme,
      setTheme: (next: Theme) => {
        if (THEME_VALUES.includes(next)) {
          setThemeState(next);
        }
      },
    }),
    [theme, resolvedTheme],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const context = useContext(ThemeContext);
  if (!context) {
    throw new Error("useTheme must be used within a ThemeProvider");
  }
  return context;
}

/** 跟随当前主题的 toast 出口。 */
export function ThemedToaster() {
  const { resolvedTheme } = useTheme();
  return <Toaster richColors position="top-right" theme={resolvedTheme} />;
}
