import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import EnvironmentPage from "../pages/EnvironmentPage";
import { EnvironmentRepairDialog } from "../components/EnvironmentRepairDialog";
import { setLanguage } from "../lib/i18n";
import type { EnvironmentRepairPlan, EnvironmentRepairResult } from "../lib/environment";
import { environmentFixture } from "./environment-fixture";
import { makeEnvelope } from "./fixtures";
import { chooseOption, renderWithEngine, screen, userEvent, within } from "./render-with-engine";

const target = { region: "US", locale: "en_US.UTF-8", strict_locale: true };
const profile = { id: "sample", name: "Example standard", revision: "b".repeat(64), target: environmentFixture(target).target, auto_sync: false };
const plan: EnvironmentRepairPlan = { base: "a".repeat(64), target: profile.target, session_scope: "uid:1000 launchd:10", items: [
  { id: "language.region", scope: "user", before: "en_GB", expected: "en_US", status: "change" },
  { id: "session.LC_CTYPE", scope: "session", before: "C.UTF-8", expected: "en_US.UTF-8", status: "change" },
] };
function result(status: EnvironmentRepairResult["status"] = "applied"): EnvironmentRepairResult {
  return { status, items: plan.items.map((item) => ({ ...item, status: status === "applied" ? "verified" : "not_applied" })),
    report: environmentFixture(target), restart_required: status === "applied", clock_error: null };
}
function setup(outcome = result()) {
  return renderWithEngine(<EnvironmentPage />, {
    "environment:profiles": makeEnvelope({ checks_supported: true, builtins: [], items: [profile] }),
    "environment:check": async (args) => makeEnvelope(environmentFixture(JSON.parse(args[1]))),
    "environment:repair-preview": makeEnvelope(plan), "environment:repair": makeEnvelope(outcome),
  });
}
async function open() {
  await screen.findByText("已读取环境，尚未设置比较目标。");
  await chooseOption(screen.getByRole("combobox", { name: "选择环境 Profile" }), profile.id);
  await userEvent.click(screen.getByRole("button", { name: "自动修复" }));
  const dialog = await screen.findByRole("dialog", { name: "自动修复环境" });
  await within(dialog).findByText("en_GB → en_US");
  return dialog;
}

describe("Automatic environment repair", () => {
  it.each(["en", "zh-CN"] as const)("separates Linux runtime and saved NTP receipts and retains limits in %s", async (language) => {
    setLanguage(language);
    const configured = { ...profile, target: environmentFixture({ ntp_server: "selected.example", max_clock_offset_ms: 2000 }).target };
    const preview = { ...plan, platform: "linux" as const, target: configured.target, items: [
      { id: "ntp.server", scope: "system" as const, before: "selected.example", expected: "selected.example", status: "unchanged" as const },
      { id: "ntp.server.directory", scope: "system" as const, before: false, expected: true, status: "change" as const },
      { id: "ntp.server.persistent", scope: "system" as const, before: null, expected: "selected.example", status: "change" as const },
    ] };
    renderWithEngine(<EnvironmentRepairDialog profile={configured} onResult={() => {}} onClose={() => {}} />, {
      "environment:repair-preview": makeEnvelope(preview),
      "environment:repair": makeEnvelope({ ...result(), items: preview.items.map(item => ({ ...item, status: item.status === "unchanged" ? "unchanged" : "verified" })), restart_required: false,
        verification_limits: ["Future service loading, actual peer, synchronization and clock accuracy remain unverified."] }),
    });
    await screen.findByText(language === "en" ? "Save the standard-host-path NTP override file" : "保存标准主机路径的 NTP 配置文件");
    expect(screen.getByRole("dialog")).toHaveTextContent("/etc/systemd/timesyncd.conf.d/90-agent-manager.conf");
    expect(screen.getByRole("dialog")).toHaveTextContent(language === "en" ? "adjust the system clock immediately" : "立即查询服务器并调整系统时钟");
    expect(screen.getByRole("dialog")).toHaveTextContent(language === "en" ? "Absent → create directory" : "不存在 → 创建目录");
    expect(screen.queryByText(language === "en" ? /Only correct the clock after a fresh/ : /仅在新测量明确超限时校正时钟/)).toBeNull();
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await screen.findByText(language === "en" ? "File saved and read back" : "文件已保存并读回");
    expect(screen.getByRole("dialog")).toHaveTextContent(language === "en" ? "Whether a future instance loads this host-path file remains unverified" : "以后启动的实例是否加载此主机路径文件仍未验证");
    expect(screen.getByRole("status")).toHaveTextContent(language === "en" ? "Verify their effect in the target environment" : "实际效果仍需在目标环境中确认");
  });
  it("requires a fresh explicit Windows persistence preview and forwards that choice only on confirm", async () => {
    const { calls }=renderWithEngine(<EnvironmentRepairDialog profile={profile} onResult={() => {}} onClose={() => {}} />, {
      "environment:repair-preview": async (args) => makeEnvelope({ ...plan, platform:"windows",base:(args.includes("--allow-persistent-user-env") ? "c" : "a").repeat(64) }),
      "environment:repair": makeEnvelope(result()),
    });
    const checkbox=await screen.findByRole("checkbox",{name:"允许写入 Windows 当前用户的持久环境变量（新进程生效）"});
    expect(checkbox).not.toBeChecked();
    expect(calls.find((call)=>call.command==="environment:repair-preview")!.args).not.toContain("--allow-persistent-user-env");
    await userEvent.click(checkbox);
    await waitFor(()=>expect(calls.filter((call)=>call.command==="environment:repair-preview")).toHaveLength(2));
    await waitFor(()=>expect(screen.getByTestId("confirm-modal-confirm")).toBeEnabled());
    expect(calls.filter((call)=>call.command==="environment:repair")).toHaveLength(0);
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(()=>expect(calls.filter((call)=>call.command==="environment:repair")).toHaveLength(1));
    const request=calls.find((call)=>call.command==="environment:repair")!;
    expect(request.args).toContain("--allow-persistent-user-env");
    expect(request.args[request.args.indexOf("--base")+1]).toBe("c".repeat(64));
    expect(screen.getByRole("dialog")).toHaveTextContent("不会在退出登录时清除");
  });
  it("discloses the Linux user-manager lifetime without claiming a whole-desktop environment", async () => {
    renderWithEngine(<EnvironmentRepairDialog profile={profile} onResult={() => {}} onClose={() => {}} />, {
      "environment:repair-preview": makeEnvelope({ ...plan, platform:"linux" }),
    });
    expect(await screen.findByText("Linux 会话变量仅供用户服务管理器以后启动的进程使用，管理器退出后失效；已有桌面应用不受影响。")).toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).toBeNull();
  });
  it.each(["en", "zh-CN"] as const)("keeps GNOME preference verification distinct from geolocation in %s", async (language) => {
    setLanguage(language);
    const configured = { ...profile, target: environmentFixture({ automatic_timezone: true }).target };
    const preview = { ...plan, platform: "linux" as const, target: configured.target, items: [
      { id: "time.automatic_timezone", scope: "user" as const, before: false, expected: true, status: "change" as const },
    ] };
    renderWithEngine(<EnvironmentRepairDialog profile={configured} onResult={() => {}} onClose={() => {}} />, {
      "environment:repair-preview": makeEnvelope(preview),
      "environment:repair": makeEnvelope({ ...result(), items: preview.items.map(item => ({ ...item, status: "verified" })),
        restart_required: false, verification_limits: ["GNOME geolocation and automatic switching remain unverified."] }),
    });
    await screen.findByText(language === "en" ? /GNOME automatic time zone is verified by preference readback only/ : /GNOME 自动时区仅验证偏好回读/);
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await screen.findByText(language === "en" ? "Preference saved and read back" : "偏好已保存并读回");
    expect(screen.getByRole("status")).toHaveTextContent(language === "en" ? "Verify their effect in the target environment" : "实际效果仍需在目标环境中确认");
    expect(screen.queryByText(language === "en" ? "Repaired and verified" : "已修复并验证")).toBeNull();
    expect(screen.queryByText(language === "en" ? /^Save your work and restart the affected apps/ : /^请保存工作并重新启动相关应用/)).toBeNull();
    expect(screen.queryByText(language === "en" ? /Save your work, restart them/ : /请保存工作后重新启动/)).toBeNull();
    expect(screen.queryByText(language === "en" ? /Environment variable scope:/ : /环境变量范围：/)).toBeNull();
  });
  it("previews the selected criteria and cancellation performs no repair or correction", async () => {
    const { calls } = setup();
    expect(screen.getByRole("button", { name: "自动修复" })).toBeDisabled();
    const dialog = await open();
    expect(dialog).toHaveTextContent("退出登录后失效");
    expect(dialog).toHaveTextContent("已有应用和终端不会被退出或改写");
    const preview = calls.find((call) => call.command === "environment:repair-preview")!;
    expect(JSON.parse(preview.args[1])).toEqual({ languages: [], ...target });
    expect(preview.args).not.toContain("--confirm");
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(calls.filter((call) => ["environment:repair", "environment:sync-time", "launch:run"].includes(call.command))).toHaveLength(0);
  });
  it.each(["en", "zh-CN"] as const)("explains GNOME formats readback and a user-controlled new login in %s", async (language) => {
    setLanguage(language);
    const configured = { ...profile, target: environmentFixture({ region: "US" }).target };
    const preview = { ...plan, platform: "linux" as const, target: configured.target, items: [plan.items[0]] };
    const { calls } = renderWithEngine(<EnvironmentRepairDialog profile={configured} onResult={() => {}} onClose={() => {}} />, {
      "environment:repair-preview": makeEnvelope(preview),
      "environment:repair": makeEnvelope({ ...result(), items: [{ ...plan.items[0], status: "verified" }], relogin_required: true,
        verification_limits: ["Desktop and new-login formats remain unverified."] }),
    });
    await screen.findByText(language === "en" ? /GNOME formats will use the locale shown in the preview/ : /GNOME 地区格式将保存为预览中的 locale/);
    expect(calls.filter(call => call.command === "environment:repair")).toHaveLength(0);
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await screen.findByText(language === "en" ? "Preference saved and read back" : "偏好已保存并读回");
    expect(screen.getByRole("dialog")).toHaveTextContent(language === "en" ? "This app will not sign you out" : "应用不会替你退出登录");
    expect(screen.queryByText(language === "en" ? /^Save your work and restart the affected apps/ : /^请保存工作并重新启动相关应用/)).toBeNull();
    expect(calls.filter(call => call.command === "environment:repair")).toHaveLength(1);
  });
  it("forwards the preview fingerprint only after confirmation and preserves process mismatches", async () => {
    const { calls } = setup();
    const dialog = await open();
    await userEvent.click(within(dialog).getByRole("button", { name: "确认自动修复" }));
    await within(dialog).findByText("已执行自动修复，仍有项目需要重启或继续确认。");
    const request = calls.find((call) => call.command === "environment:repair")!;
    expect(JSON.parse(request.args[1])).toEqual({ languages: [], ...target });
    expect(request.args).toEqual(["--spec", request.args[1], "--base", plan.base, "--confirm"]);
    expect(request.options?.runId).toBeTruthy();
    expect(dialog).toHaveTextContent("请保存工作并重新启动相关应用");
    expect(document.querySelector('[data-check-id="LC_CTYPE"]')).toHaveTextContent("不符");
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
    expect(calls.filter((call) => ["launch:run", "environment:sync-time"].includes(call.command))).toHaveLength(0);
  });
  it.each(["cancelled", "partial", "audit_failed"] as const)("keeps %s results visible without claiming all-clear", async (status) => {
    setup(result(status));
    const dialog = await open();
    await userEvent.click(within(dialog).getByRole("button", { name: "确认自动修复" }));
    await waitFor(() => expect(within(dialog).getByTestId("confirm-modal-confirm")).toHaveTextContent("关闭"));
    expect(dialog).toHaveTextContent("未执行");
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
  });
  it("requires a new preview after a stale rejection and keeps the failure readable", async () => {
    const { calls } = renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ checks_supported: true, builtins: [], items: [profile] }), "environment:check": makeEnvelope(environmentFixture()),
      "environment:repair-preview": makeEnvelope(plan), "environment:repair": async () => { throw new Error("Environment settings changed after preview."); },
    });
    const dialog = await open();
    await userEvent.click(within(dialog).getByRole("button", { name: "确认自动修复" }));
    await within(dialog).findByRole("alert");
    expect(within(dialog).getByRole("button", { name: "确认自动修复" })).toBeDisabled();
    await userEvent.click(within(dialog).getByRole("button", { name: "重新预览修复" }));
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "确认自动修复" })).toBeEnabled());
    expect(calls.filter((call) => call.command === "environment:repair-preview")).toHaveLength(2);
    expect(calls.filter((call) => call.command === "environment:repair")).toHaveLength(1);
  });
  it("renders repair copy in English without translating engine diagnostics", async () => {
    setLanguage("en"); setup();
    await screen.findByText("Environment read. No comparison targets selected.");
    await chooseOption(screen.getByRole("combobox", { name: "Select an environment profile" }), profile.id);
    await userEvent.click(screen.getByRole("button", { name: "Repair automatically" }));
    const dialog = await screen.findByRole("dialog", { name: "Repair environment automatically" });
    await within(dialog).findByText("en_GB → en_US");
    expect(dialog).toHaveTextContent("expire at logout");
    expect(dialog).toHaveTextContent("Shell files and login items are preserved");
  });
  it("clears the previous repair conclusion when a new comparison starts", async () => {
    setup();
    const dialog = await open();
    await userEvent.click(within(dialog).getByRole("button", { name: "确认自动修复" }));
    await within(dialog).findByText("已执行自动修复，仍有项目需要重启或继续确认。");
    await userEvent.click(within(dialog).getByTestId("confirm-modal-confirm"));
    expect(within(screen.getByTestId("environment-page")).getByText("已执行自动修复，仍有项目需要重启或继续确认。", { exact: false })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "对比 Profile" }));
    await screen.findByText("发现与目标不符的项目。");
    expect(within(screen.getByTestId("environment-page")).queryByText("已执行自动修复，仍有项目需要重启或继续确认。", { exact: false })).not.toBeInTheDocument();
  });
  it("does not announce success when clock correction could not record its outcome", async () => {
    const repaired = result();
    repaired.report!.checks.forEach((row) => { if (row.expected !== null) { row.observed = row.expected; row.state = "match"; } });
    repaired.report!.summary = { ...repaired.report!.summary, status: "match", matched: repaired.report!.summary.compared, mismatched: 0, unknown: 0 };
    repaired.report!.time_repair = { status: "verified", before_offset_ms: 3000, before_uncertainty_ms: 20, audit_recorded: false };
    setup(repaired);
    const dialog = await open();
    await userEvent.click(within(dialog).getByRole("button", { name: "确认自动修复" }));
    await within(dialog).findByText("校时结果未能写入操作记录，本页保留复测结果。");
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
    expect(document.querySelector('[data-sonner-toast][data-type="warning"]')).toHaveTextContent("校时结果未能写入操作记录");
  });
  it("keeps an explicit verification limit visible even when all reported settings match", async () => {
    const repaired = result();
    repaired.report!.summary = { status: "match", matched: 1, mismatched: 0, unknown: 0, compared: 1 };
    repaired.verification_limits = ["Automatic switching remains unverified."];
    setup(repaired);
    const dialog = await open();
    await userEvent.click(within(dialog).getByRole("button", { name: "确认自动修复" }));
    await within(dialog).findByText("已保存所选设置，实际效果仍需在目标环境中确认。");
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
    expect(document.querySelector('[data-sonner-toast][data-type="warning"]')).toHaveTextContent("实际效果仍需在目标环境中确认");
  });
  it.each([null, 2000])("discloses the actual clock tolerance only when correction is available (%s)", async (tolerance) => {
    const configured = { ...profile, target: { ...profile.target, ntp_server: "time.example", max_clock_offset_ms: tolerance } };
    renderWithEngine(<EnvironmentRepairDialog profile={configured} onResult={() => {}} onClose={() => {}} />, {
      "environment:repair-preview": makeEnvelope({ ...plan, target: configured.target }),
    });
    const dialog = await screen.findByRole("dialog", { name: "自动修复环境" });
    await within(dialog).findByText("复查会查询 time.example 的 UDP 123 端口。");
    expect(dialog.textContent!.includes("时间偏差容限：± 2000 ms")).toBe(tolerance !== null);
    expect(dialog.textContent!.includes("仅在新测量明确超限时校正时钟")).toBe(tolerance !== null);
  });
});
