import { describe, expect, it } from "vitest";
import { runtimeAlignment, runtimeComparisons } from "../lib/agentRuntime";
import { agentManagementFixture } from "./fixtures";

describe("运行设置实际值对比", () => {
  it("逐项识别非自动化字段漂移，避免只看权限档位误报一致", () => {
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
        },
      },
    };
    const saved = {
      automation_mode: "auto",
      codex_reasoning_effort: "high",
      codex_web_search: "live",
      codex_context_window: 1000000,
    };

    expect(runtimeAlignment(fields, saved, status)).toBe("drifted");
    expect(runtimeComparisons(fields, saved, status).filter((item) => item.aligned === false))
      .toEqual([expect.objectContaining({ key: "codex_reasoning_effort", saved: "high", actual: "medium" })]);
  });

  it("未创建配置文件时按客户端默认值核对，异常文件则保持未知", () => {
    const fields = agentManagementFixture("claude").runtime_fields;
    expect(runtimeAlignment(fields, { automation_mode: "manual" }, {
      state: "missing",
      file: "settings.json",
      config: null,
    })).toBe("aligned");
    expect(runtimeAlignment(fields, { automation_mode: "manual" }, {
      state: "invalid",
      file: "settings.json",
      config: null,
    })).toBe("unknown");
  });

  it("用户自定义状态栏在软件选择保持现状时不算漂移", () => {
    const fields = agentManagementFixture("claude").runtime_fields;
    const status = {
      state: "ready",
      file: "settings.json",
      config: { runtime: { automation_mode: "manual", statusline: "custom" } },
    };

    expect(runtimeAlignment(fields, { automation_mode: "manual" }, status)).toBe("aligned");
    expect(runtimeComparisons(fields, { automation_mode: "manual" }, status)
      .find((item) => item.key === "statusline")?.actual)
      .toBe("自定义状态栏（不受本工具管理）");
  });

  it("旧格式的本工具状态栏显示升级提示，并与软件配置 builtin 判为不一致", () => {
    const fields = agentManagementFixture("claude").runtime_fields;
    const status = {
      state: "ready",
      file: "settings.json",
      config: { runtime: { automation_mode: "manual", statusline: "builtin-legacy" } },
    };
    const saved = { automation_mode: "manual", statusline: "builtin" };

    expect(runtimeAlignment(fields, saved, status)).toBe("drifted");
    expect(runtimeComparisons(fields, saved, status).find((item) => item.key === "statusline"))
      .toEqual(expect.objectContaining({
        aligned: false,
        saved: "用量状态栏",
        actual: "本工具状态栏（旧格式，路径含空格时无法运行，重新应用即可修复）",
      }));

    // 重新应用后回读为 builtin：恢复一致
    expect(runtimeAlignment(fields, saved, {
      ...status,
      config: { runtime: { automation_mode: "manual", statusline: "builtin" } },
    })).toBe("aligned");
  });
});
