import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ImportLinkDialog } from "../components/ImportLinkDialog";
import { encodeShareLink } from "../lib/shareLink";

function riskOf(link: string): string {
  render(<ImportLinkDialog initialText={link} onContinue={vi.fn()} onClose={vi.fn()} />);
  return screen.getByTestId("import-link-risk").textContent ?? "";
}

describe("分享链接导入的风险说明", () => {
  it("远程 MCP：说清楚内容和请求头发往哪个主机", () => {
    const text = riskOf(encodeShareLink({ kind: "mcp", name: "r", spec: { url: "https://mcp.acme.example/mcp", headers: { Authorization: "${ACME_TOKEN}" } } }));
    expect(text).toContain("和这些请求头发往 mcp.acme.example");
    expect(text).toContain("只导入你信任的人发来的链接");
  });

  it("技能来源：说清楚从哪下载、会被当作指令", () => {
    const text = riskOf(encodeShareLink({ kind: "skills", name: "s", definition: { url: "https://github.com/acme/skills.git" } }));
    expect(text).toContain("从 github.com 下载");
    expect(text).toContain("当作指令执行");
  });
});
