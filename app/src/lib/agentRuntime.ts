import type {
  AgentCurrentConfigStatus,
  AgentRuntimeField,
} from "./observed";
import type { AgentRuntimeSettings } from "./agentDefinitionForm";

/**
 * 运行参数以客户端实际配置为准：表单用实际值预填，提交只发改动过的字段；
 * 库里的副本只是上次写入记录，不再拿来对比「是否生效」。
 */

/** 实际档位不属于本工具的三档预设时，表单用它表示「保持当前」。 */
export const CUSTOM_AUTOMATION = "custom";

function actualValue(field: AgentRuntimeField, status: AgentCurrentConfigStatus): unknown {
  if (status.state === "missing") return field.key === "automation_mode" ? "manual" : undefined;
  return status.config?.runtime?.[field.key];
}

/** 实际值 → 表单初值。只含适配器声明的字段；实际读不了时为空对象。 */
export function actualRuntimeSettings(
  fields: AgentRuntimeField[],
  status: AgentCurrentConfigStatus | null | undefined,
): AgentRuntimeSettings {
  const settings: AgentRuntimeSettings = {};
  if (!status || (status.state !== "ready" && status.state !== "missing")) return settings;
  for (const field of fields) {
    const value = actualValue(field, status);
    if (field.key === "automation_mode") {
      settings[field.key] = typeof value === "string" && value ? value : "manual";
    } else if (field.key === "statusline") {
      // 本工具的状态栏（含旧格式）= builtin；用户自己的与没有 = 保持现状
      if (value === "builtin" || value === "builtin-legacy") settings[field.key] = "builtin";
    } else if (field.type === "boolean") {
      if (value === true) settings[field.key] = true;
    } else if (value !== undefined && value !== null && value !== "") {
      settings[field.key] = value;
    }
  }
  return settings;
}

function comparable(field: AgentRuntimeField, value: unknown): string {
  if (field.type === "boolean") return value === true ? "true" : "false";
  if (value === undefined || value === null) return "";
  return String(value).trim();
}

/**
 * 提交值与初值的差异：只含改动过的字段，撤掉的项用 null（恢复客户端默认）。
 * 只校验改动过的字段——没动的实际值可能不在本工具的词汇表里（例如 acceptEdits），照样保留。
 */
export function runtimePatch(
  fields: AgentRuntimeField[],
  initial: AgentRuntimeSettings,
  edited: AgentRuntimeSettings,
): Record<string, unknown> {
  const patch: Record<string, unknown> = {};
  for (const field of fields) {
    const after = comparable(field, edited[field.key]);
    if (comparable(field, initial[field.key]) === after) continue;
    if (field.type === "boolean") {
      patch[field.key] = after === "true";
    } else if (field.type === "enum") {
      if (after === "") {
        if (field.key === "automation_mode") throw new Error(`${field.label}不能留空`);
        patch[field.key] = null;
      } else if (!field.options?.some((option) => option.value === after)) {
        throw new Error(`${field.label}不是适配器支持的取值`);
      } else {
        patch[field.key] = after;
      }
    } else if (field.type === "positive_integer") {
      if (after === "") {
        patch[field.key] = null;
      } else {
        const number = Number(after);
        if (!Number.isInteger(number) || number <= 0) {
          throw new Error(`${field.label}必须是正整数，留空表示使用客户端默认值`);
        }
        patch[field.key] = number;
      }
    } else if (field.type === "model") {
      if (after && (new TextEncoder().encode(after).length > 256 || /\s/u.test(after))) {
        throw new Error(`${field.label}必须是不超过 256 字节且不含空白字符的模型 ID`);
      }
      patch[field.key] = after || null;
    } else {
      throw new Error(`当前版本不认识运行设置类型 ${field.type}（${field.key}）`);
    }
  }
  return patch;
}

/**
 * 写入成功后回读：补丁里每一项都已是实际值，才算生效。
 *
 * 状态栏按原始观测值判断：预填时旧格式（builtin-legacy）也算本工具的状态栏，但「修复」要求回读到
 * 引用形式 builtin，旧格式仍在就是没修好。
 */
export function runtimePatchApplied(
  fields: AgentRuntimeField[],
  patch: Record<string, unknown>,
  status: AgentCurrentConfigStatus | null | undefined,
): boolean {
  if (!status || (status.state !== "ready" && status.state !== "missing")) return false;
  const actual = actualRuntimeSettings(fields, status);
  return Object.entries(patch).every(([key, value]) => {
    // 清理旧版 env 覆盖变量不是表单项：回读到一个都不剩才算生效
    if (key === "claude_clear_env_overrides") {
      const overrides = status.state === "ready" ? status.config?.claude_env_overrides : undefined;
      return !overrides || Object.keys(overrides).length === 0;
    }
    const field = fields.find((candidate) => candidate.key === key);
    if (!field) return false;
    if (key === "statusline") {
      const observed = status.state === "ready" ? status.config?.runtime?.statusline : undefined;
      return value === "builtin"
        ? observed === "builtin"
        : observed !== "builtin" && observed !== "builtin-legacy";
    }
    return comparable(field, actual[key]) === comparable(field, value);
  });
}

/** 实际档位是 custom 时，把客户端里的原始键值拼成一句可读的说明。 */
export function automationRawText(status: AgentCurrentConfigStatus | null | undefined): string {
  const raw = status?.config?.runtime?.automation_raw;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return "";
  return Object.entries(raw as Record<string, unknown>)
    .map(([key, value]) => `${key} = ${String(value)}`)
    .join("，");
}

export function runtimeValueLabel(field: AgentRuntimeField, value: unknown, rawText = ""): string {
  if (field.key === "automation_mode" && value === CUSTOM_AUTOMATION) {
    return rawText ? `自定义（${rawText}）` : "自定义";
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
