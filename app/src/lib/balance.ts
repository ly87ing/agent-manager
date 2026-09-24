/**
 * 查余额：只有厂商 API 文档写明了余额接口的才支持（与引擎 `agents:balance` 的名单一致）。
 * DeepSeek「查询余额」、Moonshot（Kimi 开放平台）「查询余额」；套餐额度一类没有公开文档的接口不收。
 */
const BALANCE_HOSTS = ["api.deepseek.com", "api.moonshot.cn", "api.moonshot.ai"];

export function balanceSupported(baseUrl: string | null | undefined): boolean {
  if (!baseUrl) return false;
  try {
    return BALANCE_HOSTS.includes(new URL(baseUrl.trim()).hostname.toLowerCase());
  } catch {
    return false;
  }
}

export interface BalanceItem {
  label: string;
  amount: string;
  currency: string | null;
}

/** 余额结果 → 一行文字：「余额 110.00 CNY」「可用余额 49.58 · 代金券 46.58 · 现金余额 3」。 */
export function formatBalance(items: BalanceItem[]): string {
  return items.map((item) => `${item.label} ${item.amount}${item.currency ? ` ${item.currency}` : ""}`).join(" · ");
}
