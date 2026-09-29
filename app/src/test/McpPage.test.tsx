import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { toast } from "sonner";
import McpPage from "../pages/McpPage";
import type { Envelope, StatusData } from "../lib/contract";
import { makeApplyResult, makeEnvelope } from "./fixtures";
import { BASE_CLAUDE_FS, BASE_CLAUDE_SNAPSHOT, BASE_CODEX_FS, BASE_CODEX_REMOTE, mcpStatusData, mcpTargets } from "./mcp-fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import { decodeShareLink } from "../lib/shareLink";

/** 等服务列表出现（开关直接在行上，不用再展开）。 */
async function expandAll() {
  await screen.findAllByTestId(/^mcp-card-/);
}

/** 删除确认框：按服务名找。 */
async function openRemove(id: string) {
  await expandAll();
  await userEvent.click(screen.getByTestId(`remove-${id}`));
  return screen.findByRole("dialog", { name: `删除 ${id.replace(/^mcp:/, "")}` });
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
    // 每个账号一个开关，悬停看到这个账号里的那一条；内容不同时行上标出来
    expect(within(card).getByTestId("action-disable-mcp:fs-claude-code").getAttribute("title")).toContain("npx -y fs-mcp");
    expect(within(card).getByTestId("action-disable-mcp:fs-codex").getAttribute("title")).toContain("npx -y fs-mcp@2");
    expect(within(card).getByText(/各账号内容不同/)).toBeInTheDocument();
    // 一句话说明取包名，完整命令在悬停里
    expect(within(card).getByText("本地命令 · fs-mcp")).toBeInTheDocument();
    expect(within(card).getByTestId("configure-secrets-mcp:fs")).toBeInTheDocument();
    expect(screen.getByTestId("mcp-counts")).toHaveTextContent("共 3 个服务，2 个已停用");
    // 停用快照与未分配的已保存定义
    expect(screen.getByTestId("action-enable-mcp:old-snap-claude-code")).toHaveAttribute("data-tone", "off");
    expect(screen.getByTestId("action-enable-mcp:old-snap-claude-code").getAttribute("title")).toContain("内容保存在本工具里");
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
    expect(screen.getByTestId("action-disable-mcp:fs-claude-code")).toBeInTheDocument();
  });

  it("编辑只有一个入口：写到所有有它的账号，各带自己条目的 base；内容不同时可选以哪个账号为准，字面量按保留原值提交", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await expandAll();
    await userEvent.click(screen.getByTestId("edit-mcp:fs"));
    let dialog = await screen.findByRole("dialog", { name: /编辑 fs/ });
    expect(within(dialog).getByTestId("server-edit-targets")).toHaveTextContent("保存后写入：claude-code、codex");
    // 两个账号内容不同：提示保存后统一，并可切换预填来源
    await userEvent.selectOptions(within(dialog).getByTestId("server-edit-base"), "codex");
    dialog = await screen.findByRole("dialog", { name: /编辑 fs/ });
    expect(within(dialog).getByTestId("server-name-input")).toBeDisabled();
    expect(within(dialog).getByTestId("server-transport-remote")).toBeDisabled();
    expect(within(dialog).getByTestId("server-args-input")).toHaveValue("-y\nfs-mcp@2");
    expect(within(dialog).getByTestId("server-env-input")).toHaveValue("TOKEN=（保留原值）");
    expect(within(dialog).getByTestId("server-timeout-input")).toHaveValue("30");
    expect(within(dialog).getByTestId("server-preserved-fields")).toHaveTextContent("cwd、enabled_tools、tools");

    const args = within(dialog).getByTestId("server-args-input");
    await userEvent.clear(args);
    await userEvent.type(args, "-y{enter}fs-mcp@3");
    const env = within(dialog).getByTestId("server-env-input");
    await userEvent.clear(env);
    await userEvent.type(env, "TOKEN=<literal>{enter}MODE=fast");
    expect(await within(dialog).findByTestId("mcp-preview-content-codex")).toHaveTextContent("fs-mcp@3");
    expect(within(dialog).getByTestId("mcp-preview-content-claude-code")).toHaveTextContent("fs-mcp@3");
    const codexPreview = calls.filter((call) => call.command === "mcp:preview" && call.args.includes("codex")).pop();
    expect(codexPreview?.args.slice(0, 8)).toEqual(["--only", "mcp:fs", "--agent", "codex", "--action", "edit", "--base", BASE_CODEX_FS]);
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));

    await waitFor(() => expect(applyCalls(calls)).toHaveLength(2));
    const [claude, codex] = applyCalls(calls);
    expect(claude.args.slice(0, 8)).toEqual(["--only", "mcp:fs", "--agent", "claude-code", "--action", "edit", "--base", BASE_CLAUDE_FS]);
    expect(specOf(claude.args)).toEqual({ command: "npx", args: ["-y", "fs-mcp@3"], env: { TOKEN: "<literal>", MODE: "fast" } });
    expect(codex.args.slice(0, 8)).toEqual(["--only", "mcp:fs", "--agent", "codex", "--action", "edit", "--base", BASE_CODEX_FS]);
    expect(specOf(codex.args)).toEqual({
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

    const dialog = await openRemove("mcp:remote-api");
    expect(applyCalls(calls)).toHaveLength(2);
    expect(await within(dialog).findByTestId("mcp-remove-preview-content")).toHaveTextContent("删除后完整配置");
    expect(within(dialog).getByText("删除后的 /isolated/codex/config")).toBeInTheDocument();
    const removePreview = calls.find((call) => call.command === "mcp:preview" && call.args.includes("remove"));
    await userEvent.click(within(dialog).getByRole("button", { name: "从 1 个账号删除" }));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(3));
    expect(applyCalls(calls)[2].args).toEqual(["--only", "mcp:remote-api", "--agent", "codex", "--action", "remove", "--base", BASE_CODEX_REMOTE]);
    expect(removePreview?.args).toEqual(applyCalls(calls)[2].args);
  });

  it("删除已保存的停用内容时明确说明文件不变；预览失败时不能删除", async () => {
    const { calls, unmount } = renderWithEngine(<McpPage />, routes());
    const dialog = await openRemove("mcp:old-snap");
    expect(within(dialog).getByText(/已停用，只删本工具保存的内容/)).toBeInTheDocument();
    expect(await within(dialog).findByText("配置文件不变。")).toBeInTheDocument();
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
    const failedDialog = await openRemove("mcp:remote-api");
    expect(await within(failedDialog).findByRole("alert")).toHaveTextContent("配置文件已变化");
    expect(within(failedDialog).getByRole("button", { name: "从 1 个账号删除" })).toBeDisabled();
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
    const first = await openRemove("mcp:remote-api");
    expect(await within(first).findByTestId("mcp-remove-preview-content")).toHaveTextContent("第一次的结果");
    await userEvent.click(within(first).getByRole("button", { name: "取消" }));
    const second = await openRemove("mcp:remote-api");
    expect(within(second).queryByTestId("mcp-remove-preview-content")).toBeNull();
    expect(within(second).getByRole("button", { name: "从 1 个账号删除" })).toBeDisabled();
    await waitFor(() => expect(finishSecond).toBeDefined());
    finishSecond?.(makeEnvelope({ files: [{ path: "/isolated/codex/config", exists: true, content: "第二次的结果" }] }));
    expect(await within(second).findByTestId("mcp-remove-preview-content")).toHaveTextContent("第二次的结果");
  });

  it("删除默认从所有有它的账号删，可以取消某个账号；只删勾选的，各带自己的 base", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    const dialog = await openRemove("mcp:fs");
    expect(within(dialog).getByTestId("mcp-remove-agent-claude-code")).toBeChecked();
    expect(within(dialog).getByTestId("mcp-remove-agent-codex")).toBeChecked();
    await userEvent.click(within(dialog).getByTestId("mcp-remove-agent-codex"));
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "从 1 个账号删除" })).toBeEnabled());
    await userEvent.click(within(dialog).getByRole("button", { name: "从 1 个账号删除" }));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(1));
    expect(applyCalls(calls)[0].args).toEqual(["--only", "mcp:fs", "--agent", "claude-code", "--action", "remove", "--base", BASE_CLAUDE_FS]);
  });

  it("新增直接写入所选助手：逐个助手各写一次，超时只给 Codex；新增时不接受 <literal>", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByTestId("add-server-button"));
    const dialog = await screen.findByRole("dialog", { name: "新增服务" });
    await userEvent.type(within(dialog).getByTestId("server-name-input"), "new-srv");
    // 新服务默认写到全部账号
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
    // 默认全选；这里只写 Codex
    await userEvent.click(within(dialog).getByTestId("server-agent-claude-code"));
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

  it("账号开关直接在行上：没有这个服务的账号点一下，从有它的账号复制过来，带来源 base、不覆盖", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    const card = await screen.findByTestId("mcp-card-mcp:remote-api");
    const absent = within(card).getByTestId("action-add-here-mcp:remote-api-claude-code");
    expect(absent).toHaveAttribute("data-tone", "off");
    expect(absent).toHaveAttribute("aria-checked", "false");
    await userEvent.click(absent);
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(1));
    expect(applyCalls(calls)[0].args).toEqual([
      "--only", "mcp:remote-api", "--agent", "codex", "--to", "claude-code", "--action", "copy", "--base", BASE_CODEX_REMOTE,
    ]);
  });

  it("补齐：表头按账号补齐，在用服务从同格式账号复制过来，带来源条目的 base", async () => {
    const status = mcpStatusData();
    status.targets = [...(status.targets ?? []), { ...(status.targets ?? [])[0], agent: "work", path: "/work/.claude.json" }];
    const { calls } = renderWithEngine(<McpPage />, routes(status));
    // 表头按账号补齐；Claude 主号和 Codex 已经齐了
    await userEvent.click(await screen.findByTestId("mcp-fill-work"));
    expect(screen.queryByTestId("mcp-fill-codex")).toBeNull();
    await waitFor(() => expect(applyCalls(calls).length).toBeGreaterThan(0));
    // fs 在 claude-code 上启用：复制给同为 Claude 格式的 work；停用或 Codex 的不参与
    expect(applyCalls(calls).map((call) => call.args)).toContainEqual(
      ["--only", "mcp:fs", "--agent", "claude-code", "--to", "work", "--action", "copy", "--base", BASE_CLAUDE_FS],
    );
    expect(applyCalls(calls).every((call) => call.args[call.args.indexOf("--to") + 1] === "work")).toBe(true);
  });

  it("全部补齐：一次补所有账号，与按账号补齐是同样的复制", async () => {
    const status = mcpStatusData();
    status.targets = [...(status.targets ?? []), { ...(status.targets ?? [])[0], agent: "work", path: "/work/.claude.json" }];
    const { calls } = renderWithEngine(<McpPage />, routes(status));
    await userEvent.click(await screen.findByTestId("mcp-fill-all"));
    await waitFor(() => expect(applyCalls(calls).length).toBeGreaterThan(0));
    expect(applyCalls(calls).map((call) => call.args)).toContainEqual(
      ["--only", "mcp:fs", "--agent", "claude-code", "--to", "work", "--action", "copy", "--base", BASE_CLAUDE_FS],
    );
  });

  it("停用后可以撤销：先重读状态拿这一条的新基准，再重新启用", async () => {
    let reads = 0;
    const base = routes();
    const { calls } = renderWithEngine(<McpPage />, {
      ...base,
      "mcp:status": async () => {
        reads += 1;
        const data = mcpStatusData();
        // 停用之后 claude-code 上 fs 的基准变了
        if (reads > 1) (data.elements[0].observed as { agents: Record<string, { base: string }> }).agents["claude-code"].base = "c".repeat(63) + "f";
        return makeEnvelope(data, { command: "mcp.status" });
      },
    });
    await userEvent.click(await screen.findByTestId("action-disable-mcp:fs-claude-code"));
    await userEvent.click(await screen.findByRole("button", { name: "撤销" }));
    await waitFor(() => expect(applyCalls(calls)).toHaveLength(2));
    expect(applyCalls(calls)[1].args).toEqual(["--only", "mcp:fs", "--agent", "claude-code", "--action", "enable", "--base", "c".repeat(63) + "f"]);
  });

  it("复制分享链接：明文取值不进链接，换成同名变量；链接解回来就是这条服务", async () => {
    const writeText = vi.fn(async () => undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByTestId("share-mcp:fs"));
    await waitFor(() => expect(writeText).toHaveBeenCalled());
    const link = (writeText.mock.calls[0] as unknown as [string])[0];
    const decoded = decodeShareLink(link);
    expect(decoded).toMatchObject({ kind: "mcp", name: "fs", spec: { command: "npx" } });
    expect(link).not.toContain("literal");
    expect(JSON.stringify(decoded)).toContain("${");
  });

  it("从分享链接导入：打开预填好的新增表单，保存前不写任何东西", async () => {
    const { calls } = renderWithEngine(
      <McpPage importPayload={{ kind: "mcp", name: "shared-srv", spec: { command: "uvx", args: ["shared-srv"], env: { KEY: "${SHARED_KEY}" } } }} />,
      routes(),
    );
    const dialog = await screen.findByRole("dialog", { name: "新增服务" });
    expect(within(dialog).getByTestId("server-name-input")).toHaveValue("shared-srv");
    expect(within(dialog).getByTestId("server-env-input")).toHaveValue("KEY=${SHARED_KEY}");
    expect(applyCalls(calls)).toHaveLength(0);
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
