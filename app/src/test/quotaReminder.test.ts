import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentUsage } from "../lib/observed";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn(async () => true),
  requestPermission: vi.fn(async () => "granted"),
  sendNotification: vi.fn(),
}));

const { dueReminders, reminderEnabled, sendDueReminders, setReminderEnabled } = await import("../lib/quotaReminder");

const NOW = new Date("2026-09-18T04:00:00Z");

function usage(windows: AgentUsage["windows"], checkedAt = "2026-09-18T03:50:00Z"): AgentUsage {
  return { source: "statusline", checked_at: checkedAt, windows };
}

describe("额度快作废提醒", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => localStorage.clear());

  it("离重置不到窗口的 1/10 且已用不超过一半时提醒，只说事实不带动作", () => {
    const due = dueReminders(
      [{ accountId: "profiles:cc2", label: "Claude Code · cc2", usage: usage([{ window_minutes: 300, used_percent: 38, resets_at: "2026-09-18T04:25:00Z" }]) }],
      NOW,
    );
    expect(due).toEqual([
      {
        key: "profiles:cc2|300|2026-09-18T04:25:00Z",
        title: "Claude Code · cc2 的 5 小时额度 25 分钟后重置",
        body: "还有 62% 没用，重置后就作废。",
      },
    ]);
  });

  it("离重置还远、用得多、记录过旧、已过重置点或重置时间未知时都不提醒", () => {
    const accounts = [
      { accountId: "far", label: "a", usage: usage([{ window_minutes: 300, used_percent: 10, resets_at: "2026-09-18T05:00:00Z" }]) },
      { accountId: "used", label: "b", usage: usage([{ window_minutes: 300, used_percent: 51, resets_at: "2026-09-18T04:10:00Z" }]) },
      { accountId: "stale", label: "c", usage: usage([{ window_minutes: 300, used_percent: 10, resets_at: "2026-09-18T04:10:00Z" }], "2026-09-16T00:00:00Z") },
      { accountId: "past", label: "d", usage: usage([{ window_minutes: 300, used_percent: 10, resets_at: "2026-09-18T03:59:00Z" }]) },
      { accountId: "unknown", label: "e", usage: usage([{ window_minutes: 300, used_percent: 10, resets_at: null }]) },
      { accountId: "none", label: "f", usage: null },
    ];
    expect(dueReminders(accounts, NOW)).toEqual([]);
  });

  it("7 天窗口按同一比例：17 小时内重置才提醒", () => {
    const weekly = (resetsAt: string) => [{ accountId: "w", label: "w", usage: usage([{ window_minutes: 10080, used_percent: 20, resets_at: resetsAt }]) }];
    expect(dueReminders(weekly("2026-09-18T20:00:00Z"), NOW)).toHaveLength(1);
    expect(dueReminders(weekly("2026-09-19T00:00:00Z"), NOW)).toHaveLength(0);
  });

  it("默认关闭；打开后同一轮只发一次", async () => {
    const send = vi.fn();
    const accounts = [{ accountId: "cc2", label: "cc2", usage: usage([{ window_minutes: 300, used_percent: 10, resets_at: "2026-09-18T04:20:00Z" }]) }];
    expect(reminderEnabled()).toBe(false);
    expect(sendDueReminders(accounts, NOW, send)).toBe(0);
    expect(await setReminderEnabled(true)).toBe(true);
    expect(sendDueReminders(accounts, NOW, send)).toBe(1);
    expect(sendDueReminders(accounts, new Date("2026-09-18T04:05:00Z"), send)).toBe(0);
    expect(send).toHaveBeenCalledTimes(1);
    await setReminderEnabled(false);
    expect(reminderEnabled()).toBe(false);
  });
});
