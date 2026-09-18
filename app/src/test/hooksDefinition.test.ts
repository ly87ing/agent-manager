import { describe, expect, it } from "vitest";
import { buildHooks, flattenHooks, hookRowError, type HookRow } from "../lib/hooksDefinition";

const source = {
  PreToolUse: [
    {
      matcher: "Edit|Write",
      hooks: [
        { type: "command", command: "./guard.sh", timeout: 30 },
        { type: "command", command: "./audit.sh" },
      ],
    },
  ],
  SessionStart: [{ hooks: [{ type: "command", command: "echo hi" }] }],
};

describe("Hooks 结构与行的互转", () => {
  it("展开为逐条命令，带上匹配器", () => {
    const rows = flattenHooks(source);
    expect(rows).toHaveLength(3);
    expect(rows[0]).toMatchObject({ event: "PreToolUse", matcher: "Edit|Write", command: "./guard.sh" });
    expect(rows[2]).toMatchObject({ event: "SessionStart", matcher: "", command: "echo hi" });
  });

  it("往返不丢字段：不认识的 timeout 原样保留", () => {
    const rebuilt = buildHooks(flattenHooks(source));
    expect(rebuilt).toEqual(source);
  });

  it("同事件同匹配器的命令合并进一个条目", () => {
    const rows: HookRow[] = [
      { id: "1", event: "Stop", matcher: "", command: "a", extra: {}, entryExtra: {} },
      { id: "2", event: "Stop", matcher: "", command: "b", extra: {}, entryExtra: {} },
      { id: "3", event: "Stop", matcher: "X", command: "c", extra: {}, entryExtra: {} },
    ];
    expect(buildHooks(rows)).toEqual({
      Stop: [
        { hooks: [{ type: "command", command: "a" }, { type: "command", command: "b" }] },
        { matcher: "X", hooks: [{ type: "command", command: "c" }] },
      ],
    });
  });

  it("空事件或空命令的行不落盘", () => {
    const rows: HookRow[] = [
      { id: "1", event: "", matcher: "", command: "a", extra: {}, entryExtra: {} },
      { id: "2", event: "Stop", matcher: "", command: "   ", extra: {}, entryExtra: {} },
    ];
    expect(buildHooks(rows)).toEqual({});
    expect(hookRowError(rows[0])).toContain("事件名");
    expect(hookRowError(rows[1])).toContain("命令");
  });

  it("结构异常的来源不会炸，只是解析不出行", () => {
    expect(flattenHooks(null)).toEqual([]);
    expect(flattenHooks({ Stop: "not-an-array" } as never)).toEqual([]);
    expect(flattenHooks({ Stop: [{ hooks: [{ type: "other", command: "x" }] }] })).toEqual([]);
  });
});
