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
      { runs: [makeRunEntry({ run_id: "20260905T235900Z-legacy1", is_latest: true })], tail: null, lines: null },
      { command: "runs.list" },
    ),
    "runs:get": async (args: string[]) => {
      const id = args[args.indexOf("--run") + 1];
      return makeEnvelope(
        { runs: [makeRunEntry({ run_id: id })], tail: `=== engine step: mcp-sync ===\n[mock log for ${id}]\n步骤完成 exit=0\n`, lines: 200 },
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
  it("历史列表渲染 + 点击查看日志尾（runs:get --run --lines 200）", async () => {
    const { calls } = renderWithEngine(<RunsPage />, routes());
    const row = await screen.findByTestId("run-row-20260905T235900Z-legacy1");
    expect(row).toHaveTextContent("latest");
    await userEvent.click(row.querySelector("button")!);
    const tail = await screen.findByTestId("run-log-tail");
    expect(tail).toHaveTextContent("mock log for 20260905T235900Z-legacy1");
    const getCall = calls.find((call) => call.command === "runs:get");
    expect(getCall!.args).toEqual(["--run", "20260905T235900Z-legacy1", "--lines", "200"]);
  });

  it("执行状态机：running → 完成（envelope ok:false → 失败），渲染步骤摘要与锁状态，并失效重拉 runs:list", async () => {
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

    // 进行中可见（apply 挂起期间）
    expect(await screen.findByText("进行中")).toBeInTheDocument();
    const statusChip = document.querySelector(`[data-testid^="execution-status-"]`)!;
    const runId = statusChip.getAttribute("data-testid")!.replace("execution-status-", "");
    expect(runId).toMatch(/^gui-/);

    // run_id 透传到引擎调用
    const applyCall = calls.find((call) => call.command === "mcp:apply");
    expect(applyCall!.options?.runId).toBe(runId);

    // 释放 → 完成：失败态 + 步骤摘要 + 锁状态 + 引擎 errors
    releaseApply!();
    expect(await screen.findByText("失败")).toBeInTheDocument();
    expect(screen.getByText("mcp-sync")).toBeInTheDocument();
    expect(screen.getByText("hooks-sync")).toBeInTheDocument();
    expect(screen.getByText(/持锁 pid 4242/)).toBeInTheDocument();
    expect(screen.getByText("STEP_FAILED")).toBeInTheDocument();

    // 完成后失效重拉 runs:list
    const listCallsAfter = calls.filter((call) => call.command === "runs:list").length;
    expect(listCallsAfter).toBeGreaterThan(listCallsBefore);

    // 执行卡 → 日志按该 run_id 拉取
    const card = screen.getByTestId(`execution-${runId}`);
    await userEvent.click(within(card).getByText("日志"));
    const tail = await screen.findByTestId("run-log-tail");
    expect(tail).toHaveTextContent(`mock log for ${runId}`);
  });
});
