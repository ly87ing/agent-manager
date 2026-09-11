import { describe, expect, it } from "vitest";
import { DriftExplanationPanel, shortenDigest } from "../components/DriftExplanationPanel";
import { makeElement, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen } from "./render-with-engine";

const drifted = makeElement({
  id: "mcp:chrome-devtools",
  domain: "mcp",
  observed: {
    agents: {
      "claude-code": { state: "drifted", target: "/tmp/demo/.claude.json", target_exists: true },
      codex: { state: "managed-ok", target: "/tmp/demo/.codex/config.toml", target_exists: true },
    },
  },
  state: "drifted",
});

const plan = makeEnvelope(
  {
    domain: "mcp",
    actions: [
      {
        op: "write-file",
        element: "mcp:chrome-devtools",
        target: "/tmp/demo/.claude.json",
        risk: "write-local",
        before_digest: "sha256:dd9d3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
        after_digest: "sha256:89b4b5c130887752148845080715867e6c3e6f40e69189067dd7d92db8b40c36",
        backup_path: null,
        cmd: null,
        note: "注入/收敛 mcp:chrome-devtools 到 agent 'claude-code'",
      },
      {
        op: "write-file",
        element: "mcp:other-server",
        target: "/tmp/demo/.other.json",
        risk: "write-local",
        before_digest: null,
        after_digest: null,
        backup_path: null,
        cmd: null,
        note: "其它元素的动作（应被过滤）",
      },
    ],
    summary: { read: 0, "write-local": 2, "exec-local": 0, network: 0, "install-remove": 0 },
  },
  { command: "mcp.plan" },
);

describe("DriftExplanationPanel", () => {
  it("渲染目标位置、各助手状态表与过滤后的操作清单", async () => {
    const { calls } = renderWithEngine(
      <DriftExplanationPanel
        element={drifted}
        planRequest={{ command: "mcp:plan", args: ["--only", "mcp:chrome-devtools", "--agent", "claude-code", "--action", "update"] }}
        cellKind="mcp"
      />,
      {
        "mcp:plan": plan,
      },
    );

    const panel = await screen.findByTestId("drift-panel");
    // plan 为异步查询：先等动作清单就位
    await screen.findByText("注入/收敛 mcp:chrome-devtools 到 agent 'claude-code'");
    // 目标位置（cell targets）
    expect(panel).toHaveTextContent("/tmp/demo/.claude.json");
    expect(panel).toHaveTextContent("/tmp/demo/.codex/config.toml");
    // per-agent 单元格状态
    expect(panel).toHaveTextContent("claude-code");
    expect(screen.getByTitle("drifted")).toHaveTextContent("不一致");
    expect(screen.getByTitle("managed-ok")).toHaveTextContent("已配置");
    // plan 动作清单：只含本元素的 1 条（mcp:other-server 被过滤）
    expect(panel).toHaveTextContent("write-file");
    expect(panel).not.toHaveTextContent("其它元素的动作");
    expect(calls.find((call) => call.command === "mcp:plan")?.args).toEqual([
      "--only",
      "mcp:chrome-devtools",
      "--agent",
      "claude-code",
      "--action",
      "update",
    ]);
    // before/after digest 截断渲染
    expect(panel).toHaveTextContent("sha256:dd9d3c8ae…de12a2");
    expect(panel).toHaveTextContent("sha256:89b4b5c13…b40c36");
  });

  it("plan 无本元素动作时显示空态文案", async () => {
    renderWithEngine(<DriftExplanationPanel element={drifted} planRequest={{ command: "mcp:plan", args: ["--only", drifted.id, "--agent", "claude-code", "--action", "update"] }} />, {
      "mcp:plan": makeEnvelope(
        { domain: "mcp", actions: [], summary: { read: 0, "write-local": 0, "exec-local": 0, network: 0, "install-remove": 0 } },
        { command: "mcp.plan" },
      ),
    });
    expect(await screen.findByText(/没有需要执行的操作/)).toBeInTheDocument();
  });

  it("没有安全动作时不发起无参 plan，仍保留目标与状态展示", async () => {
    const { calls } = renderWithEngine(<DriftExplanationPanel element={drifted} planRequest={null} cellKind="mcp" />, {
      "mcp:plan": plan,
    });

    const panel = await screen.findByTestId("drift-panel");
    expect(panel).toHaveTextContent("/tmp/demo/.claude.json");
    expect(panel).toHaveTextContent("当前没有可安全预览的同步动作。");
    expect(calls.some((call) => call.command === "mcp:plan")).toBe(false);
  });

  it("shortenDigest：长 digest 截断、null 占位", () => {
    expect(shortenDigest(null)).toBe("—");
    expect(shortenDigest("sha256:short")).toBe("sha256:short");
    expect(shortenDigest("sha256:dd9d3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2")).toBe(
      "sha256:dd9d3c8ae…de12a2",
    );
  });
});
