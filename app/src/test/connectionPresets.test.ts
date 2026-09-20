import { describe, expect, it } from "vitest";
import {
  CONNECTION_PRESETS,
  connectionFromPreset,
  presetConnectionName,
  presetsFor,
} from "../lib/connectionPresets";

/**
 * 预设表的约束：地址写错会让人连到错的地方，模型名写错会当场报错。
 * 这里锁住的是「不会把两种协议的端点弄混」与「每条都能变成一套可保存的配置」。
 */
describe("接入配置预设", () => {
  it("两种形态的端点不混用：Claude 走 anthropic 路径，Codex 不带它", () => {
    for (const preset of presetsFor("claude")) {
      expect(preset.baseUrl, preset.id).toMatch(/\/anthropic$/);
    }
    for (const preset of presetsFor("codex")) {
      // Codex 走 Responses 协议，端点不是 anthropic 那条
      expect(preset.baseUrl, preset.id).not.toMatch(/\/anthropic/);
    }
    // 同一厂商两侧地址必须不同，否则一定有一边是抄错的
    const glmClaude = CONNECTION_PRESETS.find((p) => p.id === "glm-cn-claude")!;
    const glmCodex = CONNECTION_PRESETS.find((p) => p.id === "glm-cn-codex")!;
    expect(glmClaude.baseUrl).not.toBe(glmCodex.baseUrl);
  });

  it("每条都只用 https、有令牌变量名与出处", () => {
    for (const preset of CONNECTION_PRESETS) {
      expect(preset.baseUrl, preset.id).toMatch(/^https:\/\//);
      expect(preset.tokenVar, preset.id).toMatch(/^[A-Za-z_][A-Za-z0-9_]*$/);
      expect(preset.source.length, preset.id).toBeGreaterThan(0);
    }
  });

  it("id 不重复；按形态筛选只给得到本形态的", () => {
    const ids = CONNECTION_PRESETS.map((p) => p.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(presetsFor("claude").every((p) => p.kind === "claude")).toBe(true);
    expect(presetsFor("codex").every((p) => p.kind === "codex")).toBe(true);
    expect(presetsFor(null)).toEqual([]);
  });

  it("预设变成一套配置：字段填好，令牌只带变量名不带取值", () => {
    const preset = CONNECTION_PRESETS.find((p) => p.id === "kimi-cn-claude")!;
    const entry = connectionFromPreset(preset, "kimi-cn");
    expect(entry.name).toBe("kimi-cn");
    expect(entry.baseUrl).toBe(preset.baseUrl);
    expect(entry.model).toBe(preset.model);
    expect(entry.aliasHaiku).toBe(preset.aliasHaiku);
    expect(entry.tokenVar).toBe(preset.tokenVar);
    // 取值永远不在预设里
    expect(JSON.stringify(entry)).not.toMatch(/sk-|api[-_]?key["']?\s*[:=]\s*["'][^"']{8}/i);
  });

  it("键名避让已有的，不会把同厂商第二套顶掉", () => {
    const preset = CONNECTION_PRESETS.find((p) => p.id === "deepseek-claude")!;
    expect(presetConnectionName(preset, [])).toBe("deepseek");
    expect(presetConnectionName(preset, ["deepseek"])).toBe("deepseek2");
    expect(presetConnectionName(preset, ["deepseek", "deepseek2"])).toBe("deepseek3");
  });
});
