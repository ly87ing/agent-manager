import { describe, expect, it, vi } from "vitest";
import { ShareLinkRegistration } from "../components/ShareLinkRegistration";
import { setShareLinkRegistrationClient, type ShareLinkPreview } from "../lib/shareLinkRegistration";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

const preview: ShareLinkPreview = { base: "reviewed-base", executable: "/YOUR_APPS/Agent Manager.AppImage", entry: "/YOUR_DATA/applications/agentmanager.desktop", current_handler: "foreign.desktop", registered: false };
describe("AppImage share-link registration", () => {
  it("reads without writing and cancellation preserves the association", async () => {
    const register = vi.fn(async () => preview);
    setShareLinkRegistrationClient({ preview: async () => preview, register });
    renderWithEngine(<ShareLinkRegistration />, {});
    await userEvent.click(await screen.findByRole("button", { name: "预览链接关联" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent("foreign.desktop");
    expect(dialog).toHaveTextContent(preview.executable);
    expect(dialog).toHaveTextContent(preview.entry);
    expect(register).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(register).not.toHaveBeenCalled();
  });
  it("submits the reviewed fingerprint only after confirmation and uses readback", async () => {
    let actual = preview;
    const register = vi.fn(async () => (actual = { ...preview, registered: true }));
    setShareLinkRegistrationClient({ preview: async () => actual, register });
    renderWithEngine(<ShareLinkRegistration />, {});
    await userEvent.click(await screen.findByRole("button", { name: "预览链接关联" }));
    await userEvent.click(screen.getByRole("button", { name: "确认关联链接" }));
    expect(await screen.findByText("已关联此应用")).toBeInTheDocument();
    expect(register).toHaveBeenCalledExactlyOnceWith("reviewed-base");
    expect(screen.queryByRole("dialog")).toBeNull();
  });
  it("retains a failed confirmation and allows a fresh read", async () => {
    const read = vi.fn(async () => preview);
    setShareLinkRegistrationClient({ preview: read, register: async () => { throw new Error("Association changed. Preview again."); } });
    renderWithEngine(<ShareLinkRegistration />, {});
    await userEvent.click(await screen.findByRole("button", { name: "预览链接关联" }));
    await userEvent.click(screen.getByRole("button", { name: "确认关联链接" }));
    expect(await within(screen.getByRole("dialog")).findByRole("alert")).toHaveTextContent("Association changed");
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await userEvent.click(screen.getByRole("button", { name: "重新读取" }));
    await vi.waitFor(() => expect(read.mock.calls.length).toBeGreaterThan(1));
  });
});
