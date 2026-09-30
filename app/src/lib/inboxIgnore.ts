import { useSyncExternalStore } from "react";

/**
 * 待处理里用户选择忽略的项：按「项 + 当时的说明」记，状态变了（说明不同）就重新提示。
 * 只存本机浏览器（与「不用了」的推荐同一做法），读写失败时当没忽略过；不写任何配置。
 */
const KEY = "agent-manager:inbox-ignored";
const listeners = new Set<() => void>();

function read(): string[] {
  try {
    const value = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
  } catch {
    return [];
  }
}

let snapshot = read();

function write(next: string[]) {
  snapshot = next;
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // 存不下只影响下次是否再提示
  }
  for (const listener of listeners) listener();
}

/** 忽略的键：同一项说明变了就是新的键。 */
export function ignoreKey(item: { id: string; detail: string }): string {
  return `${item.id}|${item.detail}`;
}

export function ignoreItems(items: { id: string; detail: string }[]) {
  write([...new Set([...snapshot, ...items.map(ignoreKey)])]);
}

export function restoreItems(items: { id: string; detail: string }[]) {
  const drop = new Set(items.map(ignoreKey));
  write(snapshot.filter((key) => !drop.has(key)));
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** 当前忽略的键集合；忽略、恢复后各处（导航角标、待处理页、资源页筛选）一起更新。 */
export function useIgnored(): Set<string> {
  const keys = useSyncExternalStore(subscribe, () => snapshot);
  return new Set(keys);
}

/** 测试用：清空内存里的快照（localStorage 由测试环境各自清）。 */
export function resetIgnoredForTest() {
  snapshot = read();
}
