import { useLayoutEffect, useRef, useState } from "react";
import { Braces, FormInput, Loader2, X } from "lucide-react";
import {
  normalizeAgentRuntimeSettings,
  type AgentRuntimeSettings,
} from "../lib/agentDefinitionForm";
import type { AgentCurrentConfigStatus, AgentRuntimeField, AgentRuntimeTarget } from "../lib/observed";
import { cn } from "../lib/utils";

const FOCUSABLE_SELECTOR =
  'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

function testIdOf(field: AgentRuntimeField): string {
  return `agent-runtime-${field.key.replaceAll("_", "-")}`;
}

function targetValue(target: AgentRuntimeTarget, value: unknown): string {
  if (value === undefined || value === null || value === "" || value === false) {
    return "<删除该键，使用客户端默认>";
  }
  if (target.enabled_value) return target.enabled_value;
  return typeof value === "string" ? JSON.stringify(value) : String(value);
}

function FieldHints({ field, value }: { field: AgentRuntimeField; value: unknown }) {
  return (
    <span className="mt-1 block space-y-0.5 break-all font-mono text-[11px] text-muted-foreground">
      {field.targets.map((target) => (
        <span key={target.key} className="block">
          {target.key} = {targetValue(target, value)}
        </span>
      ))}
    </span>
  );
}

function RuntimeField({
  field,
  value,
  disabled,
  onChange,
}: {
  field: AgentRuntimeField;
  value: unknown;
  disabled: boolean;
  onChange: (value: unknown) => void;
}) {
  if (field.type === "boolean") {
    const checked = value === true;
    return (
      <label
        className={cn(
          "flex min-h-28 items-start gap-3 rounded-lg border bg-background p-3",
          field.danger && "border-destructive/25 bg-destructive/5",
        )}
      >
        <input
          type="checkbox"
          data-testid={testIdOf(field)}
          checked={checked}
          disabled={disabled}
          onChange={(event) => onChange(event.target.checked)}
          className="mt-0.5"
        />
        <span className="min-w-0">
          <span className="block font-medium text-foreground">{field.label}</span>
          <span className="mt-1 block text-xs leading-5 text-muted-foreground">{field.description}</span>
          <FieldHints field={field} value={checked} />
        </span>
      </label>
    );
  }

  if (field.type === "enum") {
    const selected = typeof value === "string" ? value : "";
    const options = field.options ?? [];
    const hasDefault = options.some((option) => option.value === "");
    return (
      <label className="rounded-lg border bg-background p-3 text-foreground">
        <span className="font-medium">{field.label}</span>
        <span className="mt-1 block text-xs leading-5 text-muted-foreground">{field.description}</span>
        <select
          data-testid={testIdOf(field)}
          value={selected}
          disabled={disabled}
          onChange={(event) => onChange(event.target.value)}
          className="mt-2 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
        >
          {!hasDefault && <option value="">使用客户端默认值</option>}
          {options.map((option) => (
            <option key={option.value || "__default"} value={option.value}>{option.label}</option>
          ))}
        </select>
        <FieldHints field={field} value={selected} />
      </label>
    );
  }

  if (field.type === "positive_integer") {
    const text = value === undefined || value === null ? "" : String(value);
    return (
      <label className="rounded-lg border bg-background p-3 text-foreground">
        <span className="font-medium">{field.label}</span>
        <span className="mt-1 block text-xs leading-5 text-muted-foreground">{field.description}</span>
        <input
          data-testid={testIdOf(field)}
          inputMode="numeric"
          value={text}
          disabled={disabled}
          placeholder={field.placeholder}
          onChange={(event) => onChange(event.target.value)}
          className="mt-2 w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm"
        />
        <FieldHints field={field} value={text} />
      </label>
    );
  }

  return (
    <div className="rounded-lg border border-warning/30 bg-warning/10 p-3 text-sm text-warning">
      当前版本不认识字段类型 <code>{field.type}</code>（<code>{field.key}</code>），已停止编辑，避免误写。
    </div>
  );
}

/** 后端 schema 驱动的运行设置编辑器；JSON 模式完整保留尚未认识的设置键。 */
export function AgentRuntimeDialog({
  agentName,
  fields,
  initial,
  currentConfig,
  busy,
  error,
  onSubmit,
  onClose,
}: {
  agentName: string;
  fields: AgentRuntimeField[];
  initial: AgentRuntimeSettings;
  currentConfig?: AgentCurrentConfigStatus | null;
  busy: boolean;
  error?: string | null;
  onSubmit: (settings: AgentRuntimeSettings) => void;
  onClose: () => void;
}) {
  const rootRef = useRef<HTMLElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(
    typeof document === "undefined" ? null : (document.activeElement as HTMLElement | null),
  );
  const [settings, setSettings] = useState<AgentRuntimeSettings>(initial);
  const [mode, setMode] = useState<"form" | "json" | "observed">("form");
  const [rawText, setRawText] = useState(() => JSON.stringify(initial, null, 2));
  const [localError, setLocalError] = useState<string | null>(null);

  useLayoutEffect(() => {
    return () => restoreFocusRef.current?.focus();
  }, []);

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

  const readRaw = (): AgentRuntimeSettings | null => {
    try {
      const parsed = JSON.parse(rawText) as unknown;
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("运行设置 JSON 必须是对象");
      }
      return parsed as AgentRuntimeSettings;
    } catch (err) {
      setLocalError(err instanceof Error ? err.message : String(err));
      return null;
    }
  };

  const switchMode = (next: "form" | "json" | "observed") => {
    if (next === mode) return;
    if (next === "observed" || mode === "observed") {
      setLocalError(null);
      setMode(next);
      return;
    }
    if (next === "json") {
      setRawText(JSON.stringify(settings, null, 2));
      setLocalError(null);
      setMode(next);
      return;
    }
    const parsed = readRaw();
    if (!parsed) return;
    setSettings(parsed);
    setLocalError(null);
    setMode(next);
  };

  const submit = () => {
    const candidate = mode === "json" ? readRaw() : settings;
    if (!candidate) return;
    try {
      const normalized = normalizeAgentRuntimeSettings(candidate, fields);
      setLocalError(null);
      onSubmit(normalized);
    } catch (err) {
      setLocalError(err instanceof Error ? err.message : String(err));
    }
  };

  const sections = Array.from(new Set(fields.map((field) => field.section)));

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-3 sm:p-6 animate-fade-in"
      onClick={(event) => event.target === event.currentTarget && close()}
    >
      <section
        ref={rootRef}
        role="dialog"
        aria-modal="true"
        aria-label={`${agentName} 运行设置`}
        onKeyDown={onKeyDown}
        className="flex max-h-[92vh] w-full max-w-6xl flex-col overflow-hidden rounded-xl border bg-card shadow-xl animate-zoom-in-95"
      >
        <div className="flex items-start justify-between gap-3 border-b px-5 py-4 sm:px-6">
          <div>
            <h3 className="text-lg font-semibold text-foreground">{agentName} 运行设置</h3>
            <p className="mt-1 text-sm text-muted-foreground">
              字段由客户端适配器提供；每项都显示最终写入的真实配置键和值。账号、接口地址、模型与凭据变量在接入卡片中维护。
            </p>
          </div>
          <button type="button" aria-label="关闭" disabled={busy} onClick={close} className="rounded p-1 text-muted-foreground hover:bg-accent disabled:opacity-50">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <div className="flex items-center gap-2 border-b px-5 py-3 sm:px-6" role="group" aria-label="编辑方式">
          <button
            type="button"
            aria-pressed={mode === "form"}
            data-testid="agent-runtime-form-mode"
            onClick={() => switchMode("form")}
            className={cn("inline-flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm", mode === "form" ? "border-primary bg-primary/10 text-primary" : "text-muted-foreground hover:bg-accent")}
          >
            <FormInput className="h-4 w-4" aria-hidden /> 按项填写
          </button>
          <button
            type="button"
            aria-pressed={mode === "json"}
            data-testid="agent-runtime-json-mode"
            onClick={() => switchMode("json")}
            className={cn("inline-flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm", mode === "json" ? "border-primary bg-primary/10 text-primary" : "text-muted-foreground hover:bg-accent")}
          >
            <Braces className="h-4 w-4" aria-hidden /> 已纳管 JSON
          </button>
          <button
            type="button"
            aria-pressed={mode === "observed"}
            data-testid="agent-runtime-observed-mode"
            onClick={() => switchMode("observed")}
            className={cn("inline-flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm", mode === "observed" ? "border-primary bg-primary/10 text-primary" : "text-muted-foreground hover:bg-accent")}
          >
            <Braces className="h-4 w-4" aria-hidden /> 本机实际值
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5 sm:px-6">
          {mode === "form" ? (
            <div className="space-y-5 text-sm">
              {sections.map((section) => {
                const sectionFields = fields.filter((field) => field.section === section);
                return (
                  <section key={section}>
                    <h4 className="font-semibold text-foreground">{section}</h4>
                    {sectionFields[0]?.section_description && (
                      <p className="mt-1 text-xs text-muted-foreground">{sectionFields[0].section_description}</p>
                    )}
                    <div className="mt-3 grid grid-cols-1 gap-3 lg:grid-cols-2">
                      {sectionFields.map((field) => (
                        <RuntimeField
                          key={field.key}
                          field={field}
                          value={settings[field.key]}
                          disabled={busy}
                          onChange={(value) => setSettings((current) => {
                            const next = { ...current };
                            if (value === false || value === "") delete next[field.key];
                            else next[field.key] = value;
                            return next;
                          })}
                        />
                      ))}
                    </div>
                  </section>
                );
              })}
              <p className="rounded-lg bg-muted px-3 py-2 text-xs text-muted-foreground">
                官方账号仍在助手自身完成登录；本工具不读取账号密码或令牌明文。
              </p>
            </div>
          ) : mode === "json" ? (
            <div className="space-y-3">
              <div className="rounded-lg border border-primary/20 bg-primary/5 px-3 py-2 text-xs leading-5 text-muted-foreground">
                这里是数据库中的完整运行设置，不含由接入工作台管理的 <code>profiles</code> 和 <code>active</code>。
                未知键会原样保留，待新版适配器支持后即可生效；当前适配器不会把未知键写入客户端。不要在这里填写凭据明文。
              </div>
              <textarea
                aria-label="运行设置 JSON 原文"
                data-testid="agent-runtime-json"
                value={rawText}
                disabled={busy}
                onChange={(event) => setRawText(event.target.value)}
                spellCheck={false}
                className="min-h-[24rem] w-full resize-y rounded-lg border border-input bg-slate-950 p-4 font-mono text-xs leading-5 text-slate-100"
              />
            </div>
          ) : (
            <div className="space-y-3">
              <div className="rounded-lg border border-primary/20 bg-primary/5 px-3 py-2 text-xs leading-5 text-muted-foreground">
                这里来自客户端配置文件的只读脱敏观察值，不会显示 token/API key，也不会自动写入数据库或接管配置。
              </div>
              <pre
                data-testid="agent-runtime-observed-json"
                className="min-h-[24rem] overflow-auto rounded-lg border border-input bg-slate-950 p-4 font-mono text-xs leading-5 text-slate-100"
              >
                {JSON.stringify(currentConfig ?? { state: "unavailable", config: null }, null, 2)}
              </pre>
            </div>
          )}
          {(error || localError) && <p role="alert" className="mt-3 text-sm text-destructive">{error ?? localError}</p>}
        </div>

        <div className="flex justify-end gap-2 border-t px-5 py-4 sm:px-6">
          <button type="button" disabled={busy} onClick={close} className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50">
            取消
          </button>
          {mode !== "observed" && (
            <button
              type="button"
              data-testid="agent-runtime-submit"
              disabled={busy}
              onClick={submit}
              className="inline-flex items-center gap-1.5 rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
            >
              {busy && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
              {busy ? "正在写入…" : "保存并写入助手"}
            </button>
          )}
        </div>
      </section>
    </div>
  );
}
