import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { toast } from "sonner";
import McpPage from "../pages/McpPage";
import type { StatusData } from "../lib/contract";
import { makeApplyResult, makeElement, makeEnvelope, makeFreshness } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

const LONG_VERSION = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

function mcpRoutes(): Record<string, ReturnType<typeof makeEnvelope>> {
  const status: StatusData = {
    domain: "mcp",
    elements: [
      makeElement({
        id: "mcp:chrome-devtools",
        domain: "mcp",
        spec: { command: "npx", args: ["-y", "chrome-devtools-mcp"], startup_timeout_sec: 120, agents: null, env: {} },
        observed: {
          agents: {
            "claude-code": {
              state: "drifted",
              target: "/tmp/demo/.claude.json",
              target_exists: true,
              assigned: true,
              enabled: true,
              version: LONG_VERSION,
              available_version: LONG_VERSION,
              actions: ["update"],
            },
            codex: { state: "managed-ok", target: "/tmp/demo/.codex/config.toml", target_exists: true, assigned: true, enabled: true, actions: ["disable", "remove"] },
          },
          env: {},
          actions: ["enable", "disable", "remove"],
        },
        state: "drifted",
        freshness: makeFreshness({ state: "behind", current: "1.2.0", upstream: "1.8.0", checked_at: "2026-09-05T10:00:00Z", probe: "npm-view", cache_age_sec: 60 }),
      }),
      makeElement({
        id: "mcp:serena",
        domain: "mcp",
        spec: {
          command: "uvx",
          args: ["--from", "git+https://example.com/serena@main", "serena"],
          extra_args_by_agent: { codex: ["--context", "codex"] },
          startup_timeout_sec: 60,
          agents: null,
          env: { SERENA_KEY: "${SERENA_KEY}" },
        },
        observed: {
          agents: {
            "claude-code": { state: "leftover", target: "/tmp/demo/.claude.json", target_exists: true, assigned: false, enabled: false, actions: ["remove"] },
            codex: { state: "not-materialized", target: "/tmp/demo/.codex/config.toml", target_exists: false, assigned: true, enabled: true, actions: [] },
          },
          env: { SERENA_KEY: { placeholder: "SERENA_KEY", resolved: false } },
          actions: ["disable", "remove"],
        },
        state: "drifted",
        freshness: makeFreshness({ state: "current", current: "a1b2c3d", upstream: "a1b2c3d", checked_at: "2026-09-05T10:00:00Z", probe: "git-ls-remote", cache_age_sec: 30 }),
      }),
    ],
  };
  return {
    "mcp:status": makeEnvelope(status, { command: "mcp.status" }),
    "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
  };
}

describe("McpPage 服务库 + 助手中的服务", () => {
  it("默认服务库：卡片渲染与 freshness；点击服务名展开详情（命令/密钥状态/附加参数/写入位置，不含密钥值）", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());

    // 默认进入服务库
    expect(await screen.findByRole("tab", { name: "服务库" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText(/可更新 1\.2\.0 → 1\.8\.0/)).toBeInTheDocument();
    // 凭据只显示状态统计
    expect(screen.getByText("密钥变量 0/1 已配置")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "mcp:serena" }));
    const detail = await screen.findByTestId("mcp-detail-mcp:serena");
    expect(detail).toHaveTextContent("uvx --from git+https://example.com/serena@main serena");
    expect(detail).toHaveTextContent("启动超时 60 秒");
    expect(detail).toHaveTextContent("--context codex");
    // env 占位符：变量名 + 未解析标注，永不出现值
    expect(detail).toHaveTextContent("SERENA_KEY");
    expect(detail).toHaveTextContent("未配置");
    expect(detail).toHaveTextContent("/tmp/demo/.codex/config.toml");
    // 展开详情为纯状态展示：任何元素都不发起 plan
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("助手中的服务：单元格态 已配置/不一致/残留/尚未生成，版本短摘要 title 存完整值", async () => {
    renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    expect(await screen.findByTitle("codex: managed-ok")).toHaveTextContent("已配置");
    const drifted = screen.getByTitle("claude-code: drifted");
    expect(drifted).toHaveTextContent("不一致");
    expect(drifted.className).toContain("amber");
    expect(screen.getByTitle("claude-code: leftover")).toHaveTextContent("残留");
    expect(screen.getByTitle("codex: not-materialized")).toHaveTextContent("尚未生成");

    // 64 位摘要只显示短形，完整值在 title（资源库版本与当前配置版本两列）
    const versionCells = screen.getAllByTitle(LONG_VERSION);
    expect(versionCells).toHaveLength(2);
    expect(versionCells[0]).toHaveTextContent("0123456");
    expect(versionCells[0]).not.toHaveTextContent(LONG_VERSION);

    // 助手筛选收窄行
    const agentSelect = screen.getByTestId("filter-agent-select") as HTMLSelectElement;
    await userEvent.selectOptions(agentSelect, "codex");
    expect(screen.queryByTitle("claude-code: drifted")).toBeNull();
    expect(screen.getByTitle("codex: managed-ok")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /清除筛选/ }));
    expect(screen.getByTitle("claude-code: drifted")).toBeInTheDocument();
  });

  it("行操作：按 cell.actions 渲染，「更新」点击直接 mcp:apply 精确 --only/--agent/--action（带 run_id），不取 plan、不开弹窗、零 policy:set", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    // actions 之外的按钮不出现（更新与启用不混）
    expect(await screen.findByTestId("action-update-mcp:chrome-devtools-claude-code")).toBeInTheDocument();
    expect(screen.queryByTestId("action-enable-mcp:chrome-devtools-claude-code")).toBeNull();
    // actions 为空的 cell 不推测动作
    expect(screen.getAllByText("当前引擎未提供操作").length).toBeGreaterThan(0);

    await userEvent.click(screen.getByTestId("action-update-mcp:chrome-devtools-claude-code"));
    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));

    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "mcp:chrome-devtools", "--agent", "claude-code", "--action", "update"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("库级删除：点击先打开 ConfirmModal，取消时零 apply/零 policy:set", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());

    await userEvent.click(await screen.findByTestId("remove-server-mcp:chrome-devtools"));
    const dialog = await screen.findByRole("dialog", { name: "删除服务 chrome-devtools" });
    expect(dialog).toHaveTextContent(/但不会卸载软件/);
    expect(screen.getByTestId("confirm-modal-input")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    expect(calls.some((call) => call.command === "mcp:apply")).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("库级删除：输入服务名确认后 mcp:apply --action remove（带 run_id），无 plan、零 policy:set", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());

    await userEvent.click(await screen.findByTestId("remove-server-mcp:chrome-devtools"));
    await screen.findByRole("dialog", { name: "删除服务 chrome-devtools" });
    await userEvent.type(screen.getByTestId("confirm-modal-input"), "chrome-devtools");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));
    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "mcp:chrome-devtools", "--action", "remove"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("服务 × 助手 remove：点击先确认，确认后 mcp:apply 精确 --only/--agent/--action remove，取消零 apply", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    const removeBtn = await screen.findByTestId("action-remove-mcp:chrome-devtools-codex");
    await userEvent.click(removeBtn);
    const dialog = await screen.findByRole("dialog", { name: "从此助手删除 mcp:chrome-devtools @ codex" });
    expect(dialog).toHaveTextContent(/只清理该助手的配置/);

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls.some((call) => call.command === "mcp:apply")).toBe(false);

    await userEvent.click(removeBtn);
    await screen.findByRole("dialog", { name: "从此助手删除 mcp:chrome-devtools @ codex" });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.filter((call) => call.command === "mcp:apply")).toHaveLength(1));
    expect(calls.filter((call) => call.command === "mcp:apply")[0].args).toEqual([
      "--only", "mcp:chrome-devtools", "--agent", "codex", "--action", "remove",
    ]);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("mcp:status 返回 ok:false 且 data:{} 时显示 DOMAIN_NOT_IMPLEMENTED 错误结构，不白屏", async () => {
    const errorRoutes = {
      "mcp:status": makeEnvelope(
        {} as unknown as StatusData,
        {
          command: "mcp.status",
          ok: false,
          errors: [{ code: "DOMAIN_NOT_IMPLEMENTED", element: null, message: "MCP 域尚未实现" }],
        },
      ),
    };
    renderWithEngine(<McpPage />, errorRoutes);

    expect(await screen.findByRole("heading", { name: "MCP" })).toBeInTheDocument();
    expect(screen.getByText("DOMAIN_NOT_IMPLEMENTED")).toBeInTheDocument();
    expect(screen.getByText("MCP 域尚未实现")).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "服务库" })).toBeNull();
  });
});

describe("McpPage 显式接管通道（--allow-takeover）", () => {
  const CONFLICT_MESSAGE = "目标文件 /tmp/demo/.claude.json 中已存在非受管同名条目 'take-svc'，拒绝覆盖以防止冲突";

  function takeoverRoutes(): Record<string, ReturnType<typeof makeEnvelope> | ((args: string[]) => Promise<ReturnType<typeof makeEnvelope>>)> {
    const status: StatusData = {
      domain: "mcp",
      elements: [
        makeElement({
          id: "mcp:take-svc",
          domain: "mcp",
          spec: { command: "npx", args: ["-y", "take-mcp"], agents: null, env: {} },
          observed: {
            agents: {
              // 冲突 cell：drifted + 非受管同名条目 blocked_reason + 无动作（导入后落地断点场景）
              "claude-code": { state: "drifted", target: "/tmp/demo/.claude.json", target_exists: true, assigned: false, enabled: false, actions: [], blocked_reason: "存在外部非受管同名条目，拒绝操作以防止冲突" },
              codex: { state: "missing", target: "/tmp/demo/.codex/config.toml", target_exists: true, assigned: false, enabled: false, actions: ["add"] },
            },
            env: {},
            actions: ["disable", "remove"],
          },
          state: "drifted",
        }),
      ],
    };
    return {
      "mcp:status": makeEnvelope(status, { command: "mcp.status" }),
      // 引擎 fail-closed：无 --allow-takeover 一律拒绝，带 flag 才执行
      "mcp:apply": async (args: string[]) => {
        if (!args.includes("--allow-takeover")) {
          return makeEnvelope(
            {},
            { command: "mcp.apply", ok: false, errors: [{ code: "PREFLIGHT_FAILED", element: null, message: CONFLICT_MESSAGE }] },
          );
        }
        return makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" });
      },
    };
  }

  it("add 遇「非受管同名条目」fail-closed → 失败 toast 提供「接管原条目并重试」→ 点击直接以原范围 + --allow-takeover 重试 apply（不取 plan、无二次确认）", async () => {
    const { calls } = renderWithEngine(<McpPage />, takeoverRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    // codex cell 提供正常「添加」：点击直接 apply，首次因冲突 fail-closed
    await userEvent.click(await screen.findByTestId("action-add-mcp:take-svc-codex"));
    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));
    const firstApply = calls.find((call) => call.command === "mcp:apply");
    expect(firstApply!.args).toEqual(["--only", "mcp:take-svc", "--agent", "codex", "--action", "add"]);
    expect(firstApply!.args).not.toContain("--allow-takeover");
    expect(firstApply!.options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);

    // 失败反馈外提供接管动作；全程无弹窗、无二次确认
    const takeoverAction = await screen.findByRole("button", { name: "接管原条目并重试" });
    expect(screen.queryByRole("dialog")).toBeNull();
    await userEvent.click(takeoverAction);

    await waitFor(() => expect(calls.filter((call) => call.command === "mcp:apply")).toHaveLength(2));
    const retryApply = calls.filter((call) => call.command === "mcp:apply")[1];
    expect(retryApply.args).toEqual(["--only", "mcp:take-svc", "--agent", "codex", "--action", "add", "--allow-takeover"]);
    expect(retryApply.args).not.toContain("--dry-run");
    expect(retryApply.options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("冲突 cell 行内「接管原条目」：一键直接 apply 带 --allow-takeover，不取 plan、不开弹窗，成功走既有执行 toast", async () => {
    const toastSuccessSpy = vi.spyOn(toast, "success");
    const { calls } = renderWithEngine(<McpPage />, takeoverRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    // 冲突 cell：无常规动作，提供显式接管入口（assigned=false → add）
    expect(screen.queryByTestId("action-add-mcp:take-svc-claude-code")).toBeNull();
    await userEvent.click(await screen.findByTestId("takeover-mcp:take-svc-claude-code"));

    await waitFor(() => expect(toastSuccessSpy).toHaveBeenCalled());
    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "mcp:take-svc", "--agent", "claude-code", "--action", "add", "--allow-takeover"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(toastSuccessSpy).toHaveBeenCalledWith(expect.stringContaining("接管原条目 mcp:take-svc @ claude-code"));
  });

  it("apply 因非冲突原因失败：失败 toast 可观测，不提供接管重试", async () => {
    const failureRoutes = takeoverRoutes();
    failureRoutes["mcp:apply"] = async () =>
      makeEnvelope(
        {},
        { command: "mcp.apply", ok: false, errors: [{ code: "APPLY_FAILED", element: null, message: "凭证变量未配置: SOME_TOKEN" }] },
      );
    const toastErrorSpy = vi.spyOn(toast, "error");
    const { calls } = renderWithEngine(<McpPage />, failureRoutes);
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    await userEvent.click(await screen.findByTestId("action-add-mcp:take-svc-codex"));

    await waitFor(() => expect(toastErrorSpy).toHaveBeenCalled());
    expect(toastErrorSpy).toHaveBeenCalledWith(expect.stringContaining("凭证变量未配置"));
    // 给潜在的接管动作 toast 一个节拍，确认不会出现
    await new Promise((resolve) => setTimeout(resolve, 50));
    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "mcp:take-svc", "--agent", "codex", "--action", "add"]);
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(toastErrorSpy).not.toHaveBeenCalledWith(expect.stringContaining("可显式接管"));
    expect(screen.queryByRole("button", { name: "接管原条目并重试" })).toBeNull();
  });
});
