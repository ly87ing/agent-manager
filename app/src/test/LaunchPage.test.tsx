import { beforeEach, describe, expect, it, vi } from "vitest";
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import LaunchPage from "../pages/LaunchPage";
import { makeEnvelope } from "./fixtures";
import { chooseOption, renderWithEngine, screen, userEvent } from "./render-with-engine";

vi.mock("@tauri-apps/api/core", async (original) => ({ ...await original<typeof import("@tauri-apps/api/core")>(), isTauri: vi.fn(() => false) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
beforeEach(() => vi.mocked(isTauri).mockReturnValue(false));

const status = { platform: "macos", shell: "zsh", path: "/home/test/.zshrc", base: "missing", entries: [], external_names: [] };
const entry = { name: "example-sg", timezone: "Asia/Singapore", command: "open --env 'TZ=Asia/Singapore' -a '/Applications/Example.app'", definition: "alias example-sg='complete definition'" };
const preview = { base: "missing", files: [{ path: status.path, original: null, content: entry.definition, deleted: false }] };
function setup(overrides = {}) {
  return renderWithEngine(<LaunchPage />, {
    "launch:catalog": makeEnvelope({ platform: "macos", shell: "zsh", apps: [{ name: "Example", path: "/Applications/Example.app" }] }),
    "launch:status": makeEnvelope(status),
    "launch:build": makeEnvelope({ entries: [entry] }),
    "launch:preview": makeEnvelope(preview),
    "launch:apply": makeEnvelope({ status }),
    "launch:run": makeEnvelope({ state: "requested", timezone_verified: false }),
    ...overrides,
  });
}
async function generate() {
  await chooseOption(await screen.findByRole("combobox", { name: "本机应用" }), "/Applications/Example.app");
  await userEvent.click(screen.getByRole("button", { name: "生成命令" }));
  await screen.findByRole("button", { name: "复制完整命令" });
}

describe("launch configuration controls", () => {
  it("allows native file selection and cancellation without writing or launching", async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    const { calls } = setup();
    await screen.findByRole("combobox", { name: "本机应用" });
    vi.mocked(open).mockResolvedValueOnce(null).mockResolvedValueOnce("/Applications/Custom App.app");
    await userEvent.click(screen.getByRole("button", { name: "选择文件" }));
    expect(screen.getByRole("textbox", { name: "程序路径或命令" })).toHaveValue("");
    await userEvent.click(screen.getByRole("button", { name: "选择文件" }));
    expect(screen.getByRole("textbox", { name: "程序路径或命令" })).toHaveValue("/Applications/Custom App.app");
    expect(calls.some((call) => ["launch:apply", "launch:run"].includes(call.command))).toBe(false);
  });
  it("copies the complete command and definition without applying or launching", async () => {
    const { calls } = setup();
    await generate();
    const write = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: write } });
    await userEvent.click(screen.getByRole("button", { name: "复制完整命令" }));
    expect(write).toHaveBeenLastCalledWith(entry.command);
    await userEvent.click(screen.getByRole("button", { name: "复制全部定义" }));
    expect(write).toHaveBeenLastCalledWith(entry.definition);
    expect(calls.some((call) => ["launch:apply", "launch:run", "launch:preview"].includes(call.command))).toBe(false);
  });

  it("shows the exact preview, cancels without writes, then applies only on confirmation", async () => {
    const { calls } = setup();
    await generate();
    await userEvent.click(screen.getByRole("button", { name: "应用到本机" }));
    expect(await screen.findByRole("dialog")).toHaveTextContent(status.path);
    expect(screen.getByRole("dialog")).toHaveTextContent(entry.definition);
    expect(calls.some((call) => call.command === "launch:apply")).toBe(false);
    await userEvent.click(screen.getByTestId("confirm-modal-cancel"));
    expect(calls.some((call) => call.command === "launch:apply")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "应用到本机" }));
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await screen.findByText("快捷命令已应用。新开终端后生效；应用显示的时间仍需验证。");
    const applied = calls.find((call) => call.command === "launch:apply")!;
    const planned = calls.filter((call) => call.command === "launch:preview").slice(-1)[0]!;
    expect(applied.args).toEqual([...planned.args, "--confirm"]);
    expect(applied.options?.runId).toBeTruthy();
  });

  it("invalidates generated commands when a custom name or time zone changes", async () => {
    const { calls } = setup();
    await generate();
    await userEvent.clear(screen.getByRole("textbox", { name: "别名 1" }));
    await userEvent.type(screen.getByRole("textbox", { name: "别名 1" }), "my-custom-zone");
    await userEvent.clear(screen.getByRole("textbox", { name: "自定义时区 1" }));
    await userEvent.type(screen.getByRole("textbox", { name: "自定义时区 1" }), "Custom/Zone");
    expect(screen.queryByRole("button", { name: "应用到本机" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "生成命令" }));
    await screen.findByRole("button", { name: "复制完整命令" });
    const build = calls.filter((call) => call.command === "launch:build").slice(-1)[0]!;
    const spec = JSON.parse(build.args[1]);
    expect(spec.variants[0]).toEqual({ name: "my-custom-zone", timezone: "Custom/Zone" });
  });

  it("requires a separate launch confirmation and does not save a shortcut", async () => {
    const { calls } = setup();
    await generate();
    await userEvent.click(screen.getByRole("button", { name: "启动一次" }));
    expect(await screen.findByRole("dialog")).toHaveTextContent(entry.command);
    expect(calls.some((call) => call.command === "launch:run")).toBe(false);
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await screen.findByText("已发送启动请求。请在应用中确认时间。");
    expect(calls.find((call) => call.command === "launch:run")?.args).toContain("--confirm");
    expect(calls.some((call) => call.command === "launch:apply")).toBe(false);
  });

  it("keeps a stale-preview error visible without retrying or overwriting", async () => {
    const { calls } = setup({ "launch:apply": makeEnvelope({}, { ok: false, errors: [{ code: "LAUNCH_ERROR", element: null, message: "Modified externally" }] }) });
    await generate();
    await userEvent.click(screen.getByRole("button", { name: "应用到本机" }));
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    expect(await screen.findByRole("alert")).toHaveTextContent("Modified externally");
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(calls.filter((call) => call.command === "launch:apply")).toHaveLength(1);
  });

  it("provides copy output for another platform and keeps CLI launch in the terminal", async () => {
    setup();
    await screen.findByRole("combobox", { name: "本机应用" });
    await chooseOption(screen.getByRole("combobox", { name: "程序类型" }), "command");
    await userEvent.type(screen.getByRole("textbox", { name: "程序路径或命令" }), "example");
    await chooseOption(screen.getByRole("combobox", { name: "目标系统" }), "windows");
    await userEvent.click(screen.getByRole("button", { name: "生成命令" }));
    await screen.findByRole("button", { name: "复制完整命令" });
    expect(screen.getByRole("button", { name: "应用到本机" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "启动一次" })).toBeNull();
    expect(screen.getByText("命令行程序请在终端运行复制的命令，保留正常交互。")).toBeVisible();
    await chooseOption(screen.getByRole("combobox", { name: "目标系统" }), "linux");
    expect(screen.getByRole("combobox", { name: "Shell" })).toHaveAttribute("data-value", "bash");
  });
});
