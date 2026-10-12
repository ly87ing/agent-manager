import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import { useBatchApply } from "../lib/useBatchApply";
import { BatchResultPanel } from "../components/BatchResultPanel";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function Batch() {
  const batch = useBatchApply();
  return <>
    <button onClick={() => void batch.applyBatch({ actionLabel: "添加", rows: ["alpha", "beta"].map((name) => ({ elementId: `skill:source/${name}`, agent: "codex", action: "add", label: `${name} @ codex`, takeover: false })) })}>开始批量</button>
    {batch.result && <BatchResultPanel batch={batch.result} busy={batch.executing} onRetry={() => void batch.retryFailed(batch.result!)} />}
  </>;
}

describe("批量结果与失败重试", () => {
  for (const allowed of [true, false]) it(`保留成功项，仅重新读取并检查失败目标（当前动作允许=${allowed}）`, async () => {
    let betaAttempts = 0;
    const { calls } = renderWithEngine(<Batch />, {
      "skills:apply": async (args) => {
        const failed = args.includes("skill:source/beta") && betaAttempts++ === 0;
        return makeEnvelope({}, { ok: !failed, errors: failed ? [{ code: "FIXTURE", element: null, message: "Fixture failure" }] : [] });
      },
      "skills:status": makeEnvelope({ elements: [{ id: "skill:source/beta", observed: { agents: { codex: { actions: allowed ? ["add"] : [] } } } }] }),
    });
    await userEvent.click(screen.getByRole("button", { name: "开始批量" }));
    await screen.findByText("成功 1 · 失败 1");
    expect(screen.getByText(/beta @ codex：Fixture failure/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "重新检查并重试失败项" }));
    if (allowed) await screen.findByText("成功 2 · 失败 0");
    else await screen.findByText(/目标状态已变化/);
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(allowed ? 3 : 2));
    expect(calls.filter((call) => call.command === "skills:apply" && call.args.includes("skill:source/alpha"))).toHaveLength(1);
    expect(calls.filter((call) => call.command === "skills:status")).toHaveLength(1);
    if (allowed) expect(calls.slice(-2).map((call) => call.command)).toEqual(["skills:status", "skills:apply"]);
  });
});
