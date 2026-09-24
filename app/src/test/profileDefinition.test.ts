import { describe, expect, it } from "vitest";
import {
  PROFILE_ID_RE,
  authTerminalCommand,
  buildProfileDefinition,
  defaultConfigDir,
  fixLaunchCommandTilde,
  launchCommandNeedsTildeFix,
  normalizeConfigDir,
  parseProfileDefinition,
  profileLaunchValue,
  profileMcpConfig,
  shellConfigDir,
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
    // 配置目录写 $HOME 而非引号里的 ~：后者 shell 不展开，取到的是字面量
    expect(profileLaunchValue(claudeInput, "claude")).toBe(
      'CLAUDE_CONFIG_DIR="$HOME/.claude-cc-ly" claude --dangerously-skip-permissions',
    );
    // 第三方 Codex 环境清空 Codex 当作官方凭据读取的变量；官方账号环境不加
    expect(
      profileLaunchValue({ ...claudeInput, kind: "codex", configDir: "~/.codex-work", extraArgs: "" }, "codex"),
    ).toBe('CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME="$HOME/.codex-work" codex -p agent-manager');
    expect(
      profileLaunchValue(
        { ...claudeInput, kind: "codex", configDir: "~/.codex-work", extraArgs: "", baseUrl: "", tokenVar: "" },
        "codex",
      ),
    ).toBe('CODEX_HOME="$HOME/.codex-work" codex -p agent-manager');
  });

  it("认证命令只生成文本，实例命令严格指向自己的配置目录", () => {
    expect(authTerminalCommand("claude", "claude", "login")).toBe("claude auth login");
    expect(authTerminalCommand("codex", "codex", "logout")).toBe("codex logout");
    expect(authTerminalCommand("claude", "claude", "login", "~/.claude-work")).toBe(
      'CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude auth login',
    );
    expect(authTerminalCommand("codex", "codex", "logout", "~/.codex-work")).toBe(
      'CODEX_HOME="$HOME/.codex-work" codex logout',
    );
    expect(authTerminalCommand("claude", "claude", "login", "/tmp/a'$(touch nope)")).toBe(
      `CLAUDE_CONFIG_DIR='/tmp/a'"'"'$(touch nope)' claude auth login`,
    );
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

describe("启动命令里的配置目录", () => {
  it("写成 $HOME 而不是引号里的 ~（后者 shell 不展开）", () => {
    const value = profileLaunchValue({ ...claudeInput, configDir: "~/.claude-work", extraArgs: "" }, "claude");
    expect(value).toBe('CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude');
    expect(value).not.toContain('"~/');
  });

  it("绝对路径与单独的 ~ 都能正确转换", () => {
    expect(shellConfigDir("~")).toBe("$HOME");
    expect(shellConfigDir("~/.codex-x/")).toBe("$HOME/.codex-x");
    expect(shellConfigDir("/opt/cfg")).toBe("/opt/cfg");
  });

  it("认得出早先生成的坏写法并能订正", () => {
    const broken = `CLAUDE_CONFIG_DIR="~/.claude-work" claude --dangerously-skip-permissions`;
    expect(launchCommandNeedsTildeFix(broken)).toBe(true);
    expect(fixLaunchCommandTilde(broken)).toBe(
      `CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude --dangerously-skip-permissions`,
    );
    const good = `CODEX_HOME="$HOME/.codex-x" codex -p agent-manager`;
    expect(launchCommandNeedsTildeFix(good)).toBe(false);
    expect(fixLaunchCommandTilde(good)).toBe(good);
  });

  it("归一化后能把 ~ 与 $HOME 写法认作同一个目录", () => {
    expect(normalizeConfigDir("$HOME/.claude-work")).toBe("~/.claude-work");
    expect(normalizeConfigDir("~/.claude-work/")).toBe("~/.claude-work");
    expect(normalizeConfigDir("$HOME")).toBe("~");
  });
});
