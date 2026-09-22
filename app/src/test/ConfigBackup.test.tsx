import { describe, expect, it, vi } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import { ConfigBackupSection } from "../components/ConfigBackupSection";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

const BACKUP = {
  format: "agent-manager/config-backup/v1",
  exported_at: "2026-09-17T00:00:00Z",
  policy: { mcp: { definitions: { demo: { command: "npx", env: { T: "${T}" } } } } },
  sources: { "rules.shared": "我的规则\n" },
};

function routes() {
  return {
    "config:export": makeEnvelope(BACKUP, { command: "config.export" }),
    "config:import": makeEnvelope(
      { domain: "config", steps: [{ name: "config:import", ok: true, exit_code: 0 }], skipped: [] },
      { command: "config.import" },
    ),
  };
}

describe("配置备份", () => {
  it("导出明确不包含密钥取值与外部文件路径", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    await userEvent.click(screen.getByTestId("config-export-button"));

    const output = await screen.findByTestId("config-export-output");
    expect(output).toHaveTextContent("1 条定义");
    expect(output).toHaveTextContent("不含密钥");
    expect(screen.getByText(/不含密钥取值，也不含外部密钥文件路径/)).toBeInTheDocument();
    expect(calls.find((call) => call.command === "config:export")?.args).toEqual([]);
  });

  it("复制到剪贴板拿到完整定义与来源，但没有 secrets 段", async () => {
    const written: string[] = [];
    Object.assign(navigator, {
      clipboard: { writeText: async (text: string) => void written.push(text) },
    });
    renderWithEngine(<ConfigBackupSection />, routes());
    await userEvent.click(screen.getByTestId("config-export-button"));
    await screen.findByTestId("config-export-output");
    await userEvent.click(screen.getByTestId("config-copy-button"));
    await waitFor(() => expect(written.length).toBe(1));
    expect(JSON.parse(written[0])).toEqual(BACKUP);
    expect(written[0]).not.toContain('"secrets"');
  });

  it("导入前先给差异预览，确认后才整份提交", async () => {
    const onGoSync = vi.fn();
    const { calls } = renderWithEngine(<ConfigBackupSection onGoSync={onGoSync} />, routes());
    // 这份备份相对当前少了 demo、多了 newcomer，规则来源也换了
    fireEvent.change(screen.getByTestId("config-import-input"), {
      target: {
        value: JSON.stringify({
          format: "agent-manager/config-backup/v1",
          policy: { mcp: { definitions: { newcomer: { command: "npx" } } } },
          sources: { "rules.shared": "别的规则\n" },
        }),
      },
    });
    await userEvent.click(screen.getByTestId("config-import-button"));

    const preview = await screen.findByTestId("config-import-diff");
    expect(preview).toHaveTextContent("新增：newcomer");
    expect(preview).toHaveTextContent("消失：demo");
    expect(preview).toHaveTextContent("会被替换：共享规则");
    expect(preview).toHaveTextContent("导入是整体替换，不能撤销");
    // 看差异这一步只读当前配置，不写任何东西
    expect(calls.some((call) => call.command === "config:import")).toBe(false);

    await userEvent.click(screen.getByTestId("config-import-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "config:import")).toBe(true));
    const write = calls.find((call) => call.command === "config:import")!;
    expect(write.args[0]).toBe("--value");
    expect(JSON.parse(write.args[1]).format).toBe("agent-manager/config-backup/v1");
    const goSync = await screen.findByTestId("config-go-sync");
    expect(goSync).toHaveTextContent("去总览一键同步");
    await userEvent.click(goSync);
    expect(onGoSync).toHaveBeenCalledOnce();
  });

  it("与当前完全一致时明说不会改变任何东西", async () => {
    renderWithEngine(<ConfigBackupSection />, routes());
    fireEvent.change(screen.getByTestId("config-import-input"), {
      target: { value: JSON.stringify(BACKUP) },
    });
    await userEvent.click(screen.getByTestId("config-import-button"));
    expect(await screen.findByTestId("config-diff-identical")).toHaveTextContent("不会改变任何东西");
  });

  it("粘贴的不是合法 JSON 时当场拦下，不去读当前配置", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    fireEvent.change(screen.getByTestId("config-import-input"), { target: { value: "not json" } });
    await userEvent.click(screen.getByTestId("config-import-button"));
    expect(screen.queryByTestId("config-import-diff")).toBeNull();
    expect(calls.some((call) => call.command === "config:import")).toBe(false);
  });

  it("旧备份含明文 secrets 时当场停止，不读取也不写入", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    fireEvent.change(screen.getByTestId("config-import-input"), {
      target: {
        value: JSON.stringify({
          ...BACKUP,
          secrets: { T: "legacy-secret-under-test" },
        }),
      },
    });
    await userEvent.click(screen.getByTestId("config-import-button"));
    expect(screen.queryByTestId("config-import-diff")).toBeNull();
    expect(calls.some((call) => call.command === "config:export")).toBe(false);
    expect(calls.some((call) => call.command === "config:import")).toBe(false);
  });

  it("旧备份的 secrets 不是空对象时也在预览前停止", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    fireEvent.change(screen.getByTestId("config-import-input"), {
      target: { value: JSON.stringify({ ...BACKUP, secrets: ["legacy-secret-under-test"] }) },
    });
    await userEvent.click(screen.getByTestId("config-import-button"));
    expect(screen.queryByTestId("config-import-diff")).toBeNull();
    expect(calls.some((call) => call.command === "config:export")).toBe(false);
    expect(calls.some((call) => call.command === "config:import")).toBe(false);
  });

  it("没粘贴内容时导入按钮不可点", async () => {
    renderWithEngine(<ConfigBackupSection />, routes());
    expect(screen.getByTestId("config-import-button")).toBeDisabled();
  });

  it("导出失败时如实报错，不展示半份内容", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, {
      "config:export": makeEnvelope(
        {},
        { command: "config.export", ok: false, errors: [{ code: "CONFIG_ERROR", element: null, message: "配置项损坏" }] },
      ),
    });
    await userEvent.click(screen.getByTestId("config-export-button"));
    await waitFor(() => expect(calls.some((call) => call.command === "config:export")).toBe(true));
    expect(screen.queryByTestId("config-export-output")).toBeNull();
  });
});
