import { useState } from "react";
import { Braces, FormInput, Loader2, X } from "lucide-react";
import {
  normalizeAgentRuntimeSettings,
  type AgentRuntimeSettings,
} from "../lib/agentDefinitionForm";
import type { AgentCurrentConfigStatus, AgentRuntimeField, AgentRuntimeTarget } from "../lib/observed";
import { cn } from "../lib/utils";
import { useDialogFocus } from "../lib/dialogFocus";

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

function targetsFor(field: AgentRuntimeField, value: unknown): AgentRuntimeTarget[] {
  const option = field.options?.find((candidate) => candidate.value === value);
  return option?.targets ?? field.targets;
}

function FieldHints({ field, value }: { field: AgentRuntimeField; value: unknown }) {
  return (
    <span className="mt-1 block space-y-0.5 break-all font-mono text-[11px] text-muted-foreground">
      {targetsFor(field, value).map((target) => (
        <span key={target.key} className="block">
          {target.key} = {targetValue(target, value)}
        </span>
      ))}
    </span>
  );
}

function previewValue(target: AgentRuntimeTarget, value: unknown): unknown {
  if (!target.enabled_value) return value;
  try {
    return JSON.parse(target.enabled_value);
  } catch {
    return target.enabled_value;
  }
}

function runtimePreview(adapter: string, fields: AgentRuntimeField[], settings: AgentRuntimeSettings): string {
  const entries = fields.flatMap((field) => {
    const value = settings[field.key];
    if (value === undefined || value === null || value === "" || value === false) return [];
    return targetsFor(field, value).map((target) => [target.key, previewValue(target, value)] as const);
  });
  if (adapter === "codex") {
    return entries.length > 0
      ? `${entries.map(([key, value]) => `${key} = ${JSON.stringify(value)}`).join("\n")}\n`
      : "# 未设置运行覆盖，使用 Codex 默认值\n";
  }
  const root: Record<string, unknown> = {};
  for (const [key, value] of entries) {
    const parts = key.split(".");
    let cursor = root;
    for (const part of parts.slice(0, -1)) {
      const next = cursor[part];
      if (!next || typeof next !== "object" || Array.isArray(next)) cursor[part] = {};
      cursor = cursor[part] as Record<string, unknown>;
    }
    cursor[parts[parts.length - 1]] = value;
  }
  return JSON.stringify(root, null, 2);
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
    if (field.presentation === "choice_cards") {
      return (
        <fieldset data-testid={testIdOf(field)} className="col-span-full rounded-lg border bg-background p-3">
          <legend className="px-1 font-medium text-foreground">{field.label}</legend>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">{field.description}</p>
          <div className="mt-3 grid gap-2 md:grid-cols-3">
            {options.map((option) => {
              const checked = option.value === selected;
              return (
                <label
                  key={option.value}
                  className={cn(
                    "cursor-pointer rounded-lg border p-3 transition-colors",
                    checked ? "border-primary bg-primary/5 ring-1 ring-primary/20" : "hover:bg-accent/50",
                    option.danger && "border-destructive/30 bg-destructive/5",
                    disabled && "cursor-not-allowed opacity-60",
                  )}
                >
                  <span className="flex items-center gap-2">
                    <input
                      type="radio"
                      name={field.key}
                      value={option.value}
                      checked={checked}
                      disabled={disabled}
                      data-testid={`${testIdOf(field)}-${option.value}`}
                      onChange={() => onChange(option.value)}
                    />
                    <span className="font-medium text-foreground">{option.label}</span>
                    {option.recommended && (
                      <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary">推荐</span>
                    )}
                    {option.danger && (
                      <span className="rounded-full bg-destructive/10 px-2 py-0.5 text-[10px] font-medium text-destructive">高风险</span>
                    )}
                  </span>
                  {option.description && (
                    <span className="mt-2 block text-xs leading-5 text-muted-foreground">{option.description}</span>
                  )}
                  <FieldHints field={{ ...field, targets: option.targets ?? [] }} value={option.value} />
                </label>
              );
            })}
          </div>
        </fieldset>
      );
    }
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

/** 后端 schema 驱动的运行设置编辑器；表单修改后可即时查看真实目标键值。 */
export function AgentRuntimeDialog({
  agentName,
  adapter,
  fields,
  initial,
  currentConfig,
  busy,
  error,
  onSubmit,
  onClose,
}: {
  agentName: string;
  adapter: string;
  fields: AgentRuntimeField[];
  initial: AgentRuntimeSettings;
  currentConfig?: AgentCurrentConfigStatus | null;
  busy: boolean;
  error?: string | null;
  onSubmit: (settings: AgentRuntimeSettings) => void;
  onClose: () => void;
}) {
  const [settings, setSettings] = useState<AgentRuntimeSettings>(initial);
  const [mode, setMode] = useState<"form" | "preview" | "observed">("form");
  const [localError, setLocalError] = useState<string | null>(null);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLElement>({
    onEscape: busy ? undefined : onClose,
    initialFocusSelector: '[data-testid="agent-runtime-cancel"]',
  });

  const close = () => {
    if (!busy) onClose();
  };

  const switchMode = (next: "form" | "preview" | "observed") => {
    setLocalError(null);
    setMode(next);
  };

  const submit = () => {
    try {
      const normalized = normalizeAgentRuntimeSettings(settings, fields);
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
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={`${agentName} 运行设置`}
        onKeyDown={onDialogKeyDown}
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
            aria-pressed={mode === "preview"}
            data-testid="agent-runtime-preview-mode"
            onClick={() => switchMode("preview")}
            className={cn("inline-flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm", mode === "preview" ? "border-primary bg-primary/10 text-primary" : "text-muted-foreground hover:bg-accent")}
          >
            <Braces className="h-4 w-4" aria-hidden /> 写入预览
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
          ) : mode === "preview" ? (
            <div className="space-y-3">
              <div className="rounded-lg border border-primary/20 bg-primary/5 px-3 py-3 text-sm text-foreground">
                <span className="text-muted-foreground">保存后写入：</span>{" "}
                <code data-testid="agent-runtime-target" className="break-all font-semibold">
                  {currentConfig?.target ?? currentConfig?.file ?? "目标文件暂不可用"}
                </code>
                <p className="mt-1 text-xs leading-5 text-muted-foreground">
                  下方是 Agent Manager 负责的最终配置值；目标文件中的其他配置保持不变。返回“按项填写”可直接修改。
                </p>
              </div>
              <pre
                aria-label="最终写入配置预览"
                data-testid="agent-runtime-preview"
                className="min-h-[24rem] w-full overflow-auto rounded-lg border border-input bg-slate-950 p-4 font-mono text-xs leading-5 text-slate-100"
              >
                {runtimePreview(adapter, fields, settings)}
              </pre>
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
          <button type="button" data-testid="agent-runtime-cancel" disabled={busy} onClick={close} className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50">
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
