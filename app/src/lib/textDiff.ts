/**
 * 逐行文本差异（纯函数），用于应用规则模板/推荐/复制前展示「现有内容 → 将写入内容」。
 *
 * 用最长公共子序列（LCS）对齐行；两边行数乘积过大时不做对齐，退化为整段删除 + 整段新增，
 * 避免在超大文件上卡住界面。
 */

export type DiffLineKind = "same" | "add" | "del";

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
}

/** 超过这个规模（行数乘积）不做逐行对齐。 */
export const MAX_DIFF_CELLS = 4_000_000;

function splitLines(text: string): string[] {
  if (text === "") return [];
  const lines = text.split("\n");
  // 末尾换行不产生一条空行
  if (lines[lines.length - 1] === "") lines.pop();
  return lines;
}

export function diffLines(before: string, after: string): DiffLine[] {
  const a = splitLines(before);
  const b = splitLines(after);
  if (a.length * b.length > MAX_DIFF_CELLS) {
    return [
      ...a.map((text) => ({ kind: "del" as const, text })),
      ...b.map((text) => ({ kind: "add" as const, text })),
    ];
  }
  const n = a.length;
  const m = b.length;
  // lcs[i][j] = a[i..] 与 b[j..] 的最长公共子序列长度
  const lcs: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i -= 1) {
    for (let j = m - 1; j >= 0; j -= 1) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: "same", text: a[i] });
      i += 1;
      j += 1;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      out.push({ kind: "del", text: a[i] });
      i += 1;
    } else {
      out.push({ kind: "add", text: b[j] });
      j += 1;
    }
  }
  while (i < n) out.push({ kind: "del", text: a[i++] });
  while (j < m) out.push({ kind: "add", text: b[j++] });
  return out;
}

/** 差异里有没有真正的改动。 */
export function hasChanges(lines: DiffLine[]): boolean {
  return lines.some((line) => line.kind !== "same");
}
