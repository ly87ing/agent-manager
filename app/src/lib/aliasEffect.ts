import type { AliasEntry } from "./observed";
import type { EngineElement, StatusData } from "./contract";

/**
 * 「这条命令到底生效了没有」的聚合判断。
 *
 * aliases 有两个目标文件（交互式与登录式 shell），两个都跟上了才算已写入；
 * 任一还是旧值或没写，就要提示同步。写入之后能不能用还取决于终端会话有没有重新加载，
 * 那一层本工具看不到，由界面文案提示新开窗口，而不是替用户断言已经能用。
 */
export type AliasEffect = "applied" | "outdated" | "not-applied" | "unknown";

export function aliasEffectOf(elements: EngineElement[] | undefined, name: string): AliasEffect {
  if (!elements || elements.length === 0) return "unknown";
  const states: string[] = [];
  for (const element of elements) {
    const entries = (element.observed as { entries?: AliasEntry[] }).entries;
    if (!entries) continue;
    const entry = entries.find((item) => item.name === name);
    if (entry) states.push(entry.state);
  }
  if (states.length === 0) return "unknown";
  if (states.every((state) => state === "applied")) return "applied";
  if (states.some((state) => state === "outdated")) return "outdated";
  return "not-applied";
}

/** 从 aliases:status 响应里取元素列表（未就绪时为 undefined）。 */
export function aliasElementsOf(status: { ok: boolean; data: StatusData } | undefined): EngineElement[] | undefined {
  return status?.ok ? status.data.elements : undefined;
}

export const ALIAS_EFFECT_LABEL: Record<AliasEffect, string> = {
  applied: "已写入终端配置",
  outdated: "终端配置里还是旧的",
  "not-applied": "还没写入终端配置",
  unknown: "状态未知",
};

export const ALIAS_EFFECT_HINT: Record<AliasEffect, string> = {
  applied: "新开的终端窗口会用这一行；已经开着的窗口要重开才会加载。",
  outdated: "改动还没同步过去，点「同步」写入后新开窗口生效。",
  "not-applied": "点「同步」写入终端配置，之后新开窗口生效。",
  unknown: "读不到终端配置的状态。",
};
