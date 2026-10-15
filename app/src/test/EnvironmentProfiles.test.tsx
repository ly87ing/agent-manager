import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import EnvironmentPage from "../pages/EnvironmentPage";
import { createEnvironmentMock } from "./environment-mock";
import { environmentBuiltinsFixture, environmentFixture } from "./environment-fixture";
import { makeEnvelope } from "./fixtures";
import { chooseOption, readOptions, renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import type { LaunchRecipe } from "../lib/launch";

describe("Independent environment profiles", () => {
  it.each(environmentBuiltinsFixture())("selects $name explicitly and compares every strict criterion without writes", async (builtin) => {
    const { calls } = renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ items: [], builtins: environmentBuiltinsFixture() }),
      "environment:check": async (args) => makeEnvelope(environmentFixture(JSON.parse(args[args.indexOf("--spec") + 1]), args.includes("--probe-time"))),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    expect(screen.getByRole("combobox", { name: "选择环境 Profile" })).toHaveAttribute("data-value", "");
    await userEvent.click(screen.getByRole("tab", { name: "环境 Profiles" }));
    const presets = screen.getByRole("region", { name: "内置环境 Profiles" });
    expect(presets).toHaveTextContent(builtin.target.ntp_server!);
    await userEvent.click(within(presets).getByRole("button", { name: `选择 Profile ${builtin.name}` }));
    expect(screen.getByRole("combobox", { name: "选择环境 Profile" })).toHaveAttribute("data-value", builtin.id);
    expect(screen.getByText(`检查会向 ${builtin.target.ntp_server} 的 UDP 123 端口查询参考时间。`)).toBeVisible();
    expect(calls.filter((call) => call.command === "environment:check")).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "对比 Profile" }));
    await screen.findByText("发现与目标不符的项目。");
    const checks = calls.filter((call) => call.command === "environment:check");
    const request = checks[checks.length - 1];
    expect(JSON.parse(request.args[request.args.indexOf("--spec") + 1])).toEqual(builtin.target);
    expect(request.args).toContain("--probe-time");
    expect(document.querySelector('[data-check-id="process.raw_locale"]')).toHaveTextContent(builtin.target.locale!);
    expect(calls.some((call) => ["environment:save-profile", "environment:delete-profile", "environment:sync-time", "environment:open-settings"].includes(call.command))).toBe(false);
    await userEvent.click(screen.getByRole("tab", { name: "环境 Profiles" }));
    await userEvent.click(screen.getByRole("button", { name: "重新读取集合" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "重新读取集合" })).toBeEnabled());
    await userEvent.click(screen.getByRole("tab", { name: "对比与应用" }));
    expect(screen.getByRole("combobox", { name: "选择环境 Profile" })).toHaveAttribute("data-value", builtin.id);
    expect(screen.getByRole("region", { name: "检查结果" })).toHaveTextContent("发现与目标不符的项目。");
  });

  it("copies a built-in into an independent custom profile and never exposes edit or delete for the original", async () => {
    const mock = createEnvironmentMock();
    const routes = Object.fromEntries(["check", "profiles", "save-profile"].map((verb) => [`environment:${verb}`, async (args: string[]) => mock(`environment:${verb}`, args)]));
    const { calls } = renderWithEngine(<EnvironmentPage />, routes);
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await userEvent.click(screen.getByRole("tab", { name: "环境 Profiles" }));
    await userEvent.click(screen.getByRole("button", { name: "内置 Profile 操作 Singapore (SG)" }));
    expect(screen.queryByRole("menuitem", { name: "删除 Profile" })).toBeNull();
    expect(screen.queryByRole("button", { name: "编辑 Profile Singapore (SG)" })).toBeNull();
    await userEvent.click(screen.getByRole("menuitem", { name: "复制为自定义 Profile" }));
    expect(screen.getByRole("checkbox", { name: "同时检查已设置的原始 locale 变量" })).toBeChecked();
    expect(screen.getByRole("textbox", { name: "目标语言顺序" })).toHaveValue("en-SG");
    expect(screen.getByRole("spinbutton", { name: "容许时间偏差（毫秒）" })).toHaveValue(2000);
    await userEvent.clear(screen.getByRole("textbox", { name: "目标时间服务器" }));
    await userEvent.type(screen.getByRole("textbox", { name: "目标时间服务器" }), "time.example.org");
    await userEvent.click(screen.getByRole("button", { name: "保存 Profile" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const save = calls.find((call) => call.command === "environment:save-profile")!;
    expect(save.args).not.toContain("--id");
    expect(save.args).not.toContain("--base");
    expect(JSON.parse(save.args[1])).toMatchObject({ auto_sync: false, target: { ntp_server: "time.example.org", strict_locale: true } });
    expect(screen.getByRole("region", { name: "内置环境 Profiles" })).toHaveTextContent("nets.org.sg");
    expect(screen.getByRole("table", { name: "环境 Profiles" })).toHaveTextContent("Singapore (SG) 副本");
    await userEvent.click(screen.getByRole("button", { name: "选择 Profile Singapore (SG)" }));
    expect(screen.getByText("检查会向 nets.org.sg 的 UDP 123 端口查询参考时间。")).toBeVisible();
  });

  it("compares a legacy auto-sync profile without correcting time", async () => {
    const target = { ntp_server: "time.example.org", max_clock_offset_ms: 2000 };
    const { calls } = renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ builtins: [], items: [{ id: "legacy", revision: "a".repeat(64), name: "Legacy", target: environmentFixture(target).target, auto_sync: true }] }),
      "environment:check": async (args) => makeEnvelope(environmentFixture(JSON.parse(args[args.indexOf("--spec") + 1]), args.includes("--probe-time"))),
      "environment:sync-time": makeEnvelope(environmentFixture(target, true)),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择环境 Profile" }), "legacy");
    await userEvent.click(screen.getByRole("button", { name: "对比 Profile" }));
    await screen.findByText(/查询地址：/);
    expect(calls.filter((call) => call.command === "environment:check")).toHaveLength(2);
    expect(calls.some((call) => call.command === "environment:sync-time")).toBe(false);
  });

  it("maintains profiles separately without selecting or applying a saved definition", async () => {
    const mock = createEnvironmentMock();
    const routes = Object.fromEntries(["check", "profiles", "save-profile", "delete-profile"].map((verb) => [`environment:${verb}`, async (args: string[]) => mock(`environment:${verb}`, args)]));
    const { calls } = renderWithEngine(<EnvironmentPage />, routes);
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await userEvent.click(screen.getByRole("tab", { name: "环境 Profiles" }));
    expect(screen.queryByRole("combobox", { name: "选择环境 Profile" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "新增 Profile" }));
    expect(screen.queryByRole("checkbox", { name: "检查发现时间偏差超限时自动校时" })).toBeNull();
    expect(screen.getByRole("button", { name: "保存 Profile" })).toBeDisabled();
    expect(screen.getByText("请填写 Profile 名称。")).toBeVisible();
    await userEvent.type(screen.getByRole("textbox", { name: "Profile 名称" }), "Singapore");
    expect(screen.getByText("请至少填写一项期望值。")).toBeVisible();
    await chooseOption(screen.getByRole("combobox", { name: "目标时区" }), "Asia/Singapore");
    expect(screen.getByRole("spinbutton", { name: "容许时间偏差（毫秒）" })).toHaveValue(null);
    await userEvent.click(screen.getByRole("button", { name: "保存 Profile" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("table", { name: "环境 Profiles" })).toHaveTextContent("Singapore");
    const saved = JSON.parse(calls.find((call) => call.command === "environment:save-profile")!.args[1]);
    expect(saved).toEqual({ name: "Singapore", target: { timezone: "Asia/Singapore" }, auto_sync: false });
    await userEvent.click(screen.getByRole("tab", { name: "对比与应用" }));
    expect(screen.getByRole("combobox", { name: "选择环境 Profile" })).toHaveAttribute("data-value", "");
    expect(calls.filter((call) => call.command === "environment:check")).toHaveLength(1);
    expect(calls.some((call) => call.command === "environment:sync-time")).toBe(false);
  });

  it("previews clock application and keeps a failed attempt available without retrying", async () => {
    const target = { ntp_server: "time.example.org", max_clock_offset_ms: 2000 };
    const { calls } = renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ builtins: [], items: [{ id: "profile", revision: "a".repeat(64), name: "Clock", target: environmentFixture(target).target, auto_sync: false }] }),
      "environment:check": makeEnvelope(environmentFixture()),
      "environment:sync-time": makeEnvelope({}, { ok: false, errors: [{ code: "FAILED", element: null, message: "Clock service unavailable" }] }),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择环境 Profile" }), "profile");
    await userEvent.click(screen.getByRole("button", { name: "应用 Profile" }));
    let dialog = screen.getByRole("dialog", { name: "应用环境 Profile" });
    expect(dialog).toHaveTextContent("time.example.org");
    expect(dialog).toHaveTextContent("2000 ms");
    expect(calls.some((call) => call.command === "environment:sync-time")).toBe(false);
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "应用 Profile" }));
    dialog = screen.getByRole("dialog", { name: "应用环境 Profile" });
    await userEvent.click(within(dialog).getByRole("button", { name: "校正系统时间" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Clock service unavailable");
    expect(calls.filter((call) => call.command === "environment:sync-time")).toHaveLength(1);
    expect(screen.getByRole("dialog", { name: "应用环境 Profile" })).toBeVisible();
  });

  it("duplicates criteria and deletes another profile without discarding the selected comparison", async () => {
    const mock = createEnvironmentMock();
    mock("environment:save-profile", ["--spec", JSON.stringify({ name: "Original", target: { region: "GB" } })]);
    const routes = Object.fromEntries(["check", "profiles", "save-profile", "delete-profile"].map((verb) => [`environment:${verb}`, async (args: string[]) => mock(`environment:${verb}`, args)]));
    const { calls } = renderWithEngine(<EnvironmentPage />, routes);
    await screen.findByText("已读取环境，尚未设置比较目标。");
    const select = screen.getByRole("combobox", { name: "选择环境 Profile" });
    const original = (await readOptions(select)).find((option) => option.label === "Original")!.value;
    await chooseOption(select, original);
    await userEvent.click(screen.getByRole("button", { name: "对比 Profile" }));
    await screen.findByText("已检查的目标全部符合。");
    await userEvent.click(screen.getByRole("tab", { name: "环境 Profiles" }));
    await userEvent.click(screen.getByRole("button", { name: "Profile 操作 Original" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "复制 Profile" }));
    expect(screen.getByRole("textbox", { name: "Profile 名称" })).toHaveValue("Original 副本");
    expect(screen.getByRole("textbox", { name: "目标地区" })).toHaveValue("GB");
    await userEvent.click(screen.getByRole("button", { name: "保存 Profile" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const save = calls.find((call) => call.command === "environment:save-profile")!;
    expect(save.args).not.toContain("--id");
    expect(save.args).not.toContain("--base");
    await userEvent.click(screen.getByRole("button", { name: "Profile 操作 Original 副本" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "删除 Profile" }));
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "删除 Profile" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await userEvent.click(screen.getByRole("tab", { name: "对比与应用" }));
    expect(screen.getByRole("combobox", { name: "选择环境 Profile" })).toHaveAttribute("data-value", original);
    expect(screen.getByRole("region", { name: "检查结果" })).toHaveTextContent("已检查的目标全部符合。");
    expect(calls.filter((call) => call.command === "environment:check")).toHaveLength(2);
  });

  it("can maintain definitions while the unrelated initial observation is pending", async () => {
    let finish!: (value: ReturnType<typeof makeEnvelope>) => void;
    const pending = new Promise<ReturnType<typeof makeEnvelope>>((resolve) => { finish = resolve; });
    renderWithEngine(<EnvironmentPage />, { "environment:check": async () => pending, "environment:profiles": makeEnvelope({ builtins: [], items: [] }) });
    await userEvent.click(screen.getByRole("tab", { name: "环境 Profiles" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "新增 Profile" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "新增 Profile" }));
    expect(screen.getByRole("textbox", { name: "Profile 名称" })).toBeEnabled();
    finish(makeEnvelope(environmentFixture()));
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await userEvent.click(screen.getByRole("tab", { name: "对比与应用" }));
    expect(await screen.findByText("已读取环境，尚未设置比较目标。")).toBeVisible();
  });

  it("prefills an application draft only after confirmation and explains missing clock criteria", async () => {
    let draft: LaunchRecipe | undefined;
    const target = { timezone: "Asia/Singapore", locale: "en_SG.UTF-8" };
    const { calls } = renderWithEngine(<EnvironmentPage onConfigureLaunch={(value) => { draft = value; }} />, {
      "environment:profiles": makeEnvelope({ builtins: [], items: [{ id: "sg", revision: "a".repeat(64), name: "Singapore", target: environmentFixture(target).target, auto_sync: false }] }),
      "environment:check": makeEnvelope(environmentFixture()),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择环境 Profile" }), "sg");
    await userEvent.click(screen.getByRole("button", { name: "应用 Profile" }));
    const dialog = screen.getByRole("dialog", { name: "应用环境 Profile" });
    expect(dialog).toHaveTextContent("Asia/Singapore");
    expect(dialog).toHaveTextContent("目标应用尚未选择");
    expect(draft).toBeUndefined();
    await chooseOption(within(dialog).getByRole("combobox", { name: "应用目标" }), "clock");
    expect(within(dialog).getByRole("button", { name: "校正系统时间" })).toBeDisabled();
    expect(within(dialog).getByRole("alert")).toHaveTextContent("校时需要此 Profile 填写时间服务器和偏差容限");
    await chooseOption(within(dialog).getByRole("combobox", { name: "应用目标" }), "launch");
    await userEvent.click(within(dialog).getByRole("button", { name: "生成启动配置" }));
    expect(draft).toMatchObject({ target: "", variants: [{ name: "environment", timezone: "Asia/Singapore" }], env: expect.arrayContaining([{ name: "LANG", value: "en_SG.UTF-8" }]) });
    expect(calls.map((call) => call.command).sort()).toEqual(["environment:check", "environment:profiles"]);
  });

  it("opens only the selected manual settings pane after preview confirmation", async () => {
    const target = { timezone: "Asia/Singapore", region: "SG" };
    const { calls } = renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ builtins: [], items: [{ id: "sg", revision: "a".repeat(64), name: "Singapore", target: environmentFixture(target).target, auto_sync: false }] }),
      "environment:check": makeEnvelope(environmentFixture()),
      "environment:open-settings": makeEnvelope({ pane: "language-region" }),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择环境 Profile" }), "sg");
    await userEvent.click(screen.getByRole("button", { name: "应用 Profile" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent("本次只打开系统设置");
    expect(calls.some((call) => call.command === "environment:open-settings")).toBe(false);
    await chooseOption(within(dialog).getByRole("combobox", { name: "应用目标" }), "language-region");
    expect(dialog).toHaveTextContent("SG");
    await userEvent.click(within(dialog).getByRole("button", { name: "打开语言与地区设置" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls[calls.length - 1]).toMatchObject({ command: "environment:open-settings", args: ["--pane", "language-region"] });
    expect(calls.some((call) => call.command === "environment:sync-time")).toBe(false);
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
  });
});
