// 测试环境 hermetic：所有 toLocaleString/toLocaleTimeString 渲染固定到 UTC，
// 不依赖 runner 的 TZ（CI 与本机结果一致；真实应用仍按用户本地时区显示）。
process.env.TZ = "UTC";

import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterAll, afterEach, beforeEach } from "vitest";
import { setLanguage } from "../lib/i18n";
import { toast } from "sonner";

// Existing interaction tests retain their Chinese coverage; language tests also exercise English.
beforeEach(() => setLanguage("zh-CN"));
import { setTimeout as settleTimers } from "node:timers/promises";
import { resetIgnoredForTest } from "../lib/inboxIgnore";

// 每个用例从空的本机存储开始（主题、集合展开状态等不跨用例残留）
afterEach(() => {
  try {
    localStorage.clear();
  } catch {
    // 非 jsdom 环境没有 localStorage
  }
  resetIgnoredForTest();
});
import { setEngineClient } from "../lib/engine";
import { setSettingsClient, type SettingsClient } from "../lib/settings";
import { setAutostartClient } from "../lib/autostart";
import { setShareLinkRegistrationClient } from "../lib/shareLinkRegistration";

const defaultAutostartClient = { get: async () => false, set: async (enabled: boolean) => enabled };
setAutostartClient(defaultAutostartClient);
const defaultShareLinkClient = { preview: async () => null, register: async () => { throw new Error("No AppImage in this fixture"); } };
setShareLinkRegistrationClient(defaultShareLinkClient);

const defaultSettingsClient: SettingsClient = {
  get: async () => ({ preferences: { language: "zh-CN", logRetentionDays: 30 }, appVersion: "test", contractVersion: "agent-manager/engine/v1", resourceDir: "/resources", dataDir: "/data", updaterConfigured: false, }),
  save: async (preferences) => ({ preferences, appVersion: "test", contractVersion: "agent-manager/engine/v1", resourceDir: "/resources", dataDir: "/data", updaterConfigured: false, }),
};
setSettingsClient(defaultSettingsClient);

// ---- jsdom 交互 API 缺口的最小 shim（带守卫，不覆盖已存在实现；仅测试环境生效） ----
// user-event 的 pointer 交互会调 Element.setPointerCapture（jsdom 未实现，
// 在 forks pool 下表现为 unhandled error 而非单测失败——CI 因此红）。
if (typeof Element !== "undefined") {
  if (typeof Element.prototype.setPointerCapture !== "function") {
    Element.prototype.setPointerCapture = () => {};
  }
  if (typeof Element.prototype.hasPointerCapture !== "function") {
    Element.prototype.hasPointerCapture = () => false;
  }
  if (typeof Element.prototype.releasePointerCapture !== "function") {
    Element.prototype.releasePointerCapture = () => {};
  }
  // 组件滚日志/到底部用 scrollTo(scrollTop)；个别库会调 scrollIntoView
  if (typeof Element.prototype.scrollIntoView !== "function") {
    Element.prototype.scrollIntoView = () => {};
  }
}
if (typeof window !== "undefined") {
  // sonner 等库在测量尺寸时依赖 ResizeObserver
  if (typeof window.ResizeObserver !== "function") {
    window.ResizeObserver = class ResizeObserverStub {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  }
  // user-event 派发 PointerEvent；jsdom 缺时退化为 MouseEvent（指针属性测试不依赖）
  if (typeof window.PointerEvent !== "function") {
    window.PointerEvent = window.MouseEvent as unknown as typeof PointerEvent;
  }
}

afterEach(async () => {
  cleanup();
  toast.dismiss();
  // sonner 的通知订阅会排入零延时回调；卸载后等它执行，再允许 jsdom 销毁。
  await settleTimers(0);
  setEngineClient(null);
  setSettingsClient(defaultSettingsClient);
  setAutostartClient(defaultAutostartClient);
  setShareLinkRegistrationClient(defaultShareLinkClient);
});

afterAll(async () => {
  // Sonner 1.7.4 leaves 200 ms removal callbacks after unmount. Drain them
  // before jsdom destroys window; the per-test zero-delay barrier is insufficient.
  await settleTimers(250);
});
