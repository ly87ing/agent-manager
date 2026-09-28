import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Loader2, X } from "lucide-react";
import {
  CONNECTION_NAME_RE,
  emptyConnection,
  isPlainHttpUrl,
  validateConnectionEntry,
  type ConnectionEntry,
} from "../lib/agentDefinitionForm";
import {
  connectionFromPreset,
  presetConnectionName,
  presetsFor,
} from "../lib/connectionPresets";
import { cn } from "../lib/utils";
import { keyStatus, type SecretSourceState } from "../lib/secretSource";
import {
  SUBPROCESS_SCRUB_ENV,
  buildProfileDefinition,
  connectionToProfileInput,
  defaultConfigDir,
  PROFILE_ID_RE,
  profileConnectionUpdate,
  profileLaunchValue,
  type ProfileDraftPreview,
  type ProfilePreviewRequest,
} from "../lib/profileDefinition";
import { detectSystemPlatform } from "../lib/agentDefinitionForm";
import { ALL_SEED_ITEMS, type SeedItem } from "../lib/profileSeed";
import { ProfileSeedPicker } from "./ProfileSeedPicker";
import type { AliasFilePreview } from "../lib/useAliasSource";
import type { ElementState } from "../lib/contract";

export type ConnectionKind = "claude" | "codex";

export interface ConnectionSubmission {
  connection: ConnectionEntry;
  secretValue: string;
  environment?: {
    id: string;
    configDir: string;
    seedItems: SeedItem[];
    saveTemplate: boolean;
    scrubSubprocess: boolean;
    preview: { config: ProfileDraftPreview; alias: AliasFilePreview };
  };
  /** 用户逐环境勾选的更新目标；未勾选的环境不写入。 */
  profileUpdates?: string[];
  /** 对实例自己保留的字段，用户逐字段选择改为当前 Provider 值。 */
  profileOverrides?: Record<string, string[]>;
  /** 与本次真实内容预览绑定的逐环境文件摘要。 */
  previewBases?: Record<string, { fileBase: string; contentBase: string }>;
}

export interface ConnectionProfileImpact {
  id: string;
  label: string;
  definition: Record<string, unknown>;
  state: ElementState;
  base: string | null;
  definitionBase: string | null;
  blockedReason?: string;
}

const FOCUSABLE_SELECTOR =
  'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

export function ConnectionFormDialog({
  agentId,
  agentName,
  agentBin,
  kind,
  existingNames,
  existingProfileIds = [],
  initialConnection,
  affectedProfiles = [],
  variant,
  intent,
  secretSource,
  busy,
  error,
  onSubmit,
  onPreview,
  onAliasPreview,
  aliasPreviewBase,
  onOpenSecrets,
  onFetchModels,
  onClose,
}: {
  agentId: string;
  agentName: string;
  agentBin: string;
  kind: ConnectionKind;
  existingNames: string[];
  existingProfileIds?: string[];
  /** 编辑或复制时的预填值；具体语义由 variant 决定。 */
  initialConnection?: ConnectionEntry;
  affectedProfiles?: ConnectionProfileImpact[];
  /** 复制模式允许创建新名称；凭据取值始终为空。 */
  variant?: "edit" | "duplicate";
  /** environment：从「新建环境」进来，直接建立隔离环境，Provider 模板可选。 */
  intent?: "environment";
  /** 密钥文件的状态：说明 API Key 会写到哪、这个变量现在有没有值。 */
  secretSource?: SecretSourceState | null;
  busy: boolean;
  error?: string | null;
  onSubmit: (submission: ConnectionSubmission) => void;
  onPreview: (request: ProfilePreviewRequest) => Promise<ProfileDraftPreview | string>;
  onAliasPreview: (name: string, value: string) => Promise<AliasFilePreview | string>;
  aliasPreviewBase: string;
  onOpenSecrets?: () => void;
  /** 用表单里的地址与凭据变量问接口能用哪些模型；失败时返回原因。只在点按钮时才联网。 */
  onFetchModels?: (draft: { kind: ConnectionKind; baseUrl: string; tokenVar: string }) => Promise<string[] | string>;
  onClose: () => void;
}) {
  const [fetchedModels, setFetchedModels] = useState<string[] | null>(null);
  const [modelsMessage, setModelsMessage] = useState<string | null>(null);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [previewBusy, setPreviewBusy] = useState(false);
  const previewSerial = useRef(0);
  const [preview, setPreview] = useState<{
    key: string;
    results: { id: string; result: ProfileDraftPreview | string }[];
  } | null>(null);
  const rootRef = useRef<HTMLElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(
    typeof document === "undefined" ? null : (document.activeElement as HTMLElement | null),
  );
  const presets = presetsFor(kind);
  const prefilled = initialConnection !== undefined;
  const editing = prefilled && (variant === undefined || variant === "edit");
  const duplicating = prefilled && variant === "duplicate";
  const referenced = Boolean(initialConnection?.providerRef);
  const [choice, setChoice] = useState<string | null>(prefilled ? variant ?? "edit" : null);
  const [connection, setConnection] = useState<ConnectionEntry | null>(
    initialConnection ? { ...initialConnection } : null,
  );
  const [secretValue, setSecretValue] = useState("");
  const [environmentId, setEnvironmentId] = useState("");
  const [environmentDir, setEnvironmentDir] = useState("");
  const [saveTemplate, setSaveTemplate] = useState(false);
  const [seedItems, setSeedItems] = useState<SeedItem[]>(ALL_SEED_ITEMS);
  const [scrubSubprocess, setScrubSubprocess] = useState(kind === "claude" && detectSystemPlatform() !== "linux");
  const [selectedProfiles, setSelectedProfiles] = useState<string[]>([]);
  const [selectedOverrides, setSelectedOverrides] = useState<Record<string, string[]>>({});
  const [localError, setLocalError] = useState<string | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(!duplicating && Boolean(
    initialConnection?.timeoutMs || initialConnection?.smallFastModel || initialConnection?.aliasOpus
    || initialConnection?.aliasSonnet || initialConnection?.aliasHaiku || initialConnection?.aliasFable || initialConnection?.subagentModel || initialConnection?.auth === "api_key",
  ));
  const dialogTitle = editing
    ? `编辑 ${initialConnection.label || initialConnection.name}`
    : duplicating
      ? "复制 Provider"
      : intent === "environment"
        ? `为 ${agentName} 新建第三方环境`
        : `为 ${agentName} 添加接入`;

  // 编辑已有接入（例如从实际配置导入的那套）时地址、凭据变量与模型可以留空：它本来可能就没有
  const strict = !editing && !referenced;
  const validationError = connection
    ? (!connection.label.trim() && duplicating ? "请填写显示名称" : validateConnectionEntry(connection, strict))
    : null;
  const duplicateName = Boolean(
    connection && existingNames.includes(connection.name.trim()) && !(editing && connection.name.trim() === initialConnection?.name),
  );
  const saveDisabled = busy || !connection || (duplicating && (Boolean(validationError) || duplicateName));
  const impacts = editing && initialConnection && connection
    ? affectedProfiles.map((profile) => ({
        ...profile,
        update: profileConnectionUpdate(profile.id, profile.definition, initialConnection, connection, kind,
          selectedOverrides[profile.id] ?? []),
      }))
    : [];
  const selectedUpdates = impacts.filter((profile) => selectedProfiles.includes(profile.id)
    && profile.update && profile.update.changes.length > 0
    && (profile.state === "managed-ok" || profile.state === "not-materialized")
    && profile.base && profile.definitionBase && !profile.blockedReason);
  const newEnvironment = intent === "environment" && connection && !validationError && !duplicateName
    && PROFILE_ID_RE.test(environmentId.trim())
    ? connectionToProfileInput({ id: agentId, kind }, connection, existingProfileIds)
    : null;
  const newEnvironmentInput = newEnvironment ? {
    ...newEnvironment.input,
    id: environmentId.trim(),
    configDir: environmentDir.trim() || defaultConfigDir(kind, environmentId.trim()),
    sourceConnection: saveTemplate ? connection?.name : undefined,
    extraEnv: {
      ...newEnvironment.input.extraEnv,
      ...(kind === "claude" && scrubSubprocess ? { [SUBPROCESS_SCRUB_ENV]: "1" } : {}),
    },
  } : null;
  const environmentLaunch = newEnvironmentInput ? profileLaunchValue(newEnvironmentInput, agentBin) : "";
  const aliasPreviewKey = JSON.stringify([aliasPreviewBase, newEnvironmentInput?.id, environmentLaunch]);
  const [aliasPreview, setAliasPreview] = useState<{ key: string; result: AliasFilePreview | string } | null>(null);
  const previewRequests: ProfilePreviewRequest[] = [
    ...(newEnvironmentInput ? [{
      id: newEnvironmentInput.id,
      definition: buildProfileDefinition(newEnvironmentInput, agentBin),
      seedItems,
    }] : []),
    ...selectedUpdates.flatMap((profile) => profile.update && profile.base && profile.definitionBase ? [{
      id: profile.id,
      base: profile.base,
      definitionBase: profile.definitionBase,
      candidate: profile.update.candidate,
      ...(secretValue ? { secretVar: connection?.tokenVar.trim(), secretValue } : {}),
    }] : []),
  ];
  const previewKey = JSON.stringify(previewRequests);
  const waitingForPreview = (previewRequests.length > 0
    && (previewBusy || preview?.key !== previewKey || preview.results.length !== previewRequests.length
      || preview.results.some(({ result }) => typeof result === "string")))
    || (Boolean(newEnvironmentInput)
      && (aliasPreview?.key !== aliasPreviewKey || typeof aliasPreview.result === "string"));

  const refreshPreview = async () => {
    const serial = ++previewSerial.current;
    setPreviewBusy(true);
    try {
      const results: { id: string; result: ProfileDraftPreview | string }[] = [];
      for (const request of previewRequests) {
        const result = await onPreview(request);
        results.push({ id: request.id, result: "candidate" in request && typeof result !== "string"
          && (!result.file_base || !result.content_base) ? "预览缺少写入校验信息，请重新读取" : result });
      }
      if (serial === previewSerial.current) setPreview({ key: previewKey, results });
    } finally {
      if (serial === previewSerial.current) setPreviewBusy(false);
    }
  };

  useEffect(() => {
    if (previewRequests.length === 0) {
      setPreviewBusy(false);
      return;
    }
    setPreviewBusy(true);
    const timer = window.setTimeout(() => void refreshPreview(), 300);
    return () => {
      window.clearTimeout(timer);
      previewSerial.current += 1;
    };
  }, [previewKey]);

  useEffect(() => {
    if (!newEnvironmentInput || !environmentLaunch) return;
    let current = true;
    const timer = window.setTimeout(() => {
      void onAliasPreview(newEnvironmentInput.id, environmentLaunch)
        .then((result) => { if (current) setAliasPreview({ key: aliasPreviewKey, result }); })
        .catch((error) => { if (current) setAliasPreview({ key: aliasPreviewKey, result: String(error) }); });
    }, 300);
    return () => { current = false; window.clearTimeout(timer); };
  }, [aliasPreviewKey]);

  const currentAliasPreview = newEnvironmentInput && aliasPreview?.key === aliasPreviewKey
    ? aliasPreview.result : null;

  useLayoutEffect(() => {
    return () => restoreFocusRef.current?.focus();
  }, []);

  const choosePreset = (presetId: string) => {
    const preset = presets.find((item) => item.id === presetId);
    if (!preset) return;
    setChoice(preset.id);
    const next = connectionFromPreset(preset, presetConnectionName(preset, existingNames));
    setConnection(next);
    if (intent === "environment") {
      setEnvironmentId(connectionToProfileInput({ id: agentId, kind }, next, existingProfileIds)?.input.id ?? "");
      setEnvironmentDir("");
    }
    setLocalError(null);
  };

  const chooseCustom = () => {
    let name = "relay";
    let index = 2;
    while (existingNames.includes(name)) {
      name = `relay${index}`;
      index += 1;
    }
    setChoice("custom");
    const next = emptyConnection(name);
    setConnection(next);
    if (intent === "environment") {
      setEnvironmentId(connectionToProfileInput({ id: agentId, kind }, next, existingProfileIds)?.input.id ?? "");
      setEnvironmentDir("");
    }
    setLocalError(null);
  };

  const patch = (next: Partial<ConnectionEntry>) => {
    setConnection((current) => (current ? { ...current, ...next } : current));
    setLocalError(null);
  };

  const close = () => {
    if (!busy) onClose();
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
      return;
    }
    if (event.key !== "Tab") return;
    const root = rootRef.current;
    if (!root) return;
    const focusable = Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
      (element) => !element.hasAttribute("disabled"),
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  const submit = () => {
    if (!connection) {
      setLocalError("先选择一个厂商预设，或选择自定义接入");
      return;
    }
    const name = connection.name.trim();
    if (!CONNECTION_NAME_RE.test(name)) {
      setLocalError("唯一名称只能用字母、数字、连字符与下划线，长度不超过 64");
      return;
    }
    if ((intent !== "environment" || saveTemplate) && existingNames.includes(name) && !(editing && name === initialConnection?.name)) {
      setLocalError(`已经有名为 ${name} 的接入，请换一个名称`);
      return;
    }
    const validationError = validateConnectionEntry(connection, strict);
    if (validationError) {
      setLocalError(validationError);
      return;
    }
    const configDir = environmentDir.trim() || defaultConfigDir(kind, environmentId.trim());
    if (intent === "environment") {
      if (!PROFILE_ID_RE.test(environmentId.trim()) || existingProfileIds.includes(environmentId.trim())) {
        setLocalError("环境名只能用小写字母、数字与连字符，且不能与已有环境或助手重名");
        return;
      }
      if (!/^(~\/|\/)/.test(configDir)) {
        setLocalError("配置目录要填绝对路径或以 ~/ 开头");
        return;
      }
    }
    const environmentPreview = newEnvironmentInput
      ? preview?.results.find((item) => item.id === newEnvironmentInput.id)?.result : null;
    let environmentSnapshot: { config: ProfileDraftPreview; alias: AliasFilePreview } | undefined;
    if (intent === "environment") {
      if (!newEnvironmentInput || waitingForPreview
        || !environmentPreview || typeof environmentPreview === "string"
        || !currentAliasPreview || typeof currentAliasPreview === "string") {
        setLocalError("文件预览已失效，请重新读取后保存");
        return;
      }
      environmentSnapshot = { config: environmentPreview, alias: currentAliasPreview };
    }
    const previewBases: Record<string, { fileBase: string; contentBase: string }> = {};
    for (const profile of selectedUpdates) {
      const result = preview?.results.find((item) => item.id === profile.id)?.result;
      if (preview?.key !== previewKey || !result || typeof result === "string"
        || !result.file_base || !result.content_base) {
        setLocalError("已选环境的文件预览已失效，请重新读取");
        return;
      }
      previewBases[profile.id] = { fileBase: result.file_base, contentBase: result.content_base };
    }
    onSubmit({
      connection: {
        ...connection,
        name,
        label: connection.label.trim() || name,
        note: connection.note?.trim() ?? "",
        baseUrl: connection.baseUrl.trim(),
        tokenVar: connection.tokenVar.trim(),
        model: connection.model.trim(),
      },
      secretValue,
      environment: intent === "environment" && environmentSnapshot
        ? { id: environmentId.trim(), configDir, seedItems, saveTemplate, scrubSubprocess,
            preview: environmentSnapshot }
        : undefined,
      profileUpdates: selectedUpdates.map((profile) => profile.id),
      profileOverrides: Object.fromEntries(selectedUpdates.map((profile) =>
        [profile.id, selectedOverrides[profile.id] ?? []])),
      previewBases,
    });
  };

  const shownError = error ?? localError;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in"
      onClick={(event) => event.target === event.currentTarget && close()}
    >
      <section
        ref={rootRef}
        role="dialog"
        aria-modal="true"
        aria-label={dialogTitle}
        onKeyDown={onKeyDown}
        className="flex max-h-[92vh] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border bg-card shadow-xl animate-zoom-in-95"
      >
        <div className="flex items-start justify-between gap-3 border-b px-5 py-4">
          <div>
            <h2 className="text-lg font-semibold text-foreground">{dialogTitle}</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              {editing
                ? `修改 ${agentName} 的这套接入；已建环境可在下方逐项选择是否更新，默认不改。留空凭据取值不会覆盖外部密钥文件中的值。`
                : duplicating
                  ? "确认新名称即可创建；原 Provider、当前接入和客户端配置都不会改变。"
                : "先选厂商，再确认地址、模型和凭据；安装命令不在这里。"}
            </p>
          </div>
          <button
            type="button"
            aria-label="关闭"
            disabled={busy}
            onClick={close}
            className="rounded p-1 text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <div className="overflow-y-auto px-5 py-4">
          {!prefilled && (
            <fieldset>
              <legend className="text-sm font-medium text-foreground">选择 Provider</legend>
              <div className="mt-2 flex flex-wrap gap-2">
                {presets.map((preset, index) => (
                  <button
                    key={preset.id}
                    type="button"
                    autoFocus={index === 0}
                    data-testid={`connection-preset-${preset.id}`}
                    aria-pressed={choice === preset.id}
                    onClick={() => choosePreset(preset.id)}
                    title={preset.source}
                    className={cn(
                      "rounded-full border px-3 py-1.5 text-left text-sm transition-colors",
                      choice === preset.id ? "border-primary bg-primary/10" : "hover:bg-accent",
                    )}
                  >
                    <span className="font-medium text-foreground">{preset.label}</span>
                  </button>
                ))}
                <button
                  type="button"
                  autoFocus={presets.length === 0}
                  data-testid="connection-preset-custom"
                  aria-pressed={choice === "custom"}
                  onClick={chooseCustom}
                  className={cn(
                    "rounded-full border px-3 py-1.5 text-left text-sm transition-colors",
                    choice === "custom" ? "border-primary bg-primary/10" : "hover:bg-accent",
                  )}
                >
                  <span className="font-medium text-foreground">自定义</span>
                </button>
              </div>
            </fieldset>
          )}

          {connection && (
            <div className={cn("space-y-4", prefilled ? "" : "mt-4 border-t pt-4")}>
              {duplicating ? (
                <>
                  <label className="block text-sm font-medium text-foreground">
                    新 Provider 名称
                    <input
                      data-testid="new-connection-label"
                      autoFocus
                      value={connection.label}
                      onChange={(event) => patch({ label: event.target.value })}
                      placeholder="例如：工作线路副本"
                      className={cn(
                        "mt-1 w-full rounded border p-2 text-sm",
                        connection.label.trim() ? "border-input" : "border-amber-500",
                      )}
                    />
                    {!connection.label.trim() && (
                      <span className="mt-1 block text-xs text-warning">填写名称后才能创建副本。</span>
                    )}
                  </label>

                  <div className="overflow-hidden rounded-xl border" data-testid="duplicate-connection-summary">
                    <div className="border-b bg-muted/40 px-3 py-2 text-xs font-medium text-foreground">
                      将原样复制
                    </div>
                    <dl className="divide-y text-sm">
                      <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-3 px-3 py-2">
                        <dt className="text-muted-foreground">API 地址</dt>
                        <dd className="break-all font-mono text-xs text-foreground">{connection.baseUrl}</dd>
                      </div>
                      <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-3 px-3 py-2">
                        <dt className="text-muted-foreground">默认模型</dt>
                        <dd className="break-all font-mono text-xs text-foreground">{connection.model}</dd>
                      </div>
                      <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-3 px-3 py-2">
                        <dt className="text-muted-foreground">凭据来源</dt>
                        <dd className="break-all font-mono text-xs text-foreground">环境变量 {connection.tokenVar}</dd>
                      </div>
                    </dl>
                    {(connection.timeoutMs || connection.smallFastModel || connection.aliasOpus || connection.aliasSonnet || connection.aliasHaiku || connection.aliasFable || connection.subagentModel || connection.auth === "api_key") && (
                      <details className="border-t px-3 py-2 text-xs">
                        <summary className="cursor-pointer text-muted-foreground">查看一并复制的高级配置</summary>
                        <dl className="mt-2 grid grid-cols-[7rem_minmax(0,1fr)] gap-x-3 gap-y-1 text-foreground">
                          {connection.auth === "api_key" && <><dt>认证方式</dt><dd className="font-mono">x-api-key</dd></>}
                          {connection.timeoutMs && <><dt>请求超时</dt><dd className="font-mono">{connection.timeoutMs} ms</dd></>}
                          {connection.smallFastModel && <><dt>小任务模型（已弃用）</dt><dd className="break-all font-mono">{connection.smallFastModel}</dd></>}
                          {connection.aliasOpus && <><dt>Opus 映射</dt><dd className="break-all font-mono">{connection.aliasOpus}</dd></>}
                          {connection.aliasSonnet && <><dt>Sonnet 映射</dt><dd className="break-all font-mono">{connection.aliasSonnet}</dd></>}
                          {connection.aliasHaiku && <><dt>Haiku 映射</dt><dd className="break-all font-mono">{connection.aliasHaiku}</dd></>}
                          {connection.aliasFable && <><dt>Fable 映射</dt><dd className="break-all font-mono">{connection.aliasFable}</dd></>}
                          {connection.subagentModel && <><dt>子代理模型</dt><dd className="break-all font-mono">{connection.subagentModel}</dd></>}
                        </dl>
                      </details>
                    )}
                  </div>

                  <p className="rounded-lg border border-primary/20 bg-primary/5 px-3 py-2 text-xs leading-5 text-foreground">
                    创建后只会在 {agentName} 的 Provider 列表增加一项；不会应用，只复用凭据变量名，不读取或新存一份 API Key。需要改地址或模型，可创建后单独点“编辑”。
                  </p>
                </>
              ) : (
                <>
              <div className="flex items-center justify-between gap-3">
                <h3 className="text-sm font-medium text-foreground">Provider 配置</h3>
                {!prefilled && (
                  <span data-testid="selected-connection-preset" className="text-xs text-muted-foreground">
                    已选择：{presets.find((preset) => preset.id === choice)?.label ?? "自定义"}
                  </span>
                )}
              </div>
              {!prefilled && (() => {
                const preset = presets.find((item) => item.id === choice);
                if (!preset || (!preset.apiKeyUrl && !preset.docsUrl)) return null;
                return (
                  <div data-testid="connection-preset-links" className="space-y-1 rounded border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
                    {[
                      { label: "获取密钥", url: preset.apiKeyUrl, id: "key" },
                      { label: "接入文档", url: preset.docsUrl, id: "docs" },
                    ]
                      .filter((item): item is { label: string; url: string; id: string } => Boolean(item.url))
                      .map((item) => (
                        <p key={item.id} className="flex flex-wrap items-center gap-2">
                          <span>{item.label}：</span>
                          <span className="break-all font-mono text-foreground">{item.url}</span>
                          <button
                            type="button"
                            data-testid={`connection-preset-copy-${item.id}`}
                            onClick={() => void navigator.clipboard?.writeText(item.url)}
                            className="rounded border bg-card px-1.5 py-0.5 text-[11px] hover:bg-accent"
                          >
                            复制
                          </button>
                        </p>
                      ))}
                    <p>出处：{preset.source}</p>
                  </div>
                );
              })()}
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <label className="text-sm text-foreground">
                  显示名称
                  <input
                    data-testid="new-connection-label"
                    autoFocus={prefilled}
                    value={connection.label}
                    onChange={(event) => patch({ label: event.target.value })}
                    placeholder="例如：工作账号"
                    className="mt-1 w-full rounded border border-input p-2 text-sm"
                  />
                </label>
                <label className="text-sm text-foreground">
                  <span className="flex items-center justify-between gap-2">
                    默认模型
                    {onFetchModels && !referenced && (
                      <button
                        type="button"
                        data-testid="connection-fetch-models"
                        disabled={fetchingModels || !connection.baseUrl.trim() || !connection.tokenVar.trim()}
                        title="用上面的地址与凭据变量问接口能用哪些模型；只在点它时联网，不改任何配置"
                        onClick={async () => {
                          setFetchingModels(true);
                          setModelsMessage(null);
                          try {
                            const result = await onFetchModels({ kind, baseUrl: connection.baseUrl.trim(), tokenVar: connection.tokenVar.trim() });
                            if (typeof result === "string") {
                              setFetchedModels(null);
                              setModelsMessage(result);
                            } else {
                              setFetchedModels(result);
                              setModelsMessage(`已获取 ${result.length} 个模型，模型与别名输入框可直接下拉挑选`);
                            }
                          } finally {
                            setFetchingModels(false);
                          }
                        }}
                        className="inline-flex items-center gap-1 rounded border px-2 py-0.5 text-xs font-normal text-muted-foreground hover:bg-accent disabled:opacity-50"
                      >
                        {fetchingModels && <Loader2 className="h-3 w-3 animate-spin" aria-hidden />}
                        {fetchingModels ? "正在获取…" : "获取模型列表"}
                      </button>
                    )}
                  </span>
                  <input
                    data-testid="new-connection-model"
                    value={connection.model}
                    onChange={(event) => patch({ model: event.target.value })}
                    placeholder={kind === "claude" ? "例如：claude-sonnet-4-5" : "例如：gpt-5"}
                    list={fetchedModels ? "connection-model-options" : undefined}
                    className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                    spellCheck={false}
                  />
                </label>
              </div>
              {modelsMessage && (
                <p
                  data-testid="connection-models-message"
                  className={cn("text-xs", fetchedModels ? "text-muted-foreground" : "text-destructive")}
                >
                  {modelsMessage}
                </p>
              )}
              {fetchedModels && (
                <datalist id="connection-model-options" data-testid="connection-model-options">
                  {fetchedModels.map((model) => (
                    <option key={model} value={model} />
                  ))}
                </datalist>
              )}
              {referenced ? (
                <p data-testid="connection-provider-ref" className="rounded border bg-muted px-3 py-2 text-xs text-muted-foreground">
                  引用 config.toml 中已有的接入方式 <code>{connection.providerRef}</code>：地址、凭据变量与超时由那张表决定，这里不改。
                </p>
              ) : (
              <label className="block text-sm text-foreground">
                API 地址
                <input
                  data-testid="new-connection-base-url"
                  value={connection.baseUrl}
                  onChange={(event) => patch({ baseUrl: event.target.value })}
                  placeholder="https://api.example.com"
                  className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                  spellCheck={false}
                />
                {isPlainHttpUrl(connection.baseUrl) && (
                  <span data-testid="connection-plain-http" className="mt-1 block text-xs text-warning">
                    非 https 地址：令牌将明文传输，只在可信的内网或本机使用。
                  </span>
                )}
              </label>
              )}
              <label className="block text-sm text-foreground">
                备注（可选）
                <textarea
                  data-testid="new-connection-note"
                  value={connection.note ?? ""}
                  onChange={(event) => patch({ note: event.target.value })}
                  maxLength={200}
                  rows={2}
                  placeholder="例如：工作账号、备用线路、套餐到期时间"
                  className="mt-1 w-full resize-y rounded border border-input p-2 text-sm"
                />
                <span className="mt-1 block text-xs text-muted-foreground">
                  仅用于本应用展示和搜索，不会写进助手配置；导出接入时会一并包含。
                </span>
              </label>
              {!referenced && (() => {
                const status = keyStatus(secretSource ?? null, connection.tokenVar);
                return (
                  <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                    {kind === "claude" || kind === "codex" ? (
                      <label className="text-sm text-foreground">
                        API Key
                        <input
                          type="password"
                          autoComplete="new-password"
                          data-testid="new-connection-secret"
                          value={secretValue}
                          onChange={(event) => setSecretValue(event.target.value)}
                          placeholder="粘贴厂商给你的 API Key"
                          className="mt-1 w-full rounded border border-input p-2 text-sm"
                        />
                      </label>
                    ) : null}
                    <label className="text-sm text-foreground">
                      保存为变量
                      <input
                        data-testid="new-connection-token-var"
                        value={connection.tokenVar}
                        onChange={(event) => patch({ tokenVar: event.target.value })}
                        placeholder={kind === "claude" ? "ANTHROPIC_AUTH_TOKEN" : "OPENAI_API_KEY"}
                        className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                        spellCheck={false}
                      />
                      <span className="mt-1 block text-xs text-muted-foreground">
                        密钥文件里用这个名字保存；几套接入用同一个 Key 就填同一个名字。
                      </span>
                    </label>
                    <p
                      data-testid="new-connection-key-status"
                      className={cn(
                        "text-xs sm:col-span-2",
                        status.tone === "ok" ? "text-success" : status.tone === "warn" ? "text-warning" : "text-muted-foreground",
                      )}
                    >
                      {status.text}
                    </p>
                  </div>
                );
              })()}

              <details
                className="rounded-lg border px-3 py-2"
                open={advancedOpen}
                onToggle={(event) => setAdvancedOpen(event.currentTarget.open)}
              >
                <summary data-testid="new-connection-advanced" className="cursor-pointer text-sm font-medium text-foreground">
                  高级字段
                </summary>
                <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
                  <label className="text-sm text-foreground">
                    唯一名称
                    <input
                      data-testid="new-connection-name"
                      value={connection.name}
                      disabled={editing}
                      onChange={(event) => patch({ name: event.target.value })}
                      title={editing ? "唯一名称用于配置引用，编辑时保持不变" : undefined}
                      className="mt-1 w-full rounded border border-input p-2 font-mono text-sm disabled:bg-muted disabled:text-muted-foreground"
                    />
                  </label>
                  {!referenced && <label className="text-sm text-foreground">
                    请求超时（毫秒，可选）
                    <input
                      value={connection.timeoutMs}
                      onChange={(event) => patch({ timeoutMs: event.target.value })}
                      inputMode="numeric"
                      className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                    />
                  </label>}
                  {kind === "claude" && (
                    <label className="text-sm text-foreground">
                      认证方式
                      <select
                        value={connection.auth}
                        onChange={(event) => patch({ auth: event.target.value as ConnectionEntry["auth"] })}
                        className="mt-1 w-full rounded border border-input bg-card p-2 text-sm"
                      >
                        <option value="token">Authorization: Bearer</option>
                        <option value="api_key">x-api-key</option>
                      </select>
                    </label>
                  )}
                  {/* 官方已弃用 ANTHROPIC_SMALL_FAST_MODEL，后台任务改用 Haiku 别名的模型：
                      只在旧接入里已有取值时显示，便于清空或挪到 Haiku 别名 */}
                  {kind === "claude" && initialConnection?.smallFastModel && (
                    <label className="text-sm text-foreground">
                      小任务模型（官方已弃用）
                      <input
                        data-testid="new-connection-small-fast-model"
                        value={connection.smallFastModel}
                        onChange={(event) => patch({ smallFastModel: event.target.value })}
                        className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                      />
                      <span className="mt-1 block text-xs text-muted-foreground">
                        Claude Code 现在用「Haiku 别名」的模型做后台任务；建成独立环境时会按它写入。
                        {!connection.aliasHaiku.trim() && connection.smallFastModel.trim() && (
                          <button
                            type="button"
                            data-testid="move-small-fast-to-haiku"
                            onClick={() => patch({ aliasHaiku: connection.smallFastModel.trim(), smallFastModel: "" })}
                            className="ml-1 underline hover:text-foreground"
                          >
                            挪到 Haiku 别名
                          </button>
                        )}
                      </span>
                    </label>
                  )}
                  {kind === "claude" && (
                    <div className="sm:col-span-2 flex flex-wrap items-center justify-between gap-2 rounded border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
                      <span>第三方接口没配的别名会把 Claude 的模型名原样发过去，多半报模型不存在；只有一个模型时可全部指向默认模型。</span>
                      <button
                        type="button"
                        data-testid="connection-fill-aliases"
                        disabled={!connection.model.trim()}
                        onClick={() => {
                          const model = connection.model.trim();
                          patch({ aliasOpus: model, aliasSonnet: model, aliasHaiku: model, aliasFable: model });
                        }}
                        className="rounded border bg-card px-2.5 py-1 text-xs text-foreground hover:bg-accent disabled:opacity-50"
                      >
                        别名都用默认模型
                      </button>
                    </div>
                  )}
                  {kind === "claude" && (
                    <label className="text-sm text-foreground">
                      Opus 别名（可选）
                      <input
                        data-testid="new-connection-alias-opus"
                        value={connection.aliasOpus}
                        onChange={(event) => patch({ aliasOpus: event.target.value })}
                        list={fetchedModels ? "connection-model-options" : undefined}
                        className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                      />
                    </label>
                  )}
                  {kind === "claude" && (
                    <label className="text-sm text-foreground">
                      Sonnet 别名（可选）
                      <input
                        data-testid="new-connection-alias-sonnet"
                        value={connection.aliasSonnet}
                        onChange={(event) => patch({ aliasSonnet: event.target.value })}
                        list={fetchedModels ? "connection-model-options" : undefined}
                        className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                      />
                    </label>
                  )}
                  {kind === "claude" && (
                    <label className="text-sm text-foreground">
                      Haiku 别名（可选）
                      <input
                        data-testid="new-connection-alias-haiku"
                        value={connection.aliasHaiku}
                        onChange={(event) => patch({ aliasHaiku: event.target.value })}
                        list={fetchedModels ? "connection-model-options" : undefined}
                        className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                      />
                    </label>
                  )}
                  {kind === "claude" && (
                    <label className="text-sm text-foreground">
                      Fable 别名（可选）
                      <input
                        data-testid="new-connection-alias-fable"
                        value={connection.aliasFable ?? ""}
                        onChange={(event) => patch({ aliasFable: event.target.value })}
                        list={fetchedModels ? "connection-model-options" : undefined}
                        className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                      />
                    </label>
                  )}
                  {kind === "claude" && (
                    <label className="text-sm text-foreground">
                      子代理模型（可选）
                      <input
                        data-testid="new-connection-subagent-model"
                        value={connection.subagentModel ?? ""}
                        onChange={(event) => patch({ subagentModel: event.target.value })}
                        list={fetchedModels ? "connection-model-options" : undefined}
                        placeholder="别名如 haiku，或完整模型名"
                        className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                      />
                      <span className="mt-1 block text-xs font-normal text-muted-foreground">
                        官方 CLAUDE_CODE_SUBAGENT_MODEL：子代理、团队成员与工作流代理没指定模型时用它；建成独立环境时写进环境变量。
                      </span>
                    </label>
                  )}
                </div>
              </details>

              {intent === "environment" && !editing && !duplicating && (
                <section className="space-y-3 rounded-lg border p-3" data-testid="connection-environment-options">
                  <h3 className="font-medium">独立环境</h3>
                  <div className="grid gap-3 sm:grid-cols-2">
                    <label className="text-sm">环境名（也是启动命令名）
                      <input data-testid="connection-environment-id" value={environmentId} onChange={(event) => setEnvironmentId(event.target.value)} className="mt-1 w-full rounded border border-input p-2 font-mono text-sm" />
                    </label>
                    <label className="text-sm">独立配置目录
                      <input data-testid="connection-environment-dir" value={environmentDir} onChange={(event) => setEnvironmentDir(event.target.value)} placeholder={defaultConfigDir(kind, environmentId)} className="mt-1 w-full rounded border border-input p-2 font-mono text-sm" />
                    </label>
                  </div>
                  <p className="text-xs text-muted-foreground">启动命令使用此目录，不修改当前默认环境。密钥取值写入外部密钥文件。</p>
                  {newEnvironmentInput && (
                    <pre data-testid="connection-environment-launch-preview" className="overflow-x-auto whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono text-xs">
                      {`alias ${newEnvironmentInput.id}='${environmentLaunch}'`}
                    </pre>
                  )}
                  {newEnvironmentInput && <div className="space-y-2 text-xs" data-testid="connection-alias-file-preview">
                    <p className="font-medium">终端配置文件写入后</p>
                    {currentAliasPreview === null && <p className="text-muted-foreground">正在读取实际文件并计算…</p>}
                    {typeof currentAliasPreview === "string" && <p role="alert" className="text-destructive">{currentAliasPreview}</p>}
                    {currentAliasPreview && typeof currentAliasPreview !== "string" && <>
                      {currentAliasPreview.files.map((file) => <div key={file.target} className="space-y-1">
                        <p className="break-all font-mono">{file.path}{file.exists ? "（已有文件）" : "（将新建）"}</p>
                        <pre data-testid={`connection-alias-file-${file.target}`} className="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono">{file.content}</pre>
                      </div>)}
                      {currentAliasPreview.skipped.map((reason) => <p key={reason} className="text-muted-foreground">{reason}</p>)}
                    </>}
                    <p className="text-muted-foreground">只读预览；原文可能含敏感值，仅在本机查看。</p>
                  </div>}
                  {kind === "claude" && detectSystemPlatform() !== "linux" && (
                    <label className="flex items-center gap-2 text-sm">
                      <input type="checkbox" data-testid="connection-environment-scrub" checked={scrubSubprocess} onChange={(event) => setScrubSubprocess(event.target.checked)} />
                      子进程不读取此环境的凭据
                    </label>
                  )}
                  <details>
                    <summary className="cursor-pointer text-sm">从默认环境带上共用设置</summary>
                    <div className="mt-2"><ProfileSeedPicker kind={kind} selected={seedItems} onChange={setSeedItems} testIdPrefix="connection-seed" /></div>
                  </details>
                  <label className="flex items-center gap-2 text-sm">
                    <input type="checkbox" data-testid="connection-save-template" checked={saveTemplate} onChange={(event) => setSaveTemplate(event.target.checked)} />
                    同时另存为 Provider 模板
                  </label>
                </section>
              )}
              <p className="rounded-lg border bg-muted/40 px-3 py-2 text-xs leading-5 text-muted-foreground">
                {editing && selectedUpdates.length > 0
                  ? "保存 Provider 后，只更新下方勾选的独立环境；每个环境按当前实际文件和原定义的基准写入并回读。默认环境不会改。"
                  : intent === "environment"
                  ? "创建时写入独立环境及其启动命令；只有勾选时才另存 Provider 模板。默认环境和当前登录不会修改。"
                  : "保存只更新接入定义，不写助手配置。之后可在接入卡片上选「建成独立环境」。"}
              </p>
              {impacts.length > 0 && (
                <section data-testid="connection-affected-profiles" className="rounded-lg border px-3 py-3 text-xs">
                  <h3 className="font-medium text-foreground">已建独立环境 · 逐项选择更新</h3>
                  <p className="mt-1 text-muted-foreground">以下是环境定义相对本次 Provider 修改的差异。环境自己改过的字段会保留；实际配置已偏离定义的环境要先到卡片处理。</p>
                  <div className="mt-2 space-y-2">
                    {impacts.map((profile) => {
                      const changes = profile.update?.changes ?? [];
                      const overrides = profile.update?.overrides ?? [];
                      const automaticChanges = changes.filter((change) => !overrides.some((item) => item.field === change.field));
                      const writable = (profile.state === "managed-ok" || profile.state === "not-materialized")
                        && Boolean(profile.base && profile.definitionBase) && !profile.blockedReason;
                      const selectable = changes.length > 0 && writable;
                      return (
                        <div key={profile.id} data-testid={`connection-affected-${profile.id}`} className="rounded border p-2">
                          <label className="flex items-center gap-2 font-medium text-foreground">
                            <input
                              type="checkbox"
                              data-testid={`connection-update-${profile.id}`}
                              disabled={!selectable || busy}
                              checked={selectable && selectedProfiles.includes(profile.id)}
                              onChange={(event) => {
                                if (!event.target.checked) {
                                  setSelectedOverrides((current) => ({ ...current, [profile.id]: [] }));
                                }
                                setSelectedProfiles((current) => event.target.checked
                                  ? [...new Set([...current, profile.id])]
                                  : current.filter((id) => id !== profile.id));
                              }}
                            />
                            {profile.label} <code className="font-normal text-muted-foreground">{profile.id}</code>
                          </label>
                          {!writable && <p className="mt-1 text-warning">{profile.blockedReason ?? (!profile.update ? "Provider 缺少可写的地址或凭据变量，不能更新环境。" : profile.state === "drifted" ? "实际配置与定义不同，先在环境卡片处理差异。" : profile.state === "unsupported" || profile.state === "disabled" ? "环境当前不可写。" : "无法读取完整写入基准，请刷新后重试。")}</p>}
                          {writable && changes.length === 0 && overrides.length === 0 && <p className="mt-1 text-muted-foreground">这次没有可带入的字段变化。</p>}
                          {automaticChanges.length > 0 && (
                            <ul className="mt-1 space-y-0.5 pl-5 text-muted-foreground">
                              {automaticChanges.map((change) => <li key={change.field} className="list-disc break-all">{change.field}：{change.before} → {change.after}</li>)}
                            </ul>
                          )}
                          {overrides.length > 0 && (
                            <div className="mt-2 space-y-1 border-t pt-2 text-muted-foreground">
                              <p>以下字段与当前 Provider 不同；默认保留环境自己的值，勾选才覆盖：</p>
                              {overrides.map((change) => (
                                <label key={change.key} className="flex items-start gap-2 break-all">
                                  <input
                                    type="checkbox"
                                    data-testid={`connection-override-${profile.id}-${change.key}`}
                                    disabled={!writable || busy}
                                    checked={(selectedOverrides[profile.id] ?? []).includes(change.key)}
                                    onChange={(event) => {
                                      const next = event.target.checked
                                        ? [...new Set([...(selectedOverrides[profile.id] ?? []), change.key])]
                                        : (selectedOverrides[profile.id] ?? []).filter((key) => key !== change.key);
                                      setSelectedOverrides((current) => ({ ...current, [profile.id]: next }));
                                      setSelectedProfiles((current) => event.target.checked
                                        ? [...new Set([...current, profile.id])]
                                        : next.length === 0 && automaticChanges.length === 0
                                          ? current.filter((id) => id !== profile.id)
                                          : current);
                                    }}
                                  />
                                  <span>{change.field}：{change.before} → {change.after}</span>
                                </label>
                              ))}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </section>
              )}
              <section className="space-y-2 rounded-lg border px-3 py-3 text-xs" data-testid="connection-file-preview">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <h3 className="font-medium text-foreground">将改动的客户端文件</h3>
                  {previewRequests.length > 0 && (
                    <button type="button" data-testid="connection-preview-button"
                      disabled={busy || previewBusy}
                      onClick={() => void refreshPreview()}
                      className="rounded border px-2.5 py-1 disabled:opacity-50"
                    >{previewBusy ? "正在预览…" : "重新读取"}</button>
                  )}
                </div>
                {previewRequests.length === 0 && <p className="text-muted-foreground">{intent === "environment"
                  ? "选定接入并填写有效的环境名后，可预览客户端配置文件和启动命令。"
                  : "只保存 Provider 模板时，客户端配置文件和启动命令不变。"}</p>}
                {previewRequests.length > 0 && <p className="text-muted-foreground">{selectedUpdates.length > 0
                  ? "已选环境按真实密钥取值计算最终文件；新建环境草稿仍以变量引用展示。保存时会重新校验原文件和候选内容。预览原文可能含敏感值，仅在本机查看，请勿分享。"
                  : "按保存流程预览文件及勾选的共用设置。新建文件的凭据以变量引用展示；已有主文件不会自动覆盖，接入字段可在环境卡片逐项应用。预览可能含敏感值，请勿直接分享。"}</p>}
                {preview && preview.key !== previewKey && previewRequests.length > 0
                  && <p className="text-muted-foreground">表单或勾选目标已变化，正在更新预览。</p>}
                {preview?.key === previewKey && preview.results.map(({ id, result }) => (
                  <div key={id} className="space-y-1 border-t pt-2">
                    <p className="font-medium text-foreground">{id}</p>
                    {typeof result === "string"
                      ? <p role="alert" className="text-destructive">{result}</p>
                      : <>
                          <p className="break-all font-mono text-muted-foreground">{result.path}{result.will_write === false ? (result.exists ? " · 保持现有文件" : " · 不创建文件") : (result.existing_profile_skipped ? " · 仅补勾选的共用设置" : (result.exists ? " · 合并现有文件" : " · 新文件"))}</p>
                          <pre data-testid={`connection-preview-content-${id}`} className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono text-xs">{result.content}</pre>
                          {result.files?.map((file, index) => <div key={file.path} className="space-y-1">
                            <p className="break-all font-mono text-muted-foreground">{file.path}{file.exists ? " · 合并现有文件" : " · 新文件"}</p>
                            <pre data-testid={`connection-preview-file-${id}-${index}`} className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono text-xs">{file.content}</pre>
                          </div>)}
                          {result.seed_results?.map((item) => <p key={item.item} className="text-muted-foreground">{item.detail}</p>)}
                        </>}
                  </div>
                ))}
              </section>
                </>
              )}
            </div>
          )}

          {shownError && (
            <div role="alert" data-testid="connection-form-error" className="mt-4 flex flex-wrap items-center justify-between gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              <span>{shownError}</span>
              {onOpenSecrets && shownError.includes("凭据") && (
                <button type="button" onClick={onOpenSecrets} className="rounded border border-destructive/30 bg-card px-2.5 py-1 text-xs font-medium hover:bg-destructive/10">
                  打开密钥设置
                </button>
              )}
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2 border-t px-5 py-4">
          <button
            type="button"
            disabled={busy}
            onClick={close}
            className="rounded border px-4 py-2 text-sm hover:bg-accent disabled:opacity-50"
          >
            取消
          </button>
          <button
            type="button"
            data-testid="connection-form-submit"
            disabled={saveDisabled || waitingForPreview}
            onClick={() => submit()}
            className="inline-flex items-center gap-1.5 rounded bg-primary px-4 py-2 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
            {busy
              ? "正在保存…"
              : editing
                ? "保存修改"
                : duplicating
                  ? "创建副本"
                  : intent === "environment"
                    ? "创建独立环境"
                    : "保存接入"}
          </button>
        </div>
      </section>
    </div>
  );
}
