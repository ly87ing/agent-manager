/**
 * 用户明确不要的推荐来源：点过「不再推荐」，或把它停用 / 移除过。只存本机浏览器，
 * 读写失败时当没点过（只影响是否再提示）。
 */
const DISMISSED_KEY = "agent-manager:dismissed-recommendations";

export function loadDismissed(): string[] {
  try {
    const value = JSON.parse(localStorage.getItem(DISMISSED_KEY) ?? "[]");
    return Array.isArray(value) ? value : [];
  } catch {
    return [];
  }
}

export function dismissRecommendation(name: string): string[] {
  const next = [...new Set([...loadDismissed(), name])];
  try {
    localStorage.setItem(DISMISSED_KEY, JSON.stringify(next));
  } catch {
    // 存不下只影响下次是否再提示
  }
  return next;
}
