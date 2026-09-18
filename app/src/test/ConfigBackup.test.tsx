import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import { ConfigBackupSection } from "../components/ConfigBackupSection";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

const SECRET = "secret-value-under-test";

const BACKUP = {
  format: "agent-manager/config-backup/v1",
  exported_at: "2026-09-17T00:00:00Z",
  policy: { mcp: { definitions: { demo: { command: "npx", env: { T: "${T}" } } } } },
  sources: { "rules.shared": "我的规则\n" },
  // 备份带凭据取值：界面不得把它铺在屏幕上
  secrets: { T: SECRET },
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
  it("导出只报里面有什么，凭据取值不铺在屏幕上", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    await userEvent.click(screen.getByTestId("config-export-button"));

    const output = await screen.findByTestId("config-export-output");
    expect(output).toHaveTextContent("1 条定义");
    expect(output).toHaveTextContent("1 个凭据取值");
    // 取值本身绝不出现在页面上（备份内容等同密钥文件）
    expect(document.body.innerHTML).not.toContain(SECRET);
    expect(screen.getByText(/等同于密钥文件/)).toBeInTheDocument();
    expect(calls.find((call) => call.command === "config:export")?.args).toEqual([]);
  });

  it("复制到剪贴板拿到的是完整备份（含取值），而不是屏幕上那份摘要", async () => {
    const written: string[] = [];
    Object.assign(navigator, {
      clipboard: { writeText: async (text: string) => void written.push(text) },
    });
    renderWithEngine(<ConfigBackupSection />, routes());
    await userEvent.click(screen.getByTestId("config-export-button"));
    await screen.findByTestId("config-export-output");
    await userEvent.click(screen.getByTestId("config-copy-button"));
    await waitFor(() => expect(written.length).toBe(1));
    expect(written[0]).toContain(SECRET);
  });

  it("导入前先给差异预览，确认后才整份提交", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
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
