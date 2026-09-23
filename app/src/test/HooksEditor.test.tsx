import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { HooksEditor } from "../components/HooksEditor";
import { screen, userEvent, within } from "./render-with-engine";

const actual = {
  PreToolUse: [{ matcher: "Edit|Write", hooks: [{ type: "command", command: "./guard.sh", timeout: 30 }] }],
};

function setup(initialHooks: Record<string, unknown> = actual) {
  const onSave = vi.fn();
  const onCancel = vi.fn();
  render(<HooksEditor initialHooks={initialHooks} busy={false} onSave={onSave} onCancel={onCancel} />);
  return { onSave, onCancel };
}

describe("Hooks 结构化编辑（实际配置）", () => {
  it("按事件与命令逐行展示实际 hooks", () => {
    setup();
    const row = screen.getByTestId("hooks-row-0");
    expect(within(row).getByTestId("hooks-event-0")).toHaveValue("PreToolUse");
    expect(within(row).getByTestId("hooks-matcher-0")).toHaveValue("Edit|Write");
    expect(within(row).getByTestId("hooks-command-0")).toHaveValue("./guard.sh");
    // 本工具不理解的字段如实说明会保留
    expect(row).toHaveTextContent("保留原有字段：timeout");
  });

  it("新增一条命令并保存：按原结构回填，未展示字段都带上", async () => {
    const { onSave } = setup();
    await userEvent.click(screen.getByTestId("hooks-add"));
    await userEvent.type(screen.getByTestId("hooks-event-1"), "SessionStart");
    await userEvent.type(screen.getByTestId("hooks-command-1"), "echo hi");
    await userEvent.click(screen.getByTestId("hooks-save"));

    expect(onSave).toHaveBeenCalledTimes(1);
    const hooks = onSave.mock.calls[0][0];
    expect(hooks.SessionStart).toEqual([{ hooks: [{ type: "command", command: "echo hi" }] }]);
    expect(hooks.PreToolUse[0].hooks[0].timeout).toBe(30);
  });

  it("命令为空时拦在保存前，不交给调用方", async () => {
    const { onSave } = setup();
    await userEvent.click(screen.getByTestId("hooks-add"));
    await userEvent.type(screen.getByTestId("hooks-event-1"), "Stop");
    await userEvent.click(screen.getByTestId("hooks-save"));
    expect(await screen.findByText(/命令不能为空/)).toBeInTheDocument();
    expect(onSave).not.toHaveBeenCalled();
  });

  it("移除全部命令后保存为空对象（引擎据此移除 Claude 的 hooks 键）", async () => {
    const { onSave } = setup();
    await userEvent.click(screen.getByTestId("hooks-remove-0"));
    expect(screen.getByTestId("hooks-empty")).toBeInTheDocument();
    await userEvent.click(screen.getByTestId("hooks-save"));
    expect(onSave).toHaveBeenCalledWith({});
  });

  it("command 以外的类型只读展示，编辑 command 后其余类型整条原样写回", async () => {
    const others = [
      { type: "http", url: "https://hooks.example.com", headers: { "X-Token": "${HOOK_TOKEN}" }, timeout: 10 },
      { type: "prompt", prompt: "Is this safe?" },
      { type: "agent", prompt: "Review", timeout: 120 },
      { type: "brand-new-kind", nested: { a: [1, 2] } },
    ];
    const initial = {
      PreToolUse: [{ matcher: "Bash", hooks: [others[0], { type: "command", command: "./guard.sh" }, ...others.slice(1)] }],
    };
    const { onSave } = setup(initial);
    // 只读行：没有输入框与移除按钮，显示类型
    expect(screen.getByTestId("hooks-type-0")).toHaveTextContent("http");
    expect(screen.queryByTestId("hooks-command-0")).toBeNull();
    expect(screen.queryByTestId("hooks-remove-0")).toBeNull();
    expect(screen.getByTestId("hooks-type-4")).toHaveTextContent("brand-new-kind");

    const command = screen.getByTestId("hooks-command-1");
    await userEvent.clear(command);
    await userEvent.type(command, "./guard-v2.sh");
    await userEvent.click(screen.getByTestId("hooks-save"));

    const hooks = onSave.mock.calls[0][0];
    const list = hooks.PreToolUse[0].hooks;
    expect(list[1]).toEqual({ type: "command", command: "./guard-v2.sh" });
    expect([list[0], ...list.slice(2)]).toEqual(others);
  });

  it("原文编辑：显示当前结构，改字段后按原文保存；非对象被拦下", async () => {
    const { onSave } = setup();
    await userEvent.click(screen.getByTestId("hooks-toggle-raw"));
    const field = screen.getByLabelText("Hooks 原文");
    expect(JSON.parse((field as HTMLTextAreaElement).value)).toEqual(actual);
    expect(field).toHaveAttribute("spellcheck", "false");
    expect(field).toHaveAttribute("autocorrect", "off");

    fireEvent.change(field, { target: { value: "[]" } });
    await userEvent.click(screen.getByTestId("hooks-save"));
    expect(await screen.findByText(/必须是 JSON 对象/)).toBeInTheDocument();
    expect(onSave).not.toHaveBeenCalled();

    const edited = { PreToolUse: [{ matcher: "Edit|Write", hooks: [{ type: "command", command: "./guard.sh", timeout: 60 }] }] };
    fireEvent.change(field, { target: { value: JSON.stringify(edited) } });
    await userEvent.click(screen.getByTestId("hooks-save"));
    expect(onSave).toHaveBeenCalledWith(edited);

    // 切回按行编辑：原文里的改动带回行里
    await userEvent.click(screen.getByTestId("hooks-toggle-raw"));
    expect(screen.getByTestId("hooks-command-0")).toHaveValue("./guard.sh");
  });
});
