import { describe, expect, it } from "vitest";
import {
  instancesUsingConnection,
  PROFILE_ID_RE,
  authTerminalCommand,
  buildProfileDefinition,
  connectionToProfileInput,
  defaultConfigDir,
  fixLaunchCommandTilde,
  launchCommandNeedsTildeFix,
  launchCommandsForProfile,
  normalizeConfigDir,
  parseProfileDefinition,
  profileConnectionUpdate,
  profileConnectionReadbackMatches,
  profileLaunchValue,
  profileMcpConfig,
  shellConfigDir,
  updateProfileDefinition,
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
    expect(def.profile).not.toHaveProperty("wire_api");
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
    ).toBe('CODEX_HOME="$HOME/.codex-work" codex');
    expect(
      profileLaunchValue(
        { ...claudeInput, kind: "codex", configDir: "~/.codex-work", extraArgs: "", baseUrl: "", tokenVar: "" },
        "codex",
      ),
    ).toBe('CODEX_HOME="$HOME/.codex-work" codex');
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

  it("修改配置目录时更新技能与 MCP 目标，保留其他定义字段", () => {
    const original = {
      ...buildProfileDefinition(claudeInput, "claude"),
      bin: "/isolated/custom-claude",
      mcp_config: { path: "~/.claude-cc-ly/.claude.json", format: "json", future: "keep" },
      future_root: "keep",
      profile: { base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-cc-ly", future_profile: "keep" },
    };
    const updated = updateProfileDefinition({ ...claudeInput, configDir: "~/.claude-new" }, "claude", original);
    expect(updated).toMatchObject({
      bin: original.bin,
      skills_dir: "~/.claude-new/skills",
      mcp_config: { path: "~/.claude-new/.claude.json", format: "json", future: "keep" },
      future_root: "keep",
      profile: { config_dir: "~/.claude-new", future_profile: "keep" },
    });
  });

  it("Codex 编辑清空地址、凭据和模型时移除对应字段", () => {
    const input = { ...claudeInput, baseAgent: "codex", kind: "codex" as const, configDir: "~/.codex-work", baseUrl: "", tokenVar: "", model: "" };
    const original = {
      bin: "/isolated/custom-codex", future_root: "keep",
      profile: {
        base_agent: "codex", kind: "codex", config_dir: "~/.codex-work",
        base_url: "https://api.example.com", env_key: "OLD_KEY", model: "old-model", future_profile: "keep",
      },
    };
    const updated = updateProfileDefinition(input, "codex", original);
    expect(updated.profile).toMatchObject({ future_profile: "keep" });
    expect(updated.profile).not.toHaveProperty("wire_api");
    expect(updated.profile).not.toHaveProperty("base_url");
    expect(updated.profile).not.toHaveProperty("env_key");
    expect(updated.profile).not.toHaveProperty("model");
    expect(updated.bin).toBe(original.bin);
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
      sourceConnection: "GLM_4.5",
      configDir: "~/.claude-glm-4-5-2",
      baseUrl: "https://open.bigmodel.cn/api/anthropic",
      tokenVar: "GLM_API_KEY",
      authHeader: "api_key",
      model: "glm-4.5",
    });
    expect(PROFILE_ID_RE.test(made.input.id)).toBe(true);
    const def = buildProfileDefinition(made.input, "claude") as Record<string, any>;
    expect(def.profile.source_connection).toBe("GLM_4.5");
    expect(parseProfileDefinition(made.input.id, def).sourceConnection).toBe("GLM_4.5");
    // 官方已弃用 ANTHROPIC_SMALL_FAST_MODEL：只填了小任务模型时按官方替代写成 Haiku 别名
    expect(def.profile.env).toEqual({
      ANTHROPIC_BASE_URL: "https://open.bigmodel.cn/api/anthropic",
      ANTHROPIC_API_KEY: "${GLM_API_KEY}",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "glm-4.5-air",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "glm-4.5",
      API_TIMEOUT_MS: "3000000",
    });
  });

  it("Fable 别名建成独立环境时写官方的 ANTHROPIC_DEFAULT_FABLE_MODEL", () => {
    const connection = {
      ...emptyConnection("relay"),
      baseUrl: "https://relay.example.com",
      tokenVar: "RELAY_KEY",
      aliasFable: "relay-fable",
    };
    const made = connectionToProfileInput({ id: "claude-code", kind: "claude" }, connection, [])!;
    const def = buildProfileDefinition(made.input, "claude") as Record<string, any>;
    expect(def.profile.env.ANTHROPIC_DEFAULT_FABLE_MODEL).toBe("relay-fable");
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

  it("只处理本工具自己的变更：已有启动命令不因缺少清空凭据的前缀被标成要订正，带着前缀的也原样不动", () => {
    const parse = (value: string) => Object.fromEntries([...value.matchAll(/(\w+)="([^"]*)"/g)].map((m) => [m[1], m[2]]));
    const aliases = {
      "cx-w": 'CODEX_HOME="$HOME/.codex-w" codex',
      "cx-old": 'CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME="$HOME/.codex-w" codex',
    };
    const commands = launchCommandsForProfile(aliases, "codex", "~/.codex-w", parse);
    expect(commands.map((c) => [c.name, c.value, c.needsTildeFix])).toEqual([
      ["cx-old", aliases["cx-old"], false],
      ["cx-w", aliases["cx-w"], false],
    ]);
    expect(Object.keys(commands[0]).sort()).toEqual(["name", "needsTildeFix", "value"]);
  });
});

describe("用着某套接入的独立环境", () => {
  it("同一助手、同一地址（忽略结尾 /）、同一凭据变量才算；删除中的与别的助手的不算", () => {
    const definitions = {
      "claude-code": { bin: "claude", settings: {} },
      "cc-glm": {
        display_name: "Claude · GLM",
        profile: { base_agent: "claude-code", env: { ANTHROPIC_BASE_URL: "https://open.bigmodel.cn/api/anthropic/", ANTHROPIC_AUTH_TOKEN: "${GLM_API_KEY}" } },
      },
      "cc-glm-2": {
        profile: { base_agent: "claude-code", env: { ANTHROPIC_BASE_URL: "https://open.bigmodel.cn/api/anthropic", ANTHROPIC_AUTH_TOKEN: "${OTHER_KEY}" } },
      },
      "cc-old": {
        _delete: true,
        profile: { base_agent: "claude-code", env: { ANTHROPIC_BASE_URL: "https://open.bigmodel.cn/api/anthropic", ANTHROPIC_AUTH_TOKEN: "${GLM_API_KEY}" } },
      },
      "cx-glm": { profile: { base_agent: "codex", base_url: "https://open.bigmodel.cn/api/anthropic", env_key: "GLM_API_KEY" } },
    };
    expect(
      instancesUsingConnection(definitions, "claude-code", { name: "glm", baseUrl: "https://open.bigmodel.cn/api/anthropic", tokenVar: "GLM_API_KEY" }),
    ).toEqual([{ id: "cc-glm", label: "Claude · GLM" }]);
    expect(instancesUsingConnection(definitions, "claude-code", { name: "glm", baseUrl: "", tokenVar: "GLM_API_KEY" })).toEqual([]);
  });

  it("新环境按 Provider 名称关联，地址和凭据变量变更后仍可找到；不串到同参数的另一个 Provider", () => {
    const definitions = {
      "cc-glm": {
        display_name: "GLM 环境",
        profile: {
          base_agent: "claude-code",
          source_connection: "glm",
          env: { ANTHROPIC_BASE_URL: "https://old.example.com", ANTHROPIC_AUTH_TOKEN: "${OLD_KEY}" },
        },
      },
    };
    expect(instancesUsingConnection(definitions, "claude-code", {
      name: "glm", baseUrl: "https://new.example.com", tokenVar: "NEW_KEY",
    })).toEqual([{ id: "cc-glm", label: "GLM 环境" }]);
    expect(instancesUsingConnection(definitions, "claude-code", {
      name: "other", baseUrl: "https://old.example.com", tokenVar: "OLD_KEY",
    })).toEqual([]);
  });
});

describe("Provider 编辑影响预览", () => {
  it("只改仍跟随旧 Provider 的字段，保留环境自己改过的模型和 env", () => {
    const old = {
      ...emptyConnection("relay"), baseUrl: "https://old.example.com", tokenVar: "OLD_KEY",
      model: "old-model", aliasOpus: "old-opus", aliasHaiku: "old-haiku",
    };
    const newer = {
      ...old, baseUrl: "https://new.example.com", tokenVar: "NEW_KEY",
      model: "new-model", aliasOpus: "new-opus", aliasHaiku: "",
    };
    const definition = buildProfileDefinition({
      ...claudeInput,
      baseUrl: old.baseUrl,
      tokenVar: old.tokenVar,
      model: "my-model",
      extraEnv: { ANTHROPIC_DEFAULT_OPUS_MODEL: "old-opus", ANTHROPIC_DEFAULT_HAIKU_MODEL: "my-haiku", MINE: "keep" },
    }, "claude");
    const update = profileConnectionUpdate("cc-ly", definition, old, newer, "claude")!;
    expect(update.changes.map((change) => change.field)).toEqual(["API 地址", "凭据变量", "ANTHROPIC_DEFAULT_OPUS_MODEL", "来源关联"]);
    expect(update.candidate).toMatchObject({
      source_connection: "relay",
      model: "my-model",
      env: {
        ANTHROPIC_BASE_URL: "https://new.example.com",
        ANTHROPIC_AUTH_TOKEN: "${NEW_KEY}",
        ANTHROPIC_DEFAULT_OPUS_MODEL: "new-opus",
        ANTHROPIC_DEFAULT_HAIKU_MODEL: "my-haiku",
        MINE: "keep",
      },
    });
  });

  it("Codex 更新地址与变量引用，候选不包含 Claude env 或凭据取值", () => {
    const old = { ...emptyConnection("relay"), baseUrl: "https://old.example.com", tokenVar: "OLD_KEY", model: "old-model" };
    const newer = { ...old, baseUrl: "https://new.example.com", tokenVar: "NEW_KEY", model: "new-model" };
    const definition = buildProfileDefinition({
      ...claudeInput, id: "cx-work", baseAgent: "codex", kind: "codex", configDir: "~/.codex-work",
      baseUrl: old.baseUrl, tokenVar: old.tokenVar, model: old.model,
    }, "codex");
    const update = profileConnectionUpdate("cx-work", definition, old, newer, "codex")!;
    expect(update.candidate).toEqual({
      source_connection: "relay", base_url: "https://new.example.com", env_key: "NEW_KEY", model: "new-model",
    });
    expect(JSON.stringify(update.candidate)).not.toContain("OLD_KEY_VALUE");
    expect(profileConnectionReadbackMatches("codex", update.candidate, [
      { field: "model", state: "same", definition: "new-model" },
      { field: "provider.base_url", state: "same", definition: "https://new.example.com" },
      { field: "provider.experimental_bearer_token", state: "same", definition: "${NEW_KEY}" },
    ])).toBe(true);
    expect(profileConnectionReadbackMatches("codex", update.candidate, [
      { field: "provider.base_url", state: "same", definition: "https://old.example.com" },
    ])).toBe(false);
  });

  it("认证请求头切换会移除旧凭据键；回读必须逐字段确认新值", () => {
    const old = {
      ...emptyConnection("relay"), baseUrl: "https://old.example.com", tokenVar: "RELAY_KEY",
      model: "old-model", aliasHaiku: "old-haiku",
    };
    const newer = { ...old, auth: "api_key" as const, aliasHaiku: "", model: "new-model" };
    const definition = buildProfileDefinition({
      ...claudeInput, sourceConnection: "relay", baseUrl: old.baseUrl, tokenVar: old.tokenVar,
      model: old.model, extraEnv: { ANTHROPIC_DEFAULT_HAIKU_MODEL: "old-haiku", MINE: "keep" },
    }, "claude");
    const update = profileConnectionUpdate("cc-ly", definition, old, newer, "claude")!;
    expect(update.candidate.env).toEqual({
      ANTHROPIC_BASE_URL: "https://old.example.com", ANTHROPIC_API_KEY: "${RELAY_KEY}", MINE: "keep",
    });
    expect(update.changes.map((item) => item.field)).toEqual(["默认模型", "请求头", "ANTHROPIC_DEFAULT_HAIKU_MODEL"]);
    expect(profileConnectionReadbackMatches("claude", update.candidate, [
      { field: "model", state: "same", definition: "new-model" },
      { field: "env.ANTHROPIC_BASE_URL", state: "same", definition: "https://old.example.com" },
      { field: "env.ANTHROPIC_API_KEY", state: "same", definition: "${RELAY_KEY}" },
      { field: "env.MINE", state: "same", definition: "keep" },
    ])).toBe(true);
    expect(profileConnectionReadbackMatches("claude", update.candidate, [
      { field: "model", state: "same", definition: "old-model" },
    ])).toBe(false);
  });

  it("上次未勾选的环境可再次逐字段补同步，自定义字段默认保留", () => {
    const provider = { ...emptyConnection("relay"), baseUrl: "https://new.example.com", tokenVar: "NEW_KEY", model: "new-model" };
    const definition = buildProfileDefinition({
      ...claudeInput, sourceConnection: "relay", baseUrl: "https://old.example.com",
      tokenVar: "OLD_KEY", model: "my-model", extraEnv: { MINE: "keep" },
    }, "claude");
    const preview = profileConnectionUpdate("cc-ly", definition, provider, provider, "claude")!;
    expect(preview.changes).toEqual([]);
    expect(preview.overrides.map((item) => item.key)).toEqual(["baseUrl", "model", "credential"]);
    const selected = profileConnectionUpdate("cc-ly", definition, provider, provider, "claude", ["baseUrl", "credential"])!;
    expect(selected.changes.map((item) => item.field)).toEqual(["API 地址", "凭据变量 / 请求头"]);
    expect(selected.candidate).toMatchObject({
      source_connection: "relay", model: "my-model",
      env: { ANTHROPIC_BASE_URL: "https://new.example.com", ANTHROPIC_AUTH_TOKEN: "${NEW_KEY}", MINE: "keep" },
    });
  });
});

describe("子代理模型随接入进独立环境", () => {
  it("接入的子代理模型写成实例 env 的 CLAUDE_CODE_SUBAGENT_MODEL", () => {
    const made = connectionToProfileInput(
      { id: "claude-code", kind: "claude" },
      { ...emptyConnection("glm"), baseUrl: "https://open.bigmodel.cn/api/anthropic", tokenVar: "GLM_API_KEY", model: "glm-5.3", subagentModel: "glm-5.3-air" },
      [],
    );
    expect(made?.input.extraEnv.CLAUDE_CODE_SUBAGENT_MODEL).toBe("glm-5.3-air");
  });
});
