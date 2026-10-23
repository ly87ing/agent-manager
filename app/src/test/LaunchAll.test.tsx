import { act, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import LaunchPage from "../pages/LaunchPage";
import type { LaunchRecipe, SavedLaunch } from "../lib/launch";
import { toast } from "../lib/notifications";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function saved(id: string, target = `/Applications/${id}.app`, platform: LaunchRecipe["platform"] = "macos", kind: LaunchRecipe["kind"] = "app"): SavedLaunch {
  const recipe: LaunchRecipe = { platform, shell: platform === "windows" ? "powershell" : platform === "linux" ? "bash" : "zsh", kind, target,
    args: ["--label", id], env: [{ name: "LANG", value: "en_US.UTF-8" }], variants: [{ name: id, timezone: "Asia/Singapore" }] };
  return { id, revision: id.padEnd(64, "a"), recipe, entries: recipe.variants.map(variant => ({ ...variant, command: "fixture command", definition: "fixture definition" })) };
}
const catalog = (platform: LaunchRecipe["platform"] = "macos") => makeEnvelope({ platform, shell: platform === "windows" ? "powershell" : "zsh", apps: [] });
const accepted = (audit_recorded = true) => makeEnvelope({ state: "requested", timezone_verified: false, audit_recorded });
const button = () => screen.getByTestId("launch-all");

describe("one-click saved application launching", () => {
  it.each(["macos", "linux", "windows"] as const)("%s launches only the first listed local app entry per literal path", async platform => {
    const path = platform === "windows" ? "C:\\Apps\\Example.exe" : platform === "linux" ? "/opt/example" : "/Applications/Example.app";
    const first = saved("first", path, platform), second = saved("second", path + "-other", platform);
    first.recipe.variants.push({ name: "alternate", timezone: "America/Los_Angeles" });
    first.entries.push({ ...first.entries[0], ...first.recipe.variants[1] });
    const duplicate = saved("duplicate", path, platform);
    const foreign = saved("foreign", second.recipe.target, platform === "windows" ? "macos" : "windows");
    const command = saved("command", path, platform, "command");
    const { calls } = renderWithEngine(<LaunchPage />, {
      "launch:catalog": catalog(platform), "launch:list": makeEnvelope({ items: [command, foreign, first, duplicate, second] }), "launch:run": accepted(),
    });
    await waitFor(() => expect(button()).toBeEnabled());
    expect(calls.some(call => call.command === "launch:run")).toBe(false);
    await userEvent.click(button());
    await screen.findByText("一键启动：已发送 2/2 个启动请求");
    const runs = calls.filter(call => call.command === "launch:run");
    expect(runs.map(call => call.args)).toEqual([first, second].map(item => ["--spec", JSON.stringify(item.recipe), "--name", item.entries[0].name, "--confirm"]));
    expect(new Set(runs.map(call => call.options?.runId)).size).toBe(2);
    expect(calls.some(call => ["launch:save", "launch:delete"].includes(call.command))).toBe(false);
    expect(document.querySelectorAll('[data-sonner-toast]')).toHaveLength(1);
  });

  it("keeps the whole batch locked and sequential across double clicks and single-item actions", async () => {
    let finishFirst!: () => void, finishSecond!: () => void;
    const firstGate = new Promise<void>(resolve => { finishFirst = resolve; });
    const secondGate = new Promise<void>(resolve => { finishSecond = resolve; });
    let runs = 0;
    const { calls } = renderWithEngine(<LaunchPage />, {
      "launch:catalog": catalog(), "launch:list": makeEnvelope({ items: [saved("first"), saved("second")] }),
      "launch:run": async () => { await (++runs === 1 ? firstGate : secondGate); return accepted(); },
    });
    await waitFor(() => expect(button()).toBeEnabled());
    await userEvent.dblClick(button());
    expect(runs).toBe(1);
    expect(button()).toHaveTextContent("启动中（0/2）");
    for (const name of ["重新读取", "新增配置"]) expect(screen.getByRole("button", { name })).toBeDisabled();
    const single = screen.getByTestId("run-saved-first-first");
    expect(single).toBeDisabled(); await userEvent.click(single); expect(runs).toBe(1);
    await act(async () => { finishFirst(); });
    await waitFor(() => expect(runs).toBe(2));
    expect(button()).toBeDisabled(); expect(button()).toHaveTextContent("启动中（1/2）");
    expect(document.querySelector('[data-sonner-toast]')).toBeNull();
    await act(async () => { finishSecond(); });
    await screen.findByText("一键启动：已发送 2/2 个启动请求");
    expect(button()).toBeEnabled(); expect(calls.filter(call => call.command === "launch:run")).toHaveLength(2);
  });

  it("continues after refusal and reports accepted requests with audit warnings once", async () => {
    let runs = 0;
    const reason = "The application is already running. Quit it normally before launching with a different time zone";
    const { calls } = renderWithEngine(<LaunchPage />, {
      "launch:catalog": catalog(), "launch:list": makeEnvelope({ items: [saved("first"), saved("second"), saved("third")] }),
      "launch:run": async () => ++runs === 1 ? makeEnvelope({}, { ok: false, errors: [{ code: "LAUNCH_ERROR", element: null, message: reason }] }) : accepted(runs !== 2),
    });
    await waitFor(() => expect(button()).toBeEnabled()); await userEvent.click(button());
    await screen.findByText("一键启动：已发送 2/3 个启动请求");
    const warning = document.querySelector('[data-sonner-toast][data-type="warning"]');
    expect(warning).toHaveTextContent(reason); expect(warning).toHaveTextContent("/Applications/second.app · second");
    expect(warning).toHaveTextContent("启动请求已发送，但记录失败"); expect(warning).toHaveTextContent("请先确认应用状态，再决定是否重试。");
    expect(document.querySelectorAll('[data-sonner-toast]')).toHaveLength(1);
    expect(calls.filter(call => call.command === "launch:run")).toHaveLength(3);
  });

  it("blocks bulk launch while a single saved application request is pending", async () => {
    let finish!: () => void; const gate = new Promise<void>(resolve => { finish = resolve; });
    const { calls } = renderWithEngine(<LaunchPage />, { "launch:catalog": catalog(), "launch:list": makeEnvelope({ items: [saved("first")] }),
      "launch:run": async () => { await gate; return accepted(); } });
    await waitFor(() => expect(button()).toBeEnabled());
    await userEvent.click(screen.getByTestId("run-saved-first-first"));
    expect(button()).toBeDisabled(); await userEvent.click(button());
    expect(calls.filter(call => call.command === "launch:run")).toHaveLength(1);
    await act(async () => { finish(); }); await waitFor(() => expect(button()).toBeEnabled());
  });

  it("does not report success when every request is refused", async () => {
    renderWithEngine(<LaunchPage />, { "launch:catalog": catalog(), "launch:list": makeEnvelope({ items: [saved("first")] }),
      "launch:run": makeEnvelope({}, { ok: false, errors: [{ code: "LAUNCH_ERROR", element: null, message: "Target unavailable" }] }) });
    await waitFor(() => expect(button()).toBeEnabled()); await userEvent.click(button());
    await screen.findByText("一键启动：已发送 0/1 个启动请求");
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
    expect(document.querySelector('[data-sonner-toast][data-type="warning"]')).toHaveTextContent("Target unavailable");
  });

  it("stops unissued requests and stale feedback when the page unmounts", async () => {
    let finish!: () => void; const gate = new Promise<void>(resolve => { finish = resolve; });
    const success = vi.spyOn(toast, "success"), warning = vi.spyOn(toast, "warning");
    const view = renderWithEngine(<LaunchPage />, { "launch:catalog": catalog(), "launch:list": makeEnvelope({ items: [saved("first"), saved("second")] }), "launch:run": async () => { await gate; return accepted(); } });
    await waitFor(() => expect(button()).toBeEnabled()); await userEvent.click(button()); view.unmount();
    await act(async () => { finish(); });
    expect(view.calls.filter(call => call.command === "launch:run")).toHaveLength(1);
    expect(success).not.toHaveBeenCalled(); expect(warning).not.toHaveBeenCalled(); success.mockRestore(); warning.mockRestore();
  });

  it.each(["catalog", "list"])("waits for %s before enabling bulk launch", async operation => {
    let finish!: () => void; const gate = new Promise<void>(resolve => { finish = resolve; });
    const { calls } = renderWithEngine(<LaunchPage />, {
      "launch:catalog": async () => { if (operation === "catalog") await gate; return catalog(); },
      "launch:list": async () => { if (operation === "list") await gate; return makeEnvelope({ items: [saved("first")] }); },
      "launch:run": accepted(),
    });
    expect(button()).toBeDisabled(); await userEvent.click(button()); expect(calls.some(call => call.command === "launch:run")).toBe(false);
    await act(async () => { finish(); }); await waitFor(() => expect(button()).toBeEnabled());
  });

  it("disables bulk launch for an empty or unsupported list", async () => {
    renderWithEngine(<LaunchPage />, { "launch:catalog": catalog(), "launch:list": makeEnvelope({ items: [saved("foreign", "/app", "linux"), saved("command", "/command", "macos", "command")] }) });
    await screen.findByRole("button", { name: "编辑配置 command" }); expect(button()).toBeDisabled(); expect(button()).toHaveTextContent("一键启动（0）");
  });

  it.each(["catalog", "list"])("does not launch after a %s read failure", async operation => {
    const failure = makeEnvelope({}, { ok: false, errors: [{ code: "READ_ERROR", element: null, message: "Read failed" }] });
    const { calls } = renderWithEngine(<LaunchPage />, {
      "launch:catalog": operation === "catalog" ? failure : catalog(),
      "launch:list": operation === "list" ? failure : makeEnvelope({ items: [saved("first")] }),
    });
    await screen.findByText("Read failed"); expect(button()).toBeDisabled(); await userEvent.click(button());
    expect(calls.some(call => call.command === "launch:run")).toBe(false);
  });
});
