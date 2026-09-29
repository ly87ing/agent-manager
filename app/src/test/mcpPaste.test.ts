import { describe, expect, it } from "vitest";
import { parsePastedServer } from "../lib/mcpPaste";

describe("粘贴 MCP 配置", () => {
  it("认 mcpServers 包着的写法，取名字、命令、参数和环境变量", () => {
    const result = parsePastedServer('{"mcpServers": {"fs": {"command": "npx", "args": ["-y", "fs-mcp"], "env": {"ROOT": "/tmp"}}}}');
    expect(result).toEqual({ name: "fs", transport: "stdio", command: "npx", args: ["-y", "fs-mcp"], env: { ROOT: "/tmp" }, url: "", headers: {}, notes: [] });
  });

  it("认没有外层大括号的片段和只有服务本身的对象", () => {
    expect(parsePastedServer('"fs": {"command": "npx", "args": ["fs"]},')).toMatchObject({ name: "fs", command: "npx", args: ["fs"] });
    expect(parsePastedServer('{"command": "uvx", "args": ["x"]}')).toMatchObject({ name: null, command: "uvx" });
  });

  it("远程服务：请求头的明文取值不填进来，改成同名变量并说明", () => {
    const result = parsePastedServer('{"api": {"type": "http", "url": "https://mcp.example.com", "headers": {"Authorization": "Bearer abc", "X-Key": "${KEY}"}}}');
    expect(result).toMatchObject({ name: "api", transport: "remote", url: "https://mcp.example.com", headers: { Authorization: "${AUTHORIZATION}", "X-Key": "${KEY}" } });
    expect(typeof result !== "string" && result.notes.join()).toContain("Authorization");
    expect(JSON.stringify(result)).not.toContain("Bearer abc");
  });

  it("本地命令：名字像密钥的明文取值改成同名变量，其余照填", () => {
    const result = parsePastedServer('{"gh": {"command": "npx", "env": {"GITHUB_TOKEN": "ghp_x", "MODE": "fast", "API_KEY": "${API_KEY}"}}}');
    expect(result).toMatchObject({ env: { GITHUB_TOKEN: "${GITHUB_TOKEN}", MODE: "fast", API_KEY: "${API_KEY}" } });
    expect(JSON.stringify(result)).not.toContain("ghp_x");
  });

  it("多个服务只取第一个并说明；读不出或没有服务时给出原因", () => {
    const result = parsePastedServer('{"mcpServers": {"a": {"command": "x"}, "b": {"command": "y"}}}');
    expect(result).toMatchObject({ name: "a" });
    expect(typeof result !== "string" && result.notes[0]).toContain("只填了第一个 a");
    expect(parsePastedServer("not json")).toContain("读不出来");
    expect(parsePastedServer('{"a": 1}')).toContain("没找到服务");
  });
});
