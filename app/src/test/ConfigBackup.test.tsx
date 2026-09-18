import { describe, expect, it } from "vitest";
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
  it("导出后展示备份内容，密钥只以变量引用出现", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    await userEvent.click(screen.getByTestId("config-export-button"));

    const output = (await screen.findByTestId("config-export-output")) as HTMLTextAreaElement;
    expect(output.value).toContain("agent-manager/config-backup/v1");
    expect(output.value).toContain("${T}");
    expect(calls.find((call) => call.command === "config:export")?.args).toEqual([]);
  });

  it("导入需要确认，确认后整份提交并说明还要应用才落地", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    fireEvent.change(screen.getByTestId("config-import-input"), {
      target: { value: '{"format":"agent-manager/config-backup/v1"}' },
    });
    await userEvent.click(screen.getByTestId("config-import-button"));

    expect(await screen.findByText(/助手配置文件不受影响/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "确认覆盖" }));

    await waitFor(() => expect(calls.some((call) => call.command === "config:import")).toBe(true));
    const write = calls.find((call) => call.command === "config:import")!;
    expect(write.args[0]).toBe("--value");
    expect(JSON.parse(write.args[1]).format).toBe("agent-manager/config-backup/v1");
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
