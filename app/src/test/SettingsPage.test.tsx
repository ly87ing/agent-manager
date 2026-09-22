import { afterEach, describe, expect, it, vi } from "vitest";
import { toast } from "sonner";
import { makeAuditData, makeEnvelope, makeMode } from "./fixtures";
import { useState } from "react";
import { check } from "@tauri-apps/plugin-updater";
import SettingsPage from "../pages/SettingsPage";
import App from "../App";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { SettingsData } from "../lib/contract";

vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
import { setSettingsClient, settingsSchema } from "../lib/settings";

const settings: SettingsData = {
  preferences: { logRetentionDays: 30 },
  appVersion: "1.2.3",
  contractVersion: "agent-manager/engine/v1",
  resourceDir: "/resources",
  dataDir: "/data",
  updaterConfigured: false,
};

describe("SettingsPage", () => {
  it("应用从设置加载态进入主界面时 Hook 顺序保持一致", async () => {
    setSettingsClient({ get: async () => settings, save: async () => settings });
    renderWithEngine(<App />, {});
    expect(screen.getByText("正在读取应用设置…")).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "助手" })).toBeInTheDocument();
    expect(screen.queryByText("正在读取应用设置…")).toBeNull();
  });
  afterEach(() => { setSettingsClient(null); window.history.pushState({}, "", "/"); vi.clearAllMocks(); });
  it("读取并显示设置，保存成功后以引擎读回值为准", async () => {
    let saved = false;
    let savedPreferences = settings.preferences;
    setSettingsClient({
      get: async () => ({ ...settings, preferences: savedPreferences }),
      save: async (preferences) => {
        saved = true;
        savedPreferences = preferences;
        return { ...settings, preferences: savedPreferences };
      },
    });
    renderWithEngine(<SettingsPage />, {});
    expect(await screen.findByText("1.2.3")).toBeInTheDocument();
    const retention = screen.getByLabelText("日志保留天数");
    await userEvent.clear(retention);
    await userEvent.type(retention, "60");
    await userEvent.click(screen.getByRole("button", { name: "保存设置" }));
    expect(await screen.findByText("设置已保存")).toBeInTheDocument();
    expect(saved).toBe(true);
    expect(retention).toHaveValue(60);
  });

  it("更新未配置时显示状态且不发起 updater 请求", async () => {
    setSettingsClient({ get: async () => settings, save: async (preferences) => ({ ...settings, preferences }) });
    const { calls } = renderWithEngine(<SettingsPage />, {});
    await screen.findByText("更新服务未配置。");
    await userEvent.click(screen.getByRole("button", { name: "检查更新" }));
    expect(calls).toEqual([]);
  });

  it("DEV mock 保存后再次读取仍保留偏好", async () => {
    window.history.pushState({}, "", "?mock=1");
    renderWithEngine(<SettingsPage />, {});
    const retention = await screen.findByLabelText("日志保留天数");
    await userEvent.clear(retention);
    await userEvent.type(retention, "45");
    await userEvent.click(screen.getByRole("button", { name: "保存设置" }));
    expect(await screen.findByText("设置已保存")).toBeInTheDocument();
    expect(retention).toHaveValue(45);
    window.history.pushState({}, "", "/");
  });

  it("保存失败显示内联错误", async () => {
    setSettingsClient({ get: async () => settings, save: async () => { throw new Error("disk full"); } });
    renderWithEngine(<SettingsPage />, {});
    const retention = await screen.findByLabelText("日志保留天数");
    await userEvent.clear(retention); await userEvent.type(retention, "45");
    await userEvent.click(screen.getByRole("button", { name: "保存设置" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("disk full");
  });

  it("已配置更新：无更新时显示当前已是最新版本", async () => {
    setSettingsClient({ get: async () => ({ ...settings, updaterConfigured: true }), save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }) });
    vi.mocked(check).mockResolvedValueOnce(null);
    renderWithEngine(<SettingsPage />, {});
    await screen.findByText("尚未检查更新。");
    await userEvent.click(screen.getByRole("button", { name: "检查更新" }));
    expect((await screen.findAllByRole("status")).some((node) => node.textContent === "当前已是最新版本")).toBe(true);
  });

  it("已配置更新：检查失败显示内联错误且支持真正重试", async () => {
    setSettingsClient({ get: async () => ({ ...settings, updaterConfigured: true }), save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }) });
    vi.mocked(check).mockRejectedValueOnce(new Error("网络连接失败")).mockResolvedValueOnce(null);
    renderWithEngine(<SettingsPage />, {});
    await screen.findByText("尚未检查更新。");
    await userEvent.click(screen.getByRole("button", { name: "检查更新" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("网络连接失败");
    // 点击重试按钮触发真正的重试行为
    await userEvent.click(screen.getByRole("button", { name: "重试" }));
    expect((await screen.findAllByRole("status")).some((node) => node.textContent === "当前已是最新版本")).toBe(true);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(check).toHaveBeenCalledTimes(2);
  });

  it("发现更新后取消确认不下载", async () => {
    const download = vi.fn().mockResolvedValue(undefined); const close = vi.fn().mockResolvedValue(undefined);
    setSettingsClient({ get: async () => ({ ...settings, updaterConfigured: true }), save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }) });
    vi.mocked(check).mockResolvedValue({ version: "2.0.0", date: undefined, body: "更新", downloadAndInstall: download, close } as never);
    renderWithEngine(<SettingsPage />, {});
    await userEvent.click(await screen.findByRole("button", { name: "检查更新" }));
    expect(await screen.findByText("发现新版本 2.0.0")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "下载并安装" }));
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(download).not.toHaveBeenCalled();
  });

  it("确认安装成功后显示重启提示并释放更新句柄", async () => {
    const download = vi.fn().mockResolvedValue(undefined); const close = vi.fn().mockResolvedValue(undefined);
    setSettingsClient({ get: async () => ({ ...settings, updaterConfigured: true }), save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }) });
    vi.mocked(check).mockResolvedValue({ version: "2.0.0", downloadAndInstall: download, close } as never);
    renderWithEngine(<SettingsPage />, {});
    await userEvent.click(await screen.findByRole("button", { name: "检查更新" }));
    await userEvent.click(await screen.findByRole("button", { name: "下载并安装" }));
    await userEvent.click(screen.getByRole("button", { name: "确认下载并安装" }));
    expect((await screen.findAllByRole("status")).some((node) => node.textContent === "更新已安装，请重启应用生效")).toBe(true);
    expect(download).toHaveBeenCalledTimes(1);
    expect(download).toHaveBeenCalledWith(undefined, { restartAfterInstall: false });
    expect(close).toHaveBeenCalled();
  });

  it("安装失败保留错误并允许真正重试安装成功", async () => {
    const download = vi.fn().mockRejectedValueOnce(new Error("校验失败")).mockResolvedValueOnce(undefined);
    const close = vi.fn().mockResolvedValue(undefined);
    setSettingsClient({ get: async () => ({ ...settings, updaterConfigured: true }), save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }) });
    vi.mocked(check).mockResolvedValue({ version: "2.0.0", downloadAndInstall: download, close } as never);
    renderWithEngine(<SettingsPage />, {});
    await userEvent.click(await screen.findByRole("button", { name: "检查更新" }));
    await userEvent.click(await screen.findByRole("button", { name: "下载并安装" }));
    await userEvent.click(screen.getByRole("button", { name: "确认下载并安装" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("校验失败");
    expect(download).toHaveBeenCalledTimes(1);
    expect(close).not.toHaveBeenCalled();

    // 真正执行失败重试：再次点击重试，验证成功完成安装与资源释放
    await userEvent.click(screen.getByRole("button", { name: "重试" }));
    expect((await screen.findAllByRole("status")).some((node) => node.textContent === "更新已安装，请重启应用生效")).toBe(true);
    expect(download).toHaveBeenCalledTimes(2);
    expect(close).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText("发现新版本 2.0.0")).toBeNull();
  });

  it("导航离开或组件卸载时释放未完成安装的更新句柄", async () => {
    const close = vi.fn().mockResolvedValue(undefined);
    setSettingsClient({ get: async () => ({ ...settings, updaterConfigured: true }), save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }) });
    vi.mocked(check).mockResolvedValue({ version: "2.0.0", downloadAndInstall: vi.fn(), close } as never);
    const { unmount } = renderWithEngine(<SettingsPage />, {});
    await userEvent.click(await screen.findByRole("button", { name: "检查更新" }));
    expect(await screen.findByText("发现新版本 2.0.0")).toBeInTheDocument();
    expect(close).not.toHaveBeenCalled();
    unmount();
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("在已有更新未安装时重新检查更新，释放旧更新句柄", async () => {
    const close1 = vi.fn().mockResolvedValue(undefined);
    const close2 = vi.fn().mockResolvedValue(undefined);
    setSettingsClient({ get: async () => ({ ...settings, updaterConfigured: true }), save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }) });
    vi.mocked(check)
      .mockResolvedValueOnce({ version: "2.0.0", downloadAndInstall: vi.fn(), close: close1 } as never)
      .mockResolvedValueOnce({ version: "2.0.1", downloadAndInstall: vi.fn(), close: close2 } as never);
    renderWithEngine(<SettingsPage />, {});
    await userEvent.click(await screen.findByRole("button", { name: "检查更新" }));
    expect(await screen.findByText("发现新版本 2.0.0")).toBeInTheDocument();
    expect(close1).not.toHaveBeenCalled();

    // 再次点击检查更新，前一个句柄必须释放
    await userEvent.click(screen.getByRole("button", { name: "检查更新" }));
    expect(await screen.findByText("发现新版本 2.0.1")).toBeInTheDocument();
    expect(close1).toHaveBeenCalledTimes(1);
    expect(close2).not.toHaveBeenCalled();
  });

  it("检查更新在挂载期间发起但在卸载后完成时，立即调用 close 释放后端资源且不抛错", async () => {
    let resolveCheck!: (value: unknown) => void;
    const checkPromise = new Promise((resolve) => {
      resolveCheck = resolve;
    });
    setSettingsClient({
      get: async () => ({ ...settings, updaterConfigured: true }),
      save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }),
    });
    vi.mocked(check).mockReturnValueOnce(checkPromise as never);

    const { unmount } = renderWithEngine(<SettingsPage />, {});
    await userEvent.click(await screen.findByRole("button", { name: "检查更新" }));

    // Unmount before check resolves
    unmount();

    const close = vi.fn().mockResolvedValue(undefined);
    resolveCheck({
      version: "2.0.0",
      downloadAndInstall: vi.fn(),
      close,
    });

    // Wait microtasks
    await Promise.resolve();
    await Promise.resolve();

    expect(close).toHaveBeenCalledTimes(1);
  });

  it("安装过程中导航离开或卸载不提前释放句柄，待安装完成后统一清理并通过全局 toast 报告成功", async () => {
    let resolveDownload!: () => void;
    const downloadPromise = new Promise<void>((resolve) => {
      resolveDownload = resolve;
    });
    const download = vi.fn().mockReturnValue(downloadPromise);
    const close = vi.fn().mockResolvedValue(undefined);
    const toastSuccessSpy = vi.spyOn(toast, "success");

    setSettingsClient({
      get: async () => ({ ...settings, updaterConfigured: true }),
      save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }),
    });
    vi.mocked(check).mockResolvedValue({
      version: "2.0.0",
      downloadAndInstall: download,
      close,
    } as never);

    const { unmount } = renderWithEngine(<SettingsPage />, {});
    await userEvent.click(await screen.findByRole("button", { name: "检查更新" }));
    await userEvent.click(await screen.findByRole("button", { name: "下载并安装" }));
    await userEvent.click(screen.getByRole("button", { name: "确认下载并安装" }));

    expect(download).toHaveBeenCalledTimes(1);
    expect(download).toHaveBeenCalledWith(undefined, { restartAfterInstall: false });
    expect(close).not.toHaveBeenCalled();

    // While download is still in progress, unmount
    unmount();

    // Close should NOT be called yet because download is still running
    expect(close).not.toHaveBeenCalled();
    expect(toastSuccessSpy).not.toHaveBeenCalled();

    // Now resolve download
    resolveDownload();
    await Promise.resolve();
    await Promise.resolve();

    // Close must now be called upon completion, and global toast reported
    expect(close).toHaveBeenCalledTimes(1);
    expect(toastSuccessSpy).toHaveBeenCalledWith("更新已安装，请重启应用生效");
  });

  it("安装过程中实际切页导航离开，完成后通过全局 toast 报告成功且延迟释放句柄", async () => {
    let resolveDownload!: () => void;
    const downloadPromise = new Promise<void>((resolve) => {
      resolveDownload = resolve;
    });
    const download = vi.fn().mockReturnValue(downloadPromise);
    const close = vi.fn().mockResolvedValue(undefined);
    const toastSuccessSpy = vi.spyOn(toast, "success");

    setSettingsClient({
      get: async () => ({ ...settings, updaterConfigured: true }),
      save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }),
    });
    vi.mocked(check).mockResolvedValue({
      version: "2.0.0",
      downloadAndInstall: download,
      close,
    } as never);

    function NavigationHost() {
      const [page, setPage] = useState("settings");
      return (
        <div>
          <button type="button" onClick={() => setPage("other")}>离开设置</button>
          {page === "settings" ? <SettingsPage /> : <div>其他页面内容</div>}
        </div>
      );
    }

    renderWithEngine(<NavigationHost />, {});
    await userEvent.click(await screen.findByRole("button", { name: "检查更新" }));
    await userEvent.click(await screen.findByRole("button", { name: "下载并安装" }));
    await userEvent.click(screen.getByRole("button", { name: "确认下载并安装" }));

    expect(download).toHaveBeenCalledWith(undefined, { restartAfterInstall: false });
    expect(close).not.toHaveBeenCalled();

    // Navigate away to other page while installation is still pending
    await userEvent.click(screen.getByRole("button", { name: "离开设置" }));
    expect(screen.queryByText("应用与运行路径")).toBeNull();
    expect(screen.getByText("其他页面内容")).toBeInTheDocument();
    expect(close).not.toHaveBeenCalled();
    expect(toastSuccessSpy).not.toHaveBeenCalled();

    // Now resolve download
    resolveDownload();
    await Promise.resolve();
    await Promise.resolve();

    expect(close).toHaveBeenCalledTimes(1);
    expect(toastSuccessSpy).toHaveBeenCalledWith("更新已安装，请重启应用生效");
    expect(await screen.findByText("更新已安装，请重启应用生效")).toBeInTheDocument();
  });

  it("安装过程中导航离开或卸载后如果安装失败，全局 toast 报告失败且释放句柄", async () => {
    let rejectDownload!: (reason: Error) => void;
    const downloadPromise = new Promise<void>((_, reject) => {
      rejectDownload = reject;
    });
    const download = vi.fn().mockReturnValue(downloadPromise);
    const close = vi.fn().mockResolvedValue(undefined);
    const toastErrorSpy = vi.spyOn(toast, "error");

    setSettingsClient({
      get: async () => ({ ...settings, updaterConfigured: true }),
      save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }),
    });
    vi.mocked(check).mockResolvedValue({
      version: "2.0.0",
      downloadAndInstall: download,
      close,
    } as never);

    const { unmount } = renderWithEngine(<SettingsPage />, {});
    await userEvent.click(await screen.findByRole("button", { name: "检查更新" }));
    await userEvent.click(await screen.findByRole("button", { name: "下载并安装" }));
    await userEvent.click(screen.getByRole("button", { name: "确认下载并安装" }));

    expect(download).toHaveBeenCalledTimes(1);
    expect(close).not.toHaveBeenCalled();

    // While download is still in progress, unmount
    unmount();

    expect(close).not.toHaveBeenCalled();
    expect(toastErrorSpy).not.toHaveBeenCalled();

    // Reject download with error
    rejectDownload(new Error("网络中断校验失败"));
    await Promise.resolve();
    await Promise.resolve();

    // Handle must be closed to prevent leak, and toast.error must be called
    expect(close).toHaveBeenCalledTimes(1);
    expect(toastErrorSpy).toHaveBeenCalledWith(expect.stringContaining("网络中断校验失败"));
  });

  it("句柄 close 抛出异常时不产生未捕获的 Promise rejection", async () => {
    const close = vi.fn().mockRejectedValue(new Error("Resource destroyed"));
    setSettingsClient({
      get: async () => ({ ...settings, updaterConfigured: true }),
      save: async (preferences) => ({ ...settings, preferences, updaterConfigured: true }),
    });
    vi.mocked(check).mockResolvedValue({
      version: "2.0.0",
      downloadAndInstall: vi.fn(),
      close,
    } as never);

    const { unmount } = renderWithEngine(<SettingsPage />, {});
    await userEvent.click(await screen.findByRole("button", { name: "检查更新" }));
    expect(await screen.findByText("发现新版本 2.0.0")).toBeInTheDocument();

    // Unmounting calls close, which rejects. This should not cause unhandled rejection.
    unmount();
    await Promise.resolve();
    await Promise.resolve();

    expect(close).toHaveBeenCalledTimes(1);
  });

  it("渲染配置工具区块，点击可下钻，常驻返回设置按钮可返回", async () => {
    setSettingsClient({
      get: async () => settings,
      save: async (preferences) => ({ ...settings, preferences }),
    });

    renderWithEngine(<SettingsPage />, {
      "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
      "agents:status": makeEnvelope({ domain: "agents", elements: [] }, { command: "agents.status" }),
      "mcp:status": makeEnvelope({ domain: "mcp", elements: [] }, { command: "mcp.status" }),
      "skills:status": makeEnvelope({ domain: "skills", elements: [] }, { command: "skills.status" }),
      "git:status": makeEnvelope({ clean: true, branch: "main", ahead: 0, behind: 0, remote_configured: false }, { command: "git.status" }),
      "system:version": makeEnvelope({ version: "0.1.0" }, { command: "system.version" }),
    });

    expect(await screen.findByRole("heading", { name: "配置工具" })).toBeInTheDocument();
    const tools = screen.getByRole("heading", { name: "配置工具" }).closest("section")!;
    const appearance = screen.getByRole("heading", { name: "外观" }).closest("section")!;
    expect(tools.compareDocumentPosition(appearance) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);

    const expectedKeys = [
      "aliases",
      "secrets",
    ];

    for (const key of expectedKeys) {
      expect(screen.getByTestId(`advanced-entry-${key}`)).toBeInTheDocument();
    }
    // 总览已提到一级导航，不再在配置工具里重复出现
    expect(screen.queryByTestId("advanced-entry-dashboard")).toBeNull();
    // 2026-09-12 裁决退役：Tools 与快照入口不再出现
    expect(screen.queryByTestId("advanced-entry-tools")).toBeNull();
    expect(screen.queryByTestId("advanced-entry-snapshots")).toBeNull();
    expect(screen.queryByTestId("advanced-entry-sync")).toBeNull();
    // 更新中心与总览清单重复；个性化配置是引擎配置原文编辑器，都已撤下
    expect(screen.queryByTestId("advanced-entry-updates")).toBeNull();
    expect(screen.queryByTestId("advanced-entry-policy")).toBeNull();
    expect(screen.queryByText("多设备同步")).toBeNull();

    // 点击终端快捷命令下钻
    await userEvent.click(screen.getByTestId("advanced-entry-aliases"));

    // 常驻返回设置按钮出现，且在面包屑中展示
    const backButton = await screen.findByTestId("back-to-settings-button");
    expect(backButton).toBeInTheDocument();
    expect(screen.getByText(/配置工具 \/ 终端快捷命令/)).toBeInTheDocument();

    // 点击返回设置
    await userEvent.click(backButton);

    // 重新回到基础设置页
    expect(await screen.findByRole("heading", { name: "配置工具" })).toBeInTheDocument();
    expect(screen.queryByTestId("back-to-settings-button")).toBeNull();
  });

  it("显示日志保留天数并校验合法范围 (0..=3650)", async () => {
    let savedPreferences = settings.preferences;
    setSettingsClient({
      get: async () => ({ ...settings, preferences: savedPreferences }),
      save: async (preferences) => {
        savedPreferences = preferences;
        return { ...settings, preferences: savedPreferences };
      },
    });
    renderWithEngine(<SettingsPage />, {});
    const input = await screen.findByLabelText("日志保留天数");
    expect(input).toHaveValue(30);
    expect(screen.getByText("（操作记录保留多久，0 为不自动清理，最大 3650 天）")).toBeInTheDocument();

    const saveBtn = screen.getByRole("button", { name: "保存设置" });

    // 超出范围 3651：禁用保存
    await userEvent.clear(input);
    await userEvent.type(input, "3651");
    expect(saveBtn).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("请输入 0 到 3650 之间的整数");

    // 负数：禁用保存
    await userEvent.clear(input);
    await userEvent.type(input, "-1");
    expect(saveBtn).toBeDisabled();

    // 合法修改为 0（不自动清理）：保存成功
    await userEvent.clear(input);
    await userEvent.type(input, "0");
    expect(saveBtn).not.toBeDisabled();
    expect(screen.queryByRole("alert")).toBeNull();
    await userEvent.click(saveBtn);
    expect(await screen.findByText("设置已保存")).toBeInTheDocument();
    expect(savedPreferences.logRetentionDays).toBe(0);
  });

  it("settingsSchema 严格校验：拒绝多余字段与已退役的偏好", () => {
    const baseValid = {
      preferences: { logRetentionDays: 30 },
      appVersion: "1.0.0",
      contractVersion: "agent-manager/engine/v1",
      resourceDir: "/res",
      dataDir: "/data",
      updaterConfigured: false,
    };
    expect(() => settingsSchema.parse(baseValid)).not.toThrow();
    expect(() => settingsSchema.parse({ ...baseValid, extraField: true })).toThrow();
    // 诊断日志与启动维护已退役：日志目录与维护告警不再是契约字段
    expect(() => settingsSchema.parse({ ...baseValid, logDir: "/logs" })).toThrow();
    expect(() => settingsSchema.parse({ ...baseValid, maintenanceWarning: "启动清理失败" })).toThrow();
  });
});
