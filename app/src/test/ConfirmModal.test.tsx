import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConfirmModal } from "../components/ConfirmModal";

function Harness() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>删除配置</button>
      {open && (
        <ConfirmModal
          title="确认删除"
          description="删除前再次确认。"
          risk="write-local"
          executing={false}
          onConfirm={vi.fn()}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}

describe("ConfirmModal 焦点管理", () => {
  it("打开后接管焦点，Tab 不逃到背景，关闭后回到触发按钮", async () => {
    render(<Harness />);
    const trigger = screen.getByRole("button", { name: "删除配置" });
    await userEvent.click(trigger);

    const cancel = await screen.findByTestId("confirm-modal-cancel");
    const confirm = screen.getByTestId("confirm-modal-confirm");
    expect(cancel).toHaveFocus();

    await userEvent.tab();
    expect(confirm).toHaveFocus();
    await userEvent.tab();
    expect(screen.getByRole("button", { name: "关闭" })).toHaveFocus();
    await userEvent.tab({ shift: true });
    expect(confirm).toHaveFocus();

    await userEvent.click(cancel);
    expect(trigger).toHaveFocus();
  });
});
