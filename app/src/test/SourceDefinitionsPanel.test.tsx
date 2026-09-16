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
    renderWithEngine(<SourceDefinitionsPanel domain="rules" />, {
      "sources:get": makeEnvelope({}, { ok: false, errors: [{ code: "CONFIG_INVALID", element: null, message: "来源文件不可用" }] }),
    });
    expect(await screen.findByText("来源文件不可用")).toBeInTheDocument();
  });
});

describe("Hooks JSON 来源", () => {
  it("编辑保存 hooks.claude 只写 sources:set，不触发 hooks:apply、不取 plan、零 policy:set", async () => {
    const { calls } = renderWithEngine(<SourceDefinitionsPanel domain="hooks" />, {
      "sources:get": makeEnvelope({ sources: [
        { key: "hooks.claude", label: "Claude Hooks", value: { hooks: {}, removed_signatures: [] }, revision: "rev-c", format: "json" },
        { key: "hooks.codex", label: "Codex Hooks", value: { hooks: {} }, revision: "rev-x", format: "json" },
      ] }),
      "sources:set": makeEnvelope({}),
    });
    expect(await screen.findByRole("button", { name: "编辑 Claude Hooks" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "编辑 Codex Hooks" })).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "编辑 Claude Hooks" }));
    fireEvent.change(screen.getByLabelText("定义内容"), { target: { value: '{"hooks":{},"removed_signatures":[]}' } });
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(calls.some((item) => item.command === "sources:set")).toBe(true));
    const setCall = calls.find((item) => item.command === "sources:set")!;
    expect(setCall.args).toEqual(["--key", "hooks.claude", "--value", JSON.stringify({ revision: "rev-c", value: { hooks: {}, removed_signatures: [] } })]);
    expect(calls.some((item) => item.command === "hooks:apply")).toBe(false);
    expect(calls.some((item) => item.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((item) => item.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog", { name: /将要执行的操作/ })).toBeNull();
  });
});

describe("输入保真", () => {
  it("定义编辑框禁用自动拼写、纠正与大小写替换，保护 rules 文本与 JSON 内容", async () => {
    renderWithEngine(<SourceDefinitionsPanel domain="hooks" />, {
      "sources:get": makeEnvelope({ sources: [{ key: "hooks.claude", label: "Claude Hooks", value: { hooks: {} }, revision: "r1", format: "json" }] }),
    });
    await userEvent.click(await screen.findByRole("button", { name: "编辑 Claude Hooks" }));
    const field = screen.getByLabelText("定义内容");
    expect(field).toHaveAttribute("spellcheck", "false");
    expect(field).toHaveAttribute("autocorrect", "off");
    expect(field).toHaveAttribute("autocapitalize", "off");
  });
});

describe("域专属说明文案", () => {
  it("rules 页显示共享/专用说明，hooks 页不出现", async () => {
    renderWithEngine(<SourceDefinitionsPanel domain="rules" />, {
      "sources:get": makeEnvelope({ sources: [{ key: "rules.shared", label: "共享规则", value: "x\n", revision: "r1", format: "text" }] }),
    });
    expect(await screen.findByText(/规则按共享内容和助手专用内容分别编辑/)).toBeInTheDocument();
  });

  it("hooks 页不出现 rules 专属说明", async () => {
    renderWithEngine(<SourceDefinitionsPanel domain="hooks" />, {
      "sources:get": makeEnvelope({ sources: [{ key: "hooks.claude", label: "Claude Hooks", value: { hooks: {} }, revision: "r1", format: "json" }] }),
    });
    expect(await screen.findByRole("button", { name: "编辑 Claude Hooks" })).toBeInTheDocument();
    expect(screen.queryByText(/规则按共享内容/)).not.toBeInTheDocument();
  });
});
