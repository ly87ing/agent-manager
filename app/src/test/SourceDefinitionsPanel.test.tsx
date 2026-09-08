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
    await screen.findByRole("dialog", { name: /将要执行的操作/ });
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
