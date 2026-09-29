import { describe, expect, it } from "vitest";
import { accountGroups, type Account, type AgentAccounts } from "../lib/accounts";

const account = (elementId: string, name: string, isMain: boolean) => ({ elementId, name, isMain }) as Account;
const groups: AgentAccounts[] = [
  { agentId: "claude-code", label: "Claude", kind: "claude", bin: "claude", accounts: [account("agents:claude-code", "Claude", true), account("profiles:claude-work", "工作", false)] },
  { agentId: "codex", label: "Codex", kind: "codex", bin: "codex", accounts: [account("agents:codex", "Codex", true)] },
];

describe("资源表格的账号分组", () => {
  it("按账号页顺序分到各助手下，主账号叫「主号」，悬停用全名", () => {
    expect(accountGroups(groups, ["codex", "claude-work", "claude-code"])).toEqual([
      {
        key: "claude-code",
        label: "Claude",
        kind: "claude",
        columns: [
          { id: "claude-code", name: "主号", fullName: "Claude" },
          { id: "claude-work", name: "工作", fullName: "Claude · 工作" },
        ],
      },
      { key: "codex", label: "Codex", kind: "codex", columns: [{ id: "codex", name: "主号", fullName: "Codex" }] },
    ]);
  });

  it("只列出现的账号；账号页不认识的键单独成组", () => {
    expect(accountGroups(groups, ["claude-work", "kimi-code"])).toEqual([
      { key: "claude-code", label: "Claude", kind: "claude", columns: [{ id: "claude-work", name: "工作", fullName: "Claude · 工作" }] },
      { key: "kimi-code", label: "kimi-code", kind: null, columns: [{ id: "kimi-code", name: "主号", fullName: "kimi-code" }] },
    ]);
  });
});
