/** GUI 发起的引擎运行的 run id：可读前缀 + UTC 时间戳 + 随机尾（字符集对齐 bridge 校验）。 */
export function newRunId(): string {
  const stamp = new Date()
    .toISOString()
    .replace(/[-:]/g, "")
    .replace(/\..+$/, "");
  const rand = Math.random().toString(16).slice(2, 8).padEnd(6, "0");
  return `gui-${stamp}Z-${rand}`;
}
