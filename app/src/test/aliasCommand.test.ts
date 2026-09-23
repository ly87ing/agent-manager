import { describe, expect, it } from "vitest";
import {
  ALIAS_NAME_RE,
  aliasTextError,
  buildAliasValue,
  listTimeZones,
  parseAliasValue,
  renderAliasPreview,
} from "../lib/aliasCommand";

describe("启动命令别名的解析与生成", () => {
  it("解析实际区块里的既有别名", () => {
    const parsed = parseAliasValue('TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" claude --dangerously-skip-permissions');
    expect(parsed.structured).toBe(true);
    expect(parsed.command).toBe("claude");
    expect(parsed.extraArgs).toBe("--dangerously-skip-permissions");
    expect(parsed.timezone).toBe("America/Los_Angeles");
    expect(parsed.lang).toBe("en_US.UTF-8");
    expect(parsed.otherEnv).toEqual({});
  });

  it("解析写死具体时区的别名并保留其它环境变量", () => {
    const parsed = parseAliasValue('HTTP_PROXY="http://127.0.0.1:7890" TZ="Asia/Shanghai" LANG="zh_CN.UTF-8" LC_ALL="zh_CN.UTF-8" codex');
    expect(parsed.structured).toBe(true);
    expect(parsed.timezone).toBe("Asia/Shanghai");
    expect(parsed.lang).toBe("zh_CN.UTF-8");
    expect(parsed.otherEnv).toEqual({ HTTP_PROXY: "http://127.0.0.1:7890" });
    expect(parsed.command).toBe("codex");
  });

  it("没有环境变量的纯命令也可结构化", () => {
    const parsed = parseAliasValue("ll -la");
    expect(parsed.structured).toBe(true);
    expect(parsed.command).toBe("ll");
    expect(parsed.extraArgs).toBe("-la");
    expect(parsed.timezone).toBeNull();
    expect(parsed.lang).toBeNull();
  });

  it("LANG 与 LC_ALL 不一致时不做结构化归并", () => {
    const parsed = parseAliasValue('LANG="en_US.UTF-8" LC_ALL="zh_CN.UTF-8" claude');
    expect(parsed.structured).toBe(false);
  });

  it("环境变量写在参数里的包装命令不做结构化", () => {
    const parsed = parseAliasValue('open --env TZ="America/Los_Angeles" -n -a "Google Chrome"');
    expect(parsed.structured).toBe(false);
  });

  it("空值不可结构化", () => {
    expect(parseAliasValue("   ").structured).toBe(false);
  });

  it("生成的值把环境变量放在命令之前且用双引号包裹", () => {
    const value = buildAliasValue({
      command: "claude",
      extraArgs: "--dangerously-skip-permissions",
      timezone: "Asia/Shanghai",
      lang: "zh_CN.UTF-8",
    });
    expect(value).toBe('TZ="Asia/Shanghai" LANG="zh_CN.UTF-8" LC_ALL="zh_CN.UTF-8" claude --dangerously-skip-permissions');
    // 生成结果必须能被自己解析回去（往返稳定）
    expect(parseAliasValue(value).command).toBe("claude");
  });

  it("不设时区与语言时只留命令", () => {
    expect(buildAliasValue({ command: "codex", timezone: null, lang: null })).toBe("codex");
  });

  it("指定具体时区时写字面量", () => {
    expect(buildAliasValue({ command: "codex", timezone: "Asia/Tokyo", lang: null })).toBe('TZ="Asia/Tokyo" codex');
  });

  it("预览就是写进 shell 配置的那一行（取值原样，没有占位替换）", () => {
    const preview = renderAliasPreview("claude", 'TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" claude');
    expect(preview).toBe(`alias claude='TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" claude'`);
  });

  it("取值校验与引擎一致：拒绝单引号与换行", () => {
    expect(aliasTextError("claude --ok")).toBeNull();
    expect(aliasTextError("claude 'x'")).toBe("不能包含单引号");
    expect(aliasTextError("claude\nx")).toBe("不能包含换行");
  });

  it("别名名称规则与引擎一致", () => {
    expect(ALIAS_NAME_RE.test("claude")).toBe(true);
    expect(ALIAS_NAME_RE.test("my-cmd")).toBe(true);
    expect(ALIAS_NAME_RE.test("-bad")).toBe(false);
    expect(ALIAS_NAME_RE.test("bad name")).toBe(false);
  });

  it("时区列表把常用时区排在前面", () => {
    const zones = listTimeZones();
    expect(zones.length).toBeGreaterThan(5);
    expect(zones).toContain("Asia/Shanghai");
    expect(zones).toContain("UTC");
    expect(zones.indexOf("Asia/Shanghai")).toBeLessThan(10);
    // 不重复
    expect(new Set(zones).size).toBe(zones.length);
  });
});
