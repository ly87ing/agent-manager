import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { isTauri } from "@tauri-apps/api/core";
import { isStale, isWindowExpired, windowLabel } from "./agentUsage";
import type { AgentUsage } from "./observed";

/**
 * 额度快作废提醒：某个账号的窗口快重置了，却还剩不少没用。
 *
 * 只发一条通知，不带任何动作——用不用、换不换终端由人决定。时机按窗口长度算：
 * 离重置不到窗口的 1/10（5 小时窗口约 30 分钟，7 天窗口约 17 小时），且已用不超过一半。
 * 数字只和助手自己的记账一样新，过旧的记录不提醒，免得报一个早已不对的剩余量。
 */

export interface ReminderAccount {
  accountId: string;
  /** 通知里的称呼，例如「Claude Code · cc2」。 */
  label: string;
  usage: AgentUsage | null;
}

export interface QuotaReminder {
  /** 账号 + 窗口 + 这一轮的重置时间：同一轮只提醒一次。 */
  key: string;
  title: string;
  body: string;
}

const LEAD_FRACTION = 0.1;
const MAX_USED_PERCENT = 50;

function minutesText(minutes: number): string {
  if (minutes < 60) return `${minutes} 分钟`;
  const hours = Math.round(minutes / 60);
  return hours < 48 ? `${hours} 小时` : `${Math.round(hours / 24)} 天`;
}

export function dueReminders(accounts: ReminderAccount[], now: Date = new Date()): QuotaReminder[] {
  const out: QuotaReminder[] = [];
  for (const account of accounts) {
    const usage = account.usage;
    if (!usage || isStale(usage.checked_at, now)) continue;
    for (const window of usage.windows) {
      if (!window.resets_at || isWindowExpired(window.resets_at, now)) continue;
      const left = Math.round((new Date(window.resets_at).getTime() - now.getTime()) / 60000);
      if (left > window.window_minutes * LEAD_FRACTION || window.used_percent > MAX_USED_PERCENT) continue;
      out.push({
        key: `${account.accountId}|${window.window_minutes}|${window.resets_at}`,
        title: `${account.label} 的 ${windowLabel(window.window_minutes)}额度 ${minutesText(Math.max(1, left))}后重置`,
        body: `还有 ${Math.round(100 - window.used_percent)}% 没用，重置后就作废。`,
      });
    }
  }
  return out;
}

const ENABLED_KEY = "agent-manager:quota-reminder";
const SENT_KEY = "agent-manager:quota-reminder-sent";
/** 已发记录只留这么多条，旧的自然挤掉（重置时间早已过去，不会再命中）。 */
const SENT_LIMIT = 200;

export function reminderEnabled(): boolean {
  try {
    return localStorage.getItem(ENABLED_KEY) === "1";
  } catch {
    return false;
  }
}

/** 打开时先向系统要通知权限；拿不到就保持关闭并返回 false。 */
export async function setReminderEnabled(enabled: boolean): Promise<boolean> {
  if (enabled && isTauri()) {
    const granted = (await isPermissionGranted()) || (await requestPermission()) === "granted";
    if (!granted) return false;
  }
  try {
    if (enabled) localStorage.setItem(ENABLED_KEY, "1");
    else localStorage.removeItem(ENABLED_KEY);
  } catch {
    return false;
  }
  return true;
}

function sentKeys(): string[] {
  try {
    const value = JSON.parse(localStorage.getItem(SENT_KEY) ?? "[]");
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
  } catch {
    return [];
  }
}

/** 发出还没发过的提醒；返回这次发出的条数。关闭时、非桌面环境里什么都不做。 */
export function sendDueReminders(accounts: ReminderAccount[], now: Date = new Date(), send = sendNotification): number {
  if (!reminderEnabled() || !isTauri()) return 0;
  const sent = sentKeys();
  const fresh = dueReminders(accounts, now).filter((item) => !sent.includes(item.key));
  if (fresh.length === 0) return 0;
  for (const item of fresh) send({ title: item.title, body: item.body });
  try {
    localStorage.setItem(SENT_KEY, JSON.stringify([...sent, ...fresh.map((item) => item.key)].slice(-SENT_LIMIT)));
  } catch {
    // 记不下已发记录时，下一分钟可能重复提醒；不影响别的功能
  }
  return fresh.length;
}
