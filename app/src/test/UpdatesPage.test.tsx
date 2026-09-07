import { describe, expect, it } from "vitest";
import UpdatesPage from "../pages/UpdatesPage";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

const CHECKED = "2026-09-05T10:00:00Z";

function statusRoutes(): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit" }),
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:z-current",
            domain: "agents",
            freshness: makeFreshness({ state: "current", current: "1.0.0", upstream: "1.0.0", checked_at: CHECKED, probe: "npm-view", cache_age_sec: 10 }),
          }),
          makeElement({
            id: "agents:a-behind",
            domain: "agents",
            freshness: makeFreshness({ state: "behind", current: "1.0.0", upstream: "2.0.0", checked_at: CHECKED, probe: "npm-view", cache_age_sec: 10 }),
          }),
        ],
      },
      { command: "agents.status" },
    ),
    "tools:status": makeEnvelope(
      {
        domain: "tools",
        elements: [
          makeElement({
            id: "tools:c-ahead",
            domain: "tools",
            freshness: makeFreshness({ state: "ahead", current: "9.9.9", upstream: "9.9.8", checked_at: CHECKED, probe: "npm-view", cache_age_sec: 90000 }),
          }),
        ],
      },
      { command: "tools.status" },
    ),
    "mcp:status": makeEnvelope(
      {
        domain: "mcp",
        elements: [
          makeElement({
            id: "mcp:b-unknown",
            domain: "mcp",
            freshness: makeFreshness({ state: "unknown" }),
          }),
          makeElement({ id: "mcp:x-na", domain: "mcp", freshness: makeFreshness({ state: "not-applicable" }) }),
        ],
      },
      { command: "mcp.status" },
    ),
    "skills:status": makeEnvelope({ domain: "skills", elements: [] }, { command: "skills.status" }),
  };
}

describe("UpdatesPage 更新中心", () => {
  it("按 behind → unknown → current → ahead 排序，not-applicable 不进表", async () => {
    renderWithEngine(<UpdatesPage />, statusRoutes());
    await screen.findByTestId("update-row-agents:a-behind");
    const rows = screen.getAllByTestId(/^update-row-/).map((row) => row.dataset.testid);
    expect(rows).toEqual([
      "update-row-agents:a-behind",
      "update-row-mcp:b-unknown",
      "update-row-agents:z-current",
      "update-row-tools:c-ahead",
    ]);
    expect(screen.queryByTestId("update-row-mcp:x-na")).toBeNull();
    // 列内容：版本对比、缓存年龄（stale 标注）、探测源
    expect(screen.getByText("1.0.0 → 2.0.0")).toBeInTheDocument();
    expect(screen.getByText("1 天前")).toBeInTheDocument();
    expect(screen.getAllByText("npm-view").length).toBeGreaterThan(0);
  });

  it("全部刷新：进行中按钮禁用，完成后恢复", async () => {
    let release: (() => void) | null = null;
    const routes: Record<string, Envelope | (() => Promise<Envelope>)> = {
      ...statusRoutes(),
      "system:refresh-upstream": () =>
        new Promise<Envelope>((resolve) => {
          release = () =>
            resolve(
              makeEnvelope(
                { domain: "system", verb: "refresh-upstream", cache_file: "/tmp/x.json", elements: [], summary: { probed: 2, current: 1, behind: 1, ahead: 0, unknown: 0 } },
                { command: "system.refresh-upstream" },
              ),
            );
        }),
    };
    renderWithEngine(<UpdatesPage />, routes);
    const button = await screen.findByTestId("refresh-all-button");
    expect(button).toBeEnabled();

    await userEvent.click(button);
    expect(button).toBeDisabled();
    expect(button).toHaveTextContent("检查全部更新");

    release!();
    expect(await screen.findByText(/检查更新完成/)).toBeInTheDocument();
    expect(screen.getByTestId("refresh-all-button")).toBeEnabled();
  });

  it("offline 模式：整表只读标注，刷新与重新探测全部禁用", async () => {
    const routes = statusRoutes();
    routes["system:audit"] = makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline: true }) });
    renderWithEngine(<UpdatesPage />, routes);

    expect(await screen.findByText(/离线模式：显示的是上次检查的结果/)).toBeInTheDocument();
    expect(screen.getByTestId("refresh-all-button")).toBeDisabled();
    expect(screen.getByTestId("reprobe-agents:a-behind")).toBeDisabled();
  });
});
