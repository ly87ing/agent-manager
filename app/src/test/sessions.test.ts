import { describe, expect, it } from "vitest";
import type { AgentAccounts } from "../lib/accounts";
import { agoLabel, launchCommandOf, resumeCommand } from "../lib/sessions";

const groups = [
  { agentId: "claude-code", label: "Claude", kind: "claude", bin: "claude", accounts: [
    { elementId: "agents:claude-code", isMain: true, command: "claude" },
    { elementId: "profiles:work", isMain: false, command: "cw" },
    { elementId: "profiles:relay", isMain: false, command: null },
  ] },
  { agentId: "codex", label: "Codex", kind: "codex", bin: "codex", accounts: [{ elementId: "agents:codex", isMain: true, command: "codex" }] },
] as unknown as AgentAccounts[];

describe("回到会话的命令", () => {
  it("按用量里的账号记法找到同一个账号的启动命令", () => {
    expect(launchCommandOf("claude", groups)).toBe("claude");
    expect(launchCommandOf("claude:work", groups)).toBe("cw");
    expect(launchCommandOf("claude:relay", groups)).toBeNull();
    expect(launchCommandOf("codex", groups)).toBe("codex");
    expect(launchCommandOf("codex:missing", groups)).toBeNull();
  });

  it("先进当时的目录再接着聊；Claude 用 --resume，Codex 用 resume 子命令", () => {
    const session = { id: "abc", cwd: "/work/my app", title: null, updated_at: "" };
    expect(resumeCommand("claude", "cw", session)).toBe("cd '/work/my app' && cw --resume abc");
    expect(resumeCommand("codex", "codex", { ...session, cwd: "/work/app" })).toBe("cd /work/app && codex resume abc");
    expect(resumeCommand("claude", "claude", { ...session, cwd: "/it's" })).toBe(`cd '/it'"'"'s' && claude --resume abc`);
    expect(resumeCommand("claude", "claude", { ...session, cwd: null })).toBe("claude --resume abc");
    expect(resumeCommand("claude", "claude", { ...session, cwd: "C:\\work\\app" })).toBe('cd /d "C:\\work\\app" && claude --resume abc');
  });

  it("相对时间", () => {
    const now = new Date("2026-10-09T10:00:00Z");
    expect(agoLabel("2026-10-09T09:59:40Z", now)).toBe("刚刚");
    expect(agoLabel("2026-10-09T09:57:00Z", now)).toBe("3 分钟前");
    expect(agoLabel("2026-10-09T07:00:00Z", now)).toBe("3 小时前");
    expect(agoLabel("2026-10-04T10:00:00Z", now)).toBe("5 天前");
  });
});
