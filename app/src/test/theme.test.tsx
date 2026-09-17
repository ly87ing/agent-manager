import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  applyTheme,
  getStoredTheme,
  initTheme,
  resolveTheme,
  THEME_STORAGE_KEY,
  ThemeProvider,
  useTheme,
} from "../lib/theme";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

type MediaListener = (event: { matches: boolean }) => void;

/** 可控的 matchMedia 桩：setDark 触发 change 监听器。 */
function stubMatchMedia(initialDark: boolean) {
  let matches = initialDark;
  const listeners = new Set<MediaListener>();
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches,
    media: query,
    addEventListener: (_event: string, cb: MediaListener) => listeners.add(cb),
    removeEventListener: (_event: string, cb: MediaListener) => listeners.delete(cb),
    addListener: () => {},
    removeListener: () => {},
    onchange: null,
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  return {
    setDark(dark: boolean) {
      matches = dark;
      for (const cb of listeners) cb({ matches });
    },
  };
}

function ThemeProbe() {
  const { theme, resolvedTheme, setTheme } = useTheme();
  return (
    <button
      type="button"
      data-testid="probe"
      data-theme={theme}
      data-resolved={resolvedTheme}
      onClick={() => setTheme("dark")}
    >
      切换
    </button>
  );
}

beforeEach(() => {
  window.localStorage.clear();
  document.documentElement.classList.remove("dark");
  // 浏览器/测试环境没有该 Tauri 命令：始终拒绝，验证调用方吞掉失败
  vi.mocked(invoke).mockReset().mockRejectedValue(new Error("not running in tauri"));
});

describe("getStoredTheme", () => {
  it("无存储时默认 system", () => {
    expect(getStoredTheme()).toBe("system");
  });

  it("读取合法存储值", () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, "dark");
    expect(getStoredTheme()).toBe("dark");
  });

  it("非法存储值回落 system", () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, "blue");
    expect(getStoredTheme()).toBe("system");
  });
});

describe("applyTheme / resolveTheme", () => {
  it("dark 加 .dark，light 去掉 .dark", () => {
    expect(applyTheme("dark")).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(applyTheme("light")).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("system 跟随 matchMedia（暗）", () => {
    stubMatchMedia(true);
    expect(resolveTheme("system")).toBe("dark");
    applyTheme("system");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("system 跟随 matchMedia（亮）", () => {
    stubMatchMedia(false);
    expect(resolveTheme("system")).toBe("light");
    applyTheme("system");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("matchMedia 不可用时 system 按亮色处理", () => {
    // @ts-expect-error 模拟无 matchMedia 的环境
    window.matchMedia = undefined;
    expect(resolveTheme("system")).toBe("light");
  });
});

describe("initTheme（首屏预置）", () => {
  it("render 前同步按存储值预置 .dark", () => {
    stubMatchMedia(false);
    window.localStorage.setItem(THEME_STORAGE_KEY, "dark");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(initTheme()).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });
});

describe("ThemeProvider", () => {
  it("setTheme 立即切 .dark、持久化到 localStorage，并 best-effort 同步原生（失败吞掉）", async () => {
    stubMatchMedia(false);
    window.localStorage.setItem(THEME_STORAGE_KEY, "light");
    render(
      <ThemeProvider>
        <ThemeProbe />
      </ThemeProvider>,
    );
    const probe = screen.getByTestId("probe");
    expect(probe.dataset.theme).toBe("light");
    expect(probe.dataset.resolved).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);

    await userEvent.click(probe);

    expect(probe.dataset.theme).toBe("dark");
    expect(probe.dataset.resolved).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_window_theme", { theme: "dark" });
    // invoke 拒绝被吞掉：无 unhandled rejection，测试正常走完即证明
  });

  it("system 模式跟随操作系统亮暗变化", async () => {
    const media = stubMatchMedia(false);
    window.localStorage.setItem(THEME_STORAGE_KEY, "system");
    render(
      <ThemeProvider>
        <ThemeProbe />
      </ThemeProvider>,
    );
    const probe = screen.getByTestId("probe");
    expect(probe.dataset.resolved).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);

    act(() => media.setDark(true));
    expect(probe.dataset.resolved).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);

    act(() => media.setDark(false));
    expect(probe.dataset.resolved).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("非 system 模式不响应系统亮暗变化", () => {
    const media = stubMatchMedia(false);
    window.localStorage.setItem(THEME_STORAGE_KEY, "dark");
    render(
      <ThemeProvider>
        <ThemeProbe />
      </ThemeProvider>,
    );
    act(() => media.setDark(true));
    expect(screen.getByTestId("probe").dataset.theme).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });
});
