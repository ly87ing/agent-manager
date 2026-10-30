import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import EnvironmentPage from "../pages/EnvironmentPage";
import { BrowserEnvironmentReport } from "../components/BrowserEnvironmentReport";
import { browserReportSchema, profileTarget, type BrowserEnvironmentReport as Report } from "../lib/environment";
import { environmentFixture } from "./environment-fixture";
import { makeEnvelope } from "./fixtures";
import { chooseOption, renderWithEngine } from "./render-with-engine";

function browser(status: Report["status"] = "received"): Report {
  return { schema_version: 1, status, session_id: "fixture-session", checked_at: new Date().toISOString(), expires_at: new Date(Date.now() + 300000).toISOString(), origin: "http://127.0.0.1:1234", source: "Test fixture", scope: "This origin only", sample: null, headers: {},
    checks: [{ id: "browser.timezone", group: "browser", observed: status === "received" ? "Europe/London" : null, expected: "Asia/Singapore", state: status === "received" ? "mismatch" : "unknown", source: "Fixture", note: "profile_timezone_exact" }],
    summary: { status: status === "received" ? "mismatch" : "incomplete", matched: 0, mismatched: status === "received" ? 1 : 0, unknown: status === "received" ? 0 : 1, compared: 1 },
    not_checked: ["geoip", "webrtc", "canvas_webgl", "fonts", "clock_drift", "tls_fingerprint"] };
}

describe("Chrome environment observations", () => {
  it("does not open Chrome on load or cancelled preview, then forwards only explicit profile criteria", async () => {
    const profile = { id: "fixture", name: "Chrome criteria", revision: "a".repeat(64), auto_sync: true, target: environmentFixture({ timezone: "Asia/Singapore", languages: ["en-SG"] }).target };
    const { calls } = renderWithEngine(<EnvironmentPage />, {
      "environment:profiles": makeEnvelope({ checks_supported: true, builtins: [], items: [profile] }),
      "environment:check": makeEnvelope(environmentFixture()),
      "environment:check-browser": makeEnvelope({ ...environmentFixture(profileTarget(profile)), browser: browser() }),
    });
    await screen.findByText("Chrome 尚未检查。");
    expect(calls.map(c => c.command)).toEqual(["environment:profiles", "environment:check"]);
    await userEvent.click(screen.getByRole("button", { name: "检查 Chrome" }));
    await userEvent.click(within(screen.getByRole("dialog", { name: "检查 Chrome" })).getByRole("button", { name: "取消" }));
    expect(calls.filter(c => c.command === "environment:check-browser")).toHaveLength(0);
    await chooseOption(screen.getByRole("combobox", { name: "选择环境 Profile" }), "fixture");
    await userEvent.click(screen.getByRole("button", { name: "检查 Chrome" }));
    await userEvent.click(screen.getByRole("button", { name: "打开 Chrome 并检查" }));
    await screen.findByText("Chrome 已读取：符合 0 项，不符 1 项，无法确认 0 项。");
    const request = calls.find(c => c.command === "environment:check-browser")!;
    expect(request.args).toEqual(["--spec", JSON.stringify(profileTarget(profile)), "--confirm"]);
    expect(calls.some(c => ["environment:sync-time", "environment:repair", "launch:run"].includes(c.command))).toBe(false);
    expect(screen.getByText(/未检查：GeoIP/)).toBeVisible();
    await chooseOption(screen.getByRole("combobox", { name: "选择环境 Profile" }), "");
    expect(screen.queryByText("Chrome 已读取：符合 0 项，不符 1 项，无法确认 0 项。")).toBeNull();
    expect(screen.getByText("Chrome 尚未检查。")).toBeVisible();
  });

  it.each(["windows", "linux"] as const)("keeps unsupported Chrome checks disabled on %s", async platform => {
    renderWithEngine(<EnvironmentPage />, { "environment:check": makeEnvelope({ ...environmentFixture(), platform }) });
    await screen.findByText("已读取环境，尚未设置比较目标。");
    expect(screen.getByRole("button", { name: "检查 Chrome" })).toBeDisabled();
  });

  it.each(["timeout", "cancelled", "unavailable"] as const)("preserves unknown observations after %s", async status => {
    renderWithEngine(<EnvironmentPage />, { "environment:check": makeEnvelope(environmentFixture()), "environment:check-browser": makeEnvelope({ ...environmentFixture(), browser: browser(status) }) });
    await screen.findByText("Chrome 尚未检查。");
    await userEvent.click(screen.getByRole("button", { name: "检查 Chrome" }));
    await userEvent.click(screen.getByRole("button", { name: "打开 Chrome 并检查" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const region = screen.getByRole("region", { name: "Chrome 检查结果" });
    expect(region).toHaveTextContent("无法确认");
    expect(region).not.toHaveTextContent("Chrome 已读取");
  });

  it("expires an otherwise matching result instead of leaving green current evidence", () => {
    vi.useFakeTimers();
    try {
      const report = browser(); report.expires_at = new Date(Date.now() + 1000).toISOString(); report.checks[0].state = "match";
      renderWithEngine(<BrowserEnvironmentReport report={report} />, {});
      act(() => { vi.advanceTimersByTime(2000); });
      expect(screen.getByText("Chrome 结果已过期，请重新检查。")).toBeVisible();
      expect(screen.getByText("已过期")).toBeVisible();
      expect(screen.queryByText("符合")).toBeNull();
    } finally { vi.useRealTimers(); }
  });

  it("rejects unsupported report versions and malformed evidence before rendering", () => {
    expect(browserReportSchema.safeParse({ ...browser(), schema_version: 2 }).success).toBe(false);
    expect(browserReportSchema.safeParse({ ...browser(), expires_at: "invalid" }).success).toBe(false);
    expect(browserReportSchema.safeParse({ ...browser(), headers: { cookie: 123 } }).success).toBe(false);
  });
});
