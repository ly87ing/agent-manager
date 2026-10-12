import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ChangedFile, FileDiff } from "../components/FileDiff";

/**
 * 回归：预览把几千行的 .claude.json 整份铺开，要拖很久才找到改的地方（2026-09-30 真机截图）。
 * 默认只显示变化与前后几行，其余折叠；完整文件收起，需要时再展开。
 */
describe("FileDiff", () => {
  const before = JSON.stringify(
    { numStartups: 208, tipsHistory: Object.fromEntries(Array.from({ length: 40 }, (_, i) => [`tip-${i}`, i])), mcpServers: {} },
    null,
    2,
  );
  const after = JSON.stringify(
    {
      numStartups: 208,
      tipsHistory: Object.fromEntries(Array.from({ length: 40 }, (_, i) => [`tip-${i}`, i])),
      mcpServers: { example: { command: "npx", args: ["-y", "example-mcp"] } },
    },
    null,
    2,
  );

  it("只显示变化的部分：无关的键折叠，标题给出路径、更新与增删行数", () => {
    render(<FileDiff path="/Users/x/.claude.json" original={before} content={after} testId="d" />);
    const box = screen.getByTestId("d");
    expect(box).toHaveTextContent("/Users/x/.claude.json");
    expect(box).toHaveTextContent("更新");
    expect(screen.getByTestId("d-stats")).toHaveTextContent("+9 −1");
    expect(box).toHaveTextContent('"example-mcp"');
    expect(box).not.toHaveTextContent('"tip-5"');
    expect(screen.getByTestId("d-fold")).toHaveTextContent(/未改动 \d+ 行/);
    expect(screen.queryByTestId("d-full")).toBeNull();
  });

  it("点折叠行展开；完整文件按需查看；新文件整份算新增；内容不变时如实说", async () => {
    const { rerender } = render(<FileDiff original={before} content={after} testId="d" />);
    await userEvent.click(screen.getByTestId("d-fold"));
    expect(screen.getByTestId("d")).toHaveTextContent('"tip-5"');
    await userEvent.click(screen.getByTestId("d-full-toggle"));
    expect(screen.getByTestId("d-full")).toHaveTextContent('"numStartups"');
    rerender(<FileDiff original={null} content={"a\nb"} testId="n" />);
    expect(screen.getByTestId("n")).toHaveTextContent("新建");
    expect(screen.getByTestId("n-stats")).toHaveTextContent("+2 −0");
    rerender(<FileDiff original={"same"} content={"same"} testId="s" />);
    expect(screen.getByTestId("s")).toHaveTextContent("内容不变");
  });
});

describe("ChangedFile", () => {
  it("点开只看这个文件改了什么；太大或二进制的只给文件名", async () => {
    render(
      <ul>
        <ChangedFile testId="f1" label="修改" change={{ status: "modified", file: "SKILL.md", original: "a\nold\nc", content: "a\nnew\nc", contents: true }} />
        <ChangedFile testId="f2" label="修改" change={{ status: "modified", file: "logo.png", contents: false }} />
      </ul>,
    );
    await userEvent.click(screen.getByRole("button", { name: /SKILL\.md/ }));
    expect(screen.getByTestId("f1")).toHaveTextContent("old");
    expect(screen.getByTestId("f1")).toHaveTextContent("new");
    expect(screen.getByRole("button", { name: /logo\.png/ })).toBeDisabled();
    expect(screen.getByTestId("f2")).toHaveTextContent("未显示内容");
  });
});
