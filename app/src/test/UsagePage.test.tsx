import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import UsagePage from "../pages/UsagePage";
import { vi } from "vitest";
import { agentManagementFixture, makeAliasStatus, makeElement, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
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

  it("按账号：主账号与独立账号分开合计，点一个账号只看它；趋势图按账号叠起来", async () => {
    const hour = (h: string) => `${new Date().toISOString().slice(0, 10)}T${h}:00:00Z`;
    const base = routes();
    const data = JSON.parse(JSON.stringify(base["usage:status"].data));
    data.rows.push({ hour: hour("04"), agent: "claude:work", model: "claude-opus-5", input: 2_000_000, output: 100_000, cache_write: 0, cache_read: 0, requests: 5 });
    renderWithEngine(<UsagePage />, { ...base, "usage:status": makeEnvelope(data, { command: "usage.status" }) });
    const table = await screen.findByTestId("usage-by-account");
    // 用量多的在前：work 2.1M > 主号 1.2M > Codex 0.51M
    const rows = within(table).getAllByTestId(/^usage-account-/).map((row) => row.getAttribute("data-testid"));
    expect(rows).toEqual(["usage-account-claude:work", "usage-account-claude", "usage-account-codex"]);
    expect(screen.getByTestId("usage-account-claude")).toHaveTextContent("Claude · 主号");
    expect(screen.getByTestId("usage-trend")).toHaveTextContent("Claude · 主号");
    await userEvent.click(within(screen.getByTestId("usage-account-claude:work")).getByRole("button"));
    expect(screen.getByTestId("usage-agent-filter")).toHaveValue("claude:work");
    expect(screen.queryByTestId("usage-by-account")).toBeNull();
  });

  it("缓存命中率 = 缓存读取 ÷（输入 + 缓存写入 + 缓存读取）", async () => {
    const base = routes();
    const data = JSON.parse(JSON.stringify(base["usage:status"].data));
    data.rows[0] = { ...data.rows[0], input: 100_000, cache_write: 100_000, cache_read: 800_000 };
    data.rows[1] = { ...data.rows[1], input: 0, cache_read: 0 };
    renderWithEngine(<UsagePage />, { ...base, "usage:status": makeEnvelope(data, { command: "usage.status" }) });
    expect(await screen.findByTestId("usage-cache-hit")).toHaveTextContent("80%");
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

  it("独立环境的用量按实例单独列出，可在助手筛选里选到", async () => {
    const withInstance = routes();
    const status = withInstance["usage:status"] as { data: { rows: Record<string, unknown>[] } };
    status.data.rows.push({
      hour: `${new Date().toISOString().slice(0, 10)}T04:00:00Z`,
      agent: "claude:cc-relay",
      model: "relay-model",
      input: 10,
      output: 1,
      cache_write: 0,
      cache_read: 0,
      requests: 1,
    });
    renderWithEngine(<UsagePage />, withInstance);
    const filter = await screen.findByTestId("usage-agent-filter");
    await waitFor(() => expect(filter).toHaveTextContent("Claude · cc-relay"));
    await userEvent.selectOptions(filter, "claude:cc-relay");
    expect(screen.getByTestId(`usage-row-${new Date().toISOString().slice(0, 10)}-relay-model`)).toBeInTheDocument();
    expect(screen.queryByTestId(`usage-row-${new Date().toISOString().slice(0, 10)}-claude-opus-5`)).toBeNull();
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

  it("最近会话：复制「进目录 + 同一个账号接着聊」的命令，只读不写", async () => {
    const writeText = vi.fn(async () => undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const now = Date.now();
    const { calls } = renderWithEngine(<UsagePage />, routes({
      "agents:status": makeEnvelope({ domain: "agents", elements: [makeElement({ id: "agents:claude-code", domain: "agents", spec: { display_name: "Claude Code", bin: "claude" }, observed: { management: agentManagementFixture("claude") } })] }, { command: "agents.status" }),
      "profiles:status": makeEnvelope({ domain: "profiles", elements: ["work", "relay"].map((id) => makeElement({ id: `profiles:${id}`, domain: "profiles", spec: { display_name: id, base_agent: "claude-code", kind: "claude", config_dir: `~/.claude-${id}`, base_url: null, model: null }, observed: { config_dir: `~/.claude-${id}`, blocked_reason: null }, state: "managed-ok" })) }, { command: "profiles.status" }),
      "aliases:status": makeEnvelope(makeAliasStatus({ cw: 'CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude' }), { command: "aliases.status" }),
      "usage:sessions": makeEnvelope({ domain: "usage", action: "sessions", accounts: [
        { agent: "claude:work", kind: "claude", sessions: [{ id: "s-work", cwd: "/work/app", title: "修登录", updated_at: new Date(now - 5 * 60_000).toISOString() }] },
        { agent: "claude:relay", kind: "claude", sessions: [{ id: "s-relay", cwd: "/work/x", title: null, updated_at: new Date(now - 3600_000).toISOString() }] },
      ] }, { command: "usage.sessions" }),
    }));
    const section = await screen.findByTestId("recent-sessions");
    const rows = within(section).getAllByRole("listitem").map((row) => row.getAttribute("data-testid"));
    expect(rows).toEqual(["session-s-work", "session-s-relay"]);
    expect(screen.getByTestId("session-s-work")).toHaveTextContent("修登录");
    expect(screen.getByTestId("session-s-work")).toHaveTextContent("5 分钟前");
    expect(screen.getByTestId("session-s-relay")).toHaveTextContent("没有标题");
    // 没有启动命令的账号不给复制
    expect(screen.getByTestId("session-copy-s-relay")).toBeDisabled();
    await userEvent.click(screen.getByTestId("session-copy-s-work"));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("cd /work/app && cw --resume s-work"));
    expect(calls.every((call) => call.command.endsWith(":status") || call.command === "usage:sessions")).toBe(true);
  });
});

