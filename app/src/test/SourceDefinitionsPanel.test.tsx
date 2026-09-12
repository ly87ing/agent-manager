import { describe, expect, it } from "vitest";
import { fireEvent } from "@testing-library/react";
import { SourceDefinitionsPanel } from "../components/SourceDefinitionsPanel";
import { makeEnvelope, makePlanAction } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

describe("来源编辑", () => {
  it("增改删 JSON 条目后带版本保存并预览同步", async () => {
    const { calls } = renderWithEngine(<SourceDefinitionsPanel domain="aliases" />, {
      "sources:get": makeEnvelope({ sources: [{ key: "aliases.shell", label: "Shell aliases", value: { old: "echo old" }, revision: "revision-1", format: "json" }] }),
      "sources:set": makeEnvelope({}),
      "aliases:plan": makeEnvelope({ domain: "aliases", actions: [makePlanAction()], summary: {} }),
    });
    await userEvent.click(await screen.findByRole("button", { name: "编辑 Shell aliases" }));
    fireEvent.change(screen.getByLabelText("定义内容"), { target: { value: '{"new":"echo new"}' } });
    await userEvent.click(screen.getByRole("button", { name: "保存并预览同步" }));
    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(dialog).not.toHaveTextContent("备份");
    expect(dialog).not.toHaveTextContent("生成");
    const call = calls.find((item) => item.command === "sources:set")!;
    expect(call.args).toEqual(["--key", "aliases.shell", "--value", JSON.stringify({ revision: "revision-1", value: { new: "echo new" } })]);
    expect(calls.some((item) => item.command === "aliases:apply")).toBe(false);
  });

  it("加载失败展示错误，不把空白页面当作成功", async () => {
    renderWithEngine(<SourceDefinitionsPanel domain="rules" />, {
      "sources:get": makeEnvelope({}, { ok: false, errors: [{ code: "CONFIG_INVALID", element: null, message: "来源文件不可用" }] }),
    });
    expect(await screen.findByText("来源文件不可用")).toBeInTheDocument();
  });
});

describe("Hooks JSON 来源", () => {
  it("编辑保存 hooks.claude 只写 sources:set 且不承诺备份/生成", async () => {
    const { calls } = renderWithEngine(<SourceDefinitionsPanel domain="hooks" />, {
      "sources:get": makeEnvelope({ sources: [
        { key: "hooks.claude", label: "Claude Hooks", value: { hooks: {}, removed_signatures: [] }, revision: "rev-c", format: "json" },
        { key: "hooks.codex", label: "Codex Hooks", value: { hooks: {} }, revision: "rev-x", format: "json" },
      ] }),
      "sources:set": makeEnvelope({}),
      "hooks:plan": makeEnvelope({ domain: "hooks", actions: [makePlanAction({ element: "hooks:claude" })], summary: {} }),
    });
    expect(await screen.findByRole("button", { name: "编辑 Claude Hooks" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "编辑 Codex Hooks" })).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "编辑 Claude Hooks" }));
    fireEvent.change(screen.getByLabelText("定义内容"), { target: { value: '{"hooks":{},"removed_signatures":[]}' } });
    await userEvent.click(screen.getByRole("button", { name: "保存并预览同步" }));
    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(dialog).not.toHaveTextContent("备份");
    expect(dialog).not.toHaveTextContent("生成");

    const setCall = calls.find((item) => item.command === "sources:set")!;
    expect(setCall.args).toEqual(["--key", "hooks.claude", "--value", JSON.stringify({ revision: "rev-c", value: { hooks: {}, removed_signatures: [] } })]);
    expect(calls.some((item) => item.command === "policy:set")).toBe(false);
    expect(calls.some((item) => item.command === "hooks:apply")).toBe(false);
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
