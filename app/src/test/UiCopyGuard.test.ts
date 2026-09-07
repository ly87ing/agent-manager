import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * 界面文案防回归：文案层（components/pages/lib）剥离注释后不得再出现工程黑话。
 * 工程原值只许出现在 tooltip 对照与引擎契约文本（code/note/枚举值）里；
 * 引擎返回的 message/note 属契约内容、原样展示，不在本扫描范围（那是运行时数据，不是文案层）。
 */

const SRC_ROOT = join(process.cwd(), "src");
const SCAN_DIRS = ["components", "pages", "lib"];

/** 禁用词（新文案见 lib/ui-copy.ts 与各徽章映射表）。 */
const BANNED = ["收敛", "漂移", "物化", "上游", "托管", "受管", "收件箱", "死链", "快照", "回滚", "策略", "探测", "占位符"];
const BANNED_PATTERNS: [string, RegExp][] = [
  ...BANNED.map((word) => [word, new RegExp(word, "g")] as [string, RegExp]),
  // dry-run 作为面向用户的文案禁用；引擎 CLI flag "--dry-run" 是机器接口，豁免。
  ["dry-run", /(?<![\w-])dry-run/g],
];

function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, "") // 块注释（含 JSDoc 与 JSX 注释）
    .replace(/^\s*\/\/.*$/gm, ""); // 整行行注释（字符串里的 https:// 等不受影响）
}

function collectFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      collectFiles(path, out);
    } else if (/\.(ts|tsx)$/.test(entry)) {
      out.push(path);
    }
  }
  return out;
}

describe("界面文案防回归", () => {
  const files = SCAN_DIRS.flatMap((dir) => collectFiles(join(SRC_ROOT, dir)));
  expect(files.length).toBeGreaterThan(10);

  for (const file of files) {
    const rel = file.replace(SRC_ROOT, "src/");
    it(`${rel} 不含工程黑话`, () => {
      const text = stripComments(readFileSync(file, "utf8"));
      const hits: string[] = [];
      for (const [word, pattern] of BANNED_PATTERNS) {
        const count = (text.match(pattern) ?? []).length;
        if (count > 0) hits.push(`${word}×${count}`);
      }
      expect(hits, `${rel} 残留文案黑话: ${hits.join(", ")}`).toEqual([]);
    });
  }
});
