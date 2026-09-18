import { useMemo, useState } from "react";
import { X } from "lucide-react";
import {
  ALIAS_NAME_RE,
  FOLLOW_GLOBAL_LANG,
  FOLLOW_GLOBAL_TZ,
  LANG_OPTIONS,
  aliasTextError,
  buildAliasValue,
  listTimeZones,
  parseAliasValue,
  renderAliasPreview,
} from "../lib/aliasCommand";

/**
 * 启动命令编辑弹窗：为一个终端快捷命令设置时区、语言与扩展参数，并实时预览最终那一行。
 *
 * 结构化编辑只处理「环境变量在前、命令在后」这一形状；读到的值不是这个形状时（例如包了
 * open/env 等其它命令）自动切到原文编辑，不猜测改写用户已有的写法。
 * 保存只写本地资源库，要写进 shell 配置需在页面上另点同步。
 */
export function AliasEditorDialog({
  initialName,
  initialValue,
  defaultCommand,
  globals,
  existingNames,
  onSubmit,
  onClose,
}: {
  /** 编辑既有别名时传入；新增时留空。 */
  initialName?: string;
  initialValue?: string;
  /** 新增时预填的命令（助手卡片进入时为该助手的可执行名）。 */
  defaultCommand?: string;
  globals: { timezone: string; lang: string };
  /** 已存在的别名名称，用于新增时的重名校验。 */
  existingNames: string[];
  onSubmit: (name: string, value: string) => void;
  onClose: () => void;
}) {
  const parsed = useMemo(() => parseAliasValue(initialValue ?? ""), [initialValue]);
  const isEdit = Boolean(initialName);
  const [rawMode, setRawMode] = useState(Boolean(initialValue) && !parsed.structured);
  const [name, setName] = useState(initialName ?? defaultCommand ?? "");
  const [command, setCommand] = useState(parsed.command || defaultCommand || "");
  const [extraArgs, setExtraArgs] = useState(parsed.extraArgs);
  const [timezone, setTimezone] = useState<string>(parsed.timezone ?? (initialValue ? "" : FOLLOW_GLOBAL_TZ));
  const [lang, setLang] = useState<string>(parsed.lang ?? (initialValue ? "" : FOLLOW_GLOBAL_LANG));
  const [rawValue, setRawValue] = useState(initialValue ?? "");
  const [error, setError] = useState<string | null>(null);

  const timeZones = useMemo(() => listTimeZones(), []);
  const value = rawMode
    ? rawValue
    : buildAliasValue({
        command,
        extraArgs,
        timezone: timezone || null,
        lang: lang || null,
        otherEnv: parsed.otherEnv,
      });
  const preview = name && value ? renderAliasPreview(name, value, globals) : "";

  const submit = () => {
    if (!ALIAS_NAME_RE.test(name)) {
      setError("名称要以字母或下划线开头，只能用字母、数字、下划线与连字符");
      return;
    }
    if (!isEdit && existingNames.includes(name)) {
      setError("已有同名快捷命令，改个名称或直接编辑那一条");
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
    const textError = aliasTextError(value);
    if (textError) {
      setError(`命令内容${textError}`);
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
        role="dialog"
        aria-modal="true"
        aria-label={isEdit ? "编辑启动命令" : "新增启动命令"}
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
              disabled={isEdit}
              onChange={(event) => setName(event.target.value)}
              data-testid="alias-name-input"
              className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs disabled:bg-muted"
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
              <p className="mt-1 text-[11px] text-muted-foreground">这条命令不是「环境变量 + 命令」的常见写法，因此按原文编辑。</p>
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
                  <select
                    id="alias-timezone"
                    value={timezone}
                    onChange={(event) => setTimezone(event.target.value)}
                    data-testid="alias-timezone-select"
                    className="w-full rounded border border-input bg-background px-2 py-1.5 text-xs"
                  >
                    <option value={FOLLOW_GLOBAL_TZ}>跟随全局（{globals.timezone}）</option>
                    <option value="">不设置</option>
                    {timeZones.map((zone) => (
                      <option key={zone} value={zone}>
                        {zone}
                      </option>
                    ))}
                  </select>
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-muted-foreground" htmlFor="alias-lang">
                    语言
                  </label>
                  <select
                    id="alias-lang"
                    value={lang}
                    onChange={(event) => setLang(event.target.value)}
                    data-testid="alias-lang-select"
                    className="w-full rounded border border-input bg-background px-2 py-1.5 text-xs"
                  >
                    <option value={FOLLOW_GLOBAL_LANG}>跟随全局（{globals.lang}）</option>
                    <option value="">不设置</option>
                    {LANG_OPTIONS.map((item) => (
                      <option key={item} value={item}>
                        {item}
                      </option>
                    ))}
                  </select>
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
              </div>
              {Object.keys(parsed.otherEnv).length > 0 && (
                <p className="text-[11px] text-muted-foreground">
                  保留原有环境变量：{Object.keys(parsed.otherEnv).join("、")}
                </p>
              )}
            </>
          )}

          <div>
            <span className="mb-1 block text-xs font-medium text-muted-foreground">写入 shell 配置的效果</span>
            <pre data-testid="alias-preview" className="overflow-x-auto rounded border bg-muted px-2 py-1.5 font-mono text-[11px] text-foreground">
              {preview || "填写名称与命令后显示"}
            </pre>
          </div>

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
          <span className="mr-auto text-xs text-muted-foreground">只保存配置，之后点同步才写入终端配置文件</span>
          <button type="button" onClick={onClose} className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent">
            取消
          </button>
          <button
            type="button"
            onClick={submit}
            data-testid="alias-form-submit"
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
          >
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
