import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { toast } from "sonner";
import McpPage from "../pages/McpPage";
import type { Envelope, StatusData } from "../lib/contract";
import { makeApplyResult, makeEnvelope } from "./fixtures";
import { BASE_CLAUDE_FS, BASE_CLAUDE_SNAPSHOT, BASE_CODEX_FS, BASE_CODEX_REMOTE, mcpStatusData, mcpTargets } from "./mcp-fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

function routes(
  status: StatusData = mcpStatusData(),
  apply: Envelope | ((args: string[]) => Promise<Envelope>) = makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
): Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> {
  return {
    "mcp:status": makeEnvelope(status, { command: "mcp.status" }),
    "mcp:preview": async (args: string[]) => makeEnvelope({
      files: [{
        path: `/isolated/${args.includes("--to") ? args[args.indexOf("--to") + 1] : args[args.indexOf("--agent") + 1]}/config`,
        exists: true,
        content: args.includes("--spec") ? JSON.stringify(specOf(args)) : "复制后完整配置：fs-mcp",
      }],
    }, { command: "mcp.preview" }),
    "mcp:apply": apply,
    "secrets:set": makeEnvelope({ domain: "secrets", variable: "FS_API_KEY", stored: true }, { command: "secrets.set" }),
  };
}

function applyCalls(calls: { command: string; args: string[] }[]) {
  return calls.filter((call) => call.command === "mcp:apply");
}

function specOf(args: string[]): Record<string, unknown> {
  return JSON.parse(args[args.indexOf("--spec") + 1]) as Record<string, unknown>;
}

describe("McpPage：实际条目为主", () => {
  it("按服务列出各助手里的实际条目：内容各不相同、只显示脱敏值，不再有服务库/导入/同步入口", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    const card = await screen.findByTestId("mcp-card-mcp:fs");
    const claude = within(card).getByTestId("mcp-entry-mcp:fs-claude-code");
    const codex = within(card).getByTestId("mcp-entry-mcp:fs-codex");
    expect(claude).toHaveTextContent("npx -y fs-mcp");
    expect(codex).toHaveTextContent("npx -y fs-mcp@2");
    expect(codex).toHaveTextContent("另有字段原样保留：cwd、enabled_tools、tools");
    expect(codex).toHaveTextContent("旧版 Agent Manager 标记");
    expect(claude).toHaveTextContent("${FS_API_KEY}");
    expect(screen.getByTestId("mcp-counts")).toHaveTextContent("共 3 个服务，2 个已停用");
    // 停用快照与未分配的已保存定义
    expect(within(screen.getByTestId("mcp-card-mcp:old-snap")).getByText("已停用（已保存）")).toBeInTheDocument();
    expect(screen.getByTestId("mcp-saved-mcp:old-snap")).toHaveTextContent("uvx old-snap");
    // 旧模型的入口都不在了
    expect(screen.queryByText("服务库")).toBeNull();
    expect(screen.queryByTestId("sync-all-mcp")).toBeNull();
    expect(screen.queryByText(/发现未导入/)).toBeNull();
    expect(calls.some((call) => call.command === "policy:get" || call.command === "policy:set")).toBe(false);
  });

  it("某个助手的文件读不了：顶部说明，其他助手照常", async () => {
    const status = mcpStatusData();
    status.targets = mcpTargets({ codex: { error: "配置文件解析失败: TOML 语法错误" } });
    renderWithEngine(<McpPage />, routes(status));
    expect(await screen.findByTestId("mcp-target-error-codex")).toHaveTextContent("配置文件解析失败");
    expect(screen.getByTestId("mcp-entry-mcp:fs-claude-code")).toBeInTheDocument();
  });

  it("编辑：表单用实际值预填，字面量显示 <literal> 并按保留原值提交；带打开时的 base，只改这一条", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByTestId("action-edit-mcp:fs-codex"));
    const dialog = await screen.findByRole("dialog", { name: /编辑 fs/ });
    expect(within(dialog).getByTestId("server-name-input")).toBeDisabled();
    expect(within(dialog).getByTestId("server-transport-remote")).toBeDisabled();
    expect(within(dialog).getByTestId("server-args-input")).toHaveValue("-y\nfs-mcp@2");
    expect(within(dialog).getByTestId("server-env-input")).toHaveValue("TOKEN=<literal>");
    expect(within(dialog).getByTestId("server-timeout-input")).toHaveValue("30");
    expect(within(dialog).getByTestId("server-preserved-fields")).toHaveTextContent("cwd、enabled_tools、tools");

    const args = within(dialog).getByTestId("server-args-input");
    await userEvent.clear(args);
    await userEvent.type(args, "-y{enter}fs-mcp@3");
    const env = within(dialog).getByTestId("server-env-input");
    await userEvent.clear(env);
    await userEvent.type(env, "TOKEN=<literal>{enter}MODE=fast");
    expect(await within(dialog).findByTestId("mcp-preview-content-codex")).toHaveTextContent("fs-mcp@3");
    const previewCalls = calls.filter((call) => call.command === "mcp:preview");
    const previewCall = previewCalls[previewCalls.length - 1];
    expect(previewCall?.args.slice(0, 8)).toEqual(["--only", "mcp:fs", "--agent", "codex", "--action", "edit", "--base", BASE_CODEX_FS]);
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));

    await waitFor(() => expect(applyCalls(calls)).toHaveLength(1));
    const call = applyCalls(calls)[0];
    expect(call.args.slice(0, 8)).toEqual(["--only", "mcp:fs", "--agent", "codex", "--action", "edit", "--base", BASE_CODEX_FS]);
    expect(specOf(call.args)).toEqual({
      command: "npx",
      args: ["-y", "fs-mcp@3"],
      env: { TOKEN: "<literal>", MODE: "fast" },
      startup_timeout_sec: 30,
    });
  });

  it("停用/启用直接执行并带 base；删除需独立确认", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByTestId("action-disable-mcp:fs-claude-code"));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(1));
    expect(applyCalls(calls)[0].args).toEqual(["--only", "mcp:fs", "--agent", "claude-code", "--action", "disable", "--base", BASE_CLAUDE_FS]);

    await userEvent.click(screen.getByTestId("action-enable-mcp:old-snap-claude-code"));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(2));
    expect(applyCalls(calls)[1].args).toEqual(["--only", "mcp:old-snap", "--agent", "claude-code", "--action", "enable", "--base", BASE_CLAUDE_SNAPSHOT]);

    await userEvent.click(screen.getByTestId("action-remove-mcp:remote-api-codex"));
    expect(applyCalls(calls)).toHaveLength(2);
    await userEvent.click(await screen.findByRole("button", { name: "确认删除" }));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(3));
    expect(applyCalls(calls)[2].args).toEqual(["--only", "mcp:remote-api", "--agent", "codex", "--action", "remove", "--base", BASE_CODEX_REMOTE]);
  });

  it("复制到另一个助手：目标有同名条目时必须勾选替换，并带目标条目的 base", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByTestId("action-copy-mcp:fs-claude-code"));
    const dialog = await screen.findByRole("dialog", { name: "复制 fs" });
    expect(within(dialog).getByTestId("mcp-copy-target")).toHaveValue("codex");
    expect(within(dialog).getByTestId("mcp-copy-submit")).toBeDisabled();
    expect(calls.some((call) => call.command === "mcp:preview")).toBe(false);
    await userEvent.click(within(dialog).getByTestId("mcp-copy-replace"));
    expect(await within(dialog).findByTestId("mcp-copy-preview-content")).toHaveTextContent("复制后完整配置");
    expect(within(dialog).getByText("/isolated/codex/config（已有文件）")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByTestId("mcp-copy-replace"));
    expect(within(dialog).queryByTestId("mcp-copy-preview-content")).toBeNull();
    await userEvent.click(within(dialog).getByTestId("mcp-copy-replace"));
    expect(await within(dialog).findByTestId("mcp-copy-preview-content")).toHaveTextContent("复制后完整配置");
    const preview = calls.find((call) => call.command === "mcp:preview")!;
    await userEvent.click(within(dialog).getByTestId("mcp-copy-submit"));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(1));
    expect(applyCalls(calls)[0].args).toEqual([
      "--only", "mcp:fs", "--agent", "claude-code", "--to", "codex", "--action", "copy",
      "--base", BASE_CLAUDE_FS, "--replace", "--to-base", BASE_CODEX_FS,
    ]);
    expect(preview.args).toEqual(applyCalls(calls)[0].args);
  });

  it("新增直接写入所选助手：逐个助手各写一次，超时只给 Codex；新增时不接受 <literal>", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByTestId("add-server-button"));
    const dialog = await screen.findByRole("dialog", { name: "新增服务" });
    await userEvent.type(within(dialog).getByTestId("server-name-input"), "new-srv");
    await userEvent.click(within(dialog).getByTestId("server-agent-claude-code"));
    await userEvent.click(within(dialog).getByTestId("server-agent-codex"));
    await userEvent.type(within(dialog).getByTestId("server-command-input"), "npx");
    await userEvent.type(within(dialog).getByTestId("server-args-input"), "-y{enter}new-pkg");
    const env = within(dialog).getByTestId("server-env-input");
    await userEvent.type(env, "KEY=<literal>");
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    expect(within(dialog).getByRole("alert")).toHaveTextContent("没有可保留的原值");
    await userEvent.clear(env);
    await userEvent.type(env, "KEY=${{NEW_KEY}{enter}NODE_ENV=production");
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));

    await waitFor(() => expect(applyCalls(calls)).toHaveLength(2));
    const [first, second] = applyCalls(calls);
    expect(first.args.slice(0, 6)).toEqual(["--only", "mcp:new-srv", "--agent", "claude-code", "--action", "add"]);
    expect(specOf(first.args)).toEqual({ command: "npx", args: ["-y", "new-pkg"], env: { KEY: "${NEW_KEY}", NODE_ENV: "production" } });
    expect(second.args.slice(0, 6)).toEqual(["--only", "mcp:new-srv", "--agent", "codex", "--action", "add"]);
    expect(specOf(second.args)).toMatchObject({ startup_timeout_sec: 120 });
  });

  it("已保存的定义：添加到助手时预填（按助手追加旧版附加参数）；删除保存项需确认", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByTestId("add-from-saved-mcp:old-snap"));
    const dialog = await screen.findByRole("dialog", { name: "新增服务" });
    expect(within(dialog).getByTestId("server-name-input")).toHaveValue("old-snap");
    expect(within(dialog).getByTestId("server-env-input")).toHaveValue("KEY=${OLD_KEY}");
    await userEvent.click(within(dialog).getByTestId("server-agent-codex"));
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(1));
    expect(specOf(applyCalls(calls)[0].args)).toMatchObject({ command: "uvx", args: ["old-snap", "--codex"] });

    await userEvent.click(screen.getByTestId("forget-mcp:old-snap"));
    await userEvent.click(await screen.findByRole("button", { name: "确认删除" }));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(2));
    expect(applyCalls(calls)[1].args).toEqual(["--only", "mcp:old-snap", "--action", "forget"]);
  });

  it("写入被拒（已在外部修改）：提示失败并重新读取状态；引擎的提示以通知显示", async () => {
    const errorSpy = vi.spyOn(toast, "error");
    const infoSpy = vi.spyOn(toast, "info");
    let attempt = 0;
    const { calls } = renderWithEngine(
      <McpPage />,
      routes(mcpStatusData(), async () => {
        attempt += 1;
        if (attempt === 1) {
          return makeEnvelope(null, {
            command: "mcp.apply",
            ok: false,
            errors: [{ code: "MCP_ERROR", element: null, message: "条目 fs 在助手 claude-code 的配置中已在外部修改，请刷新后重试" }],
          });
        }
        return makeEnvelope(makeApplyResult({ domain: "mcp" }), {
          command: "mcp.apply",
          warnings: [{ code: "MCP_NOTICE", element: null, message: "已把 1 个疑似凭据的明文取值转存到外部密钥文件：API_KEY" }],
        });
      }),
    );
    await userEvent.click(await screen.findByTestId("action-disable-mcp:fs-claude-code"));
    await waitFor(() => expect(errorSpy).toHaveBeenCalledWith(expect.stringContaining("已在外部修改")));
    await waitFor(() => expect(calls.filter((call) => call.command === "mcp:status").length).toBeGreaterThan(1));

    await userEvent.click(screen.getByTestId("action-disable-mcp:fs-claude-code"));
    await waitFor(() => expect(infoSpy).toHaveBeenCalledWith("已把 1 个疑似凭据的明文取值转存到外部密钥文件：API_KEY"));
    errorSpy.mockRestore();
    infoSpy.mockRestore();
  });

  it("按助手查看：矩阵按条目状态着色，展开后是同样的逐条操作", async () => {
    renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByRole("tab", { name: "按助手查看" }));
    await userEvent.click(await screen.findByTestId("expand-mcp:old-snap"));
    expect(await screen.findByTestId("action-enable-mcp:old-snap-claude-code")).toBeInTheDocument();
    expect(document.querySelector('[data-cell-state="saved"]')).not.toBeNull();
    expect(document.querySelector('[data-cell-state="enabled"]')).not.toBeNull();
  });

  it("填写缺失变量：打开密钥输入并写入外部文件", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByTestId("configure-secrets-mcp:fs"));
    await screen.findByRole("dialog", { name: "填写 FS_API_KEY" });
    await userEvent.type(screen.getByTestId("secret-value-input"), "value-under-test");
    await userEvent.click(screen.getByTestId("secret-value-save"));
    await waitFor(() => expect(calls.some((call) => call.command === "secrets:set")).toBe(true));
    expect(calls.find((call) => call.command === "secrets:set")?.args.slice(0, 2)).toEqual(["--var", "FS_API_KEY"]);
  });
});
