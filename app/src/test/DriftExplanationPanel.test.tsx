import { describe, expect, it } from "vitest";
import { DriftExplanationPanel, shortenDigest } from "../components/DriftExplanationPanel";
import { makeElement } from "./fixtures";
import { renderWithEngine, screen } from "./render-with-engine";

// 原先以 MCP 单元格举例；MCP 页已不再用这块面板（条目直接逐条展示），改用技能单元格词汇表。
const drifted = makeElement({
  id: "skill:demo/one",
  domain: "skills",
  observed: {
    agents: {
      "claude-code": { state: "drifted", target: "/tmp/demo/.claude/skills/one", target_exists: true },
      codex: { state: "linked", target: "/tmp/demo/.codex/skills/one", target_exists: true },
    },
  },
  state: "drifted",
});

describe("DriftExplanationPanel", () => {
  it("渲染目标位置与各助手状态表（纯 status/observed，不发起任何 plan）", async () => {
    const { calls } = renderWithEngine(<DriftExplanationPanel element={drifted} cellKind="skills" />, {});

    const panel = await screen.findByTestId("drift-panel");
    // 目标位置（cell targets）
    expect(panel).toHaveTextContent("/tmp/demo/.claude/skills/one");
    expect(panel).toHaveTextContent("/tmp/demo/.codex/skills/one");
    // per-agent 单元格状态
    expect(panel).toHaveTextContent("claude-code");
    expect(screen.getByTitle("drifted")).toHaveTextContent("不一致");
    expect(screen.getByTitle("linked")).toHaveTextContent("已启用");
    // 不请求任何 plan，也不渲染操作计划区
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(panel).not.toHaveTextContent("将要执行的操作");
  });

  it("无 cell 词汇表时纯文本渲染 state", async () => {
    renderWithEngine(<DriftExplanationPanel element={drifted} />, {});

    const panel = await screen.findByTestId("drift-panel");
    expect(panel).toHaveTextContent("drifted");
    expect(panel).toHaveTextContent("linked");
  });

  it("无 per-agent cell 时仍从 observed.target / spec.target 收集目标位置", async () => {
    const noCell = makeElement({
      id: "rules:claude_global",
      domain: "rules",
      spec: { target: "/tmp/home/.claude/CLAUDE.md" },
      observed: { target: "/tmp/home/.claude/CLAUDE.md", target_exists: true },
      state: "drifted",
    });
    const { calls } = renderWithEngine(<DriftExplanationPanel element={noCell} />, {});

    const panel = await screen.findByTestId("drift-panel");
    expect(panel).toHaveTextContent("/tmp/home/.claude/CLAUDE.md");
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("shortenDigest：长 digest 截断、null 占位", () => {
    expect(shortenDigest(null)).toBe("—");
    expect(shortenDigest("sha256:short")).toBe("sha256:short");
    expect(shortenDigest("sha256:dd9d3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2")).toBe(
      "sha256:dd9d3c8ae…de12a2",
    );
  });
});
