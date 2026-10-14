import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import { ConfigBackupSection } from "../components/ConfigBackupSection";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

const BACKUP = {
  base: "a".repeat(64),
  format: "agent-manager/config-backup/v1",
  exported_at: "2026-09-17T00:00:00Z",
  policy: { mcp: { definitions: {} } },
  // 规则与 hooks 不再是来源：导出里是规则模板（名称 + 正文），来源只剩 aliases
  sources: { "aliases.shell": "{}" },
  rule_templates: [{ name: "团队规则", body: "我的规则\n", origin: "user" }],
  // MCP 定义随「已保存项」导出（原先计在策略的服务库定义里）
  mcp_saved: [{ server: "demo", agent: "", entry: { command: "npx", env: { T: "${T}" } }, resolve: { env: [], headers: [] }, origin: "migrated" }],
  launch_recipes: [{ id: "launch-1", recipe: { platform: "macos", shell: "zsh", kind: "app", target: "/Applications/YOUR_APPLICATION.app", args: [], env: [], variants: [{ name: "example", timezone: "" }] } }],
  environment_profiles: [{ id: "environment-1", profile: { name: "Example environment", target: { region: "GB" }, auto_sync: false } }],
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
  it("只恢复勾选模块，预览不包含未选择的规则和 MCP", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    fireEvent.click(screen.getByTestId("config-import-toggle"));
    fireEvent.change(screen.getByTestId("config-import-input"), { target: { value: JSON.stringify({ ...BACKUP,
      policy: { skills: { repositories: { imported: { url: "https://example.com/skills.git" } } } },
      rule_templates: [{ name: "Unselected rule", body: "Keep current instead" }], mcp_saved: [],
    }) } });
    await userEvent.click(screen.getByTestId("restore-module-mcp"));
    await userEvent.click(screen.getByTestId("restore-module-rules"));
    await userEvent.click(screen.getByTestId("restore-module-launch"));
    await userEvent.click(screen.getByTestId("restore-module-environment"));
    await userEvent.click(screen.getByTestId("config-import-button"));
    const preview = await screen.findByTestId("config-import-diff");
    expect(preview).toHaveTextContent("新增：imported");
    expect(preview).not.toHaveTextContent("Unselected rule");
    expect(screen.queryByTestId("config-diff-MCP 已保存项")).toBeNull();
    await userEvent.click(screen.getByTestId("config-import-confirm"));
    await waitFor(() => expect(calls.find((call) => call.command === "config:import")?.args.slice(2)).toEqual(["--modules", "skills", "--base", BACKUP.base]));
  });
  it("导出明确不包含密钥取值与外部文件路径", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    await userEvent.click(screen.getByTestId("config-export-button"));

    const output = await screen.findByTestId("config-export-output");
    expect(output).toHaveTextContent("0 条定义 · 1 个 MCP 已保存项");
    expect(output).toHaveTextContent("1 个规则模板");
    expect(output).toHaveTextContent("1 个启动配置 · 1 个环境集合");
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

  it("previews only selected launch data and discloses that restoration does not execute it", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    await userEvent.click(screen.getByTestId("config-import-toggle"));
    for (const module of ["skills", "mcp", "rules", "environment"]) await userEvent.click(screen.getByTestId(`restore-module-${module}`));
    const incoming = { ...BACKUP, launch_recipes: [{ ...BACKUP.launch_recipes[0], recipe: { ...BACKUP.launch_recipes[0].recipe, target: "/Applications/YOUR_OTHER_APPLICATION.app" } }], environment_profiles: [] };
    fireEvent.change(screen.getByTestId("config-import-input"), { target: { value: JSON.stringify(incoming) } });
    await userEvent.click(screen.getByTestId("config-import-button"));
    const preview = await screen.findByTestId("config-import-diff");
    expect(preview).toHaveTextContent("YOUR_OTHER_APPLICATION.app");
    expect(preview).toHaveTextContent("恢复不会启动应用或调整系统设置");
    expect(screen.queryByTestId("config-diff-环境集合")).toBeNull();
    await userEvent.click(screen.getByTestId("config-import-confirm"));
    await waitFor(() => expect(calls.find((call) => call.command === "config:import")?.args.slice(2)).toEqual(["--modules", "launch", "--base", BACKUP.base]));
    expect(calls.some((call) => ["launch:run", "environment:sync-time"].includes(call.command))).toBe(false);
  });

  it("导入前先给差异预览，确认后才整份提交", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    fireEvent.click(screen.getByTestId("config-import-toggle"));
    // 这份旧格式备份多了 newcomer（服务库定义按 MCP 已保存项计入），还带着旧的规则来源
    //（导入时转为模板）；本机已有的 MCP 已保存项 demo 导入后保留，不算消失
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
    expect(preview).not.toHaveTextContent("移除：demo");
    expect(preview).toHaveTextContent("会被替换：旧规则来源（共享，导入后转为模板）");
    // 旧格式没带规则模板：现有模板不动，不报消失
    expect(screen.queryByTestId("config-diff-规则模板")).toBeNull();
    expect(preview).toHaveTextContent("仅恢复所选模块");
    // 看差异这一步只读当前配置，不写任何东西
    expect(calls.some((call) => call.command === "config:import")).toBe(false);

    await userEvent.click(screen.getByTestId("config-import-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "config:import")).toBe(true));
    const write = calls.find((call) => call.command === "config:import")!;
    expect(write.args[0]).toBe("--value");
    expect(write.args.slice(2)).toEqual(["--modules", "skills,mcp,rules,launch,environment", "--base", BACKUP.base]);
    expect(JSON.parse(write.args[1]).format).toBe("agent-manager/config-backup/v1");
    expect(await screen.findByTestId("config-import-success")).toHaveTextContent("逐项预览和应用");
    expect(screen.queryByTestId("config-go-sync")).toBeNull();
    fireEvent.change(screen.getByTestId("config-import-input"), { target: { value: "{}" } });
    expect(screen.queryByTestId("config-import-success")).toBeNull();
  });

  it("与当前完全一致时明说不会改变任何东西", async () => {
    renderWithEngine(<ConfigBackupSection />, routes());
    fireEvent.click(screen.getByTestId("config-import-toggle"));
    fireEvent.change(screen.getByTestId("config-import-input"), {
      target: { value: JSON.stringify(BACKUP) },
    });
    await userEvent.click(screen.getByTestId("config-import-button"));
    expect(await screen.findByTestId("config-diff-identical")).toHaveTextContent("不会改变任何东西");
  });

  it("粘贴的不是合法 JSON 时当场拦下，不去读当前配置", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    fireEvent.click(screen.getByTestId("config-import-toggle"));
    fireEvent.change(screen.getByTestId("config-import-input"), { target: { value: "not json" } });
    await userEvent.click(screen.getByTestId("config-import-button"));
    expect(screen.queryByTestId("config-import-diff")).toBeNull();
    expect(calls.some((call) => call.command === "config:import")).toBe(false);
  });

  it("旧备份含明文 secrets 时当场停止，不读取也不写入", async () => {
    const { calls } = renderWithEngine(<ConfigBackupSection />, routes());
    fireEvent.click(screen.getByTestId("config-import-toggle"));
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
    fireEvent.click(screen.getByTestId("config-import-toggle"));
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
    fireEvent.click(screen.getByTestId("config-import-toggle"));
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
