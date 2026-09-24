import { describe, expect, it } from "vitest";
import { balanceSupported, formatBalance } from "../lib/balance";

describe("查余额", () => {
  it("只认厂商 API 文档写明了余额接口的主机", () => {
    expect(balanceSupported("https://api.deepseek.com/anthropic")).toBe(true);
    expect(balanceSupported("https://api.moonshot.cn/anthropic")).toBe(true);
    expect(balanceSupported("https://api.moonshot.ai/v1")).toBe(true);
    expect(balanceSupported("https://open.bigmodel.cn/api/anthropic")).toBe(false);
    expect(balanceSupported("https://api.deepseek.com.evil.example")).toBe(false);
    expect(balanceSupported("")).toBe(false);
    expect(balanceSupported("not a url")).toBe(false);
  });

  it("结果排成一行", () => {
    expect(formatBalance([{ label: "余额", amount: "110.00", currency: "CNY" }])).toBe("余额 110.00 CNY");
    expect(
      formatBalance([
        { label: "可用余额", amount: "49.58", currency: null },
        { label: "代金券", amount: "46.58", currency: null },
      ]),
    ).toBe("可用余额 49.58 · 代金券 46.58");
  });
});
