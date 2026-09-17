import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import UpdatesPage from "../pages/UpdatesPage";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode } from "./fixtures";
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
    "skills:status": makeEnvelope(
      {
        domain: "skills",
        elements: [
          makeElement({
            id: "skill:c-ahead",
            domain: "skills",
            freshness: makeFreshness({ state: "ahead", current: "9.9.9", upstream: "9.9.8", checked_at: CHECKED, probe: "git-ls-remote", cache_age_sec: 90000 }),
          }),
        ],
      },
      { command: "skills.status" },
    ),
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
      "update-row-skill:c-ahead",
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
                { domain: "system", verb: "refresh-upstream", cache_file: null, elements: [], summary: { probed: 2, current: 1, behind: 1, ahead: 0, unknown: 0 } },
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

  it("全部更新：点击直接按域顺序执行各域 scoped apply（agents → mcp → skills），不取 plan、不开弹窗", async () => {
    const routes: Record<string, Envelope> = {
      ...statusRoutes(),
      "skills:status": makeEnvelope(
        {
          domain: "skills",
          elements: [
            makeElement({
              id: "skill:agent-skills/behind-skill",
              domain: "skills",
              freshness: makeFreshness({ state: "behind", current: "1.0.0", upstream: "2.0.0" }),
            }),
          ],
        },
        { command: "skills.status" },
      ),
      "mcp:status": makeEnvelope(
        {
          domain: "mcp",
          elements: [
            makeElement({
              id: "mcp:mcp-behind",
              domain: "mcp",
              freshness: makeFreshness({ state: "behind", current: "0.1.0", upstream: "0.2.0" }),
            }),
          ],
        },
        { command: "mcp.status" },
      ),
      "agents:apply": makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }),
      "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
      "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
    };

    const { calls } = renderWithEngine(<UpdatesPage />, routes);
    const updateAllBtn = await screen.findByTestId("update-all-button");
    expect(updateAllBtn).toHaveTextContent("全部更新（3）");

    await userEvent.click(updateAllBtn);
    await waitFor(() => expect(calls.filter((c) => c.command.endsWith(":apply"))).toHaveLength(3));

    const applyCalls = calls.filter((c) => c.command.endsWith(":apply"));
    expect(applyCalls.map((c) => c.command)).toEqual(["agents:apply", "mcp:apply", "skills:apply"]);
    expect(applyCalls[0].args).toEqual(["--only", "agents:a-behind", "--action", "upgrade"]);
    expect(applyCalls[1].args).toEqual(["--only", "mcp:mcp-behind", "--action", "update"]);
    expect(applyCalls[2].args).toEqual(["--only", "skill:agent-skills/behind-skill", "--action", "update"]);
    for (const call of applyCalls) {
      expect(call.args).not.toContain("--dry-run");
      expect(call.options?.runId).toMatch(/^gui-/);
    }
    expect(calls.some((c) => c.command.endsWith(":plan"))).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("全部更新：第二域执行失败时中断，不执行第三域（成功域与失败域均有执行 toast）", async () => {
    const routes: Record<string, Envelope> = {
      ...statusRoutes(),
      "skills:status": makeEnvelope(
        {
          domain: "skills",
          elements: [
            makeElement({
              id: "skill:agent-skills/behind-skill",
              domain: "skills",
              freshness: makeFreshness({ state: "behind", current: "1.0.0", upstream: "2.0.0" }),
            }),
          ],
        },
        { command: "skills.status" },
      ),
      "mcp:status": makeEnvelope(
        {
          domain: "mcp",
          elements: [
            makeElement({
              id: "mcp:mcp-behind",
              domain: "mcp",
              freshness: makeFreshness({ state: "behind", current: "0.1.0", upstream: "0.2.0" }),
            }),
          ],
        },
        { command: "mcp.status" },
      ),
      "agents:apply": makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }),
      "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), {
        command: "mcp.apply",
        ok: false,
        errors: [{ code: "MCP_APPLY_FAILED", element: null, message: "mcp update failed" }],
      }),
      "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
    };

    const { calls } = renderWithEngine(<UpdatesPage />, routes);
    const updateAllBtn = await screen.findByTestId("update-all-button");
    await userEvent.click(updateAllBtn);

    await waitFor(() => expect(calls.filter((c) => c.command.endsWith(":apply"))).toHaveLength(2));
    const applyCalls = calls.filter((c) => c.command.endsWith(":apply"));
    expect(applyCalls.map((c) => c.command)).toEqual(["agents:apply", "mcp:apply"]);
    expect(calls.some((c) => c.command === "skills:apply")).toBe(false);
    expect(calls.some((c) => c.command.endsWith(":plan"))).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("行内更新：点击直接 apply，agents 带 --action upgrade，mcp/skills 带 --action update（带 run_id）", async () => {
    let releaseAgents: (() => void) | null = null;
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...statusRoutes(),
      "agents:apply": () =>
        new Promise<Envelope>((resolve) => {
          releaseAgents = () => resolve(makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }));
        }),
      "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
    };

    const { calls } = renderWithEngine(<UpdatesPage />, routes);

    // agents 行：behind → 更新此项，agents→upgrade
    await userEvent.click(await screen.findByTestId("update-agents:a-behind"));
    await waitFor(() => expect(calls.some((c) => c.command === "agents:apply")).toBe(true));
    const agentApply = calls.filter((c) => c.command === "agents:apply");
    expect(agentApply.map((c) => c.args)).toEqual([["--only", "agents:a-behind", "--action", "upgrade"]]);
    expect(agentApply[0].options?.runId).toMatch(/^gui-/);

    // 行内更新互斥：agents apply 在途期间 mcp 更新按钮禁用
    expect(screen.getByTestId("update-mcp:b-unknown")).toBeDisabled();
    releaseAgents!();
    await waitFor(() => expect(screen.getByTestId("update-mcp:b-unknown")).toBeEnabled());

    // mcp 行：unknown → 更新此项，mcp→update
    await userEvent.click(screen.getByTestId("update-mcp:b-unknown"));
    await waitFor(() => expect(calls.some((c) => c.command === "mcp:apply")).toBe(true));
    const mcpApply = calls.filter((c) => c.command === "mcp:apply");
    expect(mcpApply.map((c) => c.args)).toEqual([["--only", "mcp:b-unknown", "--action", "update"]]);
    expect(mcpApply[0].options?.runId).toMatch(/^gui-/);

    expect(calls.some((c) => c.command.endsWith(":plan"))).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();

    // skills 行（ahead）不出现更新按钮；not-applicable 行不进表
    expect(screen.queryByTestId("update-skill:c-ahead")).toBeNull();
    expect(screen.queryByTestId("update-mcp:x-na")).toBeNull();
  });

  it("行内更新连点 guard：同一行在途期间再点不触发第二次 apply", async () => {
    let release: (() => void) | null = null;
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...statusRoutes(),
      "agents:apply": () =>
        new Promise<Envelope>((resolve) => {
          release = () => resolve(makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }));
        }),
    };

    const { calls } = renderWithEngine(<UpdatesPage />, routes);
    const updateBtn = await screen.findByTestId("update-agents:a-behind");

    await userEvent.click(updateBtn);
    await userEvent.click(updateBtn);
    await userEvent.click(updateBtn);

    await waitFor(() => expect(calls.some((c) => c.command === "agents:apply")).toBe(true));
    expect(calls.filter((c) => c.command === "agents:apply")).toHaveLength(1);

    release!();
    await waitFor(() => expect(screen.getByTestId("update-agents:a-behind")).toBeEnabled());
  });

  it("批量更新期间所有行内更新按钮禁用", async () => {
    let release: (() => void) | null = null;
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...statusRoutes(),
      "agents:apply": () =>
        new Promise<Envelope>((resolve) => {
          release = () => resolve(makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }));
        }),
    };

    renderWithEngine(<UpdatesPage />, routes);
    const updateAllBtn = await screen.findByTestId("update-all-button");
    await userEvent.click(updateAllBtn);

    expect(screen.getByTestId("update-agents:a-behind")).toBeDisabled();

    release!();
    await waitFor(() => expect(screen.getByTestId("update-agents:a-behind")).toBeEnabled());
  });

  it("批量更新同一 tick 双击只启动一批", async () => {
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...statusRoutes(),
      "agents:apply": () =>
        new Promise<Envelope>((resolve) => {
          setTimeout(() => resolve(makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" })), 10);
        }),
    };

    const { calls } = renderWithEngine(<UpdatesPage />, routes);
    const updateAllBtn = await screen.findByTestId("update-all-button");

    fireEvent.click(updateAllBtn);
    fireEvent.click(updateAllBtn);

    await waitFor(() => expect(calls.filter((c) => c.command.endsWith(":apply"))).toHaveLength(1));
    expect(calls.filter((c) => c.command === "agents:apply")).toHaveLength(1);
    expect(calls.some((c) => c.command.endsWith(":plan"))).toBe(false);
  });

  it("行更新启动后同一 tick 批量更新不得进入", async () => {
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...statusRoutes(),
      "agents:status": makeEnvelope(
        {
          domain: "agents",
          elements: [
            makeElement({
              id: "agents:a-behind",
              domain: "agents",
              freshness: makeFreshness({ state: "behind", current: "1.0.0", upstream: "2.0.0", checked_at: CHECKED, probe: "npm-view", cache_age_sec: 10 }),
            }),
          ],
        },
        { command: "agents.status" },
      ),
      "mcp:status": makeEnvelope(
        {
          domain: "mcp",
          elements: [
            makeElement({
              id: "mcp:mcp-behind",
              domain: "mcp",
              freshness: makeFreshness({ state: "behind", current: "0.1.0", upstream: "0.2.0" }),
            }),
          ],
        },
        { command: "mcp.status" },
      ),
      "mcp:apply": () =>
        new Promise<Envelope>((resolve) => {
          setTimeout(() => resolve(makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" })), 10);
        }),
    };

    const { calls } = renderWithEngine(<UpdatesPage />, routes);
    const rowBtn = await screen.findByTestId("update-mcp:mcp-behind");
    const batchBtn = screen.getByTestId("update-all-button");

    fireEvent.click(rowBtn);
    fireEvent.click(batchBtn);

    await waitFor(() => expect(calls.some((c) => c.command === "mcp:apply")).toBe(true));
    expect(calls.filter((c) => c.command === "mcp:apply")).toHaveLength(1);
    expect(calls.filter((c) => c.command === "agents:apply")).toHaveLength(0);
    expect(calls.some((c) => c.command.endsWith(":plan"))).toBe(false);
  });

  it("批量更新占位后同一 tick 任意行更新不得进入", async () => {
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...statusRoutes(),
      "agents:status": makeEnvelope(
        {
          domain: "agents",
          elements: [
            makeElement({
              id: "agents:a-behind",
              domain: "agents",
              freshness: makeFreshness({ state: "behind", current: "1.0.0", upstream: "2.0.0", checked_at: CHECKED, probe: "npm-view", cache_age_sec: 10 }),
            }),
            makeElement({
              id: "agents:a2-behind",
              domain: "agents",
              freshness: makeFreshness({ state: "behind", current: "1.1.0", upstream: "2.0.0", checked_at: CHECKED, probe: "npm-view", cache_age_sec: 10 }),
            }),
          ],
        },
        { command: "agents.status" },
      ),
      "agents:apply": () =>
        new Promise<Envelope>((resolve) => {
          setTimeout(() => resolve(makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" })), 10);
        }),
    };

    const { calls } = renderWithEngine(<UpdatesPage />, routes);
    const batchBtn = await screen.findByTestId("update-all-button");
    const rowBtn = screen.getByTestId("update-agents:a2-behind");

    fireEvent.click(batchBtn);
    fireEvent.click(rowBtn);

    await waitFor(() => expect(calls.some((c) => c.command === "agents:apply")).toBe(true));
    const applyCalls = calls.filter((c) => c.command === "agents:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "agents:a-behind", "--only", "agents:a2-behind", "--action", "upgrade"]);
    expect(calls.some((c) => c.command.endsWith(":plan"))).toBe(false);
  });

  it("任一行在途时同一 tick 另一行不得进入 directApply", async () => {
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...statusRoutes(),
      "agents:status": makeEnvelope(
        {
          domain: "agents",
          elements: [
            makeElement({
              id: "agents:a-behind",
              domain: "agents",
              freshness: makeFreshness({ state: "behind", current: "1.0.0", upstream: "2.0.0", checked_at: CHECKED, probe: "npm-view", cache_age_sec: 10 }),
            }),
            makeElement({
              id: "agents:a2-behind",
              domain: "agents",
              freshness: makeFreshness({ state: "behind", current: "1.1.0", upstream: "2.0.0", checked_at: CHECKED, probe: "npm-view", cache_age_sec: 10 }),
            }),
          ],
        },
        { command: "agents.status" },
      ),
      "agents:apply": () =>
        new Promise<Envelope>((resolve) => {
          setTimeout(() => resolve(makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" })), 10);
        }),
    };

    const { calls } = renderWithEngine(<UpdatesPage />, routes);
    const row1 = await screen.findByTestId("update-agents:a-behind");
    const row2 = screen.getByTestId("update-agents:a2-behind");

    fireEvent.click(row1);
    fireEvent.click(row2);

    await waitFor(() => expect(calls.filter((c) => c.command === "agents:apply")).toHaveLength(1));
    expect(calls.filter((c) => c.command === "agents:apply")[0].args).toEqual(["--only", "agents:a-behind", "--action", "upgrade"]);
    expect(calls.some((c) => c.command.endsWith(":plan"))).toBe(false);
  });

  it("行内更新：label 携带版本对照，执行中按钮转 spinner，结束后恢复", async () => {
    let release: (() => void) | null = null;
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...statusRoutes(),
      "agents:apply": () =>
        new Promise<Envelope>((resolve) => {
          release = () => resolve(makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }));
        }),
    };

    renderWithEngine(<UpdatesPage />, routes);
    const rowBtn = await screen.findByTestId("update-agents:a-behind");
    expect(rowBtn).toHaveAttribute("title", expect.stringContaining("1.0.0 → 2.0.0"));

    await userEvent.click(rowBtn);
    await waitFor(() => expect(rowBtn).toHaveTextContent("正在执行…"));
    expect(rowBtn.querySelector("svg.animate-spin")).not.toBeNull();

    release!();
    expect(await screen.findByText("更新 agents:a-behind（1.0.0 → 2.0.0）完成")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("update-agents:a-behind")).toHaveTextContent("更新此项"));
  });

  it("全部更新：label 携带各元素版本对照，执行中按钮转 spinner，结束后恢复", async () => {
    let release: (() => void) | null = null;
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...statusRoutes(),
      "agents:apply": () =>
        new Promise<Envelope>((resolve) => {
          release = () => resolve(makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }));
        }),
    };

    renderWithEngine(<UpdatesPage />, routes);
    const updateAllBtn = await screen.findByTestId("update-all-button");
    expect(updateAllBtn).toHaveTextContent("全部更新（1）");

    await userEvent.click(updateAllBtn);
    await waitFor(() => expect(updateAllBtn).toHaveTextContent("正在执行…"));
    expect(updateAllBtn.querySelector("svg.animate-spin")).not.toBeNull();

    release!();
    expect(await screen.findByText("更新 Agents（agents:a-behind 1.0.0 → 2.0.0）完成")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("update-all-button")).toHaveTextContent("全部更新（1）"));
  });
});
