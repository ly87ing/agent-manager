import { useLayoutEffect, useRef, useState } from "react";
import { Loader2, X } from "lucide-react";
import {
  CONNECTION_NAME_RE,
  emptyConnection,
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
  activate: boolean;
}

const FOCUSABLE_SELECTOR =
  'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';
const ENV_NAME_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

function validBaseUrl(value: string): boolean {
  try {
    const url = new URL(value);
    if (url.protocol === "https:") return true;
    return url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  } catch {
    return false;
  }
}

export function ConnectionFormDialog({
  agentName,
  kind,
  existingNames,
  initialConnection,
  initialActivate,
  busy,
  error,
  onSubmit,
  onClose,
}: {
  agentName: string;
  kind: ConnectionKind;
  existingNames: string[];
  /** 有值时进入编辑模式；唯一名称保持不变，避免一次编辑暗中变成复制。 */
  initialConnection?: ConnectionEntry;
  /** 编辑当前项时为 true，确保保存后的参数立即写回助手配置。 */
  initialActivate?: boolean;
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
  const editing = initialConnection !== undefined;
  const [choice, setChoice] = useState<string | null>(editing ? "edit" : null);
  const [connection, setConnection] = useState<ConnectionEntry | null>(
    initialConnection ? { ...initialConnection } : null,
  );
  const [secretValue, setSecretValue] = useState("");
  const [activate, setActivate] = useState(initialActivate ?? !editing);
  const [localError, setLocalError] = useState<string | null>(null);
  const dialogTitle = editing
    ? `编辑 ${initialConnection.label || initialConnection.name}`
    : `为 ${agentName} 添加接入`;

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
    if (existingNames.includes(name) && name !== initialConnection?.name) {
      setLocalError(`已经有名为 ${name} 的接入，请换一个名称`);
      return;
    }
    if (!validBaseUrl(connection.baseUrl.trim())) {
      setLocalError("接口地址必须是 https://，或本机 localhost / 127.0.0.1 地址");
      return;
    }
    if (!ENV_NAME_RE.test(connection.tokenVar.trim())) {
      setLocalError("凭据变量名格式不正确");
      return;
    }
    if (!connection.model.trim()) {
      setLocalError("要填写模型名，测试连接时才能确认这套接入可用");
      return;
    }
    onSubmit({
      connection: {
        ...connection,
        name,
        label: connection.label.trim() || name,
        baseUrl: connection.baseUrl.trim(),
        tokenVar: connection.tokenVar.trim(),
        model: connection.model.trim(),
      },
      secretValue,
      activate,
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
        className="flex max-h-[90vh] w-full max-w-3xl flex-col overflow-hidden rounded-xl border bg-card shadow-xl animate-zoom-in-95"
      >
        <div className="flex items-start justify-between gap-3 border-b px-5 py-4">
          <div>
            <h2 className="text-lg font-semibold text-foreground">{dialogTitle}</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              {editing
                ? `修改 ${agentName} 的这套接入；留空凭据取值不会覆盖已经保存的值。`
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
          {!editing && <fieldset>
            <legend className="text-sm font-medium text-foreground">1. 选择接入方式</legend>
            <div className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
              {presets.map((preset, index) => (
                <button
                  key={preset.id}
                  type="button"
                  autoFocus={index === 0}
                  data-testid={`connection-preset-${preset.id}`}
                  aria-pressed={choice === preset.id}
                  onClick={() => choosePreset(preset.id)}
                  className={cn(
                    "rounded-lg border p-3 text-left",
                    choice === preset.id ? "border-primary bg-primary/10" : "hover:bg-accent",
                  )}
                >
                  <span className="block text-sm font-medium text-foreground">{preset.label}</span>
                  <span className="mt-1 block text-xs text-muted-foreground">{preset.source}</span>
                </button>
              ))}
              <button
                type="button"
                autoFocus={presets.length === 0}
                data-testid="connection-preset-custom"
                aria-pressed={choice === "custom"}
                onClick={chooseCustom}
                className={cn(
                  "rounded-lg border p-3 text-left",
                  choice === "custom" ? "border-primary bg-primary/10" : "hover:bg-accent",
                )}
              >
                <span className="block text-sm font-medium text-foreground">自定义接入</span>
                <span className="mt-1 block text-xs text-muted-foreground">
                  用于其他官方端点或你自己维护的网关
                </span>
              </button>
            </div>
          </fieldset>}

          {connection && (
            <div className="mt-5 space-y-4 border-t pt-4">
              <h3 className="text-sm font-medium text-foreground">{editing ? "接入信息" : "2. 确认必要信息"}</h3>
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <label className="text-sm text-foreground">
                  显示名称
                  <input
                    data-testid="new-connection-label"
                    autoFocus={editing}
                    value={connection.label}
                    onChange={(event) => patch({ label: event.target.value })}
                    className="mt-1 w-full rounded border border-input p-2 text-sm"
                  />
                </label>
                <label className="text-sm text-foreground">
                  模型
                  <input
                    data-testid="new-connection-model"
                    value={connection.model}
                    onChange={(event) => patch({ model: event.target.value })}
                    className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                    spellCheck={false}
                  />
                </label>
              </div>
              <label className="block text-sm text-foreground">
                接口地址
                <input
                  data-testid="new-connection-base-url"
                  value={connection.baseUrl}
                  onChange={(event) => patch({ baseUrl: event.target.value })}
                  className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                  spellCheck={false}
                />
              </label>
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <label className="text-sm text-foreground">
                  凭据变量名
                  <input
                    data-testid="new-connection-token-var"
                    value={connection.tokenVar}
                    onChange={(event) => patch({ tokenVar: event.target.value })}
                    className="mt-1 w-full rounded border border-input p-2 font-mono text-sm"
                    spellCheck={false}
                  />
                </label>
                {kind === "claude" ? (
                  <label className="text-sm text-foreground">
                    凭据取值（可选）
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
                ) : (
                  <p className="rounded border bg-muted px-3 py-2 text-xs text-muted-foreground">
                    Codex 启动时自己读取这个环境变量；本应用不会把明文凭据写进 Codex 配置。
                  </p>
                )}
              </div>

              <details className="rounded-lg border px-3 py-2">
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

              <label className="flex items-start gap-2 rounded-lg border bg-muted/40 px-3 py-2 text-sm text-foreground">
                <input
                  type="checkbox"
                  data-testid="new-connection-activate"
                  checked={activate}
                  onChange={(event) => setActivate(event.target.checked)}
                  className="mt-0.5"
                />
                <span>
                  {editing ? "保存后更新这套接入" : "保存后立即启用"}
                  <span className="mt-0.5 block text-xs text-muted-foreground">
                    会写入助手配置文件；若目标被外部改过，会先再次征求确认。
                  </span>
                </span>
              </label>
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
            disabled={busy || !connection}
            onClick={submit}
            className="inline-flex items-center gap-1.5 rounded bg-primary px-4 py-2 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
            {busy
              ? "正在保存…"
              : editing
                ? activate
                  ? "保存并更新"
                  : "保存修改"
                : activate
                  ? "保存并启用"
                  : "保存接入"}
          </button>
        </div>
      </section>
    </div>
  );
}
