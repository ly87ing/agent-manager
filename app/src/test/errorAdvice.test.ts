import { describe, expect, it } from "vitest";
import { adviseError, errorSummary } from "../lib/errorAdvice";

describe("出错说人话", () => {
  it("常见原因翻成「出了什么事 / 怎么办」", () => {
    expect(adviseError("~/.zshrc 已在外部修改（与打开时读到的内容不同）")?.what).toContain("被别处改过");
    expect(adviseError("目标路径是符号链接，拒绝操作以保证安全")?.what).toContain("链接");
    expect(adviseError("fatal: could not resolve host: github.com")?.what).toContain("连不上");
    expect(adviseError("配置文件解析失败: TOML 语法错误")?.todo).toContain("修好格式");
    expect(adviseError("Permission denied (os error 13)")?.what).toContain("没有权限");
    expect(adviseError("git@github.com: Permission denied (publickey).")?.what).toContain("仓库拉取被拒绝");
  });

  it("认不出的照原文，不硬翻", () => {
    expect(adviseError("SQLite busy")).toBeNull();
    expect(adviseError("secrets.env 不存在")).toBeNull();
    expect(adviseError("--value 不是合法 JSON")).toBeNull();
    expect(errorSummary("SQLite busy")).toEqual({ text: "SQLite busy" });
    expect(errorSummary("目标被外部修改，拒绝覆盖").description).toMatch(/^怎么办：/);
  });
});
