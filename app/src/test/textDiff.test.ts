import { describe, expect, it } from "vitest";
import { MAX_DIFF_CELLS, diffLines, hasChanges } from "../lib/textDiff";

describe("逐行差异", () => {
  it("对齐公共行，只标出删掉与新增的行", () => {
    const lines = diffLines("a\nb\nc\n", "a\nx\nc\n");
    expect(lines).toEqual([
      { kind: "same", text: "a" },
      { kind: "del", text: "b" },
      { kind: "add", text: "x" },
      { kind: "same", text: "c" },
    ]);
    expect(hasChanges(lines)).toBe(true);
  });

  it("相同内容没有改动；末尾换行不产生空行", () => {
    const lines = diffLines("one\ntwo\n", "one\ntwo\n");
    expect(lines).toEqual([
      { kind: "same", text: "one" },
      { kind: "same", text: "two" },
    ]);
    expect(hasChanges(lines)).toBe(false);
  });

  it("从空到有内容全部是新增，从有内容到空全部是删除", () => {
    expect(diffLines("", "x\ny\n")).toEqual([
      { kind: "add", text: "x" },
      { kind: "add", text: "y" },
    ]);
    expect(diffLines("x\n", "")).toEqual([{ kind: "del", text: "x" }]);
    expect(diffLines("", "")).toEqual([]);
  });

  it("插入中间行时保留两侧公共行", () => {
    expect(diffLines("a\nc\n", "a\nb\nc\n")).toEqual([
      { kind: "same", text: "a" },
      { kind: "add", text: "b" },
      { kind: "same", text: "c" },
    ]);
  });

  it("超大输入不做逐行对齐，退化为整段删除加整段新增", () => {
    const side = Math.ceil(Math.sqrt(MAX_DIFF_CELLS)) + 1;
    const before = Array.from({ length: side }, (_, i) => `l${i}`).join("\n");
    const after = Array.from({ length: side }, (_, i) => (i === 0 ? "changed" : `l${i}`)).join("\n");
    const lines = diffLines(before, after);
    expect(lines).toHaveLength(side * 2);
    expect(lines.slice(0, side).every((line) => line.kind === "del")).toBe(true);
    expect(lines.slice(side).every((line) => line.kind === "add")).toBe(true);
  });
});
