import { describe, expect, it } from "vitest";
import { buildAccounts } from "../lib/accounts";
import { agentManagementFixture, makeElement } from "./fixtures";

const claude = makeElement({
  id: "agents:claude-code",
  domain: "agents",
  spec: { display_name: "Claude Code", bin: "claude" },
  observed: {
    management: agentManagementFixture("claude"),
    usage: { source: "~/.claude.json", checked_at: null, windows: [] },
    current_config: { state: "ready", file: "~/.claude/settings.json", current: { profile: null, default: false, official: false } },
  },
});
// 实例同时出现在 agents:status 里；账号列表只从 profiles:status 取，不能重复成主账号
const instanceAsAgent = makeElement({ id: "agents:cc2", domain: "agents", spec: { display_name: "Claude Code · 公司 Pro" }, observed: { management: agentManagementFixture("claude") } });

function profile(id: string, spec: Record<string, unknown>, observed: Record<string, unknown> = {}, state = "managed-ok") {
  return makeElement({
    id: `profiles:${id}`,
    domain: "profiles",
    spec: { base_agent: "claude-code", kind: "claude", base_url: null, model: null, ...spec },
    observed: { config_dir: spec.config_dir, blocked_reason: null, ...observed },
    state: state as never,
  });
}

describe("buildAccounts", () => {
  it("主账号在前，实例按名称排序；启动命令按配置目录反查 alias", () => {
    const groups = buildAccounts(
      [claude, instanceAsAgent],
      [
        profile("cc2", { display_name: "Claude Code · 公司 Pro", config_dir: "~/.claude-cc2" }),
        profile("ds", { display_name: "DeepSeek", config_dir: "~/.claude-ds", base_url: "https://api.deepseek.com/anthropic", model: "deepseek-v4" }, { third_party: true }, "drifted"),
      ],
      { cc2: 'CLAUDE_CONFIG_DIR="$HOME/.claude-cc2" claude', other: "ls -la" },
    );
    expect(groups).toHaveLength(1);
    const [main, ds, cc2] = groups[0].accounts;
    expect(main).toMatchObject({ elementId: "agents:claude-code", isMain: true, command: "claude", type: "official", mainHasThirdParty: true });
    expect(cc2).toMatchObject({ name: "公司 Pro", command: "cc2", type: "official", drifted: false });
    expect(ds).toMatchObject({ name: "DeepSeek", command: null, type: "api", apiHost: "api.deepseek.com", model: "deepseek-v4", drifted: true });
  });
});
