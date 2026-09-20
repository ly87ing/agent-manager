import { describe, expect, it } from "vitest";
import {
  buildAgentDefinition,
  detectSystemPlatform,
  extractCommandForPlatform,
  extractUnknownFields,
  getProviderNames,
  parseInitialToFormValues,
  removeAgentConnection,
  upsertAgentConnection,
  updateCommandWithPlatform,
} from "../lib/agentDefinitionForm";

describe("agentDefinitionForm helpers", () => {
  it("extractUnknownFields captures unknown non-standard keys", () => {
    const unknown = extractUnknownFields({
      display_name: "Test",
      bin: "bin",
      custom_meta: "123",
      custom_flag: true,
    });
    expect(unknown).toEqual({ custom_meta: "123", custom_flag: true });
  });

  it("detectSystemPlatform returns darwin, linux or windows", () => {
    const platform = detectSystemPlatform();
    expect(["darwin", "linux", "windows"]).toContain(platform);
  });

  it("getProviderNames returns provider keys or default custom", () => {
    expect(getProviderNames(null)).toEqual(["custom"]);
    expect(getProviderNames({})).toEqual(["custom"]);
    expect(
      getProviderNames({
        providers: {
          native: { managed: true },
          npm: { managed: true },
        },
      })
    ).toEqual(["native", "npm"]);
  });

  it("新增接入只替换目标 profile，并保留未知设置、其他接入和当前生效项", () => {
    const initial = {
      display_name: "Claude Code",
      bin: "claude",
      custom_meta: { owner: "local" },
      settings: {
        active: "official",
        yolo: true,
        unknown_runtime_flag: "keep-me",
        profiles: {
          official: { label: "官方", custom_profile_flag: 7 },
        },
      },
    };

    const result = upsertAgentConnection(
      initial,
      {
        name: "relay",
        label: "我的中转",
        baseUrl: "https://relay.example.com",
        auth: "token",
        tokenVar: "RELAY_KEY",
        model: "relay-model",
        smallFastModel: "",
        aliasOpus: "",
        aliasSonnet: "",
        aliasHaiku: "",
        timeoutMs: "30000",
      },
    );

    expect(result).toEqual({
      ...initial,
      settings: {
        ...initial.settings,
        profiles: {
          ...initial.settings.profiles,
          relay: {
            label: "我的中转",
            base_url: "https://relay.example.com",
            token_var: "RELAY_KEY",
            model: "relay-model",
            timeout_ms: 30000,
          },
        },
      },
    });
    expect(result).not.toBe(initial);
  });

  it("编辑接入保留目标项的未知字段，同时允许清空已知可选字段", () => {
    const initial = {
      display_name: "Claude Code",
      settings: {
        active: "relay",
        profiles: {
          relay: {
            label: "旧名称",
            base_url: "https://old.example.com",
            token_var: "OLD_KEY",
            model: "old-model",
            timeout_ms: 5000,
            future_option: { mode: "keep" },
          },
        },
      },
    };

    const result = upsertAgentConnection(initial, {
      name: "relay",
      label: "新名称",
      baseUrl: "https://new.example.com",
      auth: "token",
      tokenVar: "NEW_KEY",
      model: "new-model",
      smallFastModel: "",
      aliasOpus: "",
      aliasSonnet: "",
      aliasHaiku: "",
      timeoutMs: "",
    });

    expect((result.settings as any).profiles.relay).toEqual({
      label: "新名称",
      base_url: "https://new.example.com",
      token_var: "NEW_KEY",
      model: "new-model",
      future_option: { mode: "keep" },
    });
    expect((result.settings as any).active).toBe("relay");
  });

  it("删除接入只移除目标项，并清理悬空 active 与空 profiles", () => {
    const initial = {
      display_name: "Claude Code",
      custom_meta: "keep",
      settings: {
        active: "relay",
        yolo: true,
        unknown_runtime_flag: "keep",
        profiles: {
          relay: { label: "中转" },
          official: { label: "官方", custom_profile_flag: 7 },
        },
      },
    };

    const result = removeAgentConnection(initial, "relay");
    expect(result).toEqual({
      ...initial,
      settings: {
        yolo: true,
        unknown_runtime_flag: "keep",
        profiles: { official: { label: "官方", custom_profile_flag: 7 } },
      },
    });
    expect(removeAgentConnection({ settings: { profiles: { relay: {} } } }, "relay")).toEqual({});
    expect(initial.settings.profiles.relay).toEqual({ label: "中转" });
  });

  it("extractCommandForPlatform extracts plain string and platform-specific mapping", () => {
    expect(extractCommandForPlatform("npm install -g foo", "universal")).toBe("npm install -g foo");
    expect(extractCommandForPlatform("npm install -g foo", "darwin")).toBe("npm install -g foo");

    const mapping = {
      darwin: "curl mac.sh | bash",
      linux: "curl linux.sh | bash",
      windows: "cmd /c install.cmd",
    };
    expect(extractCommandForPlatform(mapping, "darwin")).toBe("curl mac.sh | bash");
    expect(extractCommandForPlatform(mapping, "linux")).toBe("curl linux.sh | bash");
    expect(extractCommandForPlatform(mapping, "windows")).toBe("cmd /c install.cmd");
  });

  it("反例实证 1：通用平台编辑不保留平台旧覆盖（实证结果为纯字符串通用命令）", () => {
    const initialMapping = { darwin: "echo old", linux: "echo linux" };
    const result = updateCommandWithPlatform(initialMapping, "echo new", "universal");
    expect(result).toBe("echo new");
  });

  it("反例实证 2：{default: d, posix: p} 清空 windows 保留 mac/linux=p 且阻断 fallback", () => {
    const initialMapping = { default: "d", posix: "p" };
    const result = updateCommandWithPlatform(initialMapping, "", "windows");

    // windows 设为空字符串阻断回落到 default，同时保留 posix: p
    expect(result).toEqual({
      default: "d",
      posix: "p",
      windows: "",
    });

    // 验证平台提取效果：
    // windows 提取为空字符串（不 fallback 回落到 default: d）
    expect(extractCommandForPlatform(result, "windows")).toBe("");
    // darwin 与 linux 仍然获取到 posix: p（不被覆盖）
    expect(extractCommandForPlatform(result, "darwin")).toBe("p");
    expect(extractCommandForPlatform(result, "linux")).toBe("p");
  });

  it("反例实证 2b：{default: d, posix: p} 清空 darwin 阻断 fallback 且保留 linux=p, windows=d", () => {
    const initialMapping = { default: "d", posix: "p" };
    const result = updateCommandWithPlatform(initialMapping, "", "darwin");

    expect(result).toEqual({
      default: "d",
      posix: "p",
      darwin: "",
    });

    // darwin 阻断 fallback
    expect(extractCommandForPlatform(result, "darwin")).toBe("");
    // linux 仍然是 posix: p
    expect(extractCommandForPlatform(result, "linux")).toBe("p");
    // windows 仍然是 default: d
    expect(extractCommandForPlatform(result, "windows")).toBe("d");
  });

  it("反例实证 3：无修改保存深相等（不将 fallback 具体化）", () => {
    const initialDef = {
      display_name: "Claude Code",
      bin: "claude",
      preferred_provider: "native",
      require_managed_provider: true,
      providers: {
        native: {
          managed: true,
          install: {
            posix: "curl https://claude.ai/install.sh | bash",
            windows: "curl https://claude.ai/install.cmd",
          },
          uninstall: {
            posix: "rm -f ~/.local/bin/claude",
            windows: "del claude.exe",
          },
          upgrade: "claude update",
        },
      },
    };

    const formValues = parseInitialToFormValues("claude-code", initialDef);
    // 未作任何修改，dirtyFields 为空；草稿即原 providers
    const dirtyFields = new Set<string>();
    const savedDef = buildAgentDefinition(formValues, initialDef, dirtyFields, initialDef.providers);

    // 必须深相等，不可将 posix/windows 具体化为额外平台字段或改写格式
    expect(savedDef).toEqual(initialDef);
  });

  it("反例实证 4：高级 JSON 拒绝保留键 _delete 与 _replace", () => {
    const initialDef = {
      display_name: "Custom",
      bin: "custom",
      preferred_provider: "custom",
      providers: { custom: { managed: true, install: "install.sh", upgrade: "upgrade.sh" } },
    };

    const formValues = {
      name: "custom",
      displayName: "Custom",
      bin: "custom",
      channel: "custom",
      platformScope: "universal" as const,
      installCmd: "install.sh",
      upgradeCmd: "upgrade.sh",
      uninstallCmd: "",
      preferredProvider: "custom",
      advancedJson: JSON.stringify({
        _delete: true,
        _replace: true,
        legit_field: "allowed",
      }),
      connections: [],
      activeConnection: "",
      yolo: false,
      statusline: "untouched" as const,
    };

    const dirtyFields = new Set<string>(["advancedJson"]);
    const savedDef = buildAgentDefinition(formValues, initialDef, dirtyFields, initialDef.providers);

    // _delete 与 _replace 必须被严格过滤，不能翻转定义层语义
    expect(savedDef._delete).toBeUndefined();
    expect(savedDef._replace).toBeUndefined();
    expect(savedDef.legit_field).toBe("allowed");
  });

  it("反例实证 5：保存不重放当前平台命令，draftProviders 原样采用", () => {
    const initialDef = {
      display_name: "Claude Code",
      bin: "claude",
      preferred_provider: "native",
      providers: {
        native: {
          managed: true,
          install: { posix: "curl old.sh | bash" },
          upgrade: "claude update",
        },
      },
    };

    // 用户编辑了 darwin 安装命令（输入时已写入草稿），随后切到 linux 未编辑直接保存
    const draftProviders = {
      native: {
        managed: true,
        install: { posix: "curl old.sh | bash", darwin: "curl new-mac.sh | bash" },
        upgrade: "claude update",
      },
    };

    const formValues = {
      name: "claude-code",
      displayName: "Claude Code",
      bin: "claude",
      channel: "native",
      // 保存时当前作用域是 linux，提取到的 installCmd 为空 —— 重放会误伤 fallback
      platformScope: "linux" as const,
      installCmd: "",
      upgradeCmd: "claude update",
      uninstallCmd: "",
      preferredProvider: "native",
      advancedJson: "",
      connections: [],
      activeConnection: "",
      yolo: false,
      statusline: "untouched" as const,
    };

    const dirtyFields = new Set<string>(["providers.native.install"]);
    const savedDef = buildAgentDefinition(formValues, initialDef, dirtyFields, draftProviders);

    // 草稿原样进入定义：darwin 编辑保留，posix fallback 不被改写，不新增 linux 键
    expect((savedDef.providers as any).native.install).toEqual({
      posix: "curl old.sh | bash",
      darwin: "curl new-mac.sh | bash",
    });
    expect((savedDef.providers as any).native.upgrade).toBe("claude update");
  });

  it("反例实证 6：草稿中的空字符串平台覆盖在保存时不被重放抹掉", () => {
    const initialDef = {
      bin: "codex",
      preferred_provider: "native",
      providers: {
        native: {
          managed: true,
          install: { default: "d", posix: "p" },
          upgrade: "u",
        },
      },
    };

    // 用户清空了 Windows 安装命令（输入时已写入 windows: "" 阻断 fallback）
    const draftProviders = {
      native: {
        managed: true,
        install: { default: "d", posix: "p", windows: "" },
        upgrade: "u",
      },
    };

    const formValues = {
      name: "codex",
      displayName: "",
      bin: "codex",
      channel: "native",
      platformScope: "darwin" as const,
      installCmd: "p",
      upgradeCmd: "u",
      uninstallCmd: "",
      preferredProvider: "native",
      advancedJson: "",
      connections: [],
      activeConnection: "",
      yolo: false,
      statusline: "untouched" as const,
    };

    const dirtyFields = new Set<string>(["providers.native.install"]);
    const savedDef = buildAgentDefinition(formValues, initialDef, dirtyFields, draftProviders);

    expect((savedDef.providers as any).native.install).toEqual({
      default: "d",
      posix: "p",
      windows: "",
    });
    // mac/linux 仍取 posix: p
    expect(extractCommandForPlatform((savedDef.providers as any).native.install, "darwin")).toBe("p");
    expect(extractCommandForPlatform((savedDef.providers as any).native.install, "linux")).toBe("p");
    expect(extractCommandForPlatform((savedDef.providers as any).native.install, "windows")).toBe("");
  });

  it("反例实证 7：编辑已有未声明 managed 的 provider，仅改显示名称时 providers 深相等", () => {
    const initialDef = {
      display_name: "Old Name",
      bin: "custom",
      preferred_provider: "custom",
      providers: {
        custom: { install: "install.sh", upgrade: "upgrade.sh" },
      },
    };

    const formValues = {
      name: "custom-agent",
      displayName: "New Name",
      bin: "custom",
      channel: "custom",
      platformScope: "universal" as const,
      installCmd: "install.sh",
      upgradeCmd: "upgrade.sh",
      uninstallCmd: "",
      preferredProvider: "custom",
      advancedJson: "",
      connections: [],
      activeConnection: "",
      yolo: false,
      statusline: "untouched" as const,
    };

    const dirtyFields = new Set<string>(["displayName"]);
    const savedDef = buildAgentDefinition(formValues, initialDef, dirtyFields, initialDef.providers);

    // providers 原样保留：未被意外补 managed:true（custom 探测要求显式声明）
    expect(savedDef.providers).toEqual(initialDef.providers);
    expect((savedDef.providers as any).custom.managed).toBeUndefined();
    expect(savedDef.display_name).toBe("New Name");
  });

  it("反例实证 8：编辑其他设置 JSON 时删除旧字段真正生效", () => {
    const initialDef = {
      display_name: "Custom",
      bin: "custom",
      preferred_provider: "custom",
      providers: { custom: { managed: true, install: "i.sh", upgrade: "u.sh" } },
      legacy_field: "old",
      keep_field: "keep",
    };

    const formValues = {
      name: "custom",
      displayName: "Custom",
      bin: "custom",
      channel: "custom",
      platformScope: "universal" as const,
      installCmd: "i.sh",
      upgradeCmd: "u.sh",
      uninstallCmd: "",
      preferredProvider: "custom",
      // 用户在 JSON 中删掉 legacy_field，只保留 keep_field
      advancedJson: JSON.stringify({ keep_field: "keep" }),
      connections: [],
      activeConnection: "",
      yolo: false,
      statusline: "untouched" as const,
    };

    const dirtyFields = new Set<string>(["advancedJson"]);
    const savedDef = buildAgentDefinition(formValues, initialDef, dirtyFields, initialDef.providers);

    expect(savedDef.legacy_field).toBeUndefined();
    expect(savedDef.keep_field).toBe("keep");
    // 标准字段不受替换影响
    expect(savedDef.bin).toBe("custom");
    expect(savedDef.providers).toEqual(initialDef.providers);
    expect(savedDef.preferred_provider).toBe("custom");
  });

  it("反例实证 9：清空其他设置 JSON（{} 或空文本）删除全部原未知字段", () => {
    const initialDef = {
      display_name: "Custom",
      bin: "custom",
      preferred_provider: "custom",
      providers: { custom: { managed: true, install: "i.sh", upgrade: "u.sh" } },
      legacy_field: "old",
      another_field: 123,
    };

    const baseValues = {
      name: "custom",
      displayName: "Custom",
      bin: "custom",
      channel: "custom",
      platformScope: "universal" as const,
      installCmd: "i.sh",
      upgradeCmd: "u.sh",
      uninstallCmd: "",
      preferredProvider: "custom",
      connections: [],
      activeConnection: "",
      yolo: false,
      statusline: "untouched" as const,
    };
    const dirtyFields = new Set<string>(["advancedJson"]);

    for (const cleared of ["{}", ""]) {
      const savedDef = buildAgentDefinition(
        { ...baseValues, advancedJson: cleared },
        initialDef,
        dirtyFields,
        initialDef.providers
      );
      expect(savedDef.legacy_field).toBeUndefined();
      expect(savedDef.another_field).toBeUndefined();
      expect(savedDef.providers).toEqual(initialDef.providers);
      expect(savedDef.bin).toBe("custom");
    }
  });

  it("其他设置 JSON 未编辑时原未知字段保持不变", () => {
    const initialDef = {
      display_name: "Custom",
      bin: "custom",
      preferred_provider: "custom",
      providers: { custom: { managed: true, install: "i.sh", upgrade: "u.sh" } },
      legacy_field: "old",
    };

    const formValues = {
      name: "custom",
      displayName: "Renamed",
      bin: "custom",
      channel: "custom",
      platformScope: "universal" as const,
      installCmd: "i.sh",
      upgradeCmd: "u.sh",
      uninstallCmd: "",
      preferredProvider: "custom",
      advancedJson: JSON.stringify({ legacy_field: "old" }),
      connections: [],
      activeConnection: "",
      yolo: false,
      statusline: "untouched" as const,
    };

    // 只改了显示名称，advancedJson 非 dirty
    const dirtyFields = new Set<string>(["displayName"]);
    const savedDef = buildAgentDefinition(formValues, initialDef, dirtyFields, initialDef.providers);

    expect(savedDef.legacy_field).toBe("old");
    expect(savedDef.display_name).toBe("Renamed");
  });
});
