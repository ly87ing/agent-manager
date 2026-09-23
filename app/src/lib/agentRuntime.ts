import type {
  AgentCurrentConfigStatus,
  AgentRuntimeField,
} from "./observed";
import type { AgentRuntimeSettings } from "./agentDefinitionForm";

export type RuntimeAlignment = "aligned" | "drifted" | "unknown";

export interface RuntimeComparison {
  key: string;
  label: string;
  saved: string;
  actual: string;
  aligned: boolean | null;
}

function defaultValue(field: AgentRuntimeField): unknown {
  if (field.key === "automation_mode") return "manual";
  if (field.type === "boolean") return false;
  return null;
}

function savedValue(field: AgentRuntimeField, settings: AgentRuntimeSettings): unknown {
  const value = settings[field.key];
  if (value === undefined || value === null || value === "") return defaultValue(field);
  if (field.type === "boolean") return value === true;
  return value;
}

function actualValue(field: AgentRuntimeField, status: AgentCurrentConfigStatus): unknown {
  if (status.state === "missing") return defaultValue(field);
  if (status.state !== "ready") return undefined;
  const value = status.config?.runtime?.[field.key];
  return value === undefined || value === "" ? defaultValue(field) : value;
}

function comparableValue(field: AgentRuntimeField, value: unknown): unknown {
  if (field.key === "statusline" && value === "custom") return null;
  if (field.type === "positive_integer" && value !== null && value !== undefined) {
    const number = Number(value);
    return Number.isFinite(number) ? number : value;
  }
  return value;
}

export function runtimeValueLabel(field: AgentRuntimeField, value: unknown): string {
  if (field.key === "statusline" && value === "custom") return "自定义状态栏（不受本工具管理）";
  // 旧版写的裸路径：仍是本工具的状态栏，但与软件配置 builtin 不相等，因而判为不一致并给出重新应用入口
  if (field.key === "statusline" && value === "builtin-legacy") {
    return "本工具状态栏（旧格式，路径含空格时无法运行，重新应用即可修复）";
  }
  if (value === undefined) return "暂不可读";
  if (value === null || value === "" || value === false) return field.summary?.default ?? "客户端默认";
  if (field.type === "boolean") return field.summary?.enabled ?? "已开启";
  if (typeof value === "string") {
    return field.summary?.values?.[value]
      ?? field.options?.find((option) => option.value === value)?.label
      ?? value;
  }
  return String(value);
}

export function runtimeComparisons(
  fields: AgentRuntimeField[],
  settings: AgentRuntimeSettings,
  status: AgentCurrentConfigStatus | null | undefined,
): RuntimeComparison[] {
  return fields.map((field) => {
    const saved = savedValue(field, settings);
    const actual = status ? actualValue(field, status) : undefined;
    const readable = status?.state === "missing" || status?.state === "ready";
    return {
      key: field.key,
      label: field.summary?.label ?? field.label,
      saved: runtimeValueLabel(field, saved),
      actual: runtimeValueLabel(field, actual),
      aligned: readable
        ? comparableValue(field, saved) === comparableValue(field, actual)
        : null,
    };
  });
}

export function runtimeAlignment(
  fields: AgentRuntimeField[],
  settings: AgentRuntimeSettings,
  status: AgentCurrentConfigStatus | null | undefined,
): RuntimeAlignment {
  if (fields.length === 0) return "unknown";
  const comparisons = runtimeComparisons(fields, settings, status);
  if (comparisons.some((item) => item.aligned === null)) return "unknown";
  return comparisons.every((item) => item.aligned) ? "aligned" : "drifted";
}
