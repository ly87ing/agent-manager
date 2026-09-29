import { describe, expect, it } from "vitest";
import { decodeShareLink, encodeShareLink, SHARE_PREFIX } from "../lib/shareLink";

describe("分享链接", () => {
  it("MCP 与技能集合编码后原样解回（含中文与特殊字符）", () => {
    const mcp = { kind: "mcp" as const, name: "grok-search", spec: { command: "uvx", args: ["--from", "git+https://example.com/x", "搜索"], env: { API_KEY: "${GROK_API_KEY}" } } };
    expect(decodeShareLink(encodeShareLink(mcp))).toEqual(mcp);
    const skills = { kind: "skills" as const, name: "agent-skills", definition: { url: "https://github.com/example/agent-skills.git", skill_paths: ["skills"], rules_path: null } };
    expect(decodeShareLink(encodeShareLink(skills))).toEqual(skills);
  });

  it("带明文取值的拒收：环境变量和 header 只接受 ${变量}", () => {
    const leak = SHARE_PREFIX + btoa(JSON.stringify({ v: 1, kind: "mcp", name: "x", spec: { command: "npx", env: { TOKEN: "sk-SECRET" } } }));
    expect(decodeShareLink(leak)).toMatch(/明文取值/);
    expect(() => encodeShareLink({ kind: "mcp", name: "x", spec: { url: "https://a.example/mcp", headers: { Authorization: "Bearer abc" } } })).toThrow(/明文取值/);
  });

  it("技能集合不收本机路径，也不收越出集合的位置", () => {
    const local = SHARE_PREFIX + btoa(JSON.stringify({ v: 1, kind: "skills", name: "x", definition: { url: "~/my-skills" } }));
    expect(decodeShareLink(local)).toMatch(/只接受 git 地址/);
    const escape = SHARE_PREFIX + btoa(JSON.stringify({ v: 1, kind: "skills", name: "x", definition: { url: "https://e.x/r.git", skill_paths: ["../../etc"] } }));
    expect(decodeShareLink(escape)).toMatch(/技能位置/);
  });

  it("不是本工具的链接、内容残缺、名称非法都给出原因，不抛异常", () => {
    expect(decodeShareLink("https://example.com")).toMatch(/不是 Agent Manager/);
    expect(decodeShareLink(SHARE_PREFIX + "%%%")).toMatch(/读不出来/);
    const badName = SHARE_PREFIX + btoa(JSON.stringify({ v: 1, kind: "mcp", name: "../x", spec: { command: "npx" } }));
    expect(decodeShareLink(badName)).toMatch(/名称/);
  });
});
