import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import EnvironmentPage from "../pages/EnvironmentPage";
import { setLanguage } from "../lib/i18n";
import type { EnvironmentProfile, EnvironmentTarget } from "../lib/environment";
import { environmentFixture } from "./environment-fixture";
import { createEnvironmentMock } from "./environment-mock";
import { makeEnvelope } from "./fixtures";
import { chooseOption, renderWithEngine, screen, userEvent, within } from "./render-with-engine";

function collection(target: EnvironmentTarget = {}, auto_sync = false): EnvironmentProfile {
  return { id: "sample", revision: "a".repeat(64), name: "Example collection", target: environmentFixture(target).target, auto_sync };
}
function setup(item?: EnvironmentProfile, synchronized: boolean | null = true) {
  return renderWithEngine(<EnvironmentPage />, {
    "environment:profiles": makeEnvelope({ items: item ? [item] : [] }),
    "environment:check": async (args) => makeEnvelope(environmentFixture(JSON.parse(args[args.indexOf("--spec") + 1]), args.includes("--probe-time"), synchronized)),
  });
}

describe("Environment checks", () => {
  it("announces matching targets immediately while disclosing untargeted unknowns, then clears stale feedback", async () => {
    const view = setup(collection({ region: "GB" }));
    await screen.findByText("已读取环境，尚未设置比较目标。");
    expect(document.querySelector('[data-sonner-toast]')).toBeNull();
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    await waitFor(() => expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toHaveTextContent("检查完成：所选目标全部符合。"));
    const toast = document.querySelector('[data-sonner-toast]')!;
    expect(toast).toHaveTextContent("符合 1 项，不符 0 项，无法确认 0 项。");
    expect(toast).toHaveTextContent("未设目标且无法确认：后台实际授时来源、自动设置时间、参考时间偏差");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "");
    await waitFor(() => expect(document.querySelector('[data-sonner-toast]:not([data-removed="true"])')).toBeNull());
    view.unmount();
  });

  it("names mismatches and unknown targets in the completion toast and persistent summary", async () => {
    setup(collection({ timezone: "Asia/Tokyo", region: "US", automatic_time: true }));
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    await waitFor(() => expect(document.querySelector('[data-sonner-toast][data-type="warning"]')).toHaveTextContent("检查完成：3 项不符，1 项无法确认。"));
    for (const result of [document.querySelector('[data-sonner-toast]')!, screen.getByRole("region", { name: "检查结果" })]) {
      expect(result).toHaveTextContent("不符项目：系统时区、地区、进程实际时区");
      expect(result).toHaveTextContent("目标中无法确认：自动设置时间");
    }
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
  });

  it.each([[{}, "info", "检查完成：尚未设置比较目标。"], [{ automatic_time: true }, "warning", "检查完成：0 项不符，1 项无法确认。"]] as const)("does not claim all-clear for inventory or unknown targets (%s)", async (target, level, title) => {
    setup(collection(target));
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    await waitFor(() => expect(document.querySelector(`[data-sonner-toast][data-type="${level}"]`)).toHaveTextContent(title));
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
  });

  it("announces a failed explicit check and replaces the error with a successful retry", async () => {
    let calls = 0;
    renderWithEngine(<EnvironmentPage />, { "environment:check": async () => makeEnvelope(++calls === 2 ? { checks: [] } : environmentFixture()) });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await userEvent.click(screen.getByRole("button", { name: "检查本机环境" }));
    await waitFor(() => expect(document.querySelector('[data-sonner-toast][data-type="error"]')).toHaveTextContent("环境检查失败"));
    expect(document.querySelector('[data-sonner-toast]')).toHaveTextContent(screen.getByRole("alert").textContent!.replace("重试检查", "").replace(/\s+/g, " "));
    await userEvent.click(screen.getByRole("button", { name: "重试检查" }));
    await waitFor(() => expect(document.querySelector('[data-sonner-toast][data-type="info"]')).toHaveTextContent("尚未设置比较目标"));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("does not announce successful correction when the new sample matches after a failed correction", async () => {
    const target = { ntp_server: "pool.ntp.org", max_clock_offset_ms: 2000 };
    const report = environmentFixture(target, true);
    report.time_repair = { status: "failed", before_offset_ms: 3000, before_uncertainty_ms: 20 };
    renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ items: [collection(target, true)] }),
      "environment:check": makeEnvelope(environmentFixture()), "environment:sync-time": makeEnvelope(report),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    await waitFor(() => expect(document.querySelector('[data-sonner-toast][data-type="warning"]')).toHaveTextContent("检查完成，校时未确认成功。"));
    expect(document.querySelector('[data-sonner-toast]')).toHaveTextContent("校时未成功，请检查管理员授权和网络连接。");
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
  });

  it("opens local inventory and collections without network or writes, then compares one selected collection", async () => {
    const { calls } = setup(collection({ timezone: "Asia/Tokyo" }));
    await screen.findByText("已读取环境，尚未设置比较目标。");
    expect(calls.map((call) => call.command).sort()).toEqual(["environment:check", "environment:profiles"]);
    expect(calls.find((call) => call.command === "environment:check")!.args).not.toContain("--probe-time");
    expect(screen.queryByRole("textbox", { name: "目标地区" })).not.toBeInTheDocument();
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    expect(screen.queryByText("已读取环境，尚未设置比较目标。")).not.toBeInTheDocument();
    expect(calls).toHaveLength(2);
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    await screen.findByText("发现与目标不符的项目。");
    expect(document.querySelector('[data-check-id="time.timezone"]')).toHaveTextContent("不符");
  });

  it.each([[true, "符合"], [false, "不符"], [null, "无法确认"]] as const)("compares kernel synchronization (%s) independently", async (synchronized, state) => {
    const { calls } = setup(collection({ time_synchronized: true }), synchronized);
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    await screen.findByText("内核时间同步状态");
    expect(document.querySelector('[data-check-id="ntp.synchronized"]')).toHaveTextContent(state);
    expect(calls.every((call) => !call.args.includes("--probe-time"))).toBe(true);
    await userEvent.click(screen.getByRole("checkbox", { name: "显示取值来源与说明" }));
    expect(screen.getByText("内核报告的同步状态，不提供最近同步时间或后台采用的服务器。实际偏差请比较参考时间。")).toBeInTheDocument();
  });

  it("creates with explicit prefill, edits, reopens and deletes a reusable collection", async () => {
    const mock = createEnvironmentMock();
    const routes = Object.fromEntries(["check", "profiles", "save-profile", "delete-profile"].map((verb) => [`environment:${verb}`, async (args: string[]) => mock(`environment:${verb}`, args)]));
    let view = renderWithEngine(<EnvironmentPage />, routes);
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await userEvent.click(screen.getByRole("button", { name: "新增期望集合" }));
    expect(screen.getByRole("textbox", { name: "目标地区" })).toHaveValue("");
    await userEvent.type(screen.getByRole("textbox", { name: "集合名称" }), "My environment");
    await userEvent.click(screen.getByRole("button", { name: "用当前值填写目标" }));
    expect(screen.getByRole("textbox", { name: "目标地区" })).toHaveValue("GB");
    expect(screen.getByRole("combobox", { name: "目标时间同步状态" })).toHaveAttribute("data-value", "true");
    await userEvent.click(screen.getByRole("button", { name: "保存期望集合" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(view.calls.filter((call) => call.command === "environment:check")).toHaveLength(1);
    const savedId = screen.getByRole("combobox", { name: "选择期望集合" }).getAttribute("data-value")!;
    await userEvent.click(screen.getByRole("button", { name: "编辑期望集合" }));
    await userEvent.clear(screen.getByRole("textbox", { name: "集合名称" }));
    await userEvent.type(screen.getByRole("textbox", { name: "集合名称" }), "Renamed environment");
    await userEvent.click(screen.getByRole("button", { name: "保存期望集合" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(view.calls.filter((call) => call.command === "environment:save-profile")[1].args).toContain("--base");
    view.unmount(); view = renderWithEngine(<EnvironmentPage />, routes);
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), savedId);
    expect(screen.getByRole("combobox", { name: "选择期望集合" })).toHaveTextContent("Renamed environment");
    await userEvent.click(screen.getByRole("button", { name: "删除期望集合" }));
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "删除期望集合" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(screen.getByRole("combobox", { name: "选择期望集合" })).toHaveAttribute("data-value", "");
  });

  it("discloses the server, measures on demand and offers correction only for a clear clock mismatch", async () => {
    const { calls } = setup(collection({ ntp_server: "time.example.org", max_clock_offset_ms: 10 }));
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    expect(screen.getByText("检查会向 time.example.org 的 UDP 123 端口查询参考时间。")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "立即校时（系统授权）" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    await screen.findByText(/查询地址：198.18.0.1/);
    expect(calls[calls.length - 1].args).toContain("--probe-time");
    expect(screen.getByRole("button", { name: "立即校时（系统授权）" })).toBeDisabled();
    expect(calls.some((call) => call.command === "environment:sync-time")).toBe(false);
  });

  it.each(["verified", "cancelled", "unverified", "unavailable"] as const)("runs opted-in correction and renders the %s outcome", async (status) => {
    const item = collection({ ntp_server: "time.example.org", max_clock_offset_ms: 10 }, true);
    const report = environmentFixture({ ntp_server: "time.example.org", max_clock_offset_ms: 10 }, true);
    report.time_repair = { status, before_offset_ms: 3000, before_uncertainty_ms: 20 };
    if (status === "verified") {
      const clock = report.checks.find((row) => row.id === "ntp.clock_offset")!;
      clock.state = "match"; clock.observed = "+0.0 ms ± 1.0 ms";
      report.time_sample!.offset_ms = 0; report.time_sample!.uncertainty_ms = 1;
      report.summary.matched++; report.summary.mismatched--;
    }
    const { calls } = renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ items: [item] }), "environment:check": makeEnvelope(environmentFixture()), "environment:sync-time": makeEnvelope(report),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    expect(screen.getByText(/此集合已启用自动校时/)).toBeInTheDocument();
    expect(calls.some((call) => call.command === "environment:sync-time")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    const messages = { verified: "系统时间已校正，复测偏差符合目标。", cancelled: "已取消管理员授权，未完成校时。", unverified: "校时命令已完成，但复测尚未确认符合目标。", unavailable: "无法可靠判断时间偏差，未执行校时。" };
    await screen.findByText(messages[status]);
    expect(calls[calls.length - 1].command).toBe("environment:sync-time"); expect(calls[calls.length - 1].args).toContain("--confirm");
    if (status !== "verified") expect(screen.queryByText(messages.verified)).not.toBeInTheDocument();
  });

  it("retains correction evidence and warns when its completion audit fails", async () => {
    const target = { ntp_server: "pool.ntp.org", max_clock_offset_ms: 10 };
    const report = environmentFixture(target, true);
    report.time_repair = { status: "verified", before_offset_ms: 3000, before_uncertainty_ms: 20, audit_recorded: false };
    const clock = report.checks.find((row) => row.id === "ntp.clock_offset")!;
    clock.state = "match"; clock.observed = "+0.0 ms ± 1.0 ms";
    report.time_sample!.offset_ms = 0; report.time_sample!.uncertainty_ms = 1;
    report.summary.matched++; report.summary.mismatched--; report.summary.status = "match";
    renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ items: [collection(target, true)] }),
      "environment:check": makeEnvelope(environmentFixture()), "environment:sync-time": makeEnvelope(report),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("校时结果未能写入操作记录，本页保留复测结果。");
    expect(screen.getByText("系统时间已校正，复测偏差符合目标。")).toBeVisible();
    expect(document.querySelector('[data-sonner-toast][data-type="warning"]')).toHaveTextContent("校时结果未能写入操作记录");
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
  });

  it("prevents saving an incomplete automatic-correction policy", async () => {
    const { calls } = setup(); await screen.findByText("已读取环境，尚未设置比较目标。");
    await userEvent.click(screen.getByRole("button", { name: "新增期望集合" }));
    await userEvent.type(screen.getByRole("textbox", { name: "集合名称" }), "Example");
    await userEvent.click(screen.getByRole("checkbox", { name: "检查发现时间偏差超限时自动校时" }));
    expect(screen.getByRole("button", { name: "保存期望集合" })).toBeDisabled();
    expect(calls.some((call) => call.command === "environment:save-profile")).toBe(false);
  });

  it("preserves edits when saving fails and recovers an unreadable collection list", async () => {
    let reads = 0;
    renderWithEngine(<EnvironmentPage />, {
      "environment:check": makeEnvelope(environmentFixture()),
      "environment:profiles": async () => makeEnvelope(++reads === 1 ? { items: [{}] } : { items: [] }),
      "environment:save-profile": makeEnvelope({}, { ok: false, errors: [{ code: "CONFLICT", element: null, message: "Collection changed; reload before editing." }] }),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    expect(screen.getByRole("button", { name: "新增期望集合" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "重新读取集合" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "新增期望集合" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "新增期望集合" }));
    await userEvent.type(screen.getByRole("textbox", { name: "集合名称" }), "Preserve this");
    await userEvent.type(screen.getByRole("textbox", { name: "目标地区" }), "US");
    await userEvent.click(screen.getByRole("button", { name: "保存期望集合" }));
    expect(await within(screen.getByRole("dialog")).findByRole("alert")).toHaveTextContent("Collection changed; reload before editing.");
    expect(screen.getByRole("textbox", { name: "集合名称" })).toHaveValue("Preserve this");
    expect(screen.getByRole("textbox", { name: "目标地区" })).toHaveValue("US");
    expect(screen.getByRole("button", { name: "保存期望集合" })).toBeEnabled();
  });

  it("reports malformed results and allows retry without showing success", async () => {
    let attempt = 0;
    renderWithEngine(<EnvironmentPage />, { "environment:check": async () => makeEnvelope(++attempt === 1 ? { checks: [] } : environmentFixture()) });
    await userEvent.click(await screen.findByRole("button", { name: "重试检查" }));
    await screen.findByText("已读取环境，尚未设置比较目标。");
  });

  it("corrects a measured mismatch only after the explicit manual action", async () => {
    const target = { ntp_server: "time.example.org", max_clock_offset_ms: 10 };
    const corrected = environmentFixture(target, true);
    corrected.time_repair = { status: "cancelled", before_offset_ms: 20, before_uncertainty_ms: 5 };
    const { calls } = renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ items: [collection(target)] }),
      "environment:check": async (args) => makeEnvelope(environmentFixture(args.includes("--probe-time") ? target : {}, args.includes("--probe-time"))),
      "environment:sync-time": makeEnvelope(corrected),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    const correction = await screen.findByRole("button", { name: "立即校时（系统授权）" });
    expect(calls.some((call) => call.command === "environment:sync-time")).toBe(false);
    await waitFor(() => expect(correction).toBeEnabled(), { timeout: 5000 });
    await userEvent.click(correction);
    await screen.findByText("已取消管理员授权，未完成校时。");
    expect(calls[calls.length - 1].command).toBe("environment:sync-time");
  }, 10000);

  it("renders collections and the editor in English", async () => {
    setLanguage("en"); setup(); await screen.findByText("Environment read. No comparison targets selected.");
    expect(screen.getByRole("combobox", { name: "Choose expected-value collection" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check with administrator authorization" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Add expected-value collection" }));
    expect(screen.getByRole("button", { name: "Save expected-value collection" })).toBeInTheDocument();
    expect(screen.getByText("Save criteria only")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Correct time automatically when a check finds excessive clock offset" })).toBeInTheDocument();
  });

  it("shows remediation without enabling source details and opens only the requested settings pane", async () => {
    const target = { timezone: "Asia/Tokyo", region: "JP" };
    const { calls } = renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ items: [collection(target)] }),
      "environment:check": async (args) => makeEnvelope(environmentFixture(JSON.parse(args[args.indexOf("--spec") + 1]))),
      "environment:open-settings": async (args) => makeEnvelope({ pane: args[1] }),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    expect(screen.getByText("本次无法可靠读取后台当前采用的授时节点。配置文件和单次查询不能代替此项证据。")).toBeInTheDocument();
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    await screen.findByText("发现与目标不符的项目。");
    expect(screen.getByRole("checkbox", { name: "显示取值来源与说明" })).not.toBeChecked();
    const time = document.querySelector('[data-check-id="time.timezone"]') as HTMLElement;
    expect(time).toHaveTextContent("影响范围：本机系统");
    await userEvent.click(within(time).getByRole("button", { name: "打开日期与时间设置" }));
    await screen.findByText("已请求打开系统设置。完成调整后，请重新检查。");
    expect(calls[calls.length - 1]).toMatchObject({ command: "environment:open-settings", args: ["--pane", "date-time"] });
    const region = document.querySelector('[data-check-id="language.region"]') as HTMLElement;
    await userEvent.click(within(region).getByRole("button", { name: "打开语言与地区设置" }));
    expect(calls[calls.length - 1]).toMatchObject({ command: "environment:open-settings", args: ["--pane", "language-region"] });
    expect(calls.some((call) => call.command === "environment:sync-time")).toBe(false);
  });

  it("explains shadowed raw-locale differences without claiming an effective locale failure", async () => {
    const target = { locale: "en_GB.UTF-8", strict_locale: true };
    const report = environmentFixture(target);
    const raw = report.checks.find((row) => row.id === "process.raw_locale")!;
    raw.state = "mismatch"; raw.observed = '{"LANG":"C.UTF-8"}';
    report.summary.matched--; report.summary.mismatched++; report.summary.status = "mismatch";
    let draft: unknown;
    renderWithEngine(<EnvironmentPage onConfigureLaunch={(recipe) => { draft = recipe; }} />, {
      "environment:profiles": makeEnvelope({ items: [collection(target)] }),
      "environment:check": async (args) => makeEnvelope(JSON.parse(args[args.indexOf("--spec") + 1]).locale ? report : environmentFixture()),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    await screen.findByText("实际生效的 locale 符合目标，但已设置的原始变量存在差异。");
    await userEvent.click(within(document.querySelector('[data-check-id="process.raw_locale"]') as HTMLElement).getByRole("button", { name: "创建启动配置" }));
    expect(draft).toMatchObject({ target: "", env: expect.arrayContaining([{ name: "LANG", value: "en_GB.UTF-8" }, { name: "LC_ALL", value: "en_GB.UTF-8" }]) });
  });

  it.each(["read", "cancelled", "failed"] as const)("keeps privileged inspection separate from clock correction (%s)", async (status) => {
    const target = { automatic_time: true };
    const updated = environmentFixture(target);
    const row = updated.checks.find((item) => item.id === "ntp.automatic_time")!;
    row.state = "match"; row.observed = "true"; row.note = "automatic_time";
    updated.summary.unknown--; updated.summary.matched++; updated.summary.status = "match";
    const { calls } = renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ items: [collection(target)] }),
      "environment:check": async (args) => makeEnvelope(environmentFixture(JSON.parse(args[args.indexOf("--spec") + 1]))),
      "environment:check-time-settings": makeEnvelope({ status, report: status === "read" ? updated : null }),
    });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    await chooseOption(screen.getByRole("combobox", { name: "选择期望集合" }), "sample");
    await userEvent.click(screen.getByRole("button", { name: "检查所选集合" }));
    await screen.findByText("仍有目标项目无法确认。");
    const previous = document.querySelector('[data-check-id="ntp.automatic_time"]')!.textContent;
    await userEvent.click(screen.getByRole("button", { name: "授权后重新检查" }));
    const dialog = screen.getByRole("dialog", { name: "授权后重新检查" });
    expect(dialog).toHaveTextContent("/usr/sbin/systemsetup -getusingnetworktime");
    expect(dialog).toHaveTextContent("只读时间设置");
    expect(calls.some((call) => call.command === "environment:check-time-settings")).toBe(false);
    await userEvent.click(within(dialog).getByRole("button", { name: "继续只读检查" }));
    const messages = { read: "已读取自动设置时间开关并更新检查结果，未修改系统设置。", cancelled: "已取消授权，保留原检查结果。", failed: "未能读取自动设置时间开关，保留原检查结果。可在系统设置中确认。" };
    await waitFor(() => expect(document.querySelector('[data-sonner-toast]')).toHaveTextContent(messages[status]));
    await waitFor(() => expect(document.querySelector(`[data-sonner-toast][data-type="${status === "read" ? "success" : status === "cancelled" ? "info" : "error"}"]`)).not.toBeNull());
    expect(calls[calls.length - 1]).toMatchObject({ command: "environment:check-time-settings", args: ["--spec", expect.any(String), "--confirm"] });
    expect(JSON.parse(calls[calls.length - 1].args[1])).toMatchObject(target);
    expect(calls.some((call) => call.command === "environment:sync-time")).toBe(false);
    const after = document.querySelector('[data-check-id="ntp.automatic_time"]') as HTMLElement;
    if (status === "read") expect(after).toHaveTextContent("符合"); else expect(after.textContent).toEqual(previous);
  });
});
