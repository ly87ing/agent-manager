import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import UsagePage from "../pages/UsagePage";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

function routes(overrides: Record<string, Envelope> = {}): Record<string, Envelope> {
  const hour = (h: string) => `${new Date().toISOString().slice(0, 10)}T${h}:00:00Z`;
  return {
    "usage:status": makeEnvelope(
      {
        domain: "usage",
        rows: [
          { hour: hour("02"), agent: "claude", model: "claude-opus-5", input: 1_000_000, output: 200_000, cache_write: 0, cache_read: 0, requests: 12 },
          { hour: hour("03"), agent: "codex", model: "gpt-5.6-sol", input: 500_000, output: 10_000, cache_write: 0, cache_read: 0, requests: 3 },
        ],
        pricing: { "claude-opus-5": { input: 5, output: 25, cache_write: 6.25, cache_read: 0.5 } },
        last_scan_at: "2026-09-20T03:00:00.000Z",
      },
      { command: "usage.status" },
    ),
    "usage:scan": makeEnvelope({ domain: "usage", action: "scan", files: 2, bytes: 4096 }, { command: "usage.scan" }),
    ...overrides,
  };
}

describe("用量页", () => {
  it("合计只算有单价的，并点名哪些模型没算进去", async () => {
    renderWithEngine(<UsagePage />, routes());
    // opus：1M input × $5 + 0.2M output × $25 = 10
    await waitFor(() => expect(screen.getByTestId("usage-total-cost")).toHaveTextContent("$10.00"));
    const unpriced = screen.getByTestId("usage-unpriced");
    expect(unpriced).toHaveTextContent("gpt-5.6-sol");
  });

  it("说清楚这是按 API 价折算，不是账单", async () => {
    renderWithEngine(<UsagePage />, routes());
    expect(await screen.findByText(/不是你的账单/)).toBeInTheDocument();
  });

  it("助手与模型筛选同步驱动汇总、未定价提示和明细，切换时间范围会清除筛选", async () => {
    renderWithEngine(<UsagePage />, routes());
    const today = new Date().toISOString().slice(0, 10);
    const claudeRow = `usage-row-${today}-claude-opus-5`;
    const codexRow = `usage-row-${today}-gpt-5.6-sol`;

    await screen.findByTestId(claudeRow);
    await userEvent.selectOptions(screen.getByTestId("usage-agent-filter"), "codex");
    expect(screen.queryByTestId(claudeRow)).toBeNull();
    expect(screen.getByTestId(codexRow)).toBeInTheDocument();
    expect(screen.getByTestId("usage-total-cost")).toHaveTextContent("$0.00");
    expect(screen.getByTestId("usage-unpriced")).toHaveTextContent("gpt-5.6-sol");
    expect(screen.getByTestId("usage-model-filter")).toHaveTextContent("gpt-5.6-sol");

    await userEvent.selectOptions(screen.getByTestId("usage-model-filter"), "gpt-5.6-sol");
    await userEvent.click(screen.getByTestId("usage-range-30"));
    expect(screen.getByTestId("usage-agent-filter")).toHaveValue("");
    expect(screen.getByTestId("usage-model-filter")).toHaveValue("");
    expect(screen.getByTestId(claudeRow)).toBeInTheDocument();
    expect(screen.getByTestId(codexRow)).toBeInTheDocument();
    expect(screen.getByTestId("usage-total-cost")).toHaveTextContent("$10.00");
  });

  it("没单价的那行费用留空，不拿 0 冒充", async () => {
    renderWithEngine(<UsagePage />, routes());
    const row = await screen.findByTestId(
      `usage-row-${new Date().toISOString().slice(0, 10)}-gpt-5.6-sol`,
    );
    expect(row).toHaveTextContent("—");
    expect(row).not.toHaveTextContent("$0.00");
  });

  it("给没单价的模型填单价：整张表一次写回，不走带点号的键路径", async () => {
    const withSet = routes({
      "policy:set": makeEnvelope({ effective: {}, layers: {}, attribution: {} }, { command: "policy.set" }),
    });
    const { calls } = renderWithEngine(<UsagePage />, withSet);

    await userEvent.click(await screen.findByTestId("set-price-gpt-5.6-sol"));
    await userEvent.type(screen.getByTestId("price-input"), "1.5");
    await userEvent.type(screen.getByTestId("price-output"), "6");
    await userEvent.click(screen.getByTestId("price-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const write = calls.find((call) => call.command === "policy:set")!;
    // 模型名里的点号会被键路径当分隔符，所以键只能到 usage.pricing，值是整张表
    expect(write.args![write.args!.indexOf("--key") + 1]).toBe("usage.pricing");
    const payload = JSON.parse(write.args![write.args!.indexOf("--value") + 1]);
    expect(payload["gpt-5.6-sol"]).toEqual({ input: 1.5, output: 6 });
    // 原有的单价不能被这次写入抹掉
    expect(payload["claude-opus-5"].input).toBe(5);
    expect(payload._replace).toBe(true);
  });

  it("重新统计触发扫描并回读", async () => {
    const { calls } = renderWithEngine(<UsagePage />, routes());
    await userEvent.click(await screen.findByTestId("usage-rescan"));
    await waitFor(() => expect(calls.some((call) => call.command === "usage:scan")).toBe(true));
  });

  it("没有记录时给出下一步，而不是空白", async () => {
    const empty = routes({
      "usage:status": makeEnvelope(
        { domain: "usage", rows: [], pricing: {}, last_scan_at: null },
        { command: "usage.status" },
      ),
    });
    renderWithEngine(<UsagePage />, empty);
    expect(await screen.findByTestId("usage-empty")).toHaveTextContent("重新统计");
  });
});
