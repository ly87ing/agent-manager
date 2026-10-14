import { beforeEach, describe, expect, it, vi } from "vitest";
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { waitFor, within } from "@testing-library/react";
import LaunchPage from "../pages/LaunchPage";
import type { LaunchEntry, LaunchRecipe, SavedLaunch } from "../lib/launch";
import { makeEnvelope } from "./fixtures";
import { chooseOption, renderWithEngine, screen, userEvent } from "./render-with-engine";

vi.mock("@tauri-apps/api/core", async (original) => ({ ...await original<typeof import("@tauri-apps/api/core")>(), isTauri: vi.fn(() => false) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
beforeEach(() => vi.mocked(isTauri).mockReturnValue(false));

const recipe: LaunchRecipe = {
  platform: "macos", shell: "zsh", kind: "app", target: "/Applications/Example.app", args: [], env: [],
  variants: [{ name: "example-sg", timezone: "Asia/Singapore" }, { name: "example-la", timezone: "America/Los_Angeles" }],
};
const savedItem: SavedLaunch = { id: "saved-1", revision: "a".repeat(64), recipe, entries: entries(recipe) };

function entries(spec: LaunchRecipe): LaunchEntry[] {
  return spec.variants.map((variant) => {
    const command = spec.platform === "windows"
      ? `& { $env:TZ = '${variant.timezone}'; & '${spec.target}' }`
      : `TZ='${variant.timezone}' command '${spec.target}'`;
    return { ...variant, command, definition: spec.platform === "windows"
      ? `function ${variant.name} { ${command} }`
      : `alias ${variant.name}='${command}'` };
  });
}

function setup(options: { items?: SavedLaunch[]; saveError?: string; runError?: string } = {}) {
  let items = [...(options.items ?? [])];
  let revision = 1;
  const value = (args: string[], flag: string) => args[args.indexOf(flag) + 1];
  return renderWithEngine(<LaunchPage />, {
    "launch:catalog": makeEnvelope({ platform: "macos", shell: "zsh", apps: [{ name: "Example", path: "/Applications/Example.app" }] }),
    "launch:list": async () => makeEnvelope({ items }),
    "launch:build": async (args) => {
      const next = JSON.parse(value(args, "--spec")) as LaunchRecipe;
      return makeEnvelope({ entries: entries(next), timezone_verified: false });
    },
    "launch:save": async (args) => {
      if (options.saveError) return makeEnvelope({}, { ok: false, errors: [{ code: "LAUNCH_ERROR", element: null, message: options.saveError }] });
      const next = JSON.parse(value(args, "--spec")) as LaunchRecipe;
      const id = args.includes("--id") ? value(args, "--id") : `saved-${++revision}`;
      const item = { id, revision: (++revision).toString(16).padStart(64, "0"), recipe: next, entries: entries(next) };
      items = [...items.filter((old) => old.id !== id), item];
      return makeEnvelope({ item });
    },
    "launch:delete": async (args) => {
      const id = value(args, "--id");
      items = items.filter((item) => item.id !== id);
      return makeEnvelope({ id });
    },
    "launch:run": options.runError ? makeEnvelope({}, { ok: false, errors: [{ code: "LAUNCH_ERROR", element: null, message: options.runError }] }) : makeEnvelope({ state: "requested", timezone_verified: false }),
  });
}

async function chooseExample() {
  await chooseOption(await screen.findByRole("combobox", { name: "本机应用" }), "/Applications/Example.app");
}

async function startNew() {
  await userEvent.click(await screen.findByRole("button", { name: "新增配置" }));
  await screen.findByRole("heading", { name: "新建配置" });
}

async function generate() {
  await startNew();
  await chooseExample();
  await userEvent.click(screen.getByRole("button", { name: "预览命令" }));
  await screen.findAllByRole("button", { name: "复制运行命令" });
}

async function savedAction(name: string) {
  await userEvent.click(await screen.findByRole("button", { name: "配置操作 example-sg" }));
  await userEvent.click(await screen.findByRole("menuitem", { name }));
}

describe("launch configuration controls", () => {
  it.each(["saved", "preview"])("announces %s launch failures immediately and retains details beside the triggering entry", async (location) => {
    const reason = "The application is already running. Quit it normally before launching with a different time zone";
    const { calls } = setup({ items: location === "saved" ? [savedItem] : [], runError: reason });
    if (location === "preview") await generate();
    const launch = location === "saved" ? await screen.findByTestId("run-saved-saved-1-example-sg") : screen.getAllByRole("button", { name: "启动一次" })[0];
    await userEvent.click(launch);
    await waitFor(() => expect(document.querySelector('[data-sonner-toast][data-type="error"]')).toHaveTextContent(reason));
    expect(document.querySelector('[data-sonner-toast][data-type="error"]')).toHaveTextContent("启动失败");
    expect(document.querySelector('[data-sonner-toast][data-type="error"]')?.closest('[data-sonner-toaster]'))
      .toHaveAttribute("data-y-position", location === "preview" ? "top" : "bottom");
    const detail = screen.getByRole("alert");
    expect(detail).toHaveTextContent(reason);
    expect(launch.closest('[data-launch-entry]')).toContainElement(detail);
    expect(calls.filter((call) => call.command === "launch:run")).toHaveLength(1);
    expect(calls.some((call) => ["launch:save", "launch:delete"].includes(call.command))).toBe(false);
  });

  it("shows saved launchers first and opens the editor only on request", async () => {
    setup();
    expect(await screen.findByText("尚未保存配置。点击“新增配置”创建常用应用或命令。")).toBeVisible();
    expect(screen.queryByRole("textbox", { name: "程序路径或命令" })).toBeNull();
    await startNew();
    expect(screen.getByRole("textbox", { name: "程序路径或命令" })).toBeVisible();
    const dialog = screen.getByRole("dialog", { name: "新建配置" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveTextContent("预览命令不会保存；保存到 Agent Manager 仅保存常用配置，不写入 Shell 配置或修改系统环境。");
    const actions = screen.getByTestId("launch-editor-actions");
    expect(dialog).toContainElement(actions);
    expect(screen.getByTestId("launch-editor-body")).not.toContainElement(actions);
    expect(actions).toContainElement(screen.getByRole("button", { name: "取消" }));
    expect(actions).toContainElement(screen.getByRole("button", { name: "预览命令" }));
    expect(actions).toContainElement(screen.getByRole("button", { name: "保存到 Agent Manager" }));
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("textbox", { name: "程序路径或命令" })).toBeNull();
    expect(screen.getByRole("button", { name: "新增配置" })).toHaveFocus();
  });

  it("opens a saved row in a modal and cancels draft changes without mutating the recipe", async () => {
    const { calls } = setup({ items: [savedItem] });
    const trigger = await screen.findByRole("button", { name: "编辑配置 example-sg" });
    await userEvent.click(trigger);
    expect(screen.getByRole("dialog", { name: "编辑配置" })).toContainElement(screen.getByRole("textbox", { name: "程序路径或命令" }));
    await userEvent.type(screen.getByRole("textbox", { name: "程序路径或命令" }), " changed");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(trigger).toHaveFocus();
    await userEvent.click(trigger);
    expect(screen.getByRole("textbox", { name: "程序路径或命令" })).toHaveValue(recipe.target);
    expect(screen.getByRole("textbox", { name: "别名 1" })).toHaveValue("example-sg");
    expect(calls.some((call) => ["launch:build", "launch:save", "launch:delete", "launch:run"].includes(call.command))).toBe(false);
  });

  it("keeps each saved variant in its own row with one primary action and the full definition in its menu", async () => {
    setup({ items: [savedItem] });
    const launch = await screen.findByTestId("run-saved-saved-1-example-la");
    const row = launch.closest("tr")!;
    expect(within(row).getByText(recipe.target)).toBeVisible();
    expect(within(row).getByText("example-la")).toBeVisible();
    expect(within(row).getByText("America/Los_Angeles")).toBeVisible();
    expect(within(row).queryByRole("button", { name: "复制完整别名定义" })).toBeNull();
    const write = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: write } });
    await userEvent.click(within(row).getByRole("button", { name: "配置操作 example-la" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "复制完整别名定义" }));
    expect(write).toHaveBeenCalledWith(savedItem.entries[1].definition);
  });

  it("allows native file selection and cancellation without saving, deleting or launching", async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    const { calls } = setup();
    await startNew();
    await screen.findByRole("combobox", { name: "本机应用" });
    vi.mocked(open).mockResolvedValueOnce(null).mockResolvedValueOnce("/Applications/Custom App.app");
    await userEvent.click(screen.getByRole("button", { name: "选择文件" }));
    expect(screen.getByRole("textbox", { name: "程序路径或命令" })).toHaveValue("");
    await userEvent.click(screen.getByRole("button", { name: "选择文件" }));
    expect(screen.getByRole("textbox", { name: "程序路径或命令" })).toHaveValue("/Applications/Custom App.app");
    expect(calls.some((call) => ["launch:save", "launch:delete", "launch:run"].includes(call.command))).toBe(false);
  });

  it("copies the launch command and complete definition without saving or launching", async () => {
    const { calls } = setup();
    await generate();
    const write = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: write } });
    await userEvent.click(screen.getAllByRole("button", { name: "复制运行命令" })[0]);
    expect(write).toHaveBeenLastCalledWith(entries(recipe)[0].command);
    await userEvent.click(screen.getByRole("button", { name: "复制全部定义" }));
    expect(write).toHaveBeenLastCalledWith(entries(recipe).map((item) => item.definition).join("\n"));
    expect(calls.some((call) => ["launch:save", "launch:delete", "launch:run"].includes(call.command))).toBe(false);
    expect(calls.some((call) => ["launch:status", "launch:preview", "launch:apply"].includes(call.command))).toBe(false);
  });

  it("saves a configuration without generating, launching or writing shell files", async () => {
    const { calls } = setup();
    await startNew();
    await chooseExample();
    await userEvent.click(screen.getByRole("button", { name: "保存到 Agent Manager" }));
    expect(await screen.findByRole("status")).toHaveTextContent("配置已保存到 Agent Manager。");
    const saved = calls.find((call) => call.command === "launch:save")!;
    expect(JSON.parse(saved.args[1])).toMatchObject({ target: recipe.target, variants: recipe.variants });
    expect(saved.args).not.toContain("--confirm");
    expect(saved.options?.runId).toBeTruthy();
    expect(calls.some((call) => ["launch:build", "launch:run", "launch:status", "launch:preview", "launch:apply"].includes(call.command))).toBe(false);
    expect((await screen.findAllByText("example-sg"))[0]).toBeVisible();
  });

  it("opens and updates a saved configuration with its revision", async () => {
    const { calls } = setup({ items: [savedItem] });
    await userEvent.click(await screen.findByRole("button", { name: "编辑配置 example-sg" }));
    expect(screen.getByRole("textbox", { name: "程序路径或命令" })).toHaveValue(recipe.target);
    await userEvent.clear(screen.getByRole("textbox", { name: "别名 1" }));
    await userEvent.type(screen.getByRole("textbox", { name: "别名 1" }), "example-sin");
    await userEvent.click(screen.getByRole("button", { name: "保存到 Agent Manager" }));
    expect((await screen.findAllByText("example-sin"))[0]).toBeVisible();
    const saves = calls.filter((call) => call.command === "launch:save");
    const saved = saves[saves.length - 1]!;
    expect(saved.args).toEqual(expect.arrayContaining(["--id", savedItem.id, "--base", savedItem.revision]));
  });

  it("deletes only after a separate confirmation", async () => {
    const { calls } = setup({ items: [savedItem] });
    await screen.findAllByText("example-sg");
    await savedAction("删除配置");
    expect(await screen.findByRole("dialog")).toHaveTextContent("你自行添加的别名不受影响");
    expect(calls.some((call) => call.command === "launch:delete")).toBe(false);
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    expect(await screen.findByRole("status")).toHaveTextContent("已删除保存的配置。");
    expect(calls.find((call) => call.command === "launch:delete")?.args).toEqual(["--id", savedItem.id, "--base", savedItem.revision]);
    expect(calls.some((call) => ["launch:status", "launch:preview", "launch:apply"].includes(call.command))).toBe(false);
  });

  it("treats the launch button as confirmation and does not save", async () => {
    const { calls } = setup();
    await generate();
    const editor = screen.getByRole("dialog");
    await userEvent.click(screen.getAllByRole("button", { name: "启动一次" })[0]);
    expect(await screen.findByRole("status")).toHaveTextContent("已发送启动请求。请在应用中确认时间和环境。");
    expect(screen.getAllByRole("dialog")).toEqual([editor]);
    expect(calls.find((call) => call.command === "launch:run")?.args).toContain("--confirm");
    expect(calls.some((call) => call.command === "launch:save")).toBe(false);
  });

  it("copies and directly launches a saved graphical application without opening the editor", async () => {
    const { calls } = setup({ items: [savedItem] });
    const write = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: write } });
    await savedAction("复制运行命令");
    expect(write).toHaveBeenLastCalledWith(savedItem.entries[0].command);
    await userEvent.click(screen.getByTestId("run-saved-saved-1-example-sg"));
    expect(await screen.findByRole("status")).toHaveTextContent("已发送启动请求");
    expect(calls.find((call) => call.command === "launch:run")?.args).toEqual([
      "--spec", JSON.stringify(recipe), "--name", "example-sg", "--confirm",
    ]);
    expect(screen.queryByRole("heading", { name: "新建配置" })).toBeNull();
  });

  it("saves generic environment variables and component-specific arguments", async () => {
    const { calls } = setup();
    await startNew();
    await chooseExample();
    await userEvent.click(screen.getByText("环境变量（0）"));
    await userEvent.click(screen.getByRole("button", { name: "添加环境变量" }));
    await userEvent.type(screen.getByRole("textbox", { name: "环境变量名称 1" }), "LANG");
    await userEvent.type(screen.getByRole("textbox", { name: "环境变量取值 1" }), "en_US.UTF-8");
    await userEvent.click(screen.getByText("启动参数（0）"));
    await userEvent.click(screen.getByRole("button", { name: "添加参数" }));
    await userEvent.type(screen.getByRole("textbox", { name: "启动参数 1" }), "--lang=en-US");
    await userEvent.click(screen.getByRole("button", { name: "保存到 Agent Manager" }));
    await screen.findByRole("status");
    const saved = calls.find((call) => call.command === "launch:save")!;
    expect(JSON.parse(saved.args[1])).toMatchObject({
      env: [{ name: "LANG", value: "en_US.UTF-8" }], args: ["--lang=en-US"],
    });
  });

  it("keeps a stale save error visible without retrying", async () => {
    const { calls } = setup({ items: [savedItem], saveError: "Modified externally" });
    await userEvent.click(await screen.findByRole("button", { name: "编辑配置 example-sg" }));
    await userEvent.type(screen.getByRole("textbox", { name: "程序路径或命令" }), " changed");
    await userEvent.click(screen.getByRole("button", { name: "保存到 Agent Manager" }));
    const detail = await screen.findByRole("alert");
    expect(detail).toHaveTextContent("Modified externally");
    expect(screen.getByTestId("launch-editor-actions")).toContainElement(detail);
    await waitFor(() => expect(document.querySelector('[data-sonner-toast][data-type="error"]')).toHaveTextContent("保存配置失败"));
    expect(document.querySelector('[data-sonner-toast][data-type="error"]')?.closest('[data-sonner-toaster]')).toHaveAttribute("data-y-position", "top");
    expect(calls.filter((call) => call.command === "launch:save")).toHaveLength(1);
  });

  it("announces clipboard failure at the entry and clears it on a successful retry", async () => {
    setup({ items: [savedItem] });
    const write = vi.fn().mockRejectedValueOnce(new Error("Clipboard unavailable")).mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: write } });
    const row = (await screen.findByTestId("run-saved-saved-1-example-sg")).closest("[data-launch-entry]");
    await savedAction("复制运行命令");
    await waitFor(() => expect(document.querySelector('[data-sonner-toast][data-type="error"]')).toHaveTextContent("Clipboard unavailable"));
    expect(row).toContainElement(screen.getByRole("alert"));
    await savedAction("复制运行命令");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    await waitFor(() => expect(document.querySelector('[data-sonner-toast][data-type="error"]')).not.toBeInTheDocument());
    expect(write).toHaveBeenCalledTimes(2);
  });

  it("saves and copies another platform while keeping launch unavailable", async () => {
    const { calls } = setup();
    await startNew();
    await screen.findByRole("combobox", { name: "本机应用" });
    await chooseOption(screen.getByRole("combobox", { name: "程序类型" }), "command");
    await userEvent.type(screen.getByRole("textbox", { name: "程序路径或命令" }), "example.exe");
    await chooseOption(screen.getByRole("combobox", { name: "目标系统" }), "windows");
    await userEvent.click(screen.getByRole("button", { name: "预览命令" }));
    const copy = await screen.findAllByRole("button", { name: "复制完整函数定义" });
    const write = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: write } });
    await userEvent.click(copy[0]);
    expect(write).toHaveBeenCalledWith(expect.stringMatching(/^function example-sg \{/));
    expect(screen.queryByRole("button", { name: "启动一次" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "保存到 Agent Manager" }));
    await screen.findByRole("status");
    expect(calls.some((call) => call.command === "launch:save")).toBe(true);
    expect(screen.getByText("命令行程序请在终端运行复制的命令，保留正常交互。")).toBeVisible();
  });
});
