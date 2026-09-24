import { describe, expect, it } from "vitest";
import { overrideEffect } from "../lib/overrideEffects";

/**
 * 引擎能报出的每个键都要有自己的后果说明，不能落到笼统的兜底句：
 * 名单与 engine-rs/src/native_auth.rs 里的常量一致。
 */
const CLAUDE_ENV = [
  "ANTHROPIC_AUTH_TOKEN",
  "ANTHROPIC_API_KEY",
  "ANTHROPIC_BASE_URL",
  "CLAUDE_CODE_OAUTH_TOKEN",
  "CLAUDE_CODE_USE_BEDROCK",
  "CLAUDE_CODE_USE_VERTEX",
  "CLAUDE_CODE_USE_FOUNDRY",
  "CLAUDE_CODE_USE_ANTHROPIC_AWS",
  "CLAUDE_CODE_USE_MANTLE",
  "ANTHROPIC_PROFILE",
  "ANTHROPIC_CUSTOM_HEADERS",
];
const CLAUDE_MANAGED = [...CLAUDE_ENV, "apiKeyHelper", "forceLoginMethod", "forceLoginOrgUUID"];
const CODEX_SYSTEM = [
  "model_provider",
  "model_providers",
  "openai_base_url",
  "chatgpt_base_url",
  "forced_login_method",
  "forced_chatgpt_workspace_id",
  "allowed_login_methods",
  "allowed_chatgpt_workspaces",
];

describe("外部设置的后果说明", () => {
  it("主配置能报出的键都有专门说明", () => {
    const names = [...CLAUDE_MANAGED, "ANTHROPIC_FEDERATION_RULE_ID", "oidc_federation", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN", "CLAUDE_CONFIG_DIR", "CODEX_HOME", ...CODEX_SYSTEM];
    for (const name of names) {
      expect(overrideEffect(name, "main"), name).not.toBe("会改变助手使用的凭据");
    }
  });

  it("独立环境能报出的系统级键都有专门说明", () => {
    for (const name of [...CLAUDE_MANAGED, "oidc_federation", ...CODEX_SYSTEM]) {
      for (const scope of ["official-instance", "third-party-instance"] as const) {
        expect(overrideEffect(name, scope), `${scope} ${name}`).not.toBe("会改变这个环境使用的凭据");
      }
    }
  });

  it("限定登录方式对第三方环境是直接拦下，对官方账号环境只是限定", () => {
    expect(overrideEffect("forceLoginMethod", "third-party-instance")).toContain("会被拦下");
    expect(overrideEffect("forceLoginMethod", "official-instance")).not.toContain("拦下");
  });
});
