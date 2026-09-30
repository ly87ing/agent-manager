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
  const all = { a: splitLines(before), b: splitLines(after) };
  // 先去掉首尾相同的行，只对中间变化的那段做逐行对齐：大文件里改一小块也能算准
  let head = 0;
  while (head < all.a.length && head < all.b.length && all.a[head] === all.b[head]) head += 1;
  let tail = 0;
  while (
    tail < all.a.length - head &&
    tail < all.b.length - head &&
    all.a[all.a.length - 1 - tail] === all.b[all.b.length - 1 - tail]
  ) {
    tail += 1;
  }
  const same = (text: string): DiffLine => ({ kind: "same", text });
  return [
    ...all.a.slice(0, head).map(same),
    ...diffMiddle(all.a.slice(head, all.a.length - tail), all.b.slice(head, all.b.length - tail)),
    ...all.a.slice(all.a.length - tail).map(same),
  ];
}

function diffMiddle(a: string[], b: string[]): DiffLine[] {
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

/** 带行号的一行：oldNo / newNo 是它在原文件 / 新文件里的行号（新增行没有 oldNo，删除行没有 newNo）。 */
export interface NumberedLine extends DiffLine {
  oldNo: number | null;
  newNo: number | null;
}

/** 折叠后的片段：show 照常显示；fold 是一段没变的行，默认收起。 */
export type DiffSegment = { type: "show"; lines: NumberedLine[] } | { type: "fold"; lines: NumberedLine[] };

/**
 * 只留变化的行及前后各 context 行，其余没变的连续行折叠起来（同 git diff / GitHub 的写法）。
 * 没有任何变化时整段折叠。
 */
export function foldDiff(lines: DiffLine[], context = 3): DiffSegment[] {
  let oldNo = 0;
  let newNo = 0;
  const numbered: NumberedLine[] = lines.map((line) => {
    if (line.kind !== "add") oldNo += 1;
    if (line.kind !== "del") newNo += 1;
    return { ...line, oldNo: line.kind === "add" ? null : oldNo, newNo: line.kind === "del" ? null : newNo };
  });
  // 每行离最近一处变化有多远，近的显示，远的折叠
  const keep = numbered.map(() => false);
  numbered.forEach((line, index) => {
    if (line.kind === "same") return;
    for (let k = Math.max(0, index - context); k <= Math.min(numbered.length - 1, index + context); k += 1) keep[k] = true;
  });
  const segments: DiffSegment[] = [];
  numbered.forEach((line, index) => {
    const type = keep[index] ? "show" : "fold";
    const last = segments[segments.length - 1];
    if (last && last.type === type) last.lines.push(line);
    else segments.push({ type, lines: [line] });
  });
  // 只折叠一两行不划算：直接显示
  return segments.map((segment) => (segment.type === "fold" && segment.lines.length <= 2 ? { type: "show", lines: segment.lines } : segment));
}

/** 新增与删除的行数。 */
export function diffStats(lines: DiffLine[]): { added: number; removed: number } {
  return {
    added: lines.filter((line) => line.kind === "add").length,
    removed: lines.filter((line) => line.kind === "del").length,
  };
}
