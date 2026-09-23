import { describe, expect, it } from "vitest";
import {
  actualRuntimeSettings,
  runtimeActualItems,
  runtimePatch,
  runtimePatchApplied,
} from "../lib/agentRuntime";
import { liveRefusalDescription } from "../lib/agentCurrent";
import { agentManagementFixture } from "./fixtures";

describe("运行设置以客户端实际值为准", () => {
  it("表单初值取自实际配置，只含适配器声明的字段", () => {
    const fields = agentManagementFixture("codex").runtime_fields;
    const status = {
      state: "ready",
      file: "config.toml",
      config: {
        runtime: {
          automation_mode: "auto",
          codex_reasoning_effort: "medium",
          codex_web_search: "live",
          codex_context_window: 1000000,
          codex_auto_compact_token_limit: null,
          not_a_field: "ignored",
        },
      },
    };

    expect(actualRuntimeSettings(fields, status)).toEqual({
      automation_mode: "auto",
      codex_reasoning_effort: "medium",
      codex_web_search: "live",
      codex_context_window: 1000000,
    });
  });

  it("未创建配置文件时按客户端默认值预填，读不了时不给初值", () => {
    const fields = agentManagementFixture("claude").runtime_fields;
    expect(actualRuntimeSettings(fields, { state: "missing", file: "settings.json", config: null }))
      .toEqual({ automation_mode: "manual" });
    expect(actualRuntimeSettings(fields, { state: "invalid", file: "settings.json", config: null }))
      .toEqual({});
  });

  it("本工具状态栏（含旧格式）预填为 builtin，用户自己的状态栏预填为保持现状", () => {
    const fields = agentManagementFixture("claude").runtime_fields;
    const status = (statusline: string) => ({
      state: "ready",
      file: "settings.json",
      config: { runtime: { automation_mode: "manual", statusline } },
    });
    expect(actualRuntimeSettings(fields, status("builtin-legacy")).statusline).toBe("builtin");
    expect(actualRuntimeSettings(fields, status("custom")).statusline).toBeUndefined();
    expect(runtimeActualItems(fields, status("custom")).find((item) => item.key === "statusline")?.actual)
      .toBe("自定义状态栏（不受本工具管理）");
  });

  it("提交只发改动过的字段，撤掉的项为 null；没改的本工具不提供的取值原样保留", () => {
    const fields = agentManagementFixture("claude").runtime_fields;
    const initial = { automation_mode: "custom", claude_effort_level: "high", claude_tool_search: true };

    expect(runtimePatch(fields, initial, { ...initial })).toEqual({});
    expect(runtimePatch(fields, initial, {
      automation_mode: "custom",
      claude_effort_level: "",
      claude_tool_search: false,
      claude_agent_teams: true,
    })).toEqual({ claude_effort_level: null, claude_tool_search: false, claude_agent_teams: true });
    expect(runtimePatch(fields, initial, { ...initial, automation_mode: "auto" }))
      .toEqual({ automation_mode: "auto" });
    expect(() => runtimePatch(fields, initial, { ...initial, claude_effort_level: "ultra" }))
      .toThrow("不是适配器支持的取值");

    const codex = agentManagementFixture("codex").runtime_fields;
    expect(runtimePatch(codex, {}, { codex_context_window: "1000000" })).toEqual({ codex_context_window: 1000000 });
    expect(() => runtimePatch(codex, {}, { codex_context_window: "-1" })).toThrow("正整数");
  });

  it("自定义档位显示原始键值；写入后按补丁逐项回读确认", () => {
    const fields = agentManagementFixture("claude").runtime_fields;
    const status = {
      state: "ready",
      file: "settings.json",
      config: {
        runtime: {
          automation_mode: "custom",
          automation_raw: { "permissions.defaultMode": "acceptEdits" },
          claude_effort_level: "low",
        },
      },
    };
    expect(runtimeActualItems(fields, status).find((item) => item.key === "automation_mode")?.actual)
      .toBe("自定义（permissions.defaultMode = acceptEdits）");
    expect(runtimePatchApplied(fields, { claude_effort_level: "low" }, status)).toBe(true);
    expect(runtimePatchApplied(fields, { claude_effort_level: "high" }, status)).toBe(false);
    expect(runtimePatchApplied(fields, { claude_effort_level: "low" }, { ...status, state: "invalid" })).toBe(false);
  });

  it("修复旧格式状态栏必须回读到引用形式才算成功，旧格式仍在不算", () => {
    const fields = agentManagementFixture("claude").runtime_fields;
    const status = (statusline: string | null) => ({
      state: "ready",
      file: "settings.json",
      config: { runtime: { automation_mode: "manual", statusline } },
    });
    expect(runtimePatchApplied(fields, { statusline: "builtin" }, status("builtin"))).toBe(true);
    expect(runtimePatchApplied(fields, { statusline: "builtin" }, status("builtin-legacy"))).toBe(false);
    expect(runtimePatchApplied(fields, { statusline: "builtin" }, status("custom"))).toBe(false);
    // 撤掉：本工具的两种写法都不能还在
    expect(runtimePatchApplied(fields, { statusline: null }, status(null))).toBe(true);
    expect(runtimePatchApplied(fields, { statusline: null }, status("custom"))).toBe(true);
    expect(runtimePatchApplied(fields, { statusline: null }, status("builtin-legacy"))).toBe(false);
  });

  it("回填被拒的弹窗正文用界面自己的话，列出存不下的原因但不带命令行参数", () => {
    const token = liveRefusalDescription("live-token", "当前实际配置里的令牌还没有保存在任何变量里，切换会丢失它：先到“设置 → 密钥”选择外部密钥文件（令牌会写进该文件，库里只存变量名），或确认放弃这个令牌后带 --discard-live-token 重试");
    expect(token).toContain("设置 → 密钥");
    expect(token).not.toContain("--");
    const unrepresentable = liveRefusalDescription("unrepresentable", "当前实际接入配置无法完整保存，切换会丢失这些内容：同时设置了 ANTHROPIC_AUTH_TOKEN 与 ANTHROPIC_API_KEY，只能保存其中一个。确认放弃当前实际配置后可带 --discard-live 重试");
    expect(unrepresentable).toContain("同时设置了 ANTHROPIC_AUTH_TOKEN 与 ANTHROPIC_API_KEY，只能保存其中一个");
    expect(unrepresentable).not.toContain("--");
    // 引擎原文格式变了也不会把参数漏给用户
    expect(liveRefusalDescription("unrepresentable", "意外格式 --discard-live")).not.toContain("--");
  });
});

describe("runtimePatchApplied 清理覆盖变量", () => {
  it("回读到一个覆盖变量都不剩才算生效", () => {
    const ready = (overrides: Record<string, string>) =>
      ({ state: "ready", config: { runtime: {}, claude_env_overrides: overrides } }) as never;
    expect(runtimePatchApplied([], { claude_clear_env_overrides: true }, ready({}))).toBe(true);
    expect(runtimePatchApplied([], { claude_clear_env_overrides: true }, ready({ CLAUDE_CODE_EFFORT_LEVEL: "low" }))).toBe(false);
  });
});

