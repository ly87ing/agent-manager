import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import {
  type ConnectionEntry,
  type PlatformScope,
  CONNECTION_NAME_RE,
  buildAgentDefinition,
  emptyConnection,
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
  // 运行参数：写进助手自己的配置文件，因此 IDE 和别的终端启动它也一样生效
  const [connections, setConnections] = useState<ConnectionEntry[]>(initialParsed.connections);
  const [activeConnection, setActiveConnection] = useState(initialParsed.activeConnection);
  const [yolo, setYolo] = useState(initialParsed.yolo);
  const [statusline, setStatusline] = useState(initialParsed.statusline);
  /** 展开编辑的那一套；同时只展开一套，避免弹窗被撑得太长。 */
  const [openConnection, setOpenConnection] = useState<string | null>(null);

  const patchConnection = (index: number, patch: Partial<ConnectionEntry>) => {
    setConnections((list) => list.map((item, i) => (i === index ? { ...item, ...patch } : item)));
    markDirty("connections");
  };
  const addConnection = () => {
    // 名字自动避让已有的，省得用户先想名字
    let candidate = "relay";
    let n = 2;
    while (connections.some((item) => item.name === candidate)) {
      candidate = `relay${n}`;
      n += 1;
    }
    setConnections((list) => [...list, emptyConnection(candidate)]);
    setOpenConnection(candidate);
    if (!activeConnection) setActiveConnection(candidate);
    markDirty("connections");
  };
  const removeConnection = (index: number) => {
    const removed = connections[index];
    setConnections((list) => list.filter((_, i) => i !== index));
    if (activeConnection === removed?.name) setActiveConnection("");
    markDirty("connections");
  };
  // 运行参数按形态写各自的配置文件；判定与引擎一致，只认可执行名
  const binName = bin.trim().replace(/\.exe$/i, "").split("/").pop();
  const settingsKind = binName === "claude" ? "claude" : binName === "codex" ? "codex" : null;

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
          connections,
          activeConnection,
          yolo,
          statusline,
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

        {/* 运行参数：只有 Claude Code 形态的助手写得进去（settings.json） */}
        {settingsKind && (
          <div className="mt-4 rounded-lg border p-3">
            <p className="text-xs font-medium text-foreground">运行参数</p>
            <p className="mt-0.5 text-xs text-muted-foreground">
              写进助手自己的配置文件，从终端、IDE、任何入口启动它都一样生效。保存即写入。
            </p>

            <div className="mt-3 flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-muted-foreground">接入配置</span>
              <button
                type="button"
                data-testid="add-connection"
                onClick={addConnection}
                className="rounded border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs text-primary hover:bg-primary/15"
              >
                添加一套
              </button>
            </div>
            {connections.length === 0 ? (
              <p className="mt-1.5 text-xs text-muted-foreground">
                还没配：用官方账号直接跑。要连中转站或换 key，添加一套。
              </p>
            ) : (
              <ul className="mt-1.5 space-y-1.5">
                {connections.map((item, index) => {
                  const open = openConnection === item.name;
                  const nameBad = !CONNECTION_NAME_RE.test(item.name);
                  return (
                    <li key={index} className="rounded border">
                      <div className="flex items-center gap-2 px-2 py-1.5">
                        <input
                          type="radio"
                          name="active-connection"
                          aria-label={`让 ${item.label || item.name} 生效`}
                          data-testid={`use-connection-${item.name}`}
                          checked={activeConnection === item.name}
                          onChange={() => { setActiveConnection(item.name); markDirty("connections"); }}
                        />
                        <span className="min-w-0 flex-1 truncate text-xs text-foreground">
                          {item.label || item.name}
                          {item.baseUrl && (
                            <span className="ml-1.5 font-mono text-xs text-muted-foreground">{item.baseUrl}</span>
                          )}
                        </span>
                        <button
                          type="button"
                          data-testid={`edit-connection-${item.name}`}
                          onClick={() => setOpenConnection(open ? null : item.name)}
                          className="rounded border px-2 py-0.5 text-xs text-muted-foreground hover:bg-accent"
                        >
                          {open ? "收起" : "编辑"}
                        </button>
                        <button
                          type="button"
                          data-testid={`remove-connection-${item.name}`}
                          onClick={() => removeConnection(index)}
                          className="rounded border border-destructive/30 px-2 py-0.5 text-xs text-destructive hover:bg-destructive/10"
                        >
                          删除
                        </button>
                      </div>
                      {open && (
                        <div className="space-y-2 border-t px-2 py-2">
                          <div className="grid grid-cols-2 gap-2">
                            <label className="block text-xs text-muted-foreground">
                              名称（写进配置）
                              <input
                                data-testid="connection-name-input"
                                value={item.name}
                                onChange={(e) => {
                                  const next = e.target.value;
                                  if (activeConnection === item.name) setActiveConnection(next);
                                  setOpenConnection(next);
                                  patchConnection(index, { name: next });
                                }}
                                className="mt-1 w-full rounded border border-input p-1.5 font-mono text-xs"
                              />
                              {nameBad && (
                                <span className="mt-1 block text-xs text-destructive">
                                  只能用字母、数字、连字符与下划线
                                </span>
                              )}
                            </label>
                            <label className="block text-xs text-muted-foreground">
                              显示名（可选）
                              <input
                                value={item.label}
                                onChange={(e) => patchConnection(index, { label: e.target.value })}
                                placeholder="如 官方 / 我的中转"
                                className="mt-1 w-full rounded border border-input p-1.5 text-xs"
                              />
                            </label>
                          </div>
                          <label className="block text-xs text-muted-foreground">
                            接口地址（可选）
                            <input
                              data-testid="connection-base-url-input"
                              value={item.baseUrl}
                              onChange={(e) => patchConnection(index, { baseUrl: e.target.value })}
                              placeholder="留空用官方地址；只接受 https:// 或本机地址"
                              className="mt-1 w-full rounded border border-input p-1.5 font-mono text-xs"
                              spellCheck={false}
                            />
                          </label>
                          <div className={settingsKind === "codex" ? "" : "grid grid-cols-2 gap-2"}>
                            <label className="block text-xs text-muted-foreground">
                              令牌所在的环境变量名
                              <input
                                data-testid="connection-token-var-input"
                                value={item.tokenVar}
                                onChange={(e) => patchConnection(index, { tokenVar: e.target.value })}
                                placeholder={
                                  settingsKind === "codex"
                                    ? "Codex 启动时自己去环境里读这个变量"
                                    : "取值在密钥页填"
                                }
                                className="mt-1 w-full rounded border border-input p-1.5 font-mono text-xs"
                                spellCheck={false}
                              />
                            </label>
                            {settingsKind === "claude" && (
                              <label className="block text-xs text-muted-foreground">
                                令牌怎么发
                                <select
                                  data-testid="connection-auth-select"
                                  value={item.auth}
                                  onChange={(e) => patchConnection(index, { auth: e.target.value as ConnectionEntry["auth"] })}
                                  className="mt-1 w-full rounded border border-input bg-card p-1.5 text-xs"
                                >
                                  <option value="token">Authorization: Bearer</option>
                                  <option value="api_key">x-api-key</option>
                                </select>
                              </label>
                            )}
                          </div>
                          <div className={settingsKind === "codex" ? "" : "grid grid-cols-2 gap-2"}>
                            <label className="block text-xs text-muted-foreground">
                              模型（可选）
                              <input
                                data-testid="connection-model-input"
                                value={item.model}
                                onChange={(e) => patchConnection(index, { model: e.target.value })}
                                className="mt-1 w-full rounded border border-input p-1.5 font-mono text-xs"
                                spellCheck={false}
                              />
                            </label>
                            {settingsKind === "claude" && (
                              <label className="block text-xs text-muted-foreground">
                                小任务模型（可选）
                                <input
                                  value={item.smallFastModel}
                                  onChange={(e) => patchConnection(index, { smallFastModel: e.target.value })}
                                  className="mt-1 w-full rounded border border-input p-1.5 font-mono text-xs"
                                  spellCheck={false}
                                />
                              </label>
                            )}
                          </div>
                          <details className="rounded border p-2">
                            <summary className="cursor-pointer text-xs text-muted-foreground select-none">
                              {settingsKind === "codex" ? "超时" : "模型别名与超时"}
                            </summary>
                            {settingsKind === "claude" && (
                              <p className="mt-1 text-xs text-muted-foreground">
                                中转站的模型名和官方对不上时，在这里把 opus / sonnet / haiku 各自指过去。
                              </p>
                            )}
                            <div className={settingsKind === "codex" ? "hidden" : "mt-2 grid grid-cols-3 gap-2"}>
                              {([["opus", "aliasOpus"], ["sonnet", "aliasSonnet"], ["haiku", "aliasHaiku"]] as const).map(
                                ([tier, field]) => (
                                  <label key={tier} className="block text-xs text-muted-foreground">
                                    {tier}
                                    <input
                                      value={item[field]}
                                      onChange={(e) => patchConnection(index, { [field]: e.target.value })}
                                      className="mt-1 w-full rounded border border-input p-1.5 font-mono text-xs"
                                      spellCheck={false}
                                    />
                                  </label>
                                ),
                              )}
                            </div>
                            <label className="mt-2 block text-xs text-muted-foreground">
                              请求超时（毫秒，可选）
                              <input
                                type="number"
                                min={0}
                                value={item.timeoutMs}
                                onChange={(e) => patchConnection(index, { timeoutMs: e.target.value })}
                                className="mt-1 w-32 rounded border border-input p-1.5 font-mono text-xs"
                              />
                            </label>
                          </details>
                        </div>
                      )}
                    </li>
                  );
                })}
              </ul>
            )}

            <div className="mt-3 space-y-2 border-t pt-3">
              {settingsKind === "claude" && (
              <label className="block text-xs text-muted-foreground">
                状态栏
                <select
                  data-testid="statusline-select"
                  value={statusline}
                  onChange={(e) => { setStatusline(e.target.value as "untouched" | "builtin"); markDirty("statusline"); }}
                  className="mt-1 w-full rounded border border-input bg-card p-1.5 text-xs"
                >
                  <option value="untouched">不碰（你自己定义的那份照旧）</option>
                  <option value="builtin">用本工具的（显示额度，并让这里也能读到实时额度）</option>
                </select>
                <span className="mt-1 block text-xs text-muted-foreground">
                  助手只把额度推给状态栏、不写文件，所以用本工具这份才能在卡片上看到与它同屏的实时额度。
                </span>
              </label>
              )}
              <label className="flex items-start gap-2 text-xs text-foreground">
                <input
                  type="checkbox"
                  data-testid="agent-yolo-checkbox"
                  checked={yolo}
                  onChange={(e) => { setYolo(e.target.checked); markDirty("yolo"); }}
                  className="mt-0.5"
                />
                <span>
                  跳过权限确认
                  <span className="mt-0.5 block text-xs text-warning">
                    {settingsKind === "codex"
                      ? "它让 Codex 不再征求你同意，也不再受沙箱限制：执行命令、改文件都直接做。保存时会再确认一次。"
                      : "它改的是助手的安全默认值：之后助手执行命令、改文件都不再征求你同意。保存时会再确认一次。"}
                  </span>
                </span>
              </label>
            </div>
          </div>
        )}

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
