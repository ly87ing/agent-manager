import { describe, expect, it } from "vitest";
import RunsPage from "../pages/RunsPage";
import { useExecutions } from "../lib/execution";
import { makeApplyResult, makeEnvelope, makeRunEntry } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

function Trigger({ command, label }: { command: string; label: string }) {
  const { execute } = useExecutions();
  return (
    <button data-testid="trigger-execute" onClick={() => void execute({ command, label })}>
      go
    </button>
  );
}

function routes(): Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> {
  return {
    "runs:list": makeEnvelope(
      { runs: [makeRunEntry({ run_id: "20260905T235900Z-legacy1", operations: 2, last_at: "2026-09-05T23:59:00Z" })] },
      { command: "runs.list" },
    ),
    "runs:get": async (args: string[]) => {
      const id = args[args.indexOf("--run") + 1];
      return makeEnvelope(
        {
          run_id: id,
          operations: [
            { id: "op-1", operation_type: "mcp:apply", target: "mcp:grok-search", detail: { result: "ok" }, created_at: "2026-09-05T23:59:00Z" },
          ],
        },
        { command: "runs.get" },
      );
    },
    "mcp:apply": makeEnvelope(
      makeApplyResult({
        domain: "mcp",
        steps: [
          { name: "mcp-sync", ok: true, exit_code: 0 },
          { name: "hooks-sync", ok: false, exit_code: 1 },
        ],
      }),
      {
        command: "mcp.apply",
        ok: false,
        lock: { held: true, pid: 4242 },
        errors: [{ code: "STEP_FAILED", element: null, message: "hooks-sync exit 1" }],
      },
    ),
  };
}

describe("执行中心", () => {
  it("历史列表展示运行 ID、操作数、最后时间，并查看结构化操作详情", async () => {
    const { calls } = renderWithEngine(<RunsPage />, routes());
    const row = await screen.findByTestId("run-row-20260905T235900Z-legacy1");
    expect(row).toHaveTextContent("20260905T235900Z-legacy1");
    expect(row).toHaveTextContent("2");
    expect(row).toHaveTextContent("2026");
    expect(row).not.toHaveTextContent("latest");
    expect(row).not.toHaveTextContent("日志");

    await userEvent.click(within(row).getByText("详情"));
    expect(screen.getAllByText("mcp:apply")).toHaveLength(1);
    expect(screen.getByText("mcp:grok-search")).toBeInTheDocument();
    expect(screen.getByText(/"result": "ok"/)).toBeInTheDocument();

    const getCall = calls.find((call) => call.command === "runs:get");
    expect(getCall?.args).toEqual(["--run", "20260905T235900Z-legacy1"]);
  });

  it("执行状态机完成后可查看该 run 的结构化操作，并失效重拉 runs:list", async () => {
    let releaseApply: (() => void) | null = null;
    const gated = routes();
    gated["mcp:apply"] = () =>
      new Promise<Envelope>((resolve) => {
        releaseApply = () =>
          resolve(
            makeEnvelope(
              makeApplyResult({
                domain: "mcp",
                steps: [
                  { name: "mcp-sync", ok: true, exit_code: 0 },
                  { name: "hooks-sync", ok: false, exit_code: 1 },
                ],
              }),
              {
                command: "mcp.apply",
                ok: false,
                lock: { held: true, pid: 4242 },
                errors: [{ code: "STEP_FAILED", element: null, message: "hooks-sync exit 1" }],
              },
            ),
          );
      });

    const { calls } = renderWithEngine(
      <>
        <Trigger command="mcp:apply" label="收敛 mcp 域" />
        <RunsPage />
      </>,
      gated,
    );
    const listCallsBefore = calls.filter((call) => call.command === "runs:list").length;

    await userEvent.click(await screen.findByTestId("trigger-execute"));
    expect(await screen.findByText("正在执行…")).toBeInTheDocument();
    const statusChip = document.querySelector(`[data-testid^="execution-status-"]`)!;
    const runId = statusChip.getAttribute("data-testid")!.replace("execution-status-", "");
    expect(runId).toMatch(/^gui-/);

    const applyCall = calls.find((call) => call.command === "mcp:apply");
    expect(applyCall!.options?.runId).toBe(runId);

    releaseApply!();
    expect(await screen.findByText("失败")).toBeInTheDocument();
    expect(screen.getByText("mcp-sync")).toBeInTheDocument();
    expect(screen.getByText("hooks-sync")).toBeInTheDocument();
    expect(screen.getByText(/独占执行 · pid 4242/)).toBeInTheDocument();
    expect(screen.getByText("STEP_FAILED")).toBeInTheDocument();

    const listCallsAfter = calls.filter((call) => call.command === "runs:list").length;
    expect(listCallsAfter).toBeGreaterThan(listCallsBefore);

    const card = screen.getByTestId(`execution-${runId}`);
    await userEvent.click(within(card).getByText("详情"));
    expect(screen.getAllByText("mcp:apply")).toHaveLength(2);
    const getCall = calls.find((call) => call.command === "runs:get");
    expect(getCall?.args).toEqual(["--run", runId]);
  });

  it("清空任务记录：二次确认弹窗、执行 runs:clear、刷新列表并关闭操作详情", async () => {
    const r = routes();
    r["runs:clear"] = makeEnvelope({ domain: "runs", deleted: 1 }, { command: "runs.clear" });
    const { calls } = renderWithEngine(<RunsPage />, r);

    const row = await screen.findByTestId("run-row-20260905T235900Z-legacy1");
    await userEvent.click(within(row).getByText("详情"));
    expect(await screen.findByText("mcp:grok-search")).toBeInTheDocument();

    const clearBtn = screen.getByTestId("clear-runs-button");
    await userEvent.click(clearBtn);

    const confirmDialog = await screen.findByRole("dialog", { name: "清空任务记录" });
    expect(confirmDialog).toBeInTheDocument();
    expect(within(confirmDialog).getByText("清理本地记录")).toBeInTheDocument();
    expect(within(confirmDialog).queryByText("修改配置文件")).toBeNull();
    expect(screen.getByText(/此操作不可恢复/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog", { name: "清空任务记录" })).toBeNull();
    expect(calls.some((c) => c.command === "runs:clear")).toBe(false);

    await userEvent.click(clearBtn);
    const reopenDialog = await screen.findByRole("dialog", { name: "清空任务记录" });
    await userEvent.click(within(reopenDialog).getByRole("button", { name: "确认清空" }));

    expect(calls.some((c) => c.command === "runs:clear")).toBe(true);
    expect(screen.queryByText("mcp:grok-search")).toBeNull();
    expect(await screen.findByRole("status")).toHaveTextContent("任务记录已清空");
  });

  it("清空任务记录失败时展示错误反馈横幅", async () => {
    const r = routes();
    r["runs:clear"] = () => Promise.reject(new Error("SQLite busy"));
    renderWithEngine(<RunsPage />, r);

    const clearBtn = await screen.findByTestId("clear-runs-button");
    await userEvent.click(clearBtn);

    const confirmDialog = await screen.findByRole("dialog", { name: "清空任务记录" });
    await userEvent.click(within(confirmDialog).getByRole("button", { name: "确认清空" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("清空任务记录失败：SQLite busy");
  });
});
