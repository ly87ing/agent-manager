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
  };
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

  return baseDef;
}
