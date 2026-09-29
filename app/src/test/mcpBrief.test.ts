import { describe, expect, it } from "vitest";
import { mcpBrief } from "../components/McpAssignmentRow";
import type { McpAgentCell } from "../lib/observed";

const stdio = (command: string, args: string[]) => ({ transport: "stdio", entry: { command, args } }) as unknown as McpAgentCell;

describe("MCP 一句话说明", () => {
  it("npx / uvx 取包名，git 来源标出来", () => {
    expect(mcpBrief(stdio("npx", ["-y", "chrome-devtools-mcp@1.9.0", "--isolated"]))).toBe("本地命令 · chrome-devtools-mcp@1.9.0");
    expect(mcpBrief(stdio("uvx", ["--from", "git+https://github.com/example/grok-search", "grok-search"]))).toBe("本地命令 · grok-search（git）");
  });

  it("其他命令取可执行文件名，不带路径", () => {
    expect(mcpBrief(stdio("/Applications/ChatGPT.app/Contents/Resources/node_repl", []))).toBe("本地命令 · node_repl");
  });

  it("远程只留域名", () => {
    expect(mcpBrief({ transport: "remote", entry: { url: "https://developers.openai.com/mcp" } } as unknown as McpAgentCell)).toBe("远程 · developers.openai.com");
  });
});
