/**
 * 外部设置对官方账号或独立环境的影响说明（依据 Claude Code 官方认证优先级文档、设置参考，
 * Codex `rust-v0.156.1` 源码，并用真实 CLI 复现过）。引擎只报来源与键名，这里把键名翻成后果。
 */

/** 主配置里的官方账号会被怎样压过或限定。 */
const MAIN_EFFECT: Record<string, string> = {
  ANTHROPIC_AUTH_TOKEN: "Claude 会优先用这个令牌，不用官方账号",
  ANTHROPIC_API_KEY: "Claude 可能改用这个 API key（-p 模式直接使用，交互模式确认后使用）",
  ANTHROPIC_BASE_URL: "Claude 会把请求连同官方账号的登录凭据发往这个地址",
  CLAUDE_CODE_OAUTH_TOKEN: "Claude 会用这个长期令牌，不用主配置里的登录",
  CLAUDE_CODE_USE_BEDROCK: "Claude 改走 Amazon Bedrock，不用官方账号",
  CLAUDE_CODE_USE_VERTEX: "Claude 改走 Google Vertex，不用官方账号",
  CLAUDE_CODE_USE_FOUNDRY: "Claude 改走 Microsoft Foundry，不用官方账号",
  CLAUDE_CODE_USE_ANTHROPIC_AWS: "Claude 改走 Claude Platform on AWS，不用官方账号",
  CLAUDE_CODE_USE_MANTLE: "Claude 改走 Amazon Bedrock Mantle，不用官方账号",
  ANTHROPIC_PROFILE: "Claude 会用这个 Anthropic 配置档案的凭据，优先于 /login 登录",
  ANTHROPIC_FEDERATION_RULE_ID: "Claude 会用工作负载身份联合换取的令牌，优先于 /login 登录",
  ANTHROPIC_CUSTOM_HEADERS: "每个请求都会附加这些请求头，其中可能有凭据头",
  oidc_federation: "活动的 Anthropic 配置档案是联合身份，Claude 会优先用它换取的令牌，不用 /login 登录",
  apiKeyHelper: "Claude 会优先用这条命令输出的凭据，不用官方账号",
  forceLoginMethod: "组织统一下发的设置限定了登录方式；用第三方凭据的独立环境会在启动时被拦下",
  forceLoginOrgUUID: "组织统一下发的设置限定了可登录的组织；用第三方凭据的独立环境会在启动时被拦下",
  CODEX_API_KEY: "codex exec 等非交互命令会优先用这个 API key，不用 auth.json 里的登录（交互界面不读它）",
  CODEX_ACCESS_TOKEN: "Codex 所有入口都会优先用这个访问令牌，不用 auth.json 里的登录",
  CLAUDE_CONFIG_DIR: "Claude 会改用这个配置目录：主配置 ~/.claude 与 ~/.claude.json 里的设置与登录都不生效",
  CODEX_HOME: "Codex 会改用这个目录：主配置 ~/.codex 里的设置与登录都不生效",
  openai_base_url: "Codex 会把官方账号的凭据连同请求发往这个地址",
  chatgpt_base_url: "Codex 会把 ChatGPT 登录的凭据发往这个地址",
  forced_login_method: "限定了 Codex 的登录方式：另一种方式登录会被拒绝，已有的会在启动时被 Codex 退出",
  forced_chatgpt_workspace_id: "限定了可登录的 ChatGPT 工作区：不在其中的登录会在启动时被 Codex 退出",
  model_provider: "Codex 改用这个接入方式，不用官方账号",
  model_providers: "组织下发的同名接入定义会整个替换掉本地的",
  allowed_login_methods: "组织限定了 Codex 可用的登录方式",
  allowed_chatgpt_workspaces: "组织限定了可登录的 ChatGPT 工作区",
};

/** 独立环境：这些系统级设置对每个配置目录都生效，环境自己的配置挡不住。 */
const INSTANCE_EFFECT: Record<string, string> = {
  ANTHROPIC_AUTH_TOKEN: "改用这个令牌，不用这个环境自己的凭据或登录",
  ANTHROPIC_API_KEY: "改用这个 API key，不用这个环境自己的凭据或登录",
  ANTHROPIC_BASE_URL: "请求改发往这个地址，不用这个环境自己的",
  CLAUDE_CODE_OAUTH_TOKEN: "改用这个长期令牌，不用这个环境自己的凭据或登录",
  CLAUDE_CODE_USE_BEDROCK: "改走 Amazon Bedrock，不用这个环境自己的接入",
  CLAUDE_CODE_USE_VERTEX: "改走 Google Vertex，不用这个环境自己的接入",
  CLAUDE_CODE_USE_FOUNDRY: "改走 Microsoft Foundry，不用这个环境自己的接入",
  CLAUDE_CODE_USE_ANTHROPIC_AWS: "改走 Claude Platform on AWS，不用这个环境自己的接入",
  CLAUDE_CODE_USE_MANTLE: "改走 Amazon Bedrock Mantle，不用这个环境自己的接入",
  ANTHROPIC_PROFILE: "改用这个 Anthropic 配置档案的凭据，优先于这个环境里的登录",
  ANTHROPIC_CUSTOM_HEADERS: "每个请求都会附加这些请求头，其中可能有凭据头",
  oidc_federation: "Anthropic 配置档案不随配置目录隔离：活动档案是联合身份，会优先于这个环境里的登录",
  apiKeyHelper: "会用这条命令输出的凭据，优先于这个环境里的登录",
  forceLoginMethod: "限定了登录方式",
  forceLoginOrgUUID: "限定了可登录的组织",
  openai_base_url: "官方账号的凭据会连同请求发往这个地址",
  chatgpt_base_url: "ChatGPT 登录的凭据会发往这个地址",
  forced_login_method: "限定了登录方式：另一种方式登录会被拒绝，已有的会在启动时被 Codex 退出",
  forced_chatgpt_workspace_id: "限定了可登录的 ChatGPT 工作区：不在其中的登录会在启动时被 Codex 退出",
  model_provider: "强制改用这个接入方式，这个环境自己的接入不生效",
  model_providers: "同名接入定义被整个替换，地址与凭据改用组织下发的",
  allowed_login_methods: "限定了可用的登录方式",
  allowed_chatgpt_workspaces: "限定了可登录的 ChatGPT 工作区",
};

/** 用第三方凭据的环境：限定登录的设置不是「限定」而是直接拦下。 */
const THIRD_PARTY_EFFECT: Record<string, string> = {
  forceLoginMethod: "限定了登录方式：用第三方凭据的这个环境启动时会被拦下",
  forceLoginOrgUUID: "限定了可登录的组织：用第三方凭据的这个环境启动时会被拦下",
};

export type OverrideScope = "main" | "official-instance" | "third-party-instance";

export function overrideEffect(name: string, scope: OverrideScope): string {
  if (scope === "main") return MAIN_EFFECT[name] ?? "会改变助手使用的凭据";
  if (scope === "third-party-instance" && THIRD_PARTY_EFFECT[name]) return THIRD_PARTY_EFFECT[name];
  return INSTANCE_EFFECT[name] ?? "会改变这个环境使用的凭据";
}
