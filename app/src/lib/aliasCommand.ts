/**
 * 启动命令别名的解析与生成（纯函数，供助手卡片与终端快捷命令页共用）。
 *
 * 落盘格式由引擎 aliases 域固定：`alias <名称>='<值>'`，值内不得出现单引号与换行
 * （引擎 valid_alias_text 会拒绝）。值里的 `{timezone}` / `{lang}` 由引擎渲染时替换为
 * 来源里的全局设置，因此「跟随全局」就是写这两个记号，指定具体值就是写字面量。
 *
 * 结构化形状：`TZ="…" LANG="…" LC_ALL="…" <命令> <扩展参数>`。解析不出这个形状的
 * 别名（例如包裹了 open/env 等其它命令）标记为非结构化，界面只提供原文编辑，不猜测改写。
 */

/** 跟随全局设置的记号（引擎渲染时替换）。 */
export const FOLLOW_GLOBAL_TZ = "{timezone}";
export const FOLLOW_GLOBAL_LANG = "{lang}";

/** 常用时区：置顶显示，同时兼作 Intl 不可用时的回退列表。 */
const COMMON_TIMEZONES = [
  "America/Los_Angeles",
  "America/New_York",
  "Asia/Shanghai",
  "Asia/Hong_Kong",
  "Asia/Singapore",
  "Asia/Tokyo",
  "Europe/London",
  "Europe/Berlin",
  "UTC",
];

/**
 * 可选时区列表：常用时区置顶（否则要在几百项里翻找），其后接运行环境的完整 IANA 列表；
 * 环境不提供完整列表时只返回常用时区。
 */
export function listTimeZones(): string[] {
  const supported = (Intl as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf;
  let all: string[] = [];
  if (typeof supported === "function") {
    try {
      const values = supported("timeZone");
      if (Array.isArray(values)) all = values;
    } catch {
      all = [];
    }
  }
  return Array.from(new Set([...COMMON_TIMEZONES, ...all]));
}

/** 本机时区（用于「与本机一致」提示）；取不到时为 null。 */
export function detectTimeZone(): string | null {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || null;
  } catch {
    return null;
  }
}

export const LANG_OPTIONS = ["en_US.UTF-8", "zh_CN.UTF-8", "C.UTF-8", "C"];

export interface ParsedAlias {
  /** 命令本体（别名展开后执行的第一个词）。 */
  command: string;
  /** 命令之后的其余参数原文。 */
  extraArgs: string;
  /** TZ 取值：FOLLOW_GLOBAL_TZ 表示跟随全局；null 表示该别名没设时区。 */
  timezone: string | null;
  /** LANG/LC_ALL 取值（两者一致时才结构化识别）。 */
  lang: string | null;
  /** 除 TZ/LANG/LC_ALL 外的其它前置环境变量，原样保留。 */
  otherEnv: Record<string, string>;
  /** false = 解析不出结构化形状，界面只给原文编辑。 */
  structured: boolean;
}

const ENV_PREFIX_RE = /^([A-Za-z_][A-Za-z0-9_]*)=("([^"]*)"|'([^']*)'|(\S*))\s*/;

function unquote(raw: string): string {
  if (raw.length >= 2 && ((raw.startsWith('"') && raw.endsWith('"')) || (raw.startsWith("'") && raw.endsWith("'")))) {
    return raw.slice(1, -1);
  }
  return raw;
}

/** 解析别名值。空值或解析不出命令时 structured=false。 */
export function parseAliasValue(value: string): ParsedAlias {
  const empty: ParsedAlias = {
    command: "",
    extraArgs: "",
    timezone: null,
    lang: null,
    otherEnv: {},
    structured: false,
  };
  const text = value.trim();
  if (!text) return empty;

  const env: Record<string, string> = {};
  let rest = text;
  for (;;) {
    const match = ENV_PREFIX_RE.exec(rest);
    if (!match) break;
    env[match[1]] = unquote(match[2]);
    rest = rest.slice(match[0].length);
  }
  const trimmedRest = rest.trim();
  if (!trimmedRest) return { ...empty, otherEnv: env };

  const spaceIdx = trimmedRest.search(/\s/);
  const command = spaceIdx === -1 ? trimmedRest : trimmedRest.slice(0, spaceIdx);
  const extraArgs = spaceIdx === -1 ? "" : trimmedRest.slice(spaceIdx).trim();

  const { TZ, LANG, LC_ALL, ...otherEnv } = env;
  // LANG 与 LC_ALL 不一致时不做结构化归并，避免保存时悄悄改掉其中一个
  const langConsistent = LC_ALL === undefined || LANG === undefined || LC_ALL === LANG;
  // 环境变量出现在参数里（如 open --env TZ=… 这类包装写法）时，按项填写会给出误导的时区/语言，
  // 改用原文编辑；命令位置本身像赋值的同样不结构化。
  const envInsideArgs = /(^|\s)(TZ|LANG|LC_ALL)=/.test(extraArgs);
  return {
    command,
    extraArgs,
    timezone: TZ ?? null,
    lang: langConsistent ? (LANG ?? LC_ALL ?? null) : null,
    otherEnv,
    structured: langConsistent && !command.includes("=") && !envInsideArgs,
  };
}

export interface BuildAliasInput {
  command: string;
  extraArgs?: string;
  /** null = 不写 TZ；FOLLOW_GLOBAL_TZ = 跟随全局。 */
  timezone?: string | null;
  /** null = 不写 LANG/LC_ALL；FOLLOW_GLOBAL_LANG = 跟随全局。 */
  lang?: string | null;
  otherEnv?: Record<string, string>;
}

/** 生成别名值：环境变量在前、命令在后，值一律双引号包裹（单引号会被引擎拒绝）。 */
export function buildAliasValue(input: BuildAliasInput): string {
  const parts: string[] = [];
  for (const [key, value] of Object.entries(input.otherEnv ?? {})) {
    parts.push(`${key}="${value}"`);
  }
  if (input.timezone) parts.push(`TZ="${input.timezone}"`);
  if (input.lang) {
    parts.push(`LANG="${input.lang}"`);
    parts.push(`LC_ALL="${input.lang}"`);
  }
  parts.push(input.command.trim());
  const extra = (input.extraArgs ?? "").trim();
  if (extra) parts.push(extra);
  return parts.filter(Boolean).join(" ");
}

/** 渲染预览：把跟随全局的记号换成实际生效值，展示最终写进 shell 的那一行。 */
export function renderAliasPreview(name: string, value: string, globals: { timezone: string; lang: string }): string {
  const resolved = value.replaceAll(FOLLOW_GLOBAL_TZ, globals.timezone).replaceAll(FOLLOW_GLOBAL_LANG, globals.lang);
  return `alias ${name}='${resolved}'`;
}

/** 别名名称规则，与引擎 valid_alias_name 一致。 */
export const ALIAS_NAME_RE = /^[A-Za-z_][A-Za-z0-9_-]*$/;

/** 与引擎 valid_alias_text 一致的取值校验：单引号与换行会破坏落盘行结构。 */
export function aliasTextError(text: string): string | null {
  if (text.includes("'")) return "不能包含单引号";
  if (/[\n\r]/.test(text)) return "不能包含换行";
  return null;
}
