import type { AgentRuntimeField } from "./observed";

export type SupportedPlatform = "darwin" | "linux" | "windows";
export type PlatformScope = "universal" | SupportedPlatform;

export interface AgentFormValues {
  name: string;
  displayName: string;
  bin: string;
  channel: string;
  platformScope: PlatformScope;
  installCmd: string;
  upgradeCmd: string;
  uninstallCmd: string;
  preferredProvider?: string;
  advancedJson?: string;
  /** 多套接入配置，同一时刻一套生效。 */
  connections: ConnectionEntry[];
  /** 当前生效的那套的 name；空表示都不生效（用助手默认地址与账号）。 */
  activeConnection: string;
  /** 跳过权限确认：改的是助手的安全默认值，保存前要二次确认。 */
  yolo: boolean;
  /** 状态栏由谁维护。 */
  statusline: "untouched" | "builtin";
}

/** 一套接入配置：连到哪、拿什么认证、用什么模型。 */
export interface ConnectionEntry {
  /** 键名，写进定义；只用字母、数字、连字符与下划线。 */
  name: string;
  /** 界面上显示的名字；留空时用键名。 */
  label: string;
  baseUrl: string;
  /** token = Authorization: Bearer；api_key = x-api-key。中转站两种都有。 */
  auth: "token" | "api_key";
  /** 令牌所在的环境变量名；库里只存这个名字。 */
  tokenVar: string;
  model: string;
  /** 后台小任务用的轻量模型。 */
  smallFastModel: string;
  /** opus / sonnet / haiku 各自映射到哪个模型名。 */
  aliasOpus: string;
  aliasSonnet: string;
  aliasHaiku: string;
  /** 请求超时（毫秒）；留空用默认。 */
  timeoutMs: string;
}

/** 账号、地址、模型与凭据变量之外的完整运行设置文档；未知键必须原样往返。 */
export type AgentRuntimeSettings = Record<string, unknown>;

/** 新建一套接入配置时的空白值。 */
export function emptyConnection(name = ""): ConnectionEntry {
  return {
    name,
    label: "",
    baseUrl: "",
    auth: "token",
    tokenVar: "",
    model: "",
    smallFastModel: "",
    aliasOpus: "",
    aliasSonnet: "",
    aliasHaiku: "",
    timeoutMs: "",
  };
}

export const CONNECTION_NAME_RE = /^[A-Za-z0-9_-]{1,64}$/;
const ENV_NAME_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;
const CONNECTION_BUNDLE_FORMAT = "agent-manager/provider/v1";

export interface ConnectionBundle {
  format: typeof CONNECTION_BUNDLE_FORMAT;
  kind: "claude" | "codex";
  connection: ConnectionEntry;
}

function validConnectionBaseUrl(value: string): boolean {
  try {
    const url = new URL(value);
    if (url.protocol === "https:") return true;
    return url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  } catch {
    return false;
  }
}

/** 校验可保存的连接定义。凭据取值不属于这个结构，也不会被导入导出。 */
export function validateConnectionEntry(item: ConnectionEntry): string | null {
  if (!CONNECTION_NAME_RE.test(item.name.trim())) {
    return "唯一名称只能用字母、数字、连字符与下划线，长度不超过 64";
  }
  if (!validConnectionBaseUrl(item.baseUrl.trim())) {
    return "接口地址必须是 https://，或本机 localhost / 127.0.0.1 地址";
  }
  if (!ENV_NAME_RE.test(item.tokenVar.trim())) return "凭据变量名格式不正确";
  if (!item.model.trim()) return "要填写模型名";
  if (item.timeoutMs.trim()) {
    const timeout = Number(item.timeoutMs.trim());
    if (!Number.isFinite(timeout) || timeout <= 0) return "请求超时必须是正数";
  }
  return null;
}

/** 导出单个 Provider。结构里只有凭据变量名，不包含凭据取值或官方认证文件。 */
export function serializeConnectionBundle(
  kind: "claude" | "codex",
  connection: ConnectionEntry,
): string {
  return JSON.stringify({ format: CONNECTION_BUNDLE_FORMAT, kind, connection }, null, 2);
}

/** 解析分享内容；只接受当前版本与明确字段，避免把任意配置透传到助手文件。 */
export function parseConnectionBundle(text: string, expectedKind: "claude" | "codex"): ConnectionEntry {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    throw new Error("粘贴的内容不是合法 JSON");
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("接入分享内容必须是对象");
  const bundle = value as Record<string, unknown>;
  if (bundle.format !== CONNECTION_BUNDLE_FORMAT) throw new Error("不支持的接入分享格式");
  if (bundle.kind !== expectedKind) throw new Error(`这份接入属于 ${String(bundle.kind)}，不能导入当前助手`);
  if (!bundle.connection || typeof bundle.connection !== "object" || Array.isArray(bundle.connection)) {
    throw new Error("接入分享内容缺少 connection");
  }
  const raw = bundle.connection as Record<string, unknown>;
  const stringField = (key: keyof ConnectionEntry): string => {
    const field = raw[key];
    if (field === undefined) return "";
    if (typeof field !== "string") throw new Error(`connection.${key} 必须是字符串`);
    return field;
  };
  const auth = raw.auth === undefined ? "token" : raw.auth;
  if (auth !== "token" && auth !== "api_key") throw new Error("connection.auth 只能是 token 或 api_key");
  const connection: ConnectionEntry = {
    name: stringField("name").trim(),
    label: stringField("label").trim(),
    baseUrl: stringField("baseUrl").trim(),
    auth,
    tokenVar: stringField("tokenVar").trim(),
    model: stringField("model").trim(),
    smallFastModel: stringField("smallFastModel").trim(),
    aliasOpus: stringField("aliasOpus").trim(),
    aliasSonnet: stringField("aliasSonnet").trim(),
    aliasHaiku: stringField("aliasHaiku").trim(),
    timeoutMs: stringField("timeoutMs").trim(),
  };
  const error = validateConnectionEntry(connection);
  if (error) throw new Error(error);
  return { ...connection, label: connection.label || connection.name };
}

/** 一套接入配置写进 definition.settings.profiles 时的结构。 */
export function connectionToDefinitionValue(item: ConnectionEntry): Record<string, unknown> {
  const entry: Record<string, unknown> = {};
  if (item.label.trim()) entry.label = item.label.trim();
  if (item.baseUrl.trim()) entry.base_url = item.baseUrl.trim();
  if (item.tokenVar.trim()) {
    entry.token_var = item.tokenVar.trim();
    // token 是默认值，不额外写；api_key 才需要显式记录。
    if (item.auth === "api_key") entry.auth = "api_key";
  }
  if (item.model.trim()) entry.model = item.model.trim();
  if (item.smallFastModel.trim()) entry.small_fast_model = item.smallFastModel.trim();
  const aliases: Record<string, string> = {};
  if (item.aliasOpus.trim()) aliases.opus = item.aliasOpus.trim();
  if (item.aliasSonnet.trim()) aliases.sonnet = item.aliasSonnet.trim();
  if (item.aliasHaiku.trim()) aliases.haiku = item.aliasHaiku.trim();
  if (Object.keys(aliases).length > 0) entry.model_aliases = aliases;
  const timeout = Number(item.timeoutMs.trim());
  if (item.timeoutMs.trim() && Number.isFinite(timeout) && timeout > 0) {
    entry.timeout_ms = Math.floor(timeout);
  }
  return entry;
}

/**
 * 在完整助手定义里新增或替换一套接入配置。
 * 只改 settings.profiles 的目标项，当前 active、其他运行参数和未知字段原样保留。
 * 是否启用必须交给 agents:configure，让数据库与目标文件保持同成同败。
 */
export function upsertAgentConnection(
  definition: Record<string, unknown>,
  item: ConnectionEntry,
  preserveUnknownFrom = item.name,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(definition)) as Record<string, unknown>;
  const currentSettings =
    next.settings && typeof next.settings === "object" && !Array.isArray(next.settings)
      ? (next.settings as Record<string, unknown>)
      : {};
  const currentProfiles =
    currentSettings.profiles && typeof currentSettings.profiles === "object" && !Array.isArray(currentSettings.profiles)
      ? (currentSettings.profiles as Record<string, unknown>)
      : {};
  const name = item.name.trim();
  const currentEntry =
    currentProfiles[preserveUnknownFrom] &&
    typeof currentProfiles[preserveUnknownFrom] === "object" &&
    !Array.isArray(currentProfiles[preserveUnknownFrom])
      ? { ...(currentProfiles[preserveUnknownFrom] as Record<string, unknown>) }
      : {};
  // 编辑或复制时只替换表单真正拥有的字段。先删再合并，既能让“清空可选项”生效，
  // 也不会丢掉新版客户端或用户手工加入、而当前表单尚不认识的字段。
  for (const key of [
    "label",
    "base_url",
    "auth",
    "token_var",
    "model",
    "small_fast_model",
    "model_aliases",
    "timeout_ms",
  ]) {
    delete currentEntry[key];
  }
  next.settings = {
    ...currentSettings,
    profiles: {
      ...currentProfiles,
      [name]: { ...currentEntry, ...connectionToDefinitionValue(item) },
    },
  };
  return next;
}

/** 复制接入时生成不冲突的草稿；只复制变量名和连接参数，不涉及凭据取值。 */
export function duplicateConnectionDraft(
  item: ConnectionEntry,
  existingNames: string[],
): ConnectionEntry {
  const existing = new Set(existingNames);
  let counter = 1;
  while (true) {
    const suffix = counter === 1 ? "-copy" : `-copy-${counter}`;
    const candidate = `${item.name.slice(0, 64 - suffix.length)}${suffix}`;
    if (!existing.has(candidate)) {
      return {
        ...item,
        name: candidate,
        label: `${item.label || item.name} 副本`,
      };
    }
    counter += 1;
  }
}

/** 删除一套接入；只动目标 profile，其他定义字段、运行参数与接入原样保留。 */
export function removeAgentConnection(
  definition: Record<string, unknown>,
  name: string,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(definition)) as Record<string, unknown>;
  const settings = next.settings;
  if (!settings || typeof settings !== "object" || Array.isArray(settings)) return next;
  const currentSettings = settings as Record<string, unknown>;
  const profiles = currentSettings.profiles;
  if (!profiles || typeof profiles !== "object" || Array.isArray(profiles)) return next;

  const nextProfiles = { ...(profiles as Record<string, unknown>) };
  delete nextProfiles[name];
  const nextSettings = { ...currentSettings };
  if (Object.keys(nextProfiles).length > 0) nextSettings.profiles = nextProfiles;
  else delete nextSettings.profiles;
  // 界面不允许删除当前项；这里仍防御旧调用方，避免 active 指向不存在的 profile。
  if (nextSettings.active === name) delete nextSettings.active;
  if (Object.keys(nextSettings).length > 0) next.settings = nextSettings;
  else delete next.settings;
  return next;
}

/** 调整接入卡片顺序；只重建 profiles 的键顺序，不改变任何接入内容或当前项。 */
export function moveAgentConnection(
  definition: Record<string, unknown>,
  name: string,
  direction: "up" | "down",
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(definition)) as Record<string, unknown>;
  const settings = next.settings;
  if (!settings || typeof settings !== "object" || Array.isArray(settings)) return next;
  const currentSettings = settings as Record<string, unknown>;
  const profiles = currentSettings.profiles;
  if (!profiles || typeof profiles !== "object" || Array.isArray(profiles)) return next;

  const entries = Object.entries(profiles as Record<string, unknown>);
  const index = entries.findIndex(([key]) => key === name);
  const nextIndex = direction === "up" ? index - 1 : index + 1;
  if (index < 0 || nextIndex < 0 || nextIndex >= entries.length) return next;
  [entries[index], entries[nextIndex]] = [entries[nextIndex], entries[index]];
  next.settings = { ...currentSettings, profiles: Object.fromEntries(entries) };
  return next;
}

export function detectSystemPlatform(): SupportedPlatform {
  if (typeof navigator === "undefined") return "darwin";
  const platform = navigator.platform || "";
  const userAgent = navigator.userAgent || "";
  if (/Win/i.test(platform) || /Win/i.test(userAgent)) return "windows";
  if (/Linux/i.test(platform) || /Linux/i.test(userAgent)) return "linux";
  return "darwin";
}

export function getProviderNames(definition?: Record<string, unknown> | null): string[] {
  if (!definition || typeof definition !== "object") return ["custom"];
  const providers = definition.providers;
  if (!providers || typeof providers !== "object" || Array.isArray(providers)) {
    return ["custom"];
  }
  const keys = Object.keys(providers);
  return keys.length > 0 ? keys : ["custom"];
}

export function extractCommandForPlatform(
  commandValue: unknown,
  platform: PlatformScope
): string {
  if (typeof commandValue === "string") {
    return commandValue;
  }
  if (!commandValue || typeof commandValue !== "object" || Array.isArray(commandValue)) {
    return "";
  }
  const mapping = commandValue as Record<string, unknown>;
  if (platform === "universal") {
    const candidate = mapping.default ?? mapping.posix ?? Object.values(mapping).find(v => typeof v === "string" && v.trim());
    return typeof candidate === "string" ? candidate : "";
  }
  const priorityKeys: string[] =
    platform === "darwin"
      ? ["darwin", "posix", "default"]
      : platform === "linux"
        ? ["linux", "posix", "default"]
        : ["windows", "default"];
  for (const key of priorityKeys) {
    if (key in mapping) {
      const val = mapping[key];
      if (typeof val === "string") return val;
    }
  }
  return "";
}

export function updateCommandWithPlatform(
  existingCommandValue: unknown,
  newCmdText: string,
  platform: PlatformScope
): string | Record<string, string> | undefined {
  const trimmed = newCmdText.trim();

  // 1. Cleared command
  if (!trimmed) {
    if (platform === "universal") {
      return undefined;
    }
    let baseMapping: Record<string, string> = {};
    if (typeof existingCommandValue === "string") {
      if (existingCommandValue.trim()) {
        baseMapping = { default: existingCommandValue.trim() };
      }
    } else if (existingCommandValue && typeof existingCommandValue === "object" && !Array.isArray(existingCommandValue)) {
      baseMapping = Object.fromEntries(
        Object.entries(existingCommandValue as Record<string, unknown>).filter(
          ([, v]) => typeof v === "string"
        ) as [string, string][]
      );
    }

    const hasFallback =
      ("default" in baseMapping && baseMapping.default.trim() !== "") ||
      (platform !== "windows" && "posix" in baseMapping && baseMapping.posix.trim() !== "");

    if (hasFallback) {
      // Set empty string to explicitly block fallback on target platform while keeping other platforms
      baseMapping[platform] = "";
    } else {
      delete baseMapping[platform];
    }

    const hasAnyRealCommand = Object.values(baseMapping).some((v) => v.trim() !== "");
    return hasAnyRealCommand ? baseMapping : undefined;
  }

  // 2. Universal platform: replaces everything with clean plain string
  if (platform === "universal") {
    return trimmed;
  }

  // 3. Platform specific update
  const baseMapping: Record<string, string> =
    existingCommandValue && typeof existingCommandValue === "object" && !Array.isArray(existingCommandValue)
      ? Object.fromEntries(
          Object.entries(existingCommandValue as Record<string, unknown>).filter(
            ([, v]) => typeof v === "string"
          ) as [string, string][]
        )
      : typeof existingCommandValue === "string" && existingCommandValue.trim()
        ? { default: existingCommandValue.trim() }
        : {};

  baseMapping[platform] = trimmed;
  return baseMapping;
}

const STANDARD_DEFINITION_KEYS = new Set([
  "display_name",
  "bin",
  "providers",
  "preferred_provider",
  // 运行参数有自己的表单字段，不再混进「其他设置 JSON」
  "settings",
  "_delete",
  "_replace",
]);

export function extractUnknownFields(
  definition?: Record<string, unknown> | null
): Record<string, unknown> {
  if (!definition || typeof definition !== "object") return {};
  const unknown: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(definition)) {
    if (!STANDARD_DEFINITION_KEYS.has(k)) {
      unknown[k] = v;
    }
  }
  return unknown;
}

export function parseInitialToFormValues(
  name: string,
  initialDefinition?: Record<string, unknown>
): AgentFormValues {
  const def = initialDefinition ?? {};
  const providerNames = getProviderNames(def);
  const preferred =
    typeof def.preferred_provider === "string" && providerNames.includes(def.preferred_provider)
      ? def.preferred_provider
      : providerNames[0] || "custom";

  const currentPlatform = detectSystemPlatform();
  const providersObj =
    def.providers && typeof def.providers === "object" && !Array.isArray(def.providers)
      ? (def.providers as Record<string, unknown>)
      : {};
  const activeProviderCfg =
    providersObj[preferred] && typeof providersObj[preferred] === "object" && !Array.isArray(providersObj[preferred])
      ? (providersObj[preferred] as Record<string, unknown>)
      : {};

  const hasPlatformSpecific =
    (activeProviderCfg.install && typeof activeProviderCfg.install === "object") ||
    (activeProviderCfg.upgrade && typeof activeProviderCfg.upgrade === "object") ||
    (activeProviderCfg.uninstall && typeof activeProviderCfg.uninstall === "object");

  const platformScope: PlatformScope = hasPlatformSpecific ? currentPlatform : "universal";

  const installCmd = extractCommandForPlatform(activeProviderCfg.install, platformScope);
  const upgradeCmd = extractCommandForPlatform(activeProviderCfg.upgrade, platformScope);
  const uninstallCmd = extractCommandForPlatform(activeProviderCfg.uninstall, platformScope);

  const displayName = typeof def.display_name === "string" ? def.display_name : "";
  const bin = typeof def.bin === "string" ? def.bin : "";

  const unknownFields = extractUnknownFields(def);
  const advancedJson = Object.keys(unknownFields).length > 0 ? JSON.stringify(unknownFields, null, 2) : "";

  return {
    name,
    displayName,
    bin,
    channel: preferred,
    platformScope,
    installCmd,
    upgradeCmd,
    uninstallCmd,
    preferredProvider: typeof def.preferred_provider === "string" ? def.preferred_provider : preferred,
    advancedJson,
    ...readAgentSettings(def.settings),
  };
}

/** 定义里的 settings 段 → 表单字段。缺段或字段缺失都按「没配过」。 */
function readAgentSettings(
  raw: unknown,
): Pick<AgentFormValues, "connections" | "activeConnection" | "yolo" | "statusline"> {
  const obj = raw && typeof raw === "object" && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  const rawProfiles =
    obj.profiles && typeof obj.profiles === "object" && !Array.isArray(obj.profiles)
      ? (obj.profiles as Record<string, unknown>)
      : {};
  const connections = Object.entries(rawProfiles).map(([name, value]) => {
    const entry = value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
    const text = (key: string) => (typeof entry[key] === "string" ? (entry[key] as string) : "");
    const aliases =
      entry.model_aliases && typeof entry.model_aliases === "object" && !Array.isArray(entry.model_aliases)
        ? (entry.model_aliases as Record<string, unknown>)
        : {};
    const alias = (tier: string) => (typeof aliases[tier] === "string" ? (aliases[tier] as string) : "");
    return {
      ...emptyConnection(name),
      label: text("label"),
      baseUrl: text("base_url"),
      auth: entry.auth === "api_key" ? ("api_key" as const) : ("token" as const),
      tokenVar: text("token_var"),
      model: text("model"),
      smallFastModel: text("small_fast_model"),
      aliasOpus: alias("opus"),
      aliasSonnet: alias("sonnet"),
      aliasHaiku: alias("haiku"),
      timeoutMs: typeof entry.timeout_ms === "number" ? String(entry.timeout_ms) : "",
    };
  });
  return {
    connections,
    activeConnection: typeof obj.active === "string" ? obj.active : "",
    yolo: obj.yolo === true,
    statusline: obj.statusline === "builtin" ? "builtin" : "untouched",
  };
}

/**
 * 表单字段 → 定义里的 settings 段。一项都没配就不写这个段：
 * 空段和没有段在引擎侧等价，但不写更干净，也让「从没配过」一眼可辨。
 */
export function buildAgentSettings(formValues: AgentFormValues): Record<string, unknown> | undefined {
  const profiles: Record<string, unknown> = {};
  for (const item of formValues.connections) {
    const name = item.name.trim();
    if (!name) continue;
    profiles[name] = connectionToDefinitionValue(item);
  }

  const settings: Record<string, unknown> = {};
  if (Object.keys(profiles).length > 0) {
    settings.profiles = profiles;
    // active 必须指向真实存在的一套，否则引擎当场拒绝
    const active = formValues.activeConnection.trim();
    if (active && profiles[active]) settings.active = active;
  }
  if (formValues.yolo) settings.yolo = true;
  if (formValues.statusline === "builtin") settings.statusline = "builtin";
  return Object.keys(settings).length > 0 ? settings : undefined;
}

/** 这份定义有没有配过运行参数（决定保存后要不要写助手的配置文件）。 */
export function hasAgentSettings(definition: Record<string, unknown> | undefined): boolean {
  const raw = definition?.settings;
  return !!raw && typeof raw === "object" && !Array.isArray(raw) && Object.keys(raw).length > 0;
}

/** 从定义读取运行设置；接入 profiles 与 active 由接入工作台单独管理。 */
export function runtimeSettingsOf(
  definition: Record<string, unknown> | undefined,
): AgentRuntimeSettings {
  const raw = definition?.settings;
  const settings = raw && typeof raw === "object" && !Array.isArray(raw)
    ? (raw as Record<string, unknown>)
    : {};
  return Object.fromEntries(
    Object.entries(settings)
      .filter(([key]) => key !== "profiles" && key !== "active")
      .map(([key, value]) => [key, JSON.parse(JSON.stringify(value))]),
  );
}

/**
 * 用完整运行设置文档替换定义里的运行设置，同时保留接入 profiles/active 与定义其他字段。
 * 表单模式从 runtimeSettingsOf 开始编辑，因此未知运行键也会原样保留；JSON 模式则允许研发
 * 明确新增或删除未知键，等待新版适配器接管。
 */
export function updateAgentRuntimeSettings(
  definition: Record<string, unknown>,
  runtime: AgentRuntimeSettings,
): Record<string, unknown> {
  const next = JSON.parse(JSON.stringify(definition)) as Record<string, unknown>;
  const raw = next.settings;
  const previous = raw && typeof raw === "object" && !Array.isArray(raw)
    ? (raw as Record<string, unknown>)
    : {};
  const settings: Record<string, unknown> = {};
  if (previous.profiles !== undefined) settings.profiles = previous.profiles;
  if (previous.active !== undefined) settings.active = previous.active;
  for (const [key, value] of Object.entries(runtime)) settings[key] = value;

  if (Object.keys(settings).length > 0) next.settings = settings;
  else delete next.settings;
  return next;
}

/** 按后端 schema 校验并归一化运行设置；未知键保留，已知空值删除。 */
export function normalizeAgentRuntimeSettings(
  input: AgentRuntimeSettings,
  fields: AgentRuntimeField[],
): AgentRuntimeSettings {
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new Error("运行设置必须是 JSON 对象");
  }
  if ("profiles" in input || "active" in input) {
    throw new Error("接入 profiles 与 active 请在接入工作台维护，不能放进运行设置 JSON");
  }
  const normalized = JSON.parse(JSON.stringify(input)) as AgentRuntimeSettings;
  for (const field of fields) {
    const value = normalized[field.key];
    if (field.type === "boolean") {
      if (value === undefined || value === false) delete normalized[field.key];
      else if (value !== true) throw new Error(`${field.label}必须是 true 或 false`);
      continue;
    }
    if (field.type === "enum") {
      if (value === undefined || value === null || value === "") {
        delete normalized[field.key];
        continue;
      }
      if (typeof value !== "string" || !field.options?.some((option) => option.value === value)) {
        throw new Error(`${field.label}不是适配器支持的取值`);
      }
      continue;
    }
    if (field.type === "positive_integer") {
      if (value === undefined || value === null || value === "") {
        delete normalized[field.key];
        continue;
      }
      const number = typeof value === "number" ? value : Number(String(value).trim());
      if (!Number.isInteger(number) || number <= 0) {
        throw new Error(`${field.label}必须是正整数，留空表示使用客户端默认值`);
      }
      normalized[field.key] = number;
      continue;
    }
    throw new Error(`当前版本不认识运行设置类型 ${field.type}（${field.key}）`);
  }
  return normalized;
}

export function runtimeRequiresYoloConfirmation(
  runtime: AgentRuntimeSettings,
  fields: AgentRuntimeField[],
): boolean {
  return fields.some((field) => field.requires_yolo_confirmation && runtime[field.key] === true);
}

/** 定义里当前生效的接入配置名；没有就返回 null。 */
export function activeConnectionOf(definition: Record<string, unknown> | undefined): string | null {
  const raw = definition?.settings;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const active = (raw as Record<string, unknown>).active;
  return typeof active === "string" && active ? active : null;
}

/** 定义里的接入配置清单（名字 + 显示名），供卡片上的切换用。 */
export function connectionsOf(
  definition: Record<string, unknown> | undefined,
): { name: string; label: string }[] {
  const raw = definition?.settings;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return [];
  const profiles = (raw as Record<string, unknown>).profiles;
  if (!profiles || typeof profiles !== "object" || Array.isArray(profiles)) return [];
  return Object.entries(profiles as Record<string, unknown>).map(([name, value]) => {
    const entry = value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
    return { name, label: typeof entry.label === "string" && entry.label ? entry.label : name };
  });
}

export function buildAgentDefinition(
  formValues: AgentFormValues,
  initialDefinition: Record<string, unknown> | undefined,
  dirtyFields: Set<string> | undefined,
  draftProviders: Record<string, Record<string, unknown>>
): Record<string, unknown> {
  // If editing an existing definition and nothing was modified, return untouched clone!
  if (initialDefinition && dirtyFields && dirtyFields.size === 0) {
    return JSON.parse(JSON.stringify(initialDefinition));
  }

  const baseDef: Record<string, unknown> = initialDefinition
    ? JSON.parse(JSON.stringify(initialDefinition))
    : {};

  if (!dirtyFields || dirtyFields.has("displayName")) {
    if (formValues.displayName.trim()) {
      baseDef.display_name = formValues.displayName.trim();
    } else if (!baseDef.display_name) {
      baseDef.display_name = formValues.name;
    }
  }

  if (!dirtyFields || dirtyFields.has("bin")) {
    baseDef.bin = formValues.bin.trim();
  }

  const channel = formValues.channel.trim() || "custom";

  if (!dirtyFields || dirtyFields.has("preferredProvider")) {
    baseDef.preferred_provider = formValues.preferredProvider?.trim() || channel;
  } else if (!baseDef.preferred_provider) {
    baseDef.preferred_provider = channel;
  }

  // draftProviders 已在输入时写入在途编辑，保存时直接采用，不按当前平台重放命令，
  // 避免“编辑 macOS 后切到 Linux 保存”把未编辑的 Linux fallback 具体化。
  const currentProviders: Record<string, Record<string, unknown>> = JSON.parse(
    JSON.stringify(draftProviders)
  );
  const active = currentProviders[channel] ?? {};
  const initialProviders =
    initialDefinition?.providers &&
    typeof initialDefinition.providers === "object" &&
    !Array.isArray(initialDefinition.providers)
      ? (initialDefinition.providers as Record<string, unknown>)
      : {};
  // 已有 provider 原样保留 managed 缺省（custom 探测要求显式 true）；仅新增渠道默认 true
  const isNewChannel = !(channel in initialProviders);
  currentProviders[channel] =
    isNewChannel && active.managed === undefined ? { managed: true, ...active } : active;
  baseDef.providers = currentProviders;

  // 其他设置 JSON 被编辑过时，以新对象整体替换可编辑的未知字段（标准字段不受影响），
  // 使“删除某个旧字段 / 清空 JSON”真正生效；未编辑时原字段保持不变。
  if (dirtyFields?.has("advancedJson")) {
    for (const k of Object.keys(baseDef)) {
      if (!STANDARD_DEFINITION_KEYS.has(k)) delete baseDef[k];
    }
  }

  if (formValues.advancedJson?.trim()) {
    try {
      const advancedParsed = JSON.parse(formValues.advancedJson);
      if (advancedParsed && typeof advancedParsed === "object" && !Array.isArray(advancedParsed)) {
        for (const [k, v] of Object.entries(advancedParsed)) {
          if (
            k !== "bin" &&
            k !== "providers" &&
            k !== "display_name" &&
            k !== "preferred_provider" &&
            k !== "settings" &&
            k !== "_delete" &&
            k !== "_replace"
          ) {
            baseDef[k] = v;
          }
        }
      }
    } catch {
      // Ignored
    }
  }

  const settings = buildAgentSettings(formValues);
  if (settings) {
    baseDef.settings = settings;
  } else {
    delete baseDef.settings;
  }

  return baseDef;
}
