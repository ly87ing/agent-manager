import { describe, expect, it } from "vitest";
import {
  connectionEntriesOf,
  connectionToDefinitionValue,
  detectSystemPlatform,
  duplicateConnectionDraft,
  removeAgentConnection,
  moveAgentConnection,
  parseConnectionBundle,
  runtimeRequiresYoloConfirmation,
  serializeConnectionBundle,
  upsertAgentConnection,
  validateConnectionEntry,
  urlHasUserinfo,
} from "../lib/agentDefinitionForm";
import { agentManagementFixture } from "./fixtures";

describe("agentDefinitionForm helpers", () => {
  it("接入分享只包含连接定义，按助手类型校验且不接受凭据取值", () => {
    const connection = {
      name: "relay",
      label: "中转",
      note: "工作账号，月底到期",
      baseUrl: "https://relay.example.com",
      auth: "token" as const,
      tokenVar: "RELAY_KEY",
      model: "relay-model",
      smallFastModel: "",
      aliasOpus: "",
      aliasSonnet: "",
      aliasHaiku: "",
      timeoutMs: "30000",
    };
    const text = serializeConnectionBundle("claude", connection);

    expect(text).not.toContain("secretValue");
    expect(text).not.toContain("auth.json");
    expect(parseConnectionBundle(text, "claude")).toEqual(connection);
    expect(() => parseConnectionBundle(text, "codex")).toThrow("不能导入当前助手");
    expect(() => parseConnectionBundle(JSON.stringify({
      format: "agent-manager/provider/v1",
      kind: "claude",
      connection: { ...connection, baseUrl: "ftp://remote.example.com", secretValue: "must-not-pass" },
    }), "claude")).toThrow("接口地址必须是 https:// 或 http://");
    // 实际配置里已有的局域网 http 中转要能存下：任意主机的 http 放行，界面另行提示明文传输
    expect(parseConnectionBundle(JSON.stringify({
      format: "agent-manager/provider/v1",
      kind: "claude",
      connection: { ...connection, baseUrl: "http://192.168.1.10:8080" },
    }), "claude").baseUrl).toBe("http://192.168.1.10:8080");
    expect(() => parseConnectionBundle(JSON.stringify({
      format: "agent-manager/provider/v1",
      kind: "claude",
      connection: { ...connection, note: "过".repeat(201) },
    }), "claude")).toThrow("备注不能超过 200 个字符");
  });

  it("detectSystemPlatform returns darwin, linux or windows", () => {
    const platform = detectSystemPlatform();
    expect(["darwin", "linux", "windows"]).toContain(platform);
  });

  it("从实际助手定义读取 Provider 列表，保留顺序和表单字段", () => {
    expect(connectionEntriesOf(undefined)).toEqual([]);
    expect(connectionEntriesOf({ settings: { profiles: [] } })).toEqual([]);
    const entries = connectionEntriesOf({
      providers: { native: { install: "unused" } },
      settings: {
        active: "second",
        profiles: {
          first: { label: "第一套", base_url: "https://first.example.com", auth: "api_key", token_var: "FIRST_KEY" },
          second: { label: "第二套", model: "second-model", model_aliases: { fable: "small-model" } },
        },
      },
    });
    expect(entries.map((entry) => entry.name)).toEqual(["first", "second"]);
    expect(entries[0]).toMatchObject({ label: "第一套", baseUrl: "https://first.example.com", auth: "api_key", tokenVar: "FIRST_KEY" });
    expect(entries[1]).toMatchObject({ label: "第二套", model: "second-model", aliasFable: "small-model" });
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
        note: "工作账号",
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
            note: "工作账号",
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
      note: "新的说明",
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
      note: "新的说明",
      base_url: "https://new.example.com",
      token_var: "NEW_KEY",
      model: "new-model",
      future_option: { mode: "keep" },
    });
    expect((result.settings as any).active).toBe("relay");
  });

  it("复制接入生成不冲突且合法的名称，并保留原草稿不变", () => {
    const original = {
      name: "a".repeat(64),
      label: "主线路",
      baseUrl: "https://relay.example.com",
      auth: "token" as const,
      tokenVar: "RELAY_KEY",
      model: "relay-model",
      smallFastModel: "",
      aliasOpus: "",
      aliasSonnet: "",
      aliasHaiku: "",
      timeoutMs: "",
    };
    const firstName = `${"a".repeat(59)}-copy`;
    const result = duplicateConnectionDraft(original, [original.name, firstName]);

    expect(result).toMatchObject({
      name: `${"a".repeat(57)}-copy-2`,
      label: "主线路 副本",
      baseUrl: "https://relay.example.com",
      tokenVar: "RELAY_KEY",
    });
    expect(result.name).toHaveLength(64);
    expect(original.name).toHaveLength(64);
    expect(original.label).toBe("主线路");
  });

  it("复制接入可从来源项带入未知字段，同时不改动来源项", () => {
    const initial = {
      settings: {
        active: "relay",
        profiles: {
          relay: {
            label: "中转",
            base_url: "https://old.example.com",
            token_var: "RELAY_KEY",
            model: "old-model",
            future_option: { mode: "keep" },
          },
        },
      },
    };
    const result = upsertAgentConnection(
      initial,
      {
        name: "relay-copy",
        label: "中转 副本",
        baseUrl: "https://new.example.com",
        auth: "token",
        tokenVar: "RELAY_KEY",
        model: "new-model",
        smallFastModel: "",
        aliasOpus: "",
        aliasSonnet: "",
        aliasHaiku: "",
        timeoutMs: "",
      },
      "relay",
    );

    expect((result.settings as any).profiles.relay).toEqual(initial.settings.profiles.relay);
    expect((result.settings as any).profiles["relay-copy"]).toEqual({
      label: "中转 副本",
      base_url: "https://new.example.com",
      token_var: "RELAY_KEY",
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

  it("只有补丁把档位改成完全放开时才要求二次确认", () => {
    const fields = agentManagementFixture("codex").runtime_fields;
    expect(runtimeRequiresYoloConfirmation({ automation_mode: "full" }, fields)).toBe(true);
    expect(runtimeRequiresYoloConfirmation({ codex_web_search: "live" }, fields)).toBe(false);
  });

  it("编辑已有接入时地址、凭据变量与模型可留空，填了仍要合法；新增时照旧必填", () => {
    const imported = {
      name: "imported-20260922-193000",
      label: "导入的配置",
      baseUrl: "",
      auth: "token" as const,
      tokenVar: "",
      model: "",
      smallFastModel: "",
      aliasOpus: "",
      aliasSonnet: "",
      aliasHaiku: "",
      timeoutMs: "",
    };
    expect(validateConnectionEntry(imported, false)).toBeNull();
    expect(validateConnectionEntry(imported)).toBe("接口地址必须是 https:// 或 http://");
    expect(validateConnectionEntry({ ...imported, tokenVar: "has-dash" }, false)).toBe("凭据变量名格式不正确");
    expect(validateConnectionEntry({ ...imported, providerRef: "kimi", model: "k2" })).toBeNull();
  });

  it("地址里内嵌账号密码的接入存不下：那部分是凭据", () => {
    expect(urlHasUserinfo("https://u:p@gw.example.com/v1")).toBe(true);
    expect(urlHasUserinfo("https://gw.example.com/v1/users/@me")).toBe(false);
    expect(urlHasUserinfo("not a url")).toBe(false);
    const entry = {
      name: "relay",
      label: "中转",
      baseUrl: "https://u:p@gw.example.com",
      auth: "token" as const,
      tokenVar: "RELAY_TOKEN",
      model: "m",
      smallFastModel: "",
      aliasOpus: "",
      aliasSonnet: "",
      aliasHaiku: "",
      timeoutMs: "",
    };
    expect(validateConnectionEntry(entry)).toBe("接口地址里不能带账号密码：凭据请用令牌变量");
    expect(validateConnectionEntry({ ...entry, baseUrl: "https://gw.example.com" })).toBeNull();
  });
});

describe("moveAgentConnection", () => {
  it("只调整 profiles 顺序，保留当前项、未知字段和接入内容", () => {
    const initial = {
      custom: { keep: true },
      settings: {
        active: "beta",
        yolo: true,
        profiles: {
          alpha: { label: "Alpha", base_url: "https://a.example.com", extra: 1 },
          beta: { label: "Beta", base_url: "https://b.example.com", extra: 2 },
          gamma: { label: "Gamma", base_url: "https://c.example.com", extra: 3 },
        },
      },
    };

    const moved = moveAgentConnection(initial, "beta", "up");
    expect(Object.keys((moved.settings as any).profiles)).toEqual(["beta", "alpha", "gamma"]);
    expect((moved.settings as any).active).toBe("beta");
    expect((moved.settings as any).yolo).toBe(true);
    expect(((moved.settings as any).profiles.beta as any).extra).toBe(2);
    expect(moved.custom).toEqual({ keep: true });
    expect(initial.settings.profiles).toHaveProperty("alpha");
  });

  it("边界移动与未知名称保持原定义", () => {
    const initial = { settings: { profiles: { alpha: {}, beta: {} } } };
    expect(moveAgentConnection(initial, "alpha", "up")).toEqual(initial);
    expect(moveAgentConnection(initial, "beta", "down")).toEqual(initial);
    expect(moveAgentConnection(initial, "missing", "up")).toEqual(initial);
  });

  it("Fable 别名存进 model_aliases.fable，读回表单也还在；没填时不多出字段", () => {
    const values = connectionEntriesOf({
      settings: { profiles: { relay: { base_url: "https://r.example.com", model_aliases: { fable: "relay-fable", haiku: "h" } } } },
    });
    const relay = values.find((item) => item.name === "relay")!;
    expect(relay.aliasFable).toBe("relay-fable");
    expect(connectionToDefinitionValue(relay).model_aliases).toEqual({ fable: "relay-fable", haiku: "h" });
    const plain = connectionEntriesOf({
      settings: { profiles: { p: { base_url: "https://r.example.com" } } },
    }).find((item) => item.name === "p")!;
    expect("aliasFable" in plain).toBe(false);
  });
});
