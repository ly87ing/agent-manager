import { describe, expect, it } from "vitest";
import { MAX_DIFF_CELLS, diffLines, diffStats, foldDiff, hasChanges } from "../lib/textDiff";

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

  it("中间不同的部分超大时不做逐行对齐，退化为整段删除加整段新增", () => {
    const side = Math.ceil(Math.sqrt(MAX_DIFF_CELLS)) + 1;
    const before = Array.from({ length: side }, (_, i) => `a${i}`).join("\n");
    const after = Array.from({ length: side }, (_, i) => `b${i}`).join("\n");
    const lines = diffLines(before, after);
    expect(lines).toHaveLength(side * 2);
    expect(lines.slice(0, side).every((line) => line.kind === "del")).toBe(true);
    expect(lines.slice(side).every((line) => line.kind === "add")).toBe(true);
  });

  it("大文件里只改一小块：去掉首尾相同的行后精确算出（如几千行的 .claude.json 加一个 MCP 服务）", () => {
    const side = Math.ceil(Math.sqrt(MAX_DIFF_CELLS)) + 1;
    const before = Array.from({ length: side }, (_, i) => `l${i}`).join("\n");
    const after = Array.from({ length: side }, (_, i) => (i === 0 ? "changed" : `l${i}`)).join("\n");
    const lines = diffLines(before, after);
    expect(lines.filter((line) => line.kind !== "same")).toEqual([
      { kind: "del", text: "l0" },
      { kind: "add", text: "changed" },
    ]);
    expect(lines).toHaveLength(side + 1);
  });
});

describe("折叠：只留变化与前后几行", () => {
  it("没变的长段折叠，行号按原文件 / 新文件各自计数，统计增删行数", () => {
    const before = Array.from({ length: 30 }, (_, i) => `l${i + 1}`).join("\n");
    const after = before.replace("l15", "L15");
    const lines = diffLines(before, after);
    const segments = foldDiff(lines, 3);
    expect(segments.map((s) => [s.type, s.lines.length])).toEqual([
      ["fold", 11],
      ["show", 8],
      ["fold", 12],
    ]);
    const shown = segments[1].lines;
    expect(shown[3]).toMatchObject({ kind: "del", text: "l15", oldNo: 15, newNo: null });
    expect(shown[4]).toMatchObject({ kind: "add", text: "L15", oldNo: null, newNo: 15 });
    expect(diffStats(lines)).toEqual({ added: 1, removed: 1 });
  });

  it("只折叠一两行不划算，直接显示", () => {
    const lines = diffLines("a\nb\nc\nd\ne\nf\ng\nh\ni", "a\nb\nc\nd\nX\nf\ng\nh\ni");
    expect(foldDiff(lines, 3).every((s) => s.type === "show")).toBe(true);
  });
});
