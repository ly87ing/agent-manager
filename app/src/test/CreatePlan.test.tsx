import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { CreatePlan } from "../components/CreatePlan";

// 与真实引擎 profiles:preview --on-save 的返回同形（2026-09-28 隔离 HOME 实测）
const preview = {
  config: {
    path: "~/.claude-glm/settings.json",
    exists: false,
    content: JSON.stringify({ env: { ANTHROPIC_AUTH_TOKEN: "", ANTHROPIC_API_KEY: "", CLAUDE_CODE_OAUTH_TOKEN: "" }, permissions: { allow: ["Bash(ls:*)"] } }),
    seed_results: [
      { item: "permissions", state: "copied", detail: "已带上权限规则" },
      { item: "statusline", state: "copied", detail: "已带上状态栏" },
      { item: "rules", state: "copied", detail: "已复制 CLAUDE.md" },
      { item: "mcp", state: "copied", detail: "已复制 demo-fs" },
    ],
    files: [
      { path: "~/.claude-glm/CLAUDE.md", exists: false, content: "# 规则\n" },
      { path: "~/.claude-glm/.claude.json", exists: false, content: "{\"mcpServers\":{}}" },
    ],
  },
  alias: { files: [{ target: "aliases:zshrc", path: "/Users/demo/.zshrc", exists: true, content: "" }], skipped: [] },
};

describe("创建前的动作清单", () => {
  it("逐条写出带上的设置、置空的变量、复制的规则与 MCP", () => {
    render(
      <CreatePlan
        preview={preview}
        facts={{
          configDir: "~/.claude-glm",
          mainDir: "~/.claude",
          command: 'CLAUDE_CONFIG_DIR="$HOME/.claude-glm" claude',
          commandName: "glm",
          api: null,
          skills: 2,
          extraMcp: [],
          loginCommand: 'CLAUDE_CONFIG_DIR="$HOME/.claude-glm" claude auth login',
        }}
      />,
    );
    const plan = screen.getByTestId("account-create-plan");
    expect(plan).toHaveTextContent("从主账号带上权限规则、状态栏");
    expect(plan).not.toHaveTextContent("Hooks");
    expect(plan).toHaveTextContent("置空 3 个凭据相关变量");
    expect(plan).toHaveTextContent("复制主账号的规则到 ~/.claude-glm/CLAUDE.md");
    expect(plan).toHaveTextContent("写入 MCP 服务 ~/.claude-glm/.claude.json已复制 demo-fs");
    expect(plan).toHaveTextContent("在 ~/.zshrc 里由本工具管理的那几行加一条启动命令");
  });

  it("API 账号写明接口、模型与 API Key 存到哪", () => {
    render(
      <CreatePlan
        preview={preview}
        facts={{
          configDir: "~/.claude-ds",
          mainDir: "~/.claude",
          command: 'CLAUDE_CONFIG_DIR="$HOME/.claude-ds" claude',
          commandName: "ds",
          api: { baseUrl: "https://api.deepseek.com/anthropic", model: "deepseek-v4", tokenVar: "DEEPSEEK_API_KEY", keyFile: "~/.config/agent-manager/secrets.env" },
          skills: 0,
          extraMcp: ["grok-search"],
          loginCommand: null,
        }}
      />,
    );
    const plan = screen.getByTestId("account-create-plan");
    expect(plan).toHaveTextContent("接口 https://api.deepseek.com/anthropic、模型 deepseek-v4、API Key（取自 DEEPSEEK_API_KEY）");
    expect(plan).toHaveTextContent("把 API Key 存进 ~/.config/agent-manager/secrets.env 的 DEEPSEEK_API_KEY");
    expect(plan).toHaveTextContent("另从其他账号复制在用的 MCP：grok-search");
    expect(plan).not.toHaveTextContent("链接");
    expect(plan).not.toHaveTextContent("登录一次");
  });
});
