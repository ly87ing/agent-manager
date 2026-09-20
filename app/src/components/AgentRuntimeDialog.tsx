import { useLayoutEffect, useRef, useState } from "react";
import { Loader2, X } from "lucide-react";
import type { AgentRuntimeOptions } from "../lib/agentDefinitionForm";

const FOCUSABLE_SELECTOR =
  'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

function FieldHint({ configKey, value }: { configKey: string; value: string }) {
  return (
    <span className="mt-1 block break-all font-mono text-[11px] text-muted-foreground">
      {configKey} = {value}
    </span>
  );
}

function ToggleField({
  checked,
  disabled,
  testId,
  title,
  description,
  configKey,
  enabledValue,
  onChange,
}: {
  checked: boolean;
  disabled: boolean;
  testId: string;
  title: string;
  description: string;
  configKey: string;
  enabledValue: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex min-h-28 items-start gap-3 rounded-lg border bg-background p-3">
      <input
        type="checkbox"
        data-testid={testId}
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
        className="mt-0.5"
      />
      <span className="min-w-0">
        <span className="block font-medium text-foreground">{title}</span>
        <span className="mt-1 block text-xs leading-5 text-muted-foreground">{description}</span>
        <FieldHint configKey={configKey} value={checked ? enabledValue : "<删除该键，使用客户端默认>"} />
      </span>
    </label>
  );
}

/** 管理助手真正支持的运行参数；接入、登录和生命周期不在这里混用。 */
export function AgentRuntimeDialog({
  agentName,
  kind,
  initial,
  busy,
  error,
  onSubmit,
  onClose,
}: {
  agentName: string;
  kind: "claude" | "codex";
  initial: AgentRuntimeOptions;
  busy: boolean;
  error?: string | null;
  onSubmit: (options: AgentRuntimeOptions) => void;
  onClose: () => void;
}) {
  const rootRef = useRef<HTMLElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(
    typeof document === "undefined" ? null : (document.activeElement as HTMLElement | null),
  );
  const [options, setOptions] = useState(initial);
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

  const submit = () => {
    if (kind === "codex") {
      for (const [label, raw] of [
        ["上下文窗口", options.codexContextWindow],
        ["自动压缩阈值", options.codexAutoCompactTokenLimit],
      ] as const) {
        if (raw.trim() && (!Number.isInteger(Number(raw)) || Number(raw) <= 0)) {
          setLocalError(`${label}必须是正整数，留空表示使用 Codex 默认值`);
          return;
        }
      }
    }
    setLocalError(null);
    onSubmit(options);
  };

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
        className="flex max-h-[92vh] w-full max-w-5xl flex-col overflow-hidden rounded-xl border bg-card shadow-xl animate-zoom-in-95"
      >
        <div className="flex items-start justify-between gap-3 border-b px-5 py-4 sm:px-6">
          <div>
            <h3 className="text-lg font-semibold text-foreground">{agentName} 运行设置</h3>
            <p className="mt-1 text-sm text-muted-foreground">
              常用项直接编辑；每项下面同时标出将写入的真实配置键和值。账号、接口地址、模型与凭据变量在接入卡片中维护。
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

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5 sm:px-6">
          <div className="space-y-5 text-sm">
            <section>
              <h4 className="font-semibold text-foreground">权限与执行</h4>
              <p className="mt-1 text-xs text-muted-foreground">高风险设置仍会在保存后要求二次确认。</p>
              <label className="mt-3 flex items-start gap-3 rounded-lg border border-destructive/25 bg-destructive/5 p-3">
                <input
                  type="checkbox"
                  data-testid="agent-runtime-yolo"
                  checked={options.yolo}
                  disabled={busy}
                  onChange={(event) => setOptions((current) => ({ ...current, yolo: event.target.checked }))}
                  className="mt-0.5"
                />
                <span>
                  <span className="block font-medium text-foreground">跳过权限确认</span>
                  <span className="mt-1 block text-xs leading-5 text-muted-foreground">
                    开启后执行命令和修改文件不再逐次询问；关闭时撤销本工具写入的权限覆盖。
                  </span>
                  {kind === "claude" ? (
                    <FieldHint
                      configKey="permissions.defaultMode"
                      value={options.yolo ? '\"bypassPermissions\"' : "<删除该键，使用客户端默认>"}
                    />
                  ) : (
                    <>
                      <FieldHint configKey="approval_policy" value={options.yolo ? '\"never\"' : "<删除该键，使用客户端默认>"} />
                      <FieldHint configKey="sandbox_mode" value={options.yolo ? '\"danger-full-access\"' : "<删除该键，使用客户端默认>"} />
                    </>
                  )}
                </span>
              </label>
            </section>

            {kind === "claude" ? (
              <>
                <section>
                  <h4 className="font-semibold text-foreground">界面与模型行为</h4>
                  <div className="mt-3 grid grid-cols-1 gap-3 lg:grid-cols-2">
                    <label className="rounded-lg border bg-background p-3 text-foreground">
                      状态栏
                      <select
                        data-testid="agent-runtime-statusline"
                        value={options.statusline}
                        disabled={busy}
                        onChange={(event) =>
                          setOptions((current) => ({
                            ...current,
                            statusline: event.target.value === "builtin" ? "builtin" : "untouched",
                          }))
                        }
                        className="mt-2 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                      >
                        <option value="untouched">保持现有状态栏</option>
                        <option value="builtin">使用 Agent Manager 用量状态栏</option>
                      </select>
                      <FieldHint
                        configKey="statusLine.command"
                        value={options.statusline === "builtin" ? "<Agent Manager statusline.sh>" : "<不写入，保留现状>"}
                      />
                    </label>
                    <label className="rounded-lg border bg-background p-3 text-foreground">
                      推理强度
                      <select
                        data-testid="agent-runtime-claude-effort"
                        value={options.claudeEffortLevel}
                        disabled={busy}
                        onChange={(event) => setOptions((current) => ({
                          ...current,
                          claudeEffortLevel: event.target.value as AgentRuntimeOptions["claudeEffortLevel"],
                        }))}
                        className="mt-2 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                      >
                        <option value="">使用 Claude 默认值</option>
                        <option value="auto">auto</option>
                        <option value="low">low</option>
                        <option value="medium">medium</option>
                        <option value="high">high</option>
                        <option value="xhigh">xhigh</option>
                        <option value="max">max</option>
                      </select>
                      <FieldHint
                        configKey="env.CLAUDE_CODE_EFFORT_LEVEL"
                        value={options.claudeEffortLevel || "<删除该键，使用客户端默认>"}
                      />
                    </label>
                  </div>
                </section>
                <section>
                  <h4 className="font-semibold text-foreground">高级能力</h4>
                  <div className="mt-3 grid grid-cols-1 gap-3 lg:grid-cols-2">
                    <ToggleField
                      checked={options.claudeToolSearch}
                      disabled={busy}
                      testId="agent-runtime-claude-tool-search"
                      title="工具搜索"
                      description="让 Claude Code 按需发现工具，减少一次性加载的工具描述。"
                      configKey="env.ENABLE_TOOL_SEARCH"
                      enabledValue='"true"'
                      onChange={(checked) => setOptions((current) => ({ ...current, claudeToolSearch: checked }))}
                    />
                    <ToggleField
                      checked={options.claudeAgentTeams}
                      disabled={busy}
                      testId="agent-runtime-claude-agent-teams"
                      title="Agent Teams"
                      description="启用 Claude Code 的实验性多 Agent 协作能力。"
                      configKey="env.CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"
                      enabledValue='"1"'
                      onChange={(checked) => setOptions((current) => ({ ...current, claudeAgentTeams: checked }))}
                    />
                    <ToggleField
                      checked={options.claudeDisableArtifact}
                      disabled={busy}
                      testId="agent-runtime-claude-disable-artifact"
                      title="禁用 Artifact"
                      description="不让 Claude Code 创建 Artifact；适合只需要代码与终端输出的环境。"
                      configKey="env.CLAUDE_CODE_DISABLE_ARTIFACT"
                      enabledValue='"1"'
                      onChange={(checked) => setOptions((current) => ({ ...current, claudeDisableArtifact: checked }))}
                    />
                    <ToggleField
                      checked={options.claudeHideAttribution}
                      disabled={busy}
                      testId="agent-runtime-claude-hide-attribution"
                      title="隐藏提交与 PR 署名"
                      description="把 Claude 的 commit 和 PR attribution 模板设为空。"
                      configKey="attribution.commit / attribution.pr"
                      enabledValue='""'
                      onChange={(checked) => setOptions((current) => ({ ...current, claudeHideAttribution: checked }))}
                    />
                  </div>
                </section>
              </>
            ) : (
              <section>
                <h4 className="font-semibold text-foreground">模型与上下文</h4>
                <p className="mt-1 text-xs text-muted-foreground">留空即删除本工具的覆盖，恢复 Codex 自己的默认值。</p>
                <div className="mt-3 grid grid-cols-1 gap-3 lg:grid-cols-2">
                  <label className="rounded-lg border bg-background p-3 text-foreground">
                    推理强度
                    <select
                      data-testid="agent-runtime-codex-reasoning"
                      value={options.codexReasoningEffort}
                      disabled={busy}
                      onChange={(event) => setOptions((current) => ({
                        ...current,
                        codexReasoningEffort: event.target.value as AgentRuntimeOptions["codexReasoningEffort"],
                      }))}
                      className="mt-2 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    >
                      <option value="">使用 Codex 默认值</option>
                      <option value="minimal">minimal</option>
                      <option value="low">low</option>
                      <option value="medium">medium</option>
                      <option value="high">high</option>
                      <option value="xhigh">xhigh</option>
                    </select>
                    <FieldHint configKey="model_reasoning_effort" value={options.codexReasoningEffort || "<删除该键，使用客户端默认>"} />
                  </label>
                  <label className="rounded-lg border bg-background p-3 text-foreground">
                    Web 搜索模式
                    <select
                      data-testid="agent-runtime-codex-web-search"
                      value={options.codexWebSearch}
                      disabled={busy}
                      onChange={(event) => setOptions((current) => ({
                        ...current,
                        codexWebSearch: event.target.value as AgentRuntimeOptions["codexWebSearch"],
                      }))}
                      className="mt-2 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    >
                      <option value="">使用 Codex 默认值</option>
                      <option value="disabled">disabled</option>
                      <option value="cached">cached</option>
                      <option value="indexed">indexed</option>
                      <option value="live">live</option>
                    </select>
                    <FieldHint configKey="web_search" value={options.codexWebSearch || "<删除该键，使用客户端默认>"} />
                  </label>
                  <label className="rounded-lg border bg-background p-3 text-foreground">
                    上下文窗口（tokens）
                    <input
                      data-testid="agent-runtime-codex-context-window"
                      inputMode="numeric"
                      value={options.codexContextWindow}
                      disabled={busy}
                      placeholder="例如 1000000"
                      onChange={(event) => setOptions((current) => ({ ...current, codexContextWindow: event.target.value }))}
                      className="mt-2 w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm"
                    />
                    <FieldHint configKey="model_context_window" value={options.codexContextWindow || "<删除该键，使用客户端默认>"} />
                  </label>
                  <label className="rounded-lg border bg-background p-3 text-foreground">
                    自动压缩阈值（tokens）
                    <input
                      data-testid="agent-runtime-codex-compact-limit"
                      inputMode="numeric"
                      value={options.codexAutoCompactTokenLimit}
                      disabled={busy}
                      placeholder="例如 900000"
                      onChange={(event) => setOptions((current) => ({ ...current, codexAutoCompactTokenLimit: event.target.value }))}
                      className="mt-2 w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm"
                    />
                    <FieldHint configKey="model_auto_compact_token_limit" value={options.codexAutoCompactTokenLimit || "<删除该键，使用客户端默认>"} />
                  </label>
                </div>
              </section>
            )}

            <p className="rounded-lg bg-muted px-3 py-2 text-xs text-muted-foreground">
              官方账号仍在助手自身完成登录；本工具只维护上述运行参数，不读取账号密码或令牌明文。
            </p>
            {(error || localError) && <p role="alert" className="text-sm text-destructive">{error ?? localError}</p>}
          </div>
        </div>

        <div className="flex justify-end gap-2 border-t px-5 py-4 sm:px-6">
          <button
            type="button"
            disabled={busy}
            onClick={close}
            className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            取消
          </button>
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
        </div>
      </section>
    </div>
  );
}
