import { useEffect, useMemo, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import type { AgentRuntimeSettings } from "../lib/agentDefinitionForm";
import { automationRawText, CUSTOM_AUTOMATION, runtimePatch } from "../lib/agentRuntime";
import { engineErrorMessage, runEngine } from "../lib/engine";
import type { AgentCurrentConfigStatus, AgentRuntimeField, AgentRuntimeTarget } from "../lib/observed";
import { cn } from "../lib/utils";

function testIdOf(field: AgentRuntimeField): string {
  return `agent-runtime-${field.key.replaceAll("_", "-")}`;
}

function targetsFor(field: AgentRuntimeField, value: unknown): AgentRuntimeTarget[] {
  const option = field.options?.find((candidate) => candidate.value === value);
  return option?.targets ?? field.targets;
}

function previewValue(target: AgentRuntimeTarget, value: unknown): unknown {
  if (!target.enabled_value) return value;
  try {
    return JSON.parse(target.enabled_value);
  } catch {
    return target.enabled_value;
  }
}

/** 只预览这次改动的字段：设值的给出最终键值，撤掉的列为删除；没改的保持客户端当前值。 */
function runtimePreview(adapter: string, fields: AgentRuntimeField[], patch: Record<string, unknown>): string {
  const changed = fields.filter((field) => field.key in patch);
  if (changed.length === 0) return "# 还没有改动；未改动的项保持客户端当前值\n";
  const removed = changed
    .filter((field) => {
      const value = patch[field.key];
      return value === null || value === "" || value === false;
    })
    .flatMap((field) => field.targets.map((target) => target.key));
  const entries = changed.flatMap((field) => {
    const value = patch[field.key];
    if (value === undefined || value === null || value === "" || value === false) return [];
    return targetsFor(field, value).map((target) => [target.key, previewValue(target, value)] as const);
  });
  const removedText = removed.length > 0 ? `${removed.map((key) => `# 删除 ${key}`).join("\n")}\n` : "";
  if (adapter === "codex") {
    return `${entries.map(([key, value]) => `${key} = ${JSON.stringify(value)}`).join("\n")}${entries.length > 0 ? "\n" : ""}${removedText}`;
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
  return `${entries.length > 0 ? `${JSON.stringify(root, null, 2)}\n` : ""}${removedText}`;
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
          "flex items-start gap-3 rounded-lg border bg-background p-3",
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
      </label>
    );
  }

  if (field.type === "positive_integer" || field.type === "model") {
    const text = value === undefined || value === null ? "" : String(value);
    return (
      <label className="rounded-lg border bg-background p-3 text-foreground">
        <span className="font-medium">{field.label}</span>
        <span className="mt-1 block text-xs leading-5 text-muted-foreground">{field.description}</span>
        <input
          data-testid={testIdOf(field)}
          inputMode={field.type === "positive_integer" ? "numeric" : "text"}
          value={text}
          disabled={disabled}
          placeholder={field.placeholder}
          onChange={(event) => onChange(event.target.value)}
          className="mt-2 w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm"
        />
      </label>
    );
  }

  return (
    <div className="rounded-lg border border-warning/30 bg-warning/10 p-3 text-sm text-warning">
      当前版本不认识字段类型 <code>{field.type}</code>（<code>{field.key}</code>），已停止编辑，避免误写。
    </div>
  );
}

/**
 * 后端 schema 驱动的运行设置，直接嵌在「运行设置」页签里。表单以客户端实际值预填，
 * 提交只发改动过的字段，没改的项（包括本工具不提供的取值）保持客户端当前值。
 * 写入后状态回读换了基准，调用方用新的 key 重建它，表单即回到新的实际值。
 */
export function AgentRuntimePanel({
  agentId,
  adapter,
  fields,
  initial,
  currentConfig,
  busy,
  onSubmit,
}: {
  agentId: string;
  adapter: string;
  fields: AgentRuntimeField[];
  /** 客户端实际值（actualRuntimeSettings）。 */
  initial: AgentRuntimeSettings;
  currentConfig?: AgentCurrentConfigStatus | null;
  busy: boolean;
  /** 只含改动过的字段；撤掉的项为 null。 */
  onSubmit: (patch: Record<string, unknown>) => void;
}) {
  const [settings, setSettings] = useState<AgentRuntimeSettings>(initial);
  const [localError, setLocalError] = useState<string | null>(null);
  const [filePreview, setFilePreview] = useState<{ key: string; path: string | null; content: string | null; error: string | null } | null>(null);
  const [previewBusy, setPreviewBusy] = useState(false);
  const previewSerial = useRef(0);
  const rawText = automationRawText(currentConfig);
  // 实际档位不在三档预设里：多给一张「保持当前」卡片，不改就原样保留
  const shownFields = useMemo(() => fields.map((field) => {
    const current = initial[field.key];
    if (field.key === "automation_mode" && current === CUSTOM_AUTOMATION) {
      return {
        ...field,
        options: [
          {
            value: CUSTOM_AUTOMATION,
            label: "保持当前",
            description: `客户端当前是 ${rawText || "本工具未提供的设置"}；不改就原样保留。`,
          },
          ...(field.options ?? []),
        ],
      };
    }
    // 其他下拉项的实际值不在可选范围里：照样显示成当前值，不改就原样保留
    if (
      field.type === "enum"
      && typeof current === "string"
      && current
      && !field.options?.some((option) => option.value === current)
    ) {
      return { ...field, options: [{ value: current, label: `${current}（当前值）` }, ...(field.options ?? [])] };
    }
    return field;
  }), [fields, initial, rawText]);
  const pendingPatch = (() => {
    try {
      return runtimePatch(fields, initial, settings);
    } catch {
      return {};
    }
  })();
  const changedCount = Object.keys(pendingPatch).length;
  const base = currentConfig?.base;
  const previewKey = JSON.stringify({ base, patch: pendingPatch });
  const previewReady = changedCount > 0 && Boolean(base);

  useEffect(() => {
    if (!previewReady) {
      setPreviewBusy(false);
      setFilePreview(null);
      return;
    }
    const serial = ++previewSerial.current;
    setPreviewBusy(true);
    const timer = window.setTimeout(async () => {
      try {
        const envelope = await runEngine<{ path: string; content: string }>("agents:preview-runtime", [
          "--only", `agents:${agentId}`,
          "--base", base!,
          "--runtime", JSON.stringify(pendingPatch),
        ]);
        if (serial === previewSerial.current) setFilePreview({
          key: previewKey,
          path: envelope.ok ? envelope.data.path : null,
          content: envelope.ok ? envelope.data.content : null,
          error: envelope.ok ? null : envelope.errors[0]?.message ?? "无法预览完整配置文件",
        });
      } catch (err) {
        if (serial === previewSerial.current) setFilePreview({ key: previewKey, path: null, content: null, error: engineErrorMessage(err) });
      } finally {
        if (serial === previewSerial.current) setPreviewBusy(false);
      }
    }, 300);
    return () => {
      window.clearTimeout(timer);
      previewSerial.current += 1;
    };
  }, [previewKey, previewReady, agentId]);

  const submit = () => {
    try {
      const patch = runtimePatch(fields, initial, settings);
      if (Object.keys(patch).length === 0) return;
      setLocalError(null);
      onSubmit(patch);
    } catch (err) {
      setLocalError(err instanceof Error ? err.message : String(err));
    }
  };

  const reset = () => {
    setSettings(initial);
    setFilePreview(null);
    setLocalError(null);
  };

  const sections = Array.from(new Set(shownFields.map((field) => field.section)));

  return (
    <div data-testid="agent-runtime-panel" className="space-y-5 text-sm">
      {sections.map((section) => {
        const sectionFields = shownFields.filter((field) => field.section === section);
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

      {changedCount > 0 && (
        <div className="space-y-2">
          <p className="text-xs text-muted-foreground">
            目标文件{" "}
            <code data-testid="agent-runtime-target" className="break-all font-semibold text-foreground">
              {filePreview?.key === previewKey && filePreview.path
                ? filePreview.path : currentConfig?.target ?? currentConfig?.file ?? "目标文件暂不可用"}
            </code>
            ：{filePreview?.key === previewKey && filePreview.content !== null
              ? "按当前文件合并后的完整内容；保存时会重新读取。现有文件可能含敏感值，请勿直接分享。"
              : filePreview?.key === previewKey && filePreview.error
                ? "完整文件尚未确认；下方只列本次改动的配置键，未改动的项保持不变。"
                : previewReady
                  ? "正在计算完整文件；下方暂列本次改动的配置键，未改动的项保持不变。"
                  : "下方只列本次改动的配置键，目标文件暂不可预览。"}
          </p>
          <pre
            aria-label="写入配置预览"
            data-testid="agent-runtime-preview"
            className="max-h-72 w-full overflow-auto rounded-lg border border-input bg-slate-950 p-4 font-mono text-xs leading-5 text-slate-100"
          >
            {filePreview?.key === previewKey && filePreview.content !== null
              ? filePreview.content
              : runtimePreview(adapter, fields, pendingPatch)}
          </pre>
          {previewBusy && <p className="text-xs text-muted-foreground">正在读取完整配置文件…</p>}
          {filePreview?.key === previewKey && filePreview.error && (
            <p role="alert" className="text-xs text-destructive">完整文件预览失败：{filePreview.error}</p>
          )}
        </div>
      )}
      {localError && <p role="alert" className="text-sm text-destructive">{localError}</p>}

      <div className="flex flex-wrap items-center justify-end gap-2 border-t pt-3">
        <span data-testid="agent-runtime-pending" className="mr-auto text-xs text-muted-foreground">
          {changedCount > 0 ? `${changedCount} 项改动还没写入` : "按客户端当前的实际配置显示"}
        </span>
        <button
          type="button"
          data-testid="agent-runtime-reset"
          disabled={busy || changedCount === 0}
          onClick={reset}
          className="rounded-md border px-3 py-1.5 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50"
        >
          撤销改动
        </button>
        <button
          type="button"
          data-testid="agent-runtime-submit"
          disabled={busy || changedCount === 0}
          onClick={submit}
          className="inline-flex items-center gap-1.5 rounded-md bg-primary px-4 py-1.5 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
        >
          {busy && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
          {busy ? "正在写入…" : "写入助手"}
        </button>
      </div>
    </div>
  );
}
