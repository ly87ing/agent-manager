import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { toast } from "sonner";
import McpPage from "../pages/McpPage";
import type { Envelope, StatusData } from "../lib/contract";
import { makeApplyResult, makeEnvelope } from "./fixtures";
import { BASE_CLAUDE_FS, BASE_CLAUDE_SNAPSHOT, BASE_CODEX_FS, BASE_CODEX_REMOTE, mcpStatusData, mcpTargets } from "./mcp-fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

/** 展开所有服务行，露出各账号上的条目与操作。 */
async function expandAll() {
  await screen.findAllByTestId(/^mcp-card-/);
  for (const button of screen.getAllByRole("button", { name: /在各账号上的条目/ })) {
    if (button.getAttribute("aria-expanded") === "false") await userEvent.click(button);
  }
}

function routes(
  status: StatusData = mcpStatusData(),
  apply: Envelope | ((args: string[]) => Promise<Envelope>) = makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
): Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> {
  return {
    "mcp:status": makeEnvelope(status, { command: "mcp.status" }),
    "mcp:preview": async (args: string[]) => makeEnvelope({
      files: args.includes("mcp:old-snap") && args.includes("remove") ? [] : [{
        path: `/isolated/${args.includes("--to") ? args[args.indexOf("--to") + 1] : args[args.indexOf("--agent") + 1]}/config`,
        exists: true,
        content: args.includes("--spec") ? JSON.stringify(specOf(args))
          : args.includes("remove") ? "删除后完整配置：保留其他服务" : "复制后完整配置：fs-mcp",
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
    await expandAll();
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

  it("停用/启用直接执行并带 base；删除前预览完整文件且需独立确认", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await expandAll();
    await userEvent.click(await screen.findByTestId("action-disable-mcp:fs-claude-code"));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(1));
    expect(applyCalls(calls)[0].args).toEqual(["--only", "mcp:fs", "--agent", "claude-code", "--action", "disable", "--base", BASE_CLAUDE_FS]);

    await userEvent.click(screen.getByTestId("action-enable-mcp:old-snap-claude-code"));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(2));
    expect(applyCalls(calls)[1].args).toEqual(["--only", "mcp:old-snap", "--agent", "claude-code", "--action", "enable", "--base", BASE_CLAUDE_SNAPSHOT]);

    await userEvent.click(screen.getByTestId("action-remove-mcp:remote-api-codex"));
    expect(applyCalls(calls)).toHaveLength(2);
    const dialog = await screen.findByRole("dialog", { name: /从 codex 删除 remote-api/ });
    expect(await within(dialog).findByTestId("mcp-remove-preview-content")).toHaveTextContent("删除后完整配置");
    expect(within(dialog).getByText("/isolated/codex/config（已有文件）")).toBeInTheDocument();
    const removePreview = calls.find((call) => call.command === "mcp:preview" && call.args.includes("remove"));
    await userEvent.click(await screen.findByRole("button", { name: "确认删除" }));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(3));
    expect(applyCalls(calls)[2].args).toEqual(["--only", "mcp:remote-api", "--agent", "codex", "--action", "remove", "--base", BASE_CODEX_REMOTE]);
    expect(removePreview?.args).toEqual(applyCalls(calls)[2].args);
  });

  it("删除已保存的停用内容时明确说明文件不变；预览失败时不能删除", async () => {
    const { calls, unmount } = renderWithEngine(<McpPage />, routes());
    await expandAll();
    await userEvent.click(await screen.findByTestId("action-remove-mcp:old-snap-claude-code"));
    const dialog = await screen.findByRole("dialog", { name: /从 claude-code 删除 old-snap/ });
    expect(within(dialog).getByText("删除已保存内容")).toBeInTheDocument();
    expect(await within(dialog).findByText("只删除已保存的停用内容，助手配置文件不变。")).toBeInTheDocument();
    expect(within(dialog).queryByTestId("mcp-remove-preview-content")).toBeNull();
    expect(applyCalls(calls)).toHaveLength(0);
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(applyCalls(calls)).toHaveLength(0);
    unmount();

    const failing = routes();
    failing["mcp:preview"] = makeEnvelope(null, {
      command: "mcp.preview", ok: false,
      errors: [{ code: "MCP_ERROR", element: null, message: "配置文件已变化" }],
    });
    const failed = renderWithEngine(<McpPage />, failing);
    await expandAll();
    await userEvent.click(await screen.findByTestId("action-remove-mcp:remote-api-codex"));
    const failedDialog = await screen.findByRole("dialog", { name: /从 codex 删除 remote-api/ });
    expect(await within(failedDialog).findByRole("alert")).toHaveTextContent("配置文件已变化");
    expect(within(failedDialog).getByRole("button", { name: "确认删除" })).toBeDisabled();
    expect(applyCalls(failed.calls)).toHaveLength(0);
  });

  it("同一条目取消后重新打开时等待新预览，不沿用上次的文件结果", async () => {
    let previews = 0;
    let finishSecond: ((result: Envelope) => void) | undefined;
    const handlers = routes();
    handlers["mcp:preview"] = async () => {
      previews += 1;
      if (previews === 2) return new Promise<Envelope>((resolve) => { finishSecond = resolve; });
      return makeEnvelope({ files: [{ path: "/isolated/codex/config", exists: true, content: "第一次的结果" }] });
    };
    renderWithEngine(<McpPage />, handlers);
    await expandAll();
    const remove = await screen.findByTestId("action-remove-mcp:remote-api-codex");
    await userEvent.click(remove);
    const first = await screen.findByRole("dialog", { name: /从 codex 删除 remote-api/ });
    expect(await within(first).findByTestId("mcp-remove-preview-content")).toHaveTextContent("第一次的结果");
    await userEvent.click(within(first).getByRole("button", { name: "取消" }));
    await userEvent.click(remove);
    const second = await screen.findByRole("dialog", { name: /从 codex 删除 remote-api/ });
    expect(within(second).queryByTestId("mcp-remove-preview-content")).toBeNull();
    expect(within(second).getByRole("button", { name: "确认删除" })).toBeDisabled();
    await waitFor(() => expect(finishSecond).toBeDefined());
    finishSecond?.(makeEnvelope({ files: [{ path: "/isolated/codex/config", exists: true, content: "第二次的结果" }] }));
    expect(await within(second).findByTestId("mcp-remove-preview-content")).toHaveTextContent("第二次的结果");
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

  it("默认收起没有问题的服务，只显示汇总；展开后是逐账号的条目与操作", async () => {
    renderWithEngine(<McpPage />, routes());
    const card = await screen.findByTestId("mcp-card-mcp:old-snap");
    expect(within(card).queryByTestId("action-enable-mcp:old-snap-claude-code")).toBeNull();
    expect(screen.getByTestId("mcp-summary-mcp:old-snap")).toHaveTextContent("已停用");
    await userEvent.click(within(card).getByRole("button", { name: /在各账号上的条目/ }));
    expect(await within(card).findByTestId("action-enable-mcp:old-snap-claude-code")).toBeInTheDocument();
  });

  it("补齐：在用服务从同格式账号复制到还没有它的账号，带来源条目的 base", async () => {
    const status = mcpStatusData();
    status.targets = [...(status.targets ?? []), { ...(status.targets ?? [])[0], agent: "work", path: "/work/.claude.json" }];
    const { calls } = renderWithEngine(<McpPage />, routes(status));
    await userEvent.click(await screen.findByTestId("mcp-fill"));
    await waitFor(() => expect(applyCalls(calls).length).toBeGreaterThan(0));
    // fs 在 claude-code 上启用：复制给同为 Claude 格式的 work；停用或 Codex 的不参与
    expect(applyCalls(calls).map((call) => call.args)).toContainEqual(
      ["--only", "mcp:fs", "--agent", "claude-code", "--to", "work", "--action", "copy", "--base", BASE_CLAUDE_FS],
    );
    expect(applyCalls(calls).every((call) => call.args[call.args.indexOf("--to") + 1] === "work")).toBe(true);
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
