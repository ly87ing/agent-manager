import { useEffect, useMemo, useState } from "react";
import { X } from "lucide-react";
import { cn } from "../lib/utils";
import { FileDiff } from "./FileDiff";
import { useDialogFocus } from "../lib/dialogFocus";
import { hasPresetArgs, presetsForCommand, togglePresetArgs } from "../lib/agentPresets";
import {
  ALIAS_NAME_RE,
  LANG_OPTIONS,
  aliasTextError,
  buildAliasValue,
  listTimeZones,
  parseAliasValue,
  renderAliasPreview,
} from "../lib/aliasCommand";
import type { AliasFilePreview } from "../lib/useAliasSource";

/** 下拉里的「自定义…」哨兵值：选中后切到可输入状态。 */
const CUSTOM_OPTION = "__custom__";

/**
 * 启动命令编辑弹窗：为一个终端快捷命令设置时区、语言与扩展参数，并实时预览最终那一行。
 *
 * 结构化编辑只处理「环境变量在前、命令在后」这一形状；读到的值不是这个形状时（例如包了
 * open/env 等其它命令）自动切到原文编辑，不猜测改写用户已有的写法。
 * 保存即按名写回 ~/.zshrc 里本工具维护的那一段（~/.zprofile 已有这一段时同步改）。
 */
export function AliasEditorDialog({
  initialName,
  initialValue,
  defaultCommand,
  presetName,
  defaults,
  existingNames,
  previewBase,
  onPreview,
  onRefresh,
  onSubmit,
  onClose,
}: {
  /** 编辑既有别名时传入；新增时留空。 */
  initialName?: string;
  initialValue?: string;
  /** 新增时预填的命令（助手卡片进入时为该助手的可执行名）。 */
  defaultCommand?: string;
  /** 新增时预填的名字（复制既有条目时用）。 */
  presetName?: string;
  /** 新增时预填的时区与语言（写进去的是字面量）。 */
  defaults: { timezone: string; lang: string };
  /** 已存在的别名名称，用于新增时的重名校验。 */
  existingNames: string[];
  previewBase: string;
  onPreview: (name: string, value: string, renameFrom?: string) => Promise<AliasFilePreview | string>;
  /** 预览基准过期时放弃当前表单，重读实际配置后再从新状态打开。 */
  onRefresh: () => void;
  /** 编辑时改了名字即为重命名：调用方据此删掉旧名那一条。 */
  onSubmit: (name: string, value: string) => void;
  onClose: () => void;
}) {
  const parsed = useMemo(() => parseAliasValue(initialValue ?? ""), [initialValue]);
  const isEdit = Boolean(initialName);
  const [rawMode, setRawMode] = useState(Boolean(initialValue) && !parsed.structured);
  const [name, setName] = useState(initialName ?? presetName ?? defaultCommand ?? "");
  const [command, setCommand] = useState(parsed.command || defaultCommand || "");
  const [extraArgs, setExtraArgs] = useState(parsed.extraArgs);
  const [timezone, setTimezone] = useState<string>(parsed.timezone ?? (initialValue ? "" : defaults.timezone));
  const [lang, setLang] = useState<string>(parsed.lang ?? (initialValue ? "" : defaults.lang));
  const [rawValue, setRawValue] = useState(initialValue ?? "");
  const [error, setError] = useState<string | null>(null);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: onClose });

  const timeZones = useMemo(() => listTimeZones(), []);
  /** 按实际执行的命令给常用参数；危险项选中后在下方明确说明后果。 */
  const presets = useMemo(() => presetsForCommand(command), [command]);
  const activeDangerPreset = presets.find((preset) => preset.danger && hasPresetArgs(extraArgs, preset));
  // 读到的取值不在下拉里（例如手写的少见时区或语言）时直接进自定义模式，避免下拉把它吞掉
  const [tzCustom, setTzCustom] = useState(Boolean(parsed.timezone) && !timeZones.includes(parsed.timezone as string));
  const [langCustom, setLangCustom] = useState(Boolean(parsed.lang) && !LANG_OPTIONS.includes(parsed.lang as string));
  /** 除 TZ/LANG/LC_ALL 外的自定义环境变量行（可增删改）。 */
  const [envRows, setEnvRows] = useState<{ key: string; value: string }[]>(
    Object.entries(parsed.otherEnv).map(([key, value]) => ({ key, value })),
  );
  const value = rawMode
    ? rawValue
    : buildAliasValue({
        command,
        extraArgs,
        timezone: timezone || null,
        lang: lang || null,
        otherEnv: Object.fromEntries(
          envRows.filter((row) => row.key.trim()).map((row) => [row.key.trim(), row.value]),
        ),
      });
  const preview = name && value ? renderAliasPreview(name, value) : "";
  const filePreviewKey = JSON.stringify([previewBase, initialName, name, value]);
  const canPreview = ALIAS_NAME_RE.test(name)
    && (name === initialName || !existingNames.includes(name))
    && (rawMode || Boolean(command.trim()))
    && Boolean(value.trim())
    && !aliasTextError(value)
    && (rawMode || envRows.every((row) => !row.key.trim() || (/^[A-Za-z_][A-Za-z0-9_]*$/.test(row.key.trim()) && !row.value.includes('"'))));
  const [filePreview, setFilePreview] = useState<{ key: string; result: AliasFilePreview | string } | null>(null);

  useEffect(() => {
    if (!canPreview) return;
    let current = true;
    const timer = window.setTimeout(() => {
      void onPreview(name, value, initialName)
        .then((result) => { if (current) setFilePreview({ key: filePreviewKey, result }); })
        .catch((error) => { if (current) setFilePreview({ key: filePreviewKey, result: String(error) }); });
    }, 250);
    return () => { current = false; window.clearTimeout(timer); };
  }, [filePreviewKey, canPreview, onPreview]);

  const currentFilePreview = canPreview && filePreview?.key === filePreviewKey ? filePreview.result : null;

  const submit = () => {
    if (!ALIAS_NAME_RE.test(name)) {
      setError("名称要以字母或下划线开头，只能用字母、数字、下划线与连字符");
      return;
    }
    // 改名允许，但不能撞上另一条；编辑时撞回自己的原名不算冲突。
    if (name !== initialName && existingNames.includes(name)) {
      setError("已有同名快捷命令，换个名称或直接编辑那一条");
      return;
    }
    if (!rawMode && !command.trim()) {
      setError("要填写实际执行的命令");
      return;
    }
    if (!value.trim()) {
      setError("命令内容不能为空");
      return;
    }
    for (const row of envRows) {
      const key = row.key.trim();
      if (!key) continue;
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) {
        setError(`环境变量名 ${key} 不合法：要以字母或下划线开头，只能用字母、数字与下划线`);
        return;
      }
      if (row.value.includes('"')) {
        setError(`环境变量 ${key} 的值不能包含双引号`);
        return;
      }
    }
    const textError = aliasTextError(value);
    if (textError) {
      setError(`命令内容${textError}`);
      return;
    }
    if (!currentFilePreview || typeof currentFilePreview === "string") {
      setError("请先确认完整文件预览");
      return;
    }
    onSubmit(name, value);
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in"
      onClick={(event) => event.target === event.currentTarget && onClose()}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={isEdit ? "编辑启动命令" : "新增启动命令"}
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        className="max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-xl border bg-card p-5 shadow-xl animate-zoom-in-95"
      >
        <div className="flex items-start justify-between">
          <h3 className="text-base font-semibold text-foreground">{isEdit ? "编辑启动命令" : "新增启动命令"}</h3>
          <button type="button" onClick={onClose} aria-label="关闭" className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <div className="mt-4 space-y-3 text-sm">
          <div>
            <label className="mb-1 block text-xs font-medium text-muted-foreground">在终端里输入的名称</label>
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              data-testid="alias-name-input"
              className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
              placeholder="claude"
            />
          </div>

          {rawMode ? (
            <div>
              <label className="mb-1 block text-xs font-medium text-muted-foreground">命令内容（原文）</label>
              <textarea
                value={rawValue}
                onChange={(event) => setRawValue(event.target.value)}
                rows={3}
                spellCheck={false}
                data-testid="alias-raw-input"
                className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
              />
              <p className="mt-1 text-[11px] text-muted-foreground">
                这条命令不是「环境变量 + 命令」的常见写法，因此按原文编辑，写进去的就是这里的内容。
              </p>
            </div>
          ) : (
            <>
              <div>
                <label className="mb-1 block text-xs font-medium text-muted-foreground">实际执行的命令</label>
                <input
                  value={command}
                  onChange={(event) => setCommand(event.target.value)}
                  data-testid="alias-command-input"
                  className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                  placeholder="claude"
                />
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="mb-1 block text-xs font-medium text-muted-foreground" htmlFor="alias-timezone">
                    时区
                  </label>
                  {tzCustom ? (
                    <div className="flex gap-1.5">
                      <input
                        value={timezone}
                        onChange={(event) => setTimezone(event.target.value)}
                        list="alias-timezone-options"
                        data-testid="alias-timezone-input"
                        placeholder="Asia/Shanghai"
                        className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                      />
                      <datalist id="alias-timezone-options">
                        {timeZones.map((zone) => (
                          <option key={zone} value={zone} />
                        ))}
                      </datalist>
                      <button
                        type="button"
                        data-testid="alias-timezone-back"
                        onClick={() => {
                          setTzCustom(false);
                          setTimezone(defaults.timezone);
                        }}
                        className="shrink-0 rounded border px-2 text-xs text-muted-foreground hover:bg-accent"
                      >
                        选常用
                      </button>
                    </div>
                  ) : (
                    <select
                      id="alias-timezone"
                      value={timezone}
                      onChange={(event) => {
                        if (event.target.value === CUSTOM_OPTION) {
                          setTzCustom(true);
                          setTimezone("");
                          return;
                        }
                        setTimezone(event.target.value);
                      }}
                      data-testid="alias-timezone-select"
                      className="w-full rounded border border-input bg-background px-2 py-1.5 text-xs"
                    >
                      <option value="">不设置</option>
                      {timeZones.map((zone) => (
                        <option key={zone} value={zone}>
                          {zone}
                        </option>
                      ))}
                      <option value={CUSTOM_OPTION}>自定义…</option>
                    </select>
                  )}
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-muted-foreground" htmlFor="alias-lang">
                    语言
                  </label>
                  {langCustom ? (
                    <div className="flex gap-1.5">
                      <input
                        value={lang}
                        onChange={(event) => setLang(event.target.value)}
                        data-testid="alias-lang-input"
                        placeholder="ja_JP.UTF-8"
                        className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                      />
                      <button
                        type="button"
                        data-testid="alias-lang-back"
                        onClick={() => {
                          setLangCustom(false);
                          setLang(defaults.lang);
                        }}
                        className="shrink-0 rounded border px-2 text-xs text-muted-foreground hover:bg-accent"
                      >
                        选常用
                      </button>
                    </div>
                  ) : (
                    <select
                      id="alias-lang"
                      value={lang}
                      onChange={(event) => {
                        if (event.target.value === CUSTOM_OPTION) {
                          setLangCustom(true);
                          setLang("");
                          return;
                        }
                        setLang(event.target.value);
                      }}
                      data-testid="alias-lang-select"
                      className="w-full rounded border border-input bg-background px-2 py-1.5 text-xs"
                    >
                      <option value="">不设置</option>
                      {LANG_OPTIONS.map((item) => (
                        <option key={item} value={item}>
                          {item}
                        </option>
                      ))}
                      <option value={CUSTOM_OPTION}>自定义…</option>
                    </select>
                  )}
                </div>
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-muted-foreground">扩展参数（每次启动都带上）</label>
                <input
                  value={extraArgs}
                  onChange={(event) => setExtraArgs(event.target.value)}
                  data-testid="alias-extra-args-input"
                  className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                  placeholder="--dangerously-skip-permissions"
                />
                {presets.length > 0 && (
                  <div className="mt-1.5 space-y-1.5">
                    <span className="block text-[11px] text-muted-foreground">常用参数（点一下加上或去掉）</span>
                    <div className="flex flex-wrap gap-1.5">
                      {presets.map((preset) => {
                        const on = hasPresetArgs(extraArgs, preset);
                        return (
                          <button
                            key={preset.id}
                            type="button"
                            data-testid={`alias-preset-${preset.id}`}
                            aria-pressed={on}
                            title={preset.hint}
                            onClick={() => setExtraArgs(togglePresetArgs(extraArgs, preset))}
                            className={cn(
                              "rounded border px-2 py-0.5 text-[11px]",
                              on
                                ? preset.danger
                                  ? "border-warning/40 bg-warning/15 text-warning"
                                  : "border-primary/30 bg-primary/10 text-primary"
                                : "border text-muted-foreground hover:bg-accent",
                            )}
                          >
                            {preset.label}
                          </button>
                        );
                      })}
                    </div>
                    {activeDangerPreset && (
                      <p data-testid="alias-preset-warning" className="rounded-md border border-warning/30 bg-warning/10 px-2 py-1.5 text-[11px] text-warning">
                        {activeDangerPreset.hint}
                      </p>
                    )}
                  </div>
                )}
              </div>
              <div>
                <div className="mb-1 flex items-center justify-between">
                  <span className="text-xs font-medium text-muted-foreground">其它环境变量（可选）</span>
                  <button
                    type="button"
                    data-testid="alias-env-add"
                    onClick={() => setEnvRows((prev) => [...prev, { key: "", value: "" }])}
                    className="text-xs text-primary hover:underline"
                  >
                    添加一项
                  </button>
                </div>
                {envRows.length === 0 ? (
                  <p className="text-[11px] text-muted-foreground">例如代理地址、自定义接口地址等，会放在命令前面。</p>
                ) : (
                  <div className="space-y-1.5">
                    {envRows.map((row, index) => (
                      <div key={index} className="flex items-center gap-1.5">
                        <input
                          value={row.key}
                          onChange={(event) =>
                            setEnvRows((prev) => prev.map((item, i) => (i === index ? { ...item, key: event.target.value } : item)))
                          }
                          data-testid={`alias-env-key-${index}`}
                          placeholder="HTTPS_PROXY"
                          className="w-2/5 rounded border border-input px-2 py-1.5 font-mono text-xs"
                        />
                        <input
                          value={row.value}
                          onChange={(event) =>
                            setEnvRows((prev) => prev.map((item, i) => (i === index ? { ...item, value: event.target.value } : item)))
                          }
                          data-testid={`alias-env-value-${index}`}
                          placeholder="http://127.0.0.1:7890"
                          className="flex-1 rounded border border-input px-2 py-1.5 font-mono text-xs"
                        />
                        <button
                          type="button"
                          data-testid={`alias-env-remove-${index}`}
                          onClick={() => setEnvRows((prev) => prev.filter((_, i) => i !== index))}
                          aria-label="移除这项环境变量"
                          className="shrink-0 rounded border px-2 py-1.5 text-xs text-destructive hover:bg-destructive/10"
                        >
                          移除
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </>
          )}

          <div>
            <span className="mb-1 block text-xs font-medium text-muted-foreground">写入 shell 配置的效果</span>
            <pre data-testid="alias-preview" className="overflow-x-auto rounded border bg-muted px-2 py-1.5 font-mono text-[11px] text-foreground">
              {preview || "填写名称与命令后显示"}
            </pre>
          </div>

          <section className="space-y-2 rounded border p-3 text-xs" data-testid="alias-file-preview">
            <p className="font-medium">终端配置文件的改动</p>
            {!canPreview && <p className="text-muted-foreground">填写有效名称和命令后显示要改的地方。</p>}
            {canPreview && currentFilePreview === null && <p className="text-muted-foreground">正在读取实际文件并计算…</p>}
            {typeof currentFilePreview === "string" && <>
              <p role="alert" className="text-destructive">{currentFilePreview}</p>
              <button type="button" onClick={onRefresh} className="rounded border px-2 py-1">关闭并重新读取</button>
            </>}
            {currentFilePreview && typeof currentFilePreview !== "string" && <>
              {currentFilePreview.files.length === 0 && <p className="text-muted-foreground">配置文件不变。</p>}
              {currentFilePreview.files.map((file) => (
                <FileDiff
                  key={file.target}
                  path={file.path}
                  original={file.exists ? file.original : null}
                  content={file.content}
                  testId={`alias-file-preview-${file.target}`}
                  maxHeight="max-h-48"
                />
              ))}
            </>}
            <p className="text-muted-foreground">只读预览，只显示变化的部分。</p>
          </section>

          <button
            type="button"
            onClick={() => setRawMode((prev) => !prev)}
            data-testid="alias-toggle-raw"
            className="text-xs text-primary hover:underline"
          >
            {rawMode ? "回到按项填写" : "改为编辑命令原文"}
          </button>

          {error && (
            <p role="alert" data-testid="alias-form-error" className="text-xs text-destructive">
              {error}
            </p>
          )}
        </div>

        <div className="mt-5 flex items-center justify-end gap-2">
          <span className="mr-auto text-xs text-muted-foreground">保存即写入 ~/.zshrc 里由本工具管理的那几行</span>
          <button type="button" onClick={onClose} className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent">
            取消
          </button>
          <button
            type="button"
            onClick={submit}
            disabled={canPreview && (!currentFilePreview || typeof currentFilePreview === "string")}
            data-testid="alias-form-submit"
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
