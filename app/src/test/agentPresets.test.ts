import { describe, expect, it } from "vitest";
import { hasPresetArgs, presetsForCommand, togglePresetArgs } from "../lib/agentPresets";

describe("启动参数预设", () => {
  it("按命令名给出预设，未知命令没有", () => {
    expect(presetsForCommand("claude").map((p) => p.args)).toEqual(["--dangerously-skip-permissions"]);
    expect(presetsForCommand("codex")).toHaveLength(2);
    expect(presetsForCommand("~/.local/bin/codex")).toHaveLength(2);
    expect(presetsForCommand("some-tool")).toEqual([]);
  });

  it("全自动预设都标为危险，保留沙箱的那个不标", () => {
    expect(presetsForCommand("claude")[0].danger).toBe(true);
    const codex = presetsForCommand("codex");
    expect(codex.find((p) => p.id === "codex-yolo")?.danger).toBe(true);
    expect(codex.find((p) => p.id === "codex-auto-sandboxed")?.danger).toBe(false);
  });

  it("切换预设：没有就追加，有了就整段移除", () => {
    const preset = presetsForCommand("claude")[0];
    expect(togglePresetArgs("", preset)).toBe("--dangerously-skip-permissions");
    expect(togglePresetArgs("--verbose", preset)).toBe("--verbose --dangerously-skip-permissions");
    expect(togglePresetArgs("--verbose --dangerously-skip-permissions", preset)).toBe("--verbose");
  });

  it("多段参数的预设整体判定与整体移除", () => {
    const preset = presetsForCommand("codex").find((p) => p.id === "codex-auto-sandboxed")!;
    const added = togglePresetArgs("", preset);
    expect(added).toBe("--ask-for-approval never --sandbox workspace-write");
    expect(hasPresetArgs(added, preset)).toBe(true);
    // 只有一半参数时不算已启用
    expect(hasPresetArgs("--ask-for-approval never", preset)).toBe(false);
    expect(togglePresetArgs(added, preset)).toBe("");
  });
});
