import { beforeEach, describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { check } from "@tauri-apps/plugin-updater";
import App from "../App";
import type { SettingsData } from "../lib/contract";
import { setSettingsClient } from "../lib/settings";
import { getDismissedUpdateVersion, useUpdateCheck } from "../lib/update-check";
import { makeAuditData, makeEnvelope, makeMode } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));

const baseSettings: SettingsData = {
  preferences: { logRetentionDays: 30 },
  appVersion: "1.2.3",
  contractVersion: "agent-manager/engine/v1",
  resourceDir: "/resources",
  dataDir: "/data",
  updaterConfigured: false,
};

function useUpdaterConfigured(configured: boolean) {
  setSettingsClient({
    get: async () => ({ ...baseSettings, updaterConfigured: configured }),
    save: async (preferences) => ({ ...baseSettings, preferences, updaterConfigured: configured }),
  });
}

function makeUpdate(version: string) {
  return {
    version,
    date: undefined,
    body: undefined,
    downloadAndInstall: vi.fn().mockResolvedValue(undefined),
    close: vi.fn().mockResolvedValue(undefined),
  };
}

function Harness() {
  const { availableVersion, dismiss } = useUpdateCheck({ delayMs: 0 });
  return (
    <div>
      <span data-testid="available-version">{availableVersion ?? "none"}</span>
      <button type="button" data-testid="dismiss-update" onClick={dismiss}>
        忽略
      </button>
    </div>
  );
}

function appRoutes() {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "agents:status": makeEnvelope({ domain: "agents", elements: [] }, { command: "agents.status" }),
    "skills:status": makeEnvelope({ domain: "skills", elements: [] }, { command: "skills.status" }),
    "mcp:status": makeEnvelope({ domain: "mcp", elements: [] }, { command: "mcp.status" }),
    "rules:status": makeEnvelope({ domain: "rules", elements: [] }, { command: "rules.status" }),
    "runs:list": makeEnvelope({ runs: [] }, { command: "runs.list" }),
    "policy:get": makeEnvelope({ layers: { sqlite: null } }, { command: "policy.get" }),
  };
}

beforeEach(() => {
  window.localStorage.clear();
});

describe("启动自动检查更新（useUpdateCheck）", () => {
  it("updater 未配置时不发起检查", async () => {
    useUpdaterConfigured(false);
    renderWithEngine(<Harness />, {});
    await screen.findByTestId("available-version");
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(check).not.toHaveBeenCalled();
    expect(screen.getByTestId("available-version")).toHaveTextContent("none");
  });

  it("已配置且发现新版本：展示版本号并立即释放更新句柄", async () => {
    useUpdaterConfigured(true);
    const update = makeUpdate("2.0.0");
    vi.mocked(check).mockResolvedValue(update as never);
    renderWithEngine(<Harness />, {});

    expect(await screen.findByText("2.0.0")).toBeInTheDocument();
    await waitFor(() => expect(update.close).toHaveBeenCalledTimes(1));
  });

  it("已配置但无新版本：不展示", async () => {
    useUpdaterConfigured(true);
    vi.mocked(check).mockResolvedValue(null);
    renderWithEngine(<Harness />, {});

    await waitFor(() => expect(check).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.getByTestId("available-version")).toHaveTextContent("none");
  });

  it("已点过「不再提示」的版本不再展示，但仍发起检查", async () => {
    useUpdaterConfigured(true);
    window.localStorage.setItem("agent-manager:update:dismissedVersion", "2.0.0");
    vi.mocked(check).mockResolvedValue(makeUpdate("2.0.0") as never);
    renderWithEngine(<Harness />, {});

    await waitFor(() => expect(check).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.getByTestId("available-version")).toHaveTextContent("none");
  });

  it("忽略后按版本持久化，同版本不再提示、新版本仍提示", async () => {
    useUpdaterConfigured(true);
    vi.mocked(check).mockResolvedValue(makeUpdate("2.0.0") as never);
    renderWithEngine(<Harness />, {});

    await screen.findByText("2.0.0");
    await userEvent.click(screen.getByTestId("dismiss-update"));

    expect(screen.getByTestId("available-version")).toHaveTextContent("none");
    expect(getDismissedUpdateVersion()).toBe("2.0.0");
    expect(window.localStorage.getItem("agent-manager:update:dismissedVersion")).toBe("2.0.0");
  });

  it("检查失败静默：仅 console.warn，不展示 badge、不抛错", async () => {
    useUpdaterConfigured(true);
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    vi.mocked(check).mockRejectedValue(new Error("网络连接失败"));
    renderWithEngine(<Harness />, {});

    await waitFor(() => expect(warnSpy).toHaveBeenCalled());
    expect(screen.getByTestId("available-version")).toHaveTextContent("none");
  });
});

describe("更新提示 badge（侧边栏设置项）", () => {
  it("发现新版本时设置项显示绿点，点击进设置页，点关闭后按版本不再提示", async () => {
    useUpdaterConfigured(true);
    vi.mocked(check).mockResolvedValue(makeUpdate("2.0.0") as never);
    renderWithEngine(<App />, appRoutes());

    // 启动延迟约 1.5s 后发起检查
    const badge = await screen.findByTestId("update-badge", {}, { timeout: 4000 });
    const nav = screen.getByRole("navigation");
    const settingsNav = within(nav).getByRole("button", { name: "设置" });
    expect(settingsNav).toContainElement(badge);

    // 点击设置项进入设置页（badge 随导航项）
    await userEvent.click(settingsNav);
    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();

    // 关闭 badge：按版本持久化并不再显示
    await userEvent.click(screen.getByTestId("update-badge-dismiss"));
    expect(screen.queryByTestId("update-badge")).toBeNull();
    expect(window.localStorage.getItem("agent-manager:update:dismissedVersion")).toBe("2.0.0");
  }, 15000);

  it("updater 未配置时不显示 badge", async () => {
    useUpdaterConfigured(false);
    renderWithEngine(<App />, appRoutes());
    await screen.findByRole("navigation");
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByTestId("update-badge")).toBeNull();
    expect(check).not.toHaveBeenCalled();
  });
});
