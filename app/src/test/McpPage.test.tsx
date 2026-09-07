import { describe, expect, it } from "vitest";
import McpPage from "../pages/McpPage";
import type { StatusData } from "../lib/contract";
import { makeElement, makeEnvelope, makeFreshness } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

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
            "claude-code": { state: "drifted", target: "/tmp/demo/.claude.json", target_exists: true },
            codex: { state: "managed-ok", target: "/tmp/demo/.codex/config.toml", target_exists: true },
          },
          env: {},
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
            "claude-code": { state: "leftover", target: "/tmp/demo/.claude.json", target_exists: true },
            codex: { state: "not-materialized", target: "/tmp/demo/.codex/config.toml", target_exists: false },
          },
          env: { SERENA_KEY: { placeholder: "SERENA_KEY", resolved: false } },
        },
        state: "drifted",
        freshness: makeFreshness({ state: "current", current: "a1b2c3d", upstream: "a1b2c3d", checked_at: "2026-09-05T10:00:00Z", probe: "git-ls-remote", cache_age_sec: 30 }),
      }),
    ],
  };
  return {
    "mcp:status": makeEnvelope(status, { command: "mcp.status" }),
    "mcp:plan": makeEnvelope(
      { domain: "mcp", actions: [], summary: { read: 0, "write-local": 0, "exec-local": 0, network: 0, "install-remove": 0 } },
      { command: "mcp.plan" },
    ),
  };
}

describe("McpPage 注入矩阵", () => {
  it("渲染 server × agent 单元格：已配置/不一致/未配置/残留/尚未生成", async () => {
    renderWithEngine(<McpPage />, mcpRoutes());

    // 矩阵行出现
    const row = await screen.findByText("mcp:chrome-devtools");
    expect(row).toBeInTheDocument();

    // chrome-devtools: codex 已注入、claude-code 漂移（漂移格琥珀色）
    const injected = screen.getByTitle("codex: managed-ok");
    expect(injected).toHaveTextContent("已配置");
    const drifted = screen.getByTitle("claude-code: drifted");
    expect(drifted).toHaveTextContent("不一致");
    expect(drifted.className).toContain("amber");

    // serena: leftover 遗留 / not-materialized 未物化
    expect(screen.getByTitle("claude-code: leftover")).toHaveTextContent("残留");
    expect(screen.getByTitle("codex: not-materialized")).toHaveTextContent("尚未生成");

    // freshness 徽章在行尾
    expect(screen.getByText(/可更新 1\.2\.0 → 1\.8\.0/)).toBeInTheDocument();
  });

  it("点击 server 展开详情：命令、密钥变量配置状态（不含值）、附加参数、写入位置", async () => {
    renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByText("mcp:serena"));

    const detail = await screen.findByTestId("mcp-detail-mcp:serena");
    expect(detail).toHaveTextContent("uvx --from git+https://example.com/serena@main serena");
    expect(detail).toHaveTextContent("启动超时 60 秒");
    expect(detail).toHaveTextContent("--context codex");
    // env 占位符：变量名 + 未解析标注，永不出现值
    expect(detail).toHaveTextContent("SERENA_KEY");
    expect(detail).toHaveTextContent("未配置");
    expect(detail).toHaveTextContent("/tmp/demo/.codex/config.toml");
  });
});
