import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import { SourceDefinitionsPanel } from "../components/SourceDefinitionsPanel";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

describe("来源编辑", () => {
  it("增改删 JSON 条目后带版本保存，不触发任何 domain:apply、不取 plan", async () => {
    const { calls } = renderWithEngine(<SourceDefinitionsPanel domain="aliases" />, {
      "sources:get": makeEnvelope({ sources: [{ key: "aliases.shell", label: "Shell aliases", value: { old: "echo old" }, revision: "revision-1", format: "json" }] }),
      "sources:set": makeEnvelope({}),
    });
    await userEvent.click(await screen.findByRole("button", { name: "编辑 Shell aliases" }));
    fireEvent.change(screen.getByLabelText("定义内容"), { target: { value: '{"new":"echo new"}' } });
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(calls.some((item) => item.command === "sources:set")).toBe(true));
    const call = calls.find((item) => item.command === "sources:set")!;
    expect(call.args).toEqual(["--key", "aliases.shell", "--value", JSON.stringify({ revision: "revision-1", value: { new: "echo new" } })]);
    expect(calls.some((item) => item.command === "aliases:apply")).toBe(false);
    expect(calls.some((item) => item.command === "rules:apply")).toBe(false);
    expect(calls.some((item) => item.command === "hooks:apply")).toBe(false);
    expect(calls.some((item) => item.command.endsWith(":plan"))).toBe(false);
    expect(screen.queryByRole("dialog", { name: /将要执行的操作/ })).toBeNull();
  });

  it("加载失败展示错误，不把空白页面当作成功", async () => {
    renderWithEngine(<SourceDefinitionsPanel domain="aliases" />, {
      "sources:get": makeEnvelope({}, { ok: false, errors: [{ code: "CONFIG_INVALID", element: null, message: "来源文件不可用" }] }),
    });
    expect(await screen.findByText("来源文件不可用")).toBeInTheDocument();
  });
});

describe("输入保真", () => {
  it("定义编辑框禁用自动拼写、纠正与大小写替换，保护 JSON 内容", async () => {
    renderWithEngine(<SourceDefinitionsPanel domain="aliases" />, {
      "sources:get": makeEnvelope({ sources: [{ key: "aliases.shell", label: "Shell aliases", value: { aliases: {} }, revision: "r1", format: "json" }] }),
    });
    await userEvent.click(await screen.findByRole("button", { name: "编辑 Shell aliases" }));
    const field = screen.getByLabelText("定义内容");
    expect(field).toHaveAttribute("spellcheck", "false");
    expect(field).toHaveAttribute("autocorrect", "off");
    expect(field).toHaveAttribute("autocapitalize", "off");
  });
});

describe("域专属说明文案", () => {
  it("来源编辑不出现已退役的规则拼接说明", async () => {
    renderWithEngine(<SourceDefinitionsPanel domain="aliases" />, {
      "sources:get": makeEnvelope({ sources: [{ key: "aliases.shell", label: "Shell aliases", value: { aliases: {} }, revision: "r1", format: "json" }] }),
    });
    expect(await screen.findByRole("button", { name: "编辑 Shell aliases" })).toBeInTheDocument();
    expect(screen.queryByText(/规则按共享内容/)).not.toBeInTheDocument();
  });
});
