import { describe, expect, it } from "vitest";
import { defaultEnvState, isOfficial, switchBackEffects } from "../lib/defaultEnvironment";

const current = (over: Record<string, unknown>) => ({ profile: null, default: false, ...over });

describe("默认环境状态", () => {
  it("官方账号只指定了模型：账号仍是官方，模型写明来源", () => {
    const state = defaultEnvState("codex", current({ default: true, official: true }), { provider: "openai", model: "gpt-6-luna" });
    expect(state.account).toBe("official");
    expect(state.accountText).toBe("官方账号");
    expect(state.modelText).toBe("gpt-6-luna");
    expect(state.modelSource).toBe("config.toml 里的 model");
    expect(state.loginNote).toContain("~/.codex/auth.json 或系统钥匙串");
  });

  it("Claude：env 的 ANTHROPIC_MODEL 压过 /model 保存的 model；只有后者时用后者；都没有用默认", () => {
    const official = current({ default: true, official: true });
    expect(defaultEnvState("claude", official, { model: "opus", settings_model: "sonnet" }).modelSource).toContain(
      "压过 /model 保存的 sonnet",
    );
    const saved = defaultEnvState("claude", official, { settings_model: "sonnet" });
    expect(saved.modelText).toBe("sonnet");
    expect(saved.modelSource).toBe("settings.json 里的 model（/model 保存的）");
    const plain = defaultEnvState("claude", official, {});
    expect(plain.modelText).toBe("Claude Code 默认模型");
    expect(plain.modelSource).toBeNull();
  });

  it("第三方接入与读不到配置", () => {
    const relay = defaultEnvState("claude", current({}), { base_url: "https://relay.example.com", model: "glm" });
    expect(relay.account).toBe("third-party");
    expect(relay.accountText).toBe("第三方接入 · https://relay.example.com");
    expect(defaultEnvState("codex", current({}), { provider: "ollama" }).accountText).toBe("第三方接入 · 接入方式 ollama");
    expect(defaultEnvState("codex", null, null).account).toBe("unknown");
    // 老引擎没有 official 字段：按 default 推断
    expect(isOfficial(current({ default: true }))).toBe(true);
    expect(isOfficial(current({ default: false }))).toBe(false);
  });

  it("切回官方账号的说明写明撤哪些键、不动登录", () => {
    const codex = switchBackEffects("codex");
    expect(codex.removes).toContain("model_provider 与 model");
    expect(codex.keeps[0]).toContain("~/.codex/auth.json");
    expect(codex.after).toContain("codex login");
    expect(switchBackEffects("claude").removes).toContain("ANTHROPIC_AUTH_TOKEN");
  });
});
