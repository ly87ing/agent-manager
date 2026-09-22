import { useLayoutEffect, useRef, useState } from "react";
import { Loader2, X } from "lucide-react";
import {
  CONNECTION_NAME_RE,
  emptyConnection,
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
  onClose,
}: {
  agentName: string;
  kind: ConnectionKind;
  existingNames: string[];
  /** 编辑、复制或纳管时的预填值；具体语义由 variant 决定。 */
  initialConnection?: ConnectionEntry;
  /** 复制和纳管模式允许创建新名称；凭据取值始终为空。 */
  variant?: "edit" | "duplicate" | "adopt";
  busy: boolean;
  error?: string | null;
  onSubmit: (submission: ConnectionSubmission) => void;
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
  const adopting = prefilled && variant === "adopt";
  const [choice, setChoice] = useState<string | null>(prefilled ? variant ?? "edit" : null);
  const [connection, setConnection] = useState<ConnectionEntry | null>(
    initialConnection ? { ...initialConnection } : null,
  );
  const [secretValue, setSecretValue] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(!adopting && !duplicating && Boolean(
    initialConnection?.timeoutMs || initialConnection?.smallFastModel || initialConnection?.aliasOpus
    || initialConnection?.aliasSonnet || initialConnection?.aliasHaiku || initialConnection?.auth === "api_key",
  ));
  const dialogTitle = editing
    ? `编辑 ${initialConnection.label || initialConnection.name}`
    : duplicating
      ? "复制 Provider"
      : adopting
        ? `纳管 ${agentName} 的客户端当前配置`
        : `为 ${agentName} 添加接入`;

  const adoptionMissing = adopting && connection
    ? [
        !connection.label.trim() ? "显示名称" : null,
        !connection.baseUrl.trim() ? "API 地址" : null,
        !connection.model.trim() ? "默认模型" : null,
        !connection.tokenVar.trim() ? "凭据变量" : null,
      ].filter((item): item is string => Boolean(item))
    : [];
  const validationError = connection
    ? (!connection.label.trim() && (adopting || duplicating) ? "请填写显示名称" : validateConnectionEntry(connection))
    : null;
  const duplicateName = Boolean(
    connection && existingNames.includes(connection.name.trim()) && !(editing && connection.name.trim() === initialConnection?.name),
  );
  const adoptionIssue = adoptionMissing.length > 0
    ? `客户端没有提供：${adoptionMissing.join("、")}。补全后才能保存。`
    : validationError
      ? `还不能保存：${validationError}`
      : duplicateName
        ? "内部名称已存在，请关闭后重试。"
        : null;
  const saveDisabled = busy || !connection || ((adopting || duplicating) && (Boolean(validationError) || duplicateName));

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
    if (existingNames.includes(name) && !(editing && name === initialConnection?.name)) {
      setLocalError(`已经有名为 ${name} 的接入，请换一个名称`);
      return;
    }
    const validationError = validateConnectionEntry(connection);
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
                ? `修改 ${agentName} 的这套接入；留空凭据取值不会覆盖已经保存的值。`
                : duplicating
                  ? "确认新名称即可创建；原 Provider、当前接入和客户端配置都不会改变。"
                  : adopting
                    ? "这里只把客户端当前值保存成一个 Provider，不会修改、切换或覆盖客户端配置。"
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
                    {(connection.timeoutMs || connection.smallFastModel || connection.aliasOpus || connection.aliasSonnet || connection.aliasHaiku || connection.auth === "api_key") && (
                      <details className="border-t px-3 py-2 text-xs">
                        <summary className="cursor-pointer text-muted-foreground">查看一并复制的高级配置</summary>
                        <dl className="mt-2 grid grid-cols-[7rem_minmax(0,1fr)] gap-x-3 gap-y-1 text-foreground">
                          {connection.auth === "api_key" && <><dt>认证方式</dt><dd className="font-mono">x-api-key</dd></>}
                          {connection.timeoutMs && <><dt>请求超时</dt><dd className="font-mono">{connection.timeoutMs} ms</dd></>}
                          {connection.smallFastModel && <><dt>小任务模型</dt><dd className="break-all font-mono">{connection.smallFastModel}</dd></>}
                          {connection.aliasOpus && <><dt>Opus 映射</dt><dd className="break-all font-mono">{connection.aliasOpus}</dd></>}
                          {connection.aliasSonnet && <><dt>Sonnet 映射</dt><dd className="break-all font-mono">{connection.aliasSonnet}</dd></>}
                          {connection.aliasHaiku && <><dt>Haiku 映射</dt><dd className="break-all font-mono">{connection.aliasHaiku}</dd></>}
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
              {adopting && (
                <div
                  data-testid="adoption-readiness"
                  className={cn(
                    "rounded-lg border px-3 py-2 text-sm",
                    adoptionIssue
                      ? "border-amber-300 bg-amber-50 text-amber-950 dark:border-amber-800 dark:bg-amber-950/30 dark:text-amber-100"
                      : "border-emerald-300 bg-emerald-50 text-emerald-950 dark:border-emerald-800 dark:bg-emerald-950/30 dark:text-emerald-100",
                  )}
                >
                  {adoptionIssue ?? "客户端当前配置已完整，可以保存为 Provider。"}
                </div>
              )}
              <div className="flex items-center justify-between gap-3">
                <h3 className="text-sm font-medium text-foreground">
                  {adopting ? "请核对这 4 项" : "Provider 配置"}
                </h3>
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
                    className={cn("mt-1 w-full rounded border p-2 text-sm", adopting && !connection.label.trim() ? "border-amber-500" : "border-input")}
                  />
                </label>
                <label className="text-sm text-foreground">
                  默认模型
                  <input
                    data-testid="new-connection-model"
                    value={connection.model}
                    onChange={(event) => patch({ model: event.target.value })}
                    placeholder={kind === "claude" ? "例如：claude-sonnet-4-5" : "例如：gpt-5"}
                    className={cn("mt-1 w-full rounded border p-2 font-mono text-sm", adopting && !connection.model.trim() ? "border-amber-500" : "border-input")}
                    spellCheck={false}
                  />
                </label>
              </div>
              <label className="block text-sm text-foreground">
                API 地址
                <input
                  data-testid="new-connection-base-url"
                  value={connection.baseUrl}
                  onChange={(event) => patch({ baseUrl: event.target.value })}
                  placeholder="https://api.example.com"
                  className={cn("mt-1 w-full rounded border p-2 font-mono text-sm", adopting && !connection.baseUrl.trim() ? "border-amber-500" : "border-input")}
                  spellCheck={false}
                />
              </label>
              {!adopting && <label className="block text-sm text-foreground">
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
              </label>}
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <label className="text-sm text-foreground">
                  API Key 环境变量
                  <input
                    data-testid="new-connection-token-var"
                    value={connection.tokenVar}
                    onChange={(event) => patch({ tokenVar: event.target.value })}
                    placeholder={kind === "claude" ? "ANTHROPIC_AUTH_TOKEN" : "OPENAI_API_KEY"}
                    className={cn("mt-1 w-full rounded border p-2 font-mono text-sm", adopting && !connection.tokenVar.trim() ? "border-amber-500" : "border-input")}
                    spellCheck={false}
                  />
                </label>
                {kind === "claude" && !adopting ? (
                  <label className="text-sm text-foreground">
                    API Key（可选）
                    <input
                      type="password"
                      autoComplete="new-password"
                      data-testid="new-connection-secret"
                      value={secretValue}
                      onChange={(event) => setSecretValue(event.target.value)}
                      placeholder="留空则使用已保存值或进程环境"
                      className="mt-1 w-full rounded border border-input p-2 text-sm"
                    />
                  </label>
                ) : kind === "codex" ? (
                  <p className="rounded border bg-muted px-3 py-2 text-xs text-muted-foreground">
                    Codex 启动时自己读取这个环境变量；本应用不会把明文凭据写进 Codex 配置。
                  </p>
                ) : null}
              </div>

              <details
                className="rounded-lg border px-3 py-2"
                open={advancedOpen}
                onToggle={(event) => setAdvancedOpen(event.currentTarget.open)}
              >
                <summary data-testid="new-connection-advanced" className="cursor-pointer text-sm font-medium text-foreground">
                  高级字段
                </summary>
                <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
                  {!adopting && <label className="text-sm text-foreground">
                    唯一名称
                    <input
                      data-testid="new-connection-name"
                      value={connection.name}
                      disabled={editing}
                      onChange={(event) => patch({ name: event.target.value })}
                      title={editing ? "唯一名称用于配置引用，编辑时保持不变" : undefined}
                      className="mt-1 w-full rounded border border-input p-2 font-mono text-sm disabled:bg-muted disabled:text-muted-foreground"
                    />
                  </label>}
                  <label className="text-sm text-foreground">
                    请求超时（毫秒，可选）
                    <input
                      value={connection.timeoutMs}
                      onChange={(event) => patch({ timeoutMs: event.target.value })}
                      inputMode="numeric"
                      className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                    />
                  </label>
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
                  {kind === "claude" && (
                    <label className="text-sm text-foreground">
                      小任务模型（可选）
                      <input
                        value={connection.smallFastModel}
                        onChange={(event) => patch({ smallFastModel: event.target.value })}
                        className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                      />
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
                </div>
              </details>

              <p className="rounded-lg border bg-muted/40 px-3 py-2 text-xs leading-5 text-muted-foreground">
                {adopting
                  ? "点击保存只会创建一个新的 Provider，不会应用到客户端。保存后可在卡片上单独选择是否应用。"
                  : "保存只更新 Provider 定义，不写助手配置。需要生效时，请回到卡片点击“应用”。"}
              </p>
                </>
              )}
            </div>
          )}

          {shownError && (
            <p role="alert" data-testid="connection-form-error" className="mt-4 text-sm text-destructive">
              {shownError}
            </p>
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
            disabled={saveDisabled}
            onClick={submit}
            className="inline-flex items-center gap-1.5 rounded bg-primary px-4 py-2 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
            {busy
              ? "正在保存…"
              : editing
                ? "保存修改"
                : duplicating
                  ? "创建副本"
                  : adopting
                    ? "保存为 Provider"
                    : "保存接入"}
          </button>
        </div>
      </section>
    </div>
  );
}
