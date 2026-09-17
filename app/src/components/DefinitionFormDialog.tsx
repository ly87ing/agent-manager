import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import {
  type PlatformScope,
  buildAgentDefinition,
  extractCommandForPlatform,
  parseInitialToFormValues,
  updateCommandWithPlatform,
} from "../lib/agentDefinitionForm";

const FOCUSABLE_SELECTOR =
  'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

/**
 * 对话框骨架：焦点环（Tab/Shift+Tab 不逃到背景）、Escape 关闭、
 * 关闭后焦点恢复到触发按钮；正文滚动、底部操作固定，避免小窗口截断按钮。
 */
function DialogShell({
  title,
  onClose,
  footer,
  children,
}: {
  title: string;
  onClose: () => void;
  footer: ReactNode;
  children: ReactNode;
}) {
  const sectionRef = useRef<HTMLElement>(null);
  // 渲染期捕获触发元素：autoFocus 在提交阶段已把焦点移入对话框，effect 里再读已晚
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  if (restoreFocusRef.current === null && typeof document !== "undefined") {
    const active = document.activeElement;
    restoreFocusRef.current = active instanceof HTMLElement && active !== document.body ? active : null;
  }

  // 用 layout effect：清理在对话框节点移除前执行，焦点恢复不会被移除操作重置
  useLayoutEffect(() => {
    const previous = restoreFocusRef.current;
    return () => {
      if (previous && previous.isConnected) previous.focus();
    };
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      onClose();
      return;
    }
    if (e.key !== "Tab") return;
    const root = sectionRef.current;
    if (!root) return;
    const focusables = Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
      (el) => !el.hasAttribute("disabled")
    );
    if (focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || !root.contains(active))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && (active === last || !root.contains(active))) {
      e.preventDefault();
      first.focus();
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in" onKeyDown={onKeyDown}>
      <section
        ref={sectionRef}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-xl bg-card shadow-xl animate-zoom-in-95"
      >
        <div className="flex-1 overflow-y-auto p-5">{children}</div>
        <div className="flex flex-shrink-0 justify-end gap-2 border-t bg-card px-5 py-3">
          {footer}
        </div>
      </section>
    </div>
  );
}

export function DefinitionFormDialog({
  initial,
  existingNames,
  onSubmit,
  onClose,
}: {
  initial?: { name: string; definition: Record<string, unknown> };
  existingNames: string[];
  onSubmit: (name: string, definition: Record<string, unknown>) => void;
  onClose: () => void;
}) {
  return (
    <AgentForm
      initial={initial}
      existingNames={existingNames}
      onSubmit={onSubmit}
      onClose={onClose}
    />
  );
}

const PLATFORM_OPTIONS: { id: PlatformScope; label: string }[] = [
  { id: "universal", label: "通用（所有系统）" },
  { id: "darwin", label: "macOS" },
  { id: "linux", label: "Linux" },
  { id: "windows", label: "Windows" },
];

function AgentForm({
  initial,
  existingNames,
  onSubmit,
  onClose,
}: {
  initial?: { name: string; definition: Record<string, unknown> };
  existingNames: string[];
  onSubmit: (name: string, definition: Record<string, unknown>) => void;
  onClose: () => void;
}) {
  const initialParsed = parseInitialToFormValues(initial?.name ?? "", initial?.definition);

  const [name, setName] = useState(initialParsed.name);
  const [displayName, setDisplayName] = useState(initialParsed.displayName);
  const [bin, setBin] = useState(initialParsed.bin);
  const [channel, setChannel] = useState(initialParsed.channel);
  const [platformScope, setPlatformScope] = useState<PlatformScope>(initialParsed.platformScope);

  // Unified draft providers preserving in-flight edits across channel and platform switches
  const [draftProviders, setDraftProviders] = useState<Record<string, Record<string, unknown>>>(() => {
    if (
      initial?.definition?.providers &&
      typeof initial.definition.providers === "object" &&
      !Array.isArray(initial.definition.providers)
    ) {
      return JSON.parse(JSON.stringify(initial.definition.providers));
    }
    return {
      [initialParsed.channel]: {
        managed: true,
      },
    };
  });

  // Track dirty fields for untouched preservation (deep equality)
  const [dirtyFields, setDirtyFields] = useState<Set<string>>(new Set());

  const [installCmd, setInstallCmd] = useState(initialParsed.installCmd);
  const [upgradeCmd, setUpgradeCmd] = useState(initialParsed.upgradeCmd);
  const [uninstallCmd, setUninstallCmd] = useState(initialParsed.uninstallCmd);

  const [preferredProvider, setPreferredProvider] = useState(
    initialParsed.preferredProvider || initialParsed.channel
  );
  const [advancedJson, setAdvancedJson] = useState(initialParsed.advancedJson ?? "");
  const [error, setError] = useState<string | null>(null);

  const title = `${initial ? "编辑" : "添加"}助手`;
  const channelNames = Object.keys(draftProviders);
  if (!channelNames.includes(channel)) channelNames.push(channel);

  const markDirty = (key: string) => {
    setDirtyFields((prev) => {
      const next = new Set(prev);
      next.add(key);
      return next;
    });
  };

  const updateDraftCommand = (action: "install" | "upgrade" | "uninstall", newText: string) => {
    markDirty(`providers.${channel}.${action}`);
    setDraftProviders((prev) => {
      const active = prev[channel] ?? { managed: true };
      const updated = updateCommandWithPlatform(active[action], newText, platformScope);
      const nextActive = { ...active };
      if (updated !== undefined) {
        nextActive[action] = updated;
      } else {
        delete nextActive[action];
      }
      return { ...prev, [channel]: nextActive };
    });
  };

  const handleInstallChange = (newText: string) => {
    setInstallCmd(newText);
    updateDraftCommand("install", newText);
  };

  const handleUpgradeChange = (newText: string) => {
    setUpgradeCmd(newText);
    updateDraftCommand("upgrade", newText);
  };

  const handleUninstallChange = (newText: string) => {
    setUninstallCmd(newText);
    updateDraftCommand("uninstall", newText);
  };

  const handleChannelChange = (newChannel: string) => {
    setChannel(newChannel);
    const targetProvider = draftProviders[newChannel] ?? {};
    setInstallCmd(extractCommandForPlatform(targetProvider.install, platformScope));
    setUpgradeCmd(extractCommandForPlatform(targetProvider.upgrade, platformScope));
    setUninstallCmd(extractCommandForPlatform(targetProvider.uninstall, platformScope));
  };

  const handlePlatformChange = (newPlatform: PlatformScope) => {
    setPlatformScope(newPlatform);
    const targetProvider = draftProviders[channel] ?? {};
    setInstallCmd(extractCommandForPlatform(targetProvider.install, newPlatform));
    setUpgradeCmd(extractCommandForPlatform(targetProvider.upgrade, newPlatform));
    setUninstallCmd(extractCommandForPlatform(targetProvider.uninstall, newPlatform));
  };

  const submit = () => {
    try {
      if (!/^[a-z0-9][a-z0-9-]*$/.test(name)) throw new Error("名称只能用小写字母、数字和连字符");
      if (!initial && existingNames.includes(name)) throw new Error("名称已存在，请编辑已有条目");
      if (!bin.trim()) throw new Error("助手定义需要填写 bin 命令名");
      // 仅新定义强制必要命令；编辑已有定义不强迫补当前平台不支持的命令（如无 Windows 升级命令的既有定义）
      if (!initial && !installCmd.trim()) throw new Error("请填写安装命令");
      if (!initial && !upgradeCmd.trim()) throw new Error("请填写升级命令");

      const allInputs = [name, displayName, bin, channel, installCmd, upgradeCmd, uninstallCmd, preferredProvider, advancedJson];
      if (allInputs.some((item) => item.includes("YOUR_"))) {
        throw new Error("请先填写示例中 YOUR_* 对应的实际值");
      }

      if (advancedJson.trim()) {
        let parsed: unknown;
        try {
          parsed = JSON.parse(advancedJson);
        } catch (jsonErr) {
          throw new Error(`高级设置 JSON 格式错误: ${jsonErr instanceof Error ? jsonErr.message : String(jsonErr)}`);
        }
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
          throw new Error("高级设置必须是 JSON 对象");
        }
        const obj = parsed as Record<string, unknown>;
        if ("_delete" in obj || "_replace" in obj) {
          throw new Error("高级设置不能包含保留键 _delete 或 _replace");
        }
      }

      // preferred_provider validation: must point to an existing provider
      const effectiveProviderKeys = Object.keys(draftProviders);
      if (!effectiveProviderKeys.includes(channel) && channel.trim()) {
        effectiveProviderKeys.push(channel.trim());
      }
      const targetPreferred = preferredProvider.trim() || channel.trim();
      if (!effectiveProviderKeys.includes(targetPreferred)) {
        throw new Error(`默认安装方式 "${targetPreferred}" 必须指向已存在的渠道（可选值: ${effectiveProviderKeys.join(", ")}）`);
      }

      const definition = buildAgentDefinition(
        {
          name,
          displayName,
          bin,
          channel,
          platformScope,
          installCmd,
          upgradeCmd,
          uninstallCmd,
          preferredProvider: targetPreferred,
          advancedJson,
        },
        initial?.definition,
        dirtyFields,
        draftProviders
      );

      onSubmit(name, definition);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <DialogShell
      title={title}
      onClose={onClose}
      footer={
        <>
          <button type="button" onClick={onClose} className="rounded border border-input px-4 py-2 text-sm hover:bg-accent">取消</button>
          <button type="button" onClick={submit} className="rounded bg-primary px-4 py-2 text-sm text-primary-foreground hover:bg-primary/90">保存定义</button>
        </>
      }
    >
        <h3 className="text-base font-semibold text-foreground">{title}</h3>
        <p className="mt-1 text-sm text-muted-foreground">配置生命周期命令。保存定义只保存配置，不会执行安装或升级。</p>

        <div className="mt-4 grid grid-cols-1 gap-3 sm:grid-cols-2">
          <label className="block text-sm font-medium text-foreground">唯一名称<input autoFocus disabled={!!initial} value={name} onChange={(e) => { setName(e.target.value); markDirty("name"); }} placeholder="如 claude-code" className="mt-1 w-full rounded border border-input p-2 text-sm disabled:bg-muted" /></label>
          <label className="block text-sm font-medium text-foreground">显示名称（可选）<input value={displayName} onChange={(e) => { setDisplayName(e.target.value); markDirty("displayName"); }} placeholder="如 Claude Code" className="mt-1 w-full rounded border border-input p-2 text-sm" /></label>
        </div>

        <div className="mt-3">
          <label className="block text-sm font-medium text-foreground">命令名或可执行文件路径<input value={bin} onChange={(e) => { setBin(e.target.value); markDirty("bin"); }} placeholder="如 claude" className="mt-1 w-full rounded border border-input p-2 text-sm font-mono" /></label>
        </div>

        <div className="mt-3 flex items-center justify-between rounded-lg border bg-muted px-3 py-2 text-xs text-muted-foreground font-mono">
          <span>版本检查：{bin.trim() || "YOUR_COMMAND"} --version</span>
          <span className="text-muted-foreground font-sans">执行环境：系统默认 Shell</span>
        </div>

        <div className="mt-4 space-y-3">
          <label className="block text-sm font-medium text-foreground">安装命令（新增时必填）<textarea rows={2} value={installCmd} onChange={(e) => handleInstallChange(e.target.value)} placeholder="YOUR_INSTALL_COMMAND" className="mt-1 w-full rounded border border-input p-2 font-mono text-xs font-normal" spellCheck={false} /></label>
          <label className="block text-sm font-medium text-foreground">升级命令（新增时必填）<textarea rows={2} value={upgradeCmd} onChange={(e) => handleUpgradeChange(e.target.value)} placeholder="YOUR_UPGRADE_COMMAND" className="mt-1 w-full rounded border border-input p-2 font-mono text-xs font-normal" spellCheck={false} /></label>
          <div>
            <label className="block text-sm font-medium text-foreground">卸载命令（可选）<textarea rows={2} value={uninstallCmd} onChange={(e) => handleUninstallChange(e.target.value)} placeholder="可选；未填则表示不支持自动卸载" className="mt-1 w-full rounded border border-input p-2 font-mono text-xs font-normal" spellCheck={false} /></label>
            {!uninstallCmd.trim() && <p className="mt-1 text-xs text-warning">未填写卸载命令（不支持自动卸载）。</p>}
          </div>
        </div>

        <details className="mt-4 rounded-lg border p-3">
          <summary className="cursor-pointer text-xs font-medium text-muted-foreground select-none">高级设置</summary>
          <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="block text-xs font-medium text-muted-foreground">
              管理渠道
              {channelNames.length > 1 ? (
                <select aria-label="管理渠道" value={channel} onChange={(e) => handleChannelChange(e.target.value)} className="mt-1 w-full rounded border border-input bg-card p-1.5 text-xs font-normal">
                  {channelNames.map((ch) => <option key={ch} value={ch}>{ch}</option>)}
                </select>
              ) : (
                <p className="mt-1 rounded border bg-muted p-1.5 font-mono text-xs font-normal text-foreground">{channel}</p>
              )}
            </div>
            <label className="block text-xs font-medium text-muted-foreground">
              适用系统
              <select value={platformScope} onChange={(e) => handlePlatformChange(e.target.value as PlatformScope)} className="mt-1 w-full rounded border border-input bg-card p-1.5 text-xs">
                {PLATFORM_OPTIONS.map((opt) => <option key={opt.id} value={opt.id}>{opt.label}</option>)}
              </select>
            </label>
          </div>
          <div className="mt-3 space-y-3">
            <label className="block text-xs font-medium text-muted-foreground">
              默认安装方式
              <input value={preferredProvider} onChange={(e) => { setPreferredProvider(e.target.value); markDirty("preferredProvider"); }} className="mt-1 w-full rounded border border-input p-1.5 text-xs font-mono" />
            </label>
            <label className="block text-xs font-medium text-muted-foreground">
              其他设置（JSON）
              <textarea rows={3} value={advancedJson} onChange={(e) => { setAdvancedJson(e.target.value); markDirty("advancedJson"); }} placeholder="{}" className="mt-1 w-full rounded border border-input p-1.5 font-mono text-xs" spellCheck={false} />
            </label>
          </div>
        </details>

        {error && <p role="alert" className="mt-3 text-sm text-destructive">{error}</p>}
    </DialogShell>
  );
}
