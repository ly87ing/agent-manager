import { useLayoutEffect, useRef, useState } from "react";
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

export type ConnectionKind = "claude" | "codex";

export interface ConnectionSubmission {
  connection: ConnectionEntry;
  secretValue: string;
  /** 保存后接着打开「建成独立环境」表单：第三方接入只在独立环境里用，一步到位。 */
  thenMakeProfile?: boolean;
}

const FOCUSABLE_SELECTOR =
  'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

export function ConnectionFormDialog({
  agentName,
  kind,
  existingNames,
  initialConnection,
  variant,
  busy,
  error,
  onSubmit,
  onOpenSecrets,
  onClose,
}: {
  agentName: string;
  kind: ConnectionKind;
  existingNames: string[];
  /** 编辑或复制时的预填值；具体语义由 variant 决定。 */
  initialConnection?: ConnectionEntry;
  /** 复制模式允许创建新名称；凭据取值始终为空。 */
  variant?: "edit" | "duplicate";
  busy: boolean;
  error?: string | null;
  onSubmit: (submission: ConnectionSubmission) => void;
  onOpenSecrets?: () => void;
  onClose: () => void;
}) {
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
  const [localError, setLocalError] = useState<string | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(!duplicating && Boolean(
    initialConnection?.timeoutMs || initialConnection?.smallFastModel || initialConnection?.aliasOpus
    || initialConnection?.aliasSonnet || initialConnection?.aliasHaiku || initialConnection?.aliasFable || initialConnection?.auth === "api_key",
  ));
  const dialogTitle = editing
    ? `编辑 ${initialConnection.label || initialConnection.name}`
    : duplicating
      ? "复制 Provider"
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

  useLayoutEffect(() => {
    return () => restoreFocusRef.current?.focus();
  }, []);

  const choosePreset = (presetId: string) => {
    const preset = presets.find((item) => item.id === presetId);
    if (!preset) return;
    setChoice(preset.id);
    setConnection(connectionFromPreset(preset, presetConnectionName(preset, existingNames)));
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
    setConnection(emptyConnection(name));
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

  const submit = (thenMakeProfile = false) => {
    if (!connection) {
      setLocalError("先选择一个厂商预设，或选择自定义接入");
      return;
    }
    const name = connection.name.trim();
    if (!CONNECTION_NAME_RE.test(name)) {
      setLocalError("唯一名称只能用字母、数字、连字符与下划线，长度不超过 64");
      return;
    }
    if (existingNames.includes(name) && !(editing && name === initialConnection?.name)) {
      setLocalError(`已经有名为 ${name} 的接入，请换一个名称`);
      return;
    }
    const validationError = validateConnectionEntry(connection, strict);
    if (validationError) {
      setLocalError(validationError);
      return;
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
      thenMakeProfile,
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
                ? `修改 ${agentName} 的这套接入；只保存定义，不写助手配置。留空凭据取值不会覆盖外部密钥文件中的值。`
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
                    {(connection.timeoutMs || connection.smallFastModel || connection.aliasOpus || connection.aliasSonnet || connection.aliasHaiku || connection.aliasFable || connection.auth === "api_key") && (
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
                  默认模型
                  <input
                    data-testid="new-connection-model"
                    value={connection.model}
                    onChange={(event) => patch({ model: event.target.value })}
                    placeholder={kind === "claude" ? "例如：claude-sonnet-4-5" : "例如：gpt-5"}
                    className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                    spellCheck={false}
                  />
                </label>
              </div>
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
              {!referenced && <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <label className="text-sm text-foreground">
                  API Key 环境变量
                  <input
                    data-testid="new-connection-token-var"
                    value={connection.tokenVar}
                    onChange={(event) => patch({ tokenVar: event.target.value })}
                    placeholder={kind === "claude" ? "ANTHROPIC_AUTH_TOKEN" : "OPENAI_API_KEY"}
                    className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                    spellCheck={false}
                  />
                </label>
                {kind === "claude" || kind === "codex" ? (
                  <label className="text-sm text-foreground">
                    API Key（可选）
                    <input
                      type="password"
                      autoComplete="new-password"
                      data-testid="new-connection-secret"
                      value={secretValue}
                      onChange={(event) => setSecretValue(event.target.value)}
                      placeholder="留空则保持外部文件或进程环境中的值"
                      className="mt-1 w-full rounded border border-input p-2 text-sm"
                    />
                    <span className="mt-1 block text-xs text-muted-foreground">
                      填写后只写入“设置 → 密钥”选择的外部文件，不进入本机数据库。
                    </span>
                  </label>
                ) : null}
              </div>}

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
                    <label className="text-sm text-foreground">
                      Opus 别名（可选）
                      <input
                        data-testid="new-connection-alias-opus"
                        value={connection.aliasOpus}
                        onChange={(event) => patch({ aliasOpus: event.target.value })}
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
                        className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                      />
                    </label>
                  )}
                </div>
              </details>

              <p className="rounded-lg border bg-muted/40 px-3 py-2 text-xs leading-5 text-muted-foreground">
                保存只更新接入定义，不写助手配置。主配置只放官方账号；要使用这套接入，在卡片菜单里选「建成独立环境」。
              </p>
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
          {!editing && !duplicating && !connection?.providerRef && (
            <button
              type="button"
              data-testid="connection-form-submit-and-make-profile"
              disabled={saveDisabled}
              onClick={() => submit(true)}
              title="主配置只放官方账号，第三方接入在独立环境里用：保存后直接打开建成独立环境的表单"
              className="rounded border border-primary/40 px-4 py-2 text-sm text-primary hover:bg-primary/10 disabled:opacity-50"
            >
              保存并建成独立环境
            </button>
          )}
          <button
            type="button"
            data-testid="connection-form-submit"
            disabled={saveDisabled}
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
                  : "保存接入"}
          </button>
        </div>
      </section>
    </div>
  );
}
