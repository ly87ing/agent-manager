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
  it("两种形态的端点不混用：Claude 走 anthropic 路径（少数厂商文档写明的专用根地址除外），Codex 不带它", () => {
    // Kimi Code、火山方舟 Coding Plan、阶跃 Step Plan 的 Claude Code 接入由厂商文档给出专用根地址
    const documentedRoots = ["kimi-code-cn-claude", "kimi-code-global-claude", "volcengine-coding-cn-claude", "stepfun-cn-claude", "stepfun-global-claude"];
    for (const preset of presetsFor("claude")) {
      if (documentedRoots.includes(preset.id)) {
        expect(preset.baseUrl, preset.id).not.toMatch(/\/v1\/?$/);
        continue;
      }
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

  it("Kimi Code 会员与 Kimi 开放平台是两套账号：密钥变量不混用", () => {
    const code = CONNECTION_PRESETS.filter((p) => p.baseUrl.includes("/coding/"));
    const open = CONNECTION_PRESETS.filter((p) => p.baseUrl.includes("moonshot."));
    expect(code.length).toBeGreaterThan(0);
    expect(open.length).toBeGreaterThan(0);
    const codeVars = new Set(code.map((p) => p.tokenVar));
    expect(open.some((p) => codeVars.has(p.tokenVar))).toBe(false);
    expect(code.every((p) => p.label.includes("Kimi Code"))).toBe(true);
  });

  it("同一厂商国内与国际是两套账号：密钥变量不混用，同一区域两种形态共用一个", () => {
    for (const vendor of ["glm", "kimi", "kimi-code", "minimax"]) {
      const cn = CONNECTION_PRESETS.filter((p) => p.id.startsWith(`${vendor}-cn-`));
      const global = CONNECTION_PRESETS.filter((p) => p.id.startsWith(`${vendor}-global-`));
      expect(cn.length, vendor).toBe(2);
      expect(global.length, vendor).toBe(2);
      expect(new Set(cn.map((p) => p.tokenVar)).size, vendor).toBe(1);
      expect(new Set(global.map((p) => p.tokenVar)).size, vendor).toBe(1);
      expect(cn[0].tokenVar, vendor).not.toBe(global[0].tokenVar);
    }
    const stepfun = CONNECTION_PRESETS.filter((p) => p.id.startsWith("stepfun-"));
    expect(new Set(stepfun.map((p) => p.tokenVar)).size).toBe(2);
    // 厂商自己的命令行怎么叫就怎么叫
    expect(CONNECTION_PRESETS.find((p) => p.id === "minimax-cn-claude")!.tokenVar).toBe("MINIMAX_CN_API_KEY");
    expect(CONNECTION_PRESETS.find((p) => p.id === "minimax-global-claude")!.tokenVar).toBe("MINIMAX_API_KEY");
  });

  it("MiniMax 国内预设使用当前官方端点和文档，国际预设保持独立", () => {
    const cnClaude = CONNECTION_PRESETS.find((p) => p.id === "minimax-cn-claude")!;
    const cnCodex = CONNECTION_PRESETS.find((p) => p.id === "minimax-cn-codex")!;
    expect(cnClaude.baseUrl).toBe("https://api.minimax.cn/anthropic");
    expect(cnCodex.baseUrl).toBe("https://api.minimax.cn/v1");
    expect(cnClaude.docsUrl).toBe("https://platform.minimax.cn/docs/token-plan/claude-code");
    expect(cnCodex.docsUrl).toBe("https://platform.minimax.cn/docs/token-plan/codex");
    expect(CONNECTION_PRESETS.find((p) => p.id === "minimax-global-claude")!.baseUrl).toBe("https://api.minimax.io/anthropic");
    expect(CONNECTION_PRESETS.find((p) => p.id === "minimax-global-codex")!.baseUrl).toBe("https://api.minimax.io/v1");
  });

  it("MiniMax 的 Claude 预设按官方当前接入文档把常用模型都指向 M3", () => {
    for (const id of ["minimax-cn-claude", "minimax-global-claude"]) {
      const preset = CONNECTION_PRESETS.find((p) => p.id === id)!;
      expect([preset.model, preset.aliasOpus, preset.aliasSonnet, preset.aliasHaiku], id)
        .toEqual(Array(4).fill("MiniMax-M3[1m]"));
    }
  });

  it("DeepSeek 的 Claude 预设按官方文档设置子代理模型", () => {
    const preset = CONNECTION_PRESETS.find((p) => p.id === "deepseek-claude")!;
    const entry = connectionFromPreset(preset, "deepseek");
    expect(entry.subagentModel).toBe("deepseek-flash");
  });

  it("密钥与文档链接只用 https；Kimi Code 的 Claude 接入按官方写法走 x-api-key 并带上 fable 与子代理模型", () => {
    for (const preset of CONNECTION_PRESETS) {
      for (const url of [preset.apiKeyUrl, preset.docsUrl]) {
        if (url) expect(url, preset.id).toMatch(/^https:\/\/[^/]+\//);
      }
    }
    const kimiCode = CONNECTION_PRESETS.find((p) => p.id === "kimi-code-cn-claude")!;
    const entry = connectionFromPreset(kimiCode, "kimi-code-cn");
    expect(entry.auth).toBe("api_key");
    expect(entry.aliasFable).toBe("k3-256k");
    expect(entry.subagentModel).toBe("k3-256k");
    expect(connectionFromPreset(CONNECTION_PRESETS.find((p) => p.id === "glm-cn-claude")!, "glm").auth).toBe("token");
  });

  it("厂商建议的请求超时带进表单", () => {
    const preset = CONNECTION_PRESETS.find((p) => p.id === "glm-cn-claude")!;
    expect(connectionFromPreset(preset, "glm-cn").timeoutMs).toBe("3000000");
    const deepseek = CONNECTION_PRESETS.find((p) => p.id === "deepseek-claude")!;
    expect(connectionFromPreset(deepseek, "deepseek").timeoutMs).toBe("");
  });

  it("键名避让已有的，不会把同厂商第二套顶掉", () => {
    const preset = CONNECTION_PRESETS.find((p) => p.id === "deepseek-claude")!;
    expect(presetConnectionName(preset, [])).toBe("deepseek");
    expect(presetConnectionName(preset, ["deepseek"])).toBe("deepseek2");
    expect(presetConnectionName(preset, ["deepseek", "deepseek2"])).toBe("deepseek3");
  });
});
