import { describe, expect, it } from "vitest";
import {
  PROFILE_ID_RE,
  buildProfileDefinition,
  defaultConfigDir,
  parseProfileDefinition,
  profileLaunchValue,
  profileMcpConfig,
} from "../lib/profileDefinition";

const claudeInput = {
  id: "cc-ly",
  displayName: "Claude Code · ly",
  baseAgent: "claude-code",
  kind: "claude" as const,
  configDir: "~/.claude-cc-ly",
  baseUrl: "https://api.example.com",
  tokenVar: "MY_TOKEN",
  model: "",
  extraArgs: "--dangerously-skip-permissions",
};

describe("助手实例定义", () => {
  it("Claude 实例：鉴权只存变量引用，技能与 MCP 目标跟着配置目录走", () => {
    const def = buildProfileDefinition(claudeInput, "claude") as Record<string, any>;
    expect(def.bin).toBe("claude");
    expect(def.skills_dir).toBe("~/.claude-cc-ly/skills");
    expect(def.mcp_config).toEqual({ path: "~/.claude-cc-ly/.claude.json", format: "json" });
    expect(def.profile.env.ANTHROPIC_AUTH_TOKEN).toBe("${MY_TOKEN}");
    expect(def.profile.env.ANTHROPIC_BASE_URL).toBe("https://api.example.com");
    // 取值本身绝不进定义
    expect(JSON.stringify(def)).not.toContain("sk-");
  });

  it("Codex 实例：只写变量名，MCP 目标是 config.toml 而本工具写叠加文件", () => {
    const def = buildProfileDefinition(
      { ...claudeInput, id: "cx-work", baseAgent: "codex", kind: "codex", configDir: "~/.codex-work" },
      "codex",
    ) as Record<string, any>;
    expect(def.profile.env_key).toBe("MY_TOKEN");
    expect(def.profile.env).toBeUndefined();
    expect(def.profile.wire_api).toBe("responses");
    expect(def.mcp_config).toEqual({ path: "~/.codex-work/config.toml", format: "toml" });
  });

  it("启动命令取值：Claude 指配置目录，Codex 额外叠加自己的配置", () => {
    expect(profileLaunchValue(claudeInput, "claude")).toBe(
      'CLAUDE_CONFIG_DIR="~/.claude-cc-ly" claude --dangerously-skip-permissions',
    );
    expect(
      profileLaunchValue({ ...claudeInput, kind: "codex", configDir: "~/.codex-work", extraArgs: "" }, "codex"),
    ).toBe('CODEX_HOME="~/.codex-work" codex -p agent-manager');
  });

  it("回填既有定义", () => {
    const def = buildProfileDefinition(claudeInput, "claude") as Record<string, unknown>;
    const back = parseProfileDefinition("cc-ly", def);
    expect(back.tokenVar).toBe("MY_TOKEN");
    expect(back.baseUrl).toBe("https://api.example.com");
    expect(back.configDir).toBe("~/.claude-cc-ly");
    expect(back.kind).toBe("claude");
  });

  it("默认配置目录与名称规则", () => {
    expect(defaultConfigDir("claude", "ly")).toBe("~/.claude-ly");
    expect(defaultConfigDir("codex", "work")).toBe("~/.codex-work");
    expect(PROFILE_ID_RE.test("cc-ly")).toBe(true);
    expect(PROFILE_ID_RE.test("Bad")).toBe(false);
    expect(PROFILE_ID_RE.test("-x")).toBe(false);
  });

  it("MCP 目标按实例类型区分", () => {
    expect(profileMcpConfig("claude", "~/.claude-a/").path).toBe("~/.claude-a/.claude.json");
    expect(profileMcpConfig("codex", "~/.codex-a").path).toBe("~/.codex-a/config.toml");
  });
});
