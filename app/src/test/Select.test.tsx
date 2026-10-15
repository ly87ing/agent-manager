import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { Select } from "../components/Select";

const OPTIONS = [
  { value: "", label: "不设置" },
  { value: "Asia/Shanghai", label: "Asia/Shanghai" },
  { value: "America/Los_Angeles", label: "America/Los_Angeles" },
  { value: "Europe/Berlin", label: "Europe/Berlin" },
];

function Harness({ onChange }: { onChange: (value: string) => void }) {
  const [value, setValue] = useState("Asia/Shanghai");
  return (
    <Select
      label="时区"
      testId="tz"
      value={value}
      options={OPTIONS}
      onChange={(next) => {
        setValue(next);
        onChange(next);
      }}
    />
  );
}

describe("应用自己的下拉", () => {
  afterEach(() => vi.restoreAllMocks());

  it("只滚动选项列表；延迟的聚焦滚动不关闭，外部滚动移动按钮后关闭", async () => {
    let top = 0;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => new DOMRect(0, top, 160, 30));
    vi.spyOn(HTMLElement.prototype, "offsetTop", "get").mockImplementation(function (this: HTMLElement) {
      return Number(this.dataset.index ?? 0) * 30;
    });
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(30);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(60);
    render(<Select label="时区" value="Europe/Berlin" options={OPTIONS} onChange={vi.fn()} />);
    const trigger = screen.getByRole("combobox", { name: "时区" });
    await userEvent.click(trigger);
    const list = screen.getByRole("listbox");
    expect(list.scrollTop).toBe(60);
    fireEvent.scroll(list);
    expect(screen.getByRole("listbox")).toBe(list);
    await userEvent.keyboard("{Home}");
    expect(list.scrollTop).toBe(0);
    fireEvent.scroll(document.body);
    expect(screen.getByRole("listbox")).toBe(list);
    top = 20;
    fireEvent.scroll(document.body);
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("显示当前项；点开后点一项即选中并关闭", async () => {
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);
    const trigger = screen.getByRole("combobox", { name: "时区" });
    expect(trigger).toHaveTextContent("Asia/Shanghai");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(trigger);
    expect(screen.getByRole("listbox", { name: "时区" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Asia/Shanghai" })).toHaveAttribute("aria-selected", "true");
    await userEvent.click(screen.getByRole("option", { name: "Europe/Berlin" }));
    expect(onChange).toHaveBeenCalledWith("Europe/Berlin");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(trigger).toHaveAttribute("data-value", "Europe/Berlin");
  });

  it("键盘：↓ 打开并移动，Enter 选中；Esc 关闭不改值；打字跳到开头匹配的一项", async () => {
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);
    const trigger = screen.getByRole("combobox", { name: "时区" });
    trigger.focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    await userEvent.keyboard("{ArrowDown}{Enter}");
    expect(onChange).toHaveBeenLastCalledWith("America/Los_Angeles");
    expect(document.activeElement).toBe(trigger);

    await userEvent.keyboard("{Enter}");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onChange).toHaveBeenCalledTimes(1);

    await userEvent.keyboard("{ArrowDown}");
    await userEvent.keyboard("eu");
    await userEvent.keyboard("{Enter}");
    expect(onChange).toHaveBeenLastCalledWith("Europe/Berlin");
  });

  it("点外面关闭，不改值", async () => {
    const onChange = vi.fn();
    render(
      <div>
        <Harness onChange={onChange} />
        <button type="button">外面</button>
      </div>,
    );
    await userEvent.click(screen.getByRole("combobox", { name: "时区" }));
    await userEvent.click(screen.getByRole("button", { name: "外面" }));
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("在对话框里按 Esc 只关下拉，不冒泡到对话框", async () => {
    const onDialogEscape = vi.fn();
    render(
      <div role="dialog" onKeyDown={(event) => event.key === "Escape" && onDialogEscape()}>
        <Harness onChange={vi.fn()} />
      </div>,
    );
    const trigger = screen.getByRole("combobox", { name: "时区" });
    await userEvent.click(trigger);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onDialogEscape).not.toHaveBeenCalled();
    await userEvent.keyboard("{Escape}");
    expect(onDialogEscape).toHaveBeenCalledTimes(1);
  });
});

