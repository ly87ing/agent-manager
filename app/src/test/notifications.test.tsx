import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";
import { toast as sonner } from "sonner";
import { notifyError, toast } from "../lib/notifications";

afterEach(() => vi.restoreAllMocks());

describe("notification policy", () => {
  it.each([["success", 4000], ["info", 4000], ["warning", 10000], ["error", 10000]] as const)("bounds and dismisses %s feedback", (kind, duration) => {
    const show = vi.spyOn(sonner, kind).mockReturnValue("feedback");
    expect(toast[kind]("Result", { description: "Long diagnostic" })).toBe("feedback");
    expect(show).toHaveBeenCalledWith("Result", expect.objectContaining({ closeButton: true, duration, position: "bottom-right",
      classNames: expect.objectContaining({ title: expect.stringContaining("max-h-20"), description: expect.stringContaining("max-h-32") }) }));
  });

  it("keeps dialog feedback above the action footer and preserves action callbacks", () => {
    render(<div role="dialog" aria-modal="true" />);
    const show = vi.spyOn(sonner, "success").mockReturnValue(1);
    const onClick = vi.fn();
    toast.success("Saved", { id: "saved", action: { label: "Undo", onClick } });
    expect(show).toHaveBeenCalledWith("Saved", expect.objectContaining({ id: "saved", position: "top-right", duration: 10000, action: { label: "Undo", onClick } }));
  });

  it("retains original diagnostics and advice without putting them in the title", () => {
    const show = vi.spyOn(sonner, "error").mockReturnValue(1);
    notifyError("Save failed", new Error("No such file or directory: /example/resource"));
    expect(show).toHaveBeenCalledWith("Save failed", expect.objectContaining({ description: expect.stringContaining("No such file or directory: /example/resource") }));
    expect(show.mock.calls[0][1]?.description).toContain("确认文件、目录或应用路径存在");
  });
});
