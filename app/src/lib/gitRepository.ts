/** 常用 Git 远程地址；本机路径由来源表单单独处理，不进入分享链接。 */
export function isRemoteGitRepository(value: string): boolean {
  if (!value || /[\s\x00-\x1f\x7f]/.test(value) || value.startsWith("-")) return false;
  if (/^(https?|ssh|git):\/\//.test(value)) {
    try {
      const url = new URL(value);
      return Boolean(url.hostname && url.pathname.length > 1);
    } catch {
      return false;
    }
  }
  // SCP 写法：账号可省略，主机可用 SSH alias；排除 Windows 盘符和 remote helper。
  return !/^[A-Za-z]:/.test(value) && /^(?:[\w.-]+@)?[\w.-]+:(?!:|\/\/).+/.test(value);
}
