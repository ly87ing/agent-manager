import { describe, expect, it } from "vitest";
import {
  PROFILE_ID_RE,
  authTerminalCommand,
  buildProfileDefinition,
  connectionToProfileInput,
  defaultConfigDir,
  fixLaunchCommandCredentialClear,
  fixLaunchCommandTilde,
  launchCommandNeedsCredentialClear,
  launchCommandNeedsTildeFix,
  launchCommandsForProfile,
  normalizeConfigDir,
  parseProfileDefinition,
  profileLaunchValue,
  profileMcpConfig,
  shellConfigDir,
} from "../lib/profileDefinition";
import { emptyConnection } from "../lib/agentDefinitionForm";

const claudeInput = {
  id: "cc-ly",
  displayName: "Claude Code · ly",
  baseAgent: "claude-code",
  kind: "claude" as const,
  configDir: "~/.claude-cc-ly",
  baseUrl: "https://api.example.com",
  tokenVar: "MY_TOKEN",
  authHeader: "token" as const,
  extraEnv: {} as Record<string, string>,
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

  it("启动命令取值：只用配置目录变量指向实例，Codex 不再 -p 叠加", () => {
    // 配置目录写 $HOME 而非引号里的 ~：后者 shell 不展开，取到的是字面量
    expect(profileLaunchValue(claudeInput, "claude")).toBe(
      'CLAUDE_CONFIG_DIR="$HOME/.claude-cc-ly" claude --dangerously-skip-permissions',
    );
    // Codex 环境都清空 Codex 当作官方凭据读取的变量：第三方环境防混用，官方账号环境防别的账号的令牌压过目录登录
    expect(
      profileLaunchValue({ ...claudeInput, kind: "codex", configDir: "~/.codex-work", extraArgs: "" }, "codex"),
    ).toBe('CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME="$HOME/.codex-work" codex');
    expect(
      profileLaunchValue(
        { ...claudeInput, kind: "codex", configDir: "~/.codex-work", extraArgs: "", baseUrl: "", tokenVar: "" },
        "codex",
      ),
    ).toBe('CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME="$HOME/.codex-work" codex');
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
    const good = `CODEX_HOME="$HOME/.codex-x" codex`;
    expect(launchCommandNeedsTildeFix(good)).toBe(false);
    expect(fixLaunchCommandTilde(good)).toBe(good);
  });

  it("归一化后能把 ~ 与 $HOME 写法认作同一个目录", () => {
    expect(normalizeConfigDir("$HOME/.claude-work")).toBe("~/.claude-work");
    expect(normalizeConfigDir("~/.claude-work/")).toBe("~/.claude-work");
    expect(normalizeConfigDir("$HOME")).toBe("~");
  });

  it("Claude 凭据请求头二选一，编辑往返不丢；表单不认识的 env 原样保留", () => {
    const def = buildProfileDefinition({ ...claudeInput, authHeader: "api_key" }, "claude") as Record<string, any>;
    expect(def.profile.env.ANTHROPIC_API_KEY).toBe("${MY_TOKEN}");
    expect(def.profile.env.ANTHROPIC_AUTH_TOKEN).toBeUndefined();
    const parsed = parseProfileDefinition("cc-ly", {
      profile: {
        base_agent: "claude-code",
        config_dir: "~/.claude-cc-ly",
        env: {
          ANTHROPIC_BASE_URL: "https://api.example.com",
          ANTHROPIC_API_KEY: "${MY_TOKEN}",
          ANTHROPIC_SMALL_FAST_MODEL: "small",
          API_TIMEOUT_MS: "600000",
        },
      },
    });
    expect(parsed.authHeader).toBe("api_key");
    expect(parsed.tokenVar).toBe("MY_TOKEN");
    expect(parsed.extraEnv).toEqual({ ANTHROPIC_SMALL_FAST_MODEL: "small", API_TIMEOUT_MS: "600000" });
    const rebuilt = buildProfileDefinition(parsed, "claude") as Record<string, any>;
    expect(rebuilt.profile.env).toEqual({
      ANTHROPIC_BASE_URL: "https://api.example.com",
      ANTHROPIC_API_KEY: "${MY_TOKEN}",
      ANTHROPIC_SMALL_FAST_MODEL: "small",
      API_TIMEOUT_MS: "600000",
    });
  });

  it("接入建成独立环境：带上地址、凭据变量名、请求头、模型与别名，名字可用且不重名", () => {
    const connection = {
      ...emptyConnection("GLM_4.5"),
      label: "GLM 4.5",
      baseUrl: "https://open.bigmodel.cn/api/anthropic",
      auth: "api_key" as const,
      tokenVar: "GLM_API_KEY",
      model: "glm-4.5",
      smallFastModel: "glm-4.5-air",
      aliasOpus: "glm-4.5",
      timeoutMs: "3000000",
    };
    const made = connectionToProfileInput({ id: "claude-code", kind: "claude" }, connection, ["glm-4-5"])!;
    expect(made.dropped).toEqual([]);
    expect(made.input).toMatchObject({
      id: "glm-4-5-2",
      displayName: "GLM 4.5",
      baseAgent: "claude-code",
      configDir: "~/.claude-glm-4-5-2",
      baseUrl: "https://open.bigmodel.cn/api/anthropic",
      tokenVar: "GLM_API_KEY",
      authHeader: "api_key",
      model: "glm-4.5",
    });
    expect(PROFILE_ID_RE.test(made.input.id)).toBe(true);
    const def = buildProfileDefinition(made.input, "claude") as Record<string, any>;
    // 官方已弃用 ANTHROPIC_SMALL_FAST_MODEL：只填了小任务模型时按官方替代写成 Haiku 别名
    expect(def.profile.env).toEqual({
      ANTHROPIC_BASE_URL: "https://open.bigmodel.cn/api/anthropic",
      ANTHROPIC_API_KEY: "${GLM_API_KEY}",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "glm-4.5-air",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "glm-4.5",
      API_TIMEOUT_MS: "3000000",
    });
  });

  it("小任务模型与 Haiku 别名都填了：以 Haiku 别名为准，并说明小任务模型没带过去", () => {
    const connection = {
      ...emptyConnection("relay"),
      baseUrl: "https://relay.example.com",
      tokenVar: "RELAY_KEY",
      smallFastModel: "small-a",
      aliasHaiku: "haiku-b",
    };
    const made = connectionToProfileInput({ id: "claude-code", kind: "claude" }, connection, [])!;
    const def = buildProfileDefinition(made.input, "claude") as Record<string, any>;
    expect(def.profile.env.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("haiku-b");
    expect(def.profile.env).not.toHaveProperty("ANTHROPIC_SMALL_FAST_MODEL");
    expect(made.dropped).toEqual(["小任务模型（官方已弃用，后台任务改用 Haiku 别名的模型）"]);
  });

  it("Codex 接入建成独立环境：超时不带过去并如实说明；引用主配置接入方式的不能搬", () => {
    const connection = {
      ...emptyConnection("kimi"),
      baseUrl: "https://api.kimi.com/coding/v1",
      tokenVar: "KIMI_API_KEY",
      timeoutMs: "90000",
    };
    const made = connectionToProfileInput({ id: "codex", kind: "codex" }, connection, [])!;
    expect(made.dropped).toEqual(["请求超时"]);
    expect(made.input.configDir).toBe("~/.codex-kimi");
    const def = buildProfileDefinition(made.input, "codex") as Record<string, any>;
    expect(def.profile).toMatchObject({ base_url: "https://api.kimi.com/coding/v1", env_key: "KIMI_API_KEY" });
    expect(
      connectionToProfileInput({ id: "codex", kind: "codex" }, { ...connection, providerRef: "ollama" }, []),
    ).toBeNull();
  });

  it("Codex 环境的启动命令要清空会压过目录登录的凭据变量：识别缺失并只补前缀，引号里的空白不动", () => {
    const old = 'CODEX_HOME="$HOME/.codex-w" codex --msg "a  b"';
    expect(launchCommandNeedsCredentialClear(old)).toBe(true);
    const fixed = fixLaunchCommandCredentialClear(old);
    expect(fixed).toBe('CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME="$HOME/.codex-w" codex --msg "a  b"');
    expect(launchCommandNeedsCredentialClear(fixed)).toBe(false);
    // 带着取值的赋值不算清空：订正时换成空值
    const valued = 'CODEX_API_KEY=abc CODEX_HOME="$HOME/.codex-w" codex';
    expect(launchCommandNeedsCredentialClear(valued)).toBe(true);
    expect(fixLaunchCommandCredentialClear(valued)).toBe('CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME="$HOME/.codex-w" codex');

    const aliases = { "cx-w": old, "cc-w": 'CLAUDE_CONFIG_DIR="$HOME/.claude-w" claude' };
    const parse = (value: string) => Object.fromEntries([...value.matchAll(/(\w+)="([^"]*)"/g)].map((m) => [m[1], m[2]]));
    // 官方账号环境与第三方环境一样要清空：shell 里给别的账号导出的令牌同样会压过目录里的登录
    expect(launchCommandsForProfile(aliases, "codex", "~/.codex-w", parse)[0].needsCredentialClear).toBe(true);
    expect(launchCommandsForProfile({ "cx-w": fixed }, "codex", "~/.codex-w", parse)[0].needsCredentialClear).toBe(false);
    expect(launchCommandsForProfile(aliases, "claude", "~/.claude-w", parse)[0].needsCredentialClear).toBe(false);
  });
});
