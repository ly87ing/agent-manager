import { X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { Account, AgentAccounts } from "../lib/accounts";
import { connectionFromPreset, presetsFor } from "../lib/connectionPresets";
import { useDialogFocus } from "../lib/dialogFocus";
import {
  PROFILE_ID_RE,
  connectionToProfileInput,
  defaultConfigDir,
  parseProfileDefinition,
  type ProfileFormInput,
} from "../lib/profileDefinition";
import { DEFAULT_SECRETS_PATH, type SecretSourceState } from "../lib/secretSource";
import type { ProfileObserved, StatusData } from "../lib/contract";
import { mcpInUseElsewhere } from "../lib/mcpFill";
import { authTerminalCommand } from "../lib/profileDefinition";
import { useEngineQuery } from "../lib/queries";
import { inUseSkillCount } from "../lib/skillFill";
import { CreatePlan } from "./CreatePlan";
import type { CreatePreview, useAccountOps } from "../lib/useAccountOps";
import { cn } from "../lib/utils";

type Ops = ReturnType<typeof useAccountOps>;

export type DrawerMode =
  | { type: "create"; group: AgentAccounts }
  | { type: "edit"; group: AgentAccounts; account: Account; definition: Record<string, unknown>; observed?: ProfileObserved };

function editDefinition(mode: Extract<DrawerMode, { type: "edit" }>): Record<string, unknown> {
  if (!mode.observed?.editable_profile) return mode.definition;
  const profile = { ...(mode.definition.profile as Record<string, unknown>) };
  const keys = mode.group.kind === "claude" ? ["env", "model"] : ["base_url", "env_key", "wire_api", "model"];
  for (const key of keys) delete profile[key];
  return { ...mode.definition, profile: { ...profile, ...mode.observed.editable_profile } };
}

/** 接口地址里的主机名；还不是合法地址时为 null。 */
function hostOf(url: string): string | null {
  try {
    return new URL(url.trim()).host || null;
  } catch {
    return null;
  }
}

const FIELD = "mt-1 w-full rounded-md border bg-background px-2.5 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-ring";

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="block text-sm">
      <span className="font-medium">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-xs text-muted-foreground">{hint}</span>}
    </label>
  );
}

/** 名称里有字母数字时，推一个命令名（小写、连字符）；中文名留空让用户自己填。 */
function suggestId(name: string): string {
  return name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
}

/** 右侧抽屉：添加或编辑一个账号。只问必要的几项，其余（目录、共用设置、启动参数）自动就位。 */
export function AccountDrawer({
  mode,
  ops,
  secretSource,
  reservedNames,
  onCreated,
  onClose,
}: {
  mode: DrawerMode;
  ops: Ops;
  secretSource: SecretSourceState | null;
  /** 不能用作启动命令的名字：助手命令本身与已登记的账号。 */
  reservedNames: string[];
  /** 新建成功：给出新账号的实例 id，账号页据此显示它的准备情况。 */
  onCreated?: (profileId: string) => void;
  onClose: () => void;
}) {
  const { group } = mode;
  const bin = group.bin;
  const editing = mode.type === "edit" ? parseProfileDefinition(mode.account.elementId.replace(/^profiles:/, ""), editDefinition(mode)) : null;
  const editBlocked = mode.type === "edit" && !mode.observed?.editable_profile
    ? mode.observed?.edit_blocked_reason ?? "读不到实际接入参数，请关闭后刷新重试"
    : null;
  const presets = presetsFor(group.kind);

  const [type, setType] = useState<"official" | "api">(editing ? (editing.baseUrl || editing.tokenVar ? "api" : "official") : "official");
  const [name, setName] = useState(editing?.displayName ?? "");
  const [id, setId] = useState(editing?.id ?? "");
  const [idTouched, setIdTouched] = useState(Boolean(editing));
  const [presetId, setPresetId] = useState<string>("custom");
  const [baseUrl, setBaseUrl] = useState(editing?.baseUrl ?? "");
  const [model, setModel] = useState(editing?.model ?? "");
  const [tokenVar, setTokenVar] = useState(editing?.tokenVar ?? "");
  const [apiKey, setApiKey] = useState("");
  const [extraArgs, setExtraArgs] = useState(editing?.extraArgs ?? ops.mainExtraArgs(bin));
  const [preview, setPreview] = useState<{ key: string; result: CreatePreview | string } | null>(null);
  const skillsQuery = useEngineQuery<StatusData>("skills:status", [], { enabled: !editing });
  const mcpQuery = useEngineQuery<StatusData>("mcp:status", [], { enabled: !editing });
  const [saving, setSaving] = useState(false);
  const serial = useRef(0);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: onClose });

  const preset = presets.find((item) => item.id === presetId) ?? null;
  const effectiveId = idTouched ? id : suggestId(name);

  const choosePreset = (next: string) => {
    setPresetId(next);
    const chosen = presets.find((item) => item.id === next);
    if (!chosen) return;
    setBaseUrl(chosen.baseUrl);
    setModel(chosen.model ?? "");
    setTokenVar(chosen.tokenVar);
    if (!name) setName(chosen.label);
  };

  const buildInput = (): ProfileFormInput => {
    const api = type === "api";
    const base: ProfileFormInput = editing ?? {
      id: effectiveId,
      displayName: name,
      baseAgent: group.agentId,
      kind: group.kind,
      configDir: defaultConfigDir(group.kind, effectiveId),
      baseUrl: "",
      tokenVar: "",
      authHeader: "token",
      model: "",
      extraArgs,
      extraEnv: {},
    };
    let input: ProfileFormInput = { ...base, displayName: name, extraEnv: { ...base.extraEnv } };
    if (!editing && api && preset) {
      const made = connectionToProfileInput({ id: group.agentId, kind: group.kind }, connectionFromPreset(preset, effectiveId || "new"), []);
      if (made) input = { ...made.input, id: effectiveId, displayName: name, configDir: defaultConfigDir(group.kind, effectiveId), extraArgs };
    }
    input = {
      ...input,
      baseUrl: api ? baseUrl.trim() : "",
      tokenVar: api ? tokenVar.trim() : "",
      model: api ? model.trim() : input.model,
    };
    return input;
  };

  const input = buildInput();
  const keyKnown = Boolean(tokenVar.trim()) && (secretSource?.names ?? []).includes(tokenVar.trim());
  const idError = !effectiveId
    ? "填一个命令名，只能用小写字母、数字和连字符"
    : !PROFILE_ID_RE.test(effectiveId)
      ? "命令名只能用小写字母、数字和连字符，且以字母或数字开头"
      : !editing && reservedNames.includes(effectiveId)
        ? `${effectiveId} 已被占用`
        : null;
  const apiError = !editBlocked && type === "api" && (!baseUrl.trim() ? "填接口地址" : !tokenVar.trim() ? "填密钥变量名" : !apiKey && !keyKnown ? "填 API Key" : null);
  const previewKey = JSON.stringify(input);

  // 新建：输入稳定后自动预览将写入的文件；保存时引擎会再比对一次
  useEffect(() => {
    if (editing || idError || (type === "api" && (!baseUrl.trim() || !tokenVar.trim()))) return;
    const current = ++serial.current;
    const timer = window.setTimeout(() => {
      void ops.previewCreate(input, bin).then((result) => {
        if (serial.current === current) setPreview({ key: previewKey, result });
      });
    }, 300);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [previewKey, idError, ops.ready]);

  const previewReady = preview?.key === previewKey && typeof preview.result !== "string" ? preview.result : null;
  const previewError = preview?.key === previewKey && typeof preview.result === "string" ? preview.result : null;
  const canSave = !saving && !idError && !apiError && (editing ? true : Boolean(previewReady));

  const submit = async () => {
    if (!canSave) return;
    setSaving(true);
    try {
      const secret = type === "api" && apiKey ? { variable: tokenVar.trim(), value: apiKey } : undefined;
      if (mode.type === "create") {
        if (await ops.create(input, bin, previewReady!, reservedNames, secret)) {
          onCreated?.(input.id);
          onClose();
        }
        return;
      }
      const profileId = mode.account.elementId.replace(/^profiles:/, "");
      const nameChanged = name.trim() !== editing!.displayName;
      const connectionChanged = !editBlocked && type === "api"
        && (baseUrl.trim() !== editing!.baseUrl || model.trim() !== editing!.model || tokenVar.trim() !== editing!.tokenVar || Boolean(apiKey));
      if (connectionChanged) {
        if (!(await ops.updateConnection(profileId, input, bin, mode.observed!, secret, nameChanged ? name.trim() : undefined))) return;
      } else if (nameChanged && !(await ops.rename(profileId, name.trim(), mode.definition))) return;
      onClose();
    } finally {
      setSaving(false);
    }
  };

  const planFacts = previewReady && !editing
    ? {
        configDir: input.configDir,
        mainDir: group.kind === "codex" ? "~/.codex" : "~/.claude",
        command: ops.launchValue(input, bin),
        commandName: input.id,
        api: type === "api"
          ? {
              baseUrl: input.baseUrl,
              model: input.model,
              tokenVar: input.tokenVar,
              keyFile: apiKey ? (secretSource?.configured && secretSource.path ? secretSource.path : DEFAULT_SECRETS_PATH) : null,
            }
          : null,
        skills: skillsQuery.data?.ok ? inUseSkillCount(skillsQuery.data.data.elements) : 0,
        extraMcp: mcpQuery.data?.ok ? mcpInUseElsewhere(mcpQuery.data.data.elements, mcpQuery.data.data.targets ?? [], group.agentId) : [],
        loginCommand: type === "official" ? authTerminalCommand(group.kind, bin, "login", input.configDir) : null,
      }
    : null;

  return (
    <div className="fixed inset-0 z-40 flex justify-end bg-black/30" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={editing ? `编辑账号 ${editing.displayName}` : "添加账号"}
        data-testid="account-drawer"
        onKeyDown={onDialogKeyDown}
        className="flex h-full w-full max-w-md flex-col border-l bg-card shadow-xl"
      >
        <header className="flex items-center justify-between border-b px-5 py-3">
          <h2 className="text-base font-semibold">{editing ? "编辑账号" : `添加 ${group.label} 账号`}</h2>
          <button type="button" aria-label="关闭" onClick={onClose} className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </header>
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4">
          {!editing && (
            <div role="radiogroup" aria-label="账号类型" className="grid grid-cols-2 gap-2">
              {(["official", "api"] as const).map((value) => (
                <button
                  key={value}
                  type="button"
                  role="radio"
                  aria-checked={type === value}
                  data-testid={`account-type-choice-${value}`}
                  onClick={() => {
                    setType(value);
                    if (value === "official" && preset) {
                      if (name === preset.label) setName("");
                      setPresetId("custom");
                    }
                  }}
                  className={cn(
                    "rounded-md border px-3 py-2 text-left text-sm",
                    type === value ? "border-primary bg-primary/10 font-medium text-primary" : "hover:bg-accent",
                  )}
                >
                  {value === "official" ? "官方账号" : "API"}
                  <span className="block text-xs font-normal text-muted-foreground">
                    {value === "official" ? "建好后在终端登录一次" : "第三方接口 + API Key"}
                  </span>
                </button>
              ))}
            </div>
          )}

          {!editing && type === "api" && presets.length > 0 && (
            <Field label="服务商">
              <select data-testid="account-preset" value={presetId} onChange={(event) => choosePreset(event.target.value)} className={FIELD}>
                <option value="custom">自定义</option>
                {presets.map((item) => (
                  <option key={item.id} value={item.id}>{item.label}</option>
                ))}
              </select>
            </Field>
          )}

          <Field label="名称">
            <input data-testid="account-name" value={name} onChange={(event) => setName(event.target.value)} placeholder="公司 Pro" className={FIELD} />
          </Field>

          <Field
            label="启动命令"
            hint={editing ? `目录 ${editing.configDir}` : `目录 ${defaultConfigDir(group.kind, effectiveId || "…")}`}
          >
            <input
              data-testid="account-id"
              value={effectiveId}
              disabled={Boolean(editing)}
              onChange={(event) => {
                setIdTouched(true);
                setId(event.target.value);
              }}
              placeholder="cc2"
              className={cn(FIELD, "font-mono")}
            />
            {!editing && effectiveId && idError && <span className="mt-1 block text-xs text-destructive">{idError}</span>}
          </Field>

          {type === "api" && (
            <fieldset disabled={Boolean(editBlocked)} className="space-y-4">
              {editBlocked && <p role="status" className="text-xs text-muted-foreground">暂不能编辑接入参数：{editBlocked}。仍可修改名称。</p>}
              <Field label="接口地址">
                <input data-testid="account-base-url" value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} placeholder="https://" className={FIELD} />
              </Field>
              <Field label="API Key" hint={`保存到外部密钥文件的 ${tokenVar || "变量"}，应用时也会写入此账号的助手配置，不进本应用数据库${keyKnown ? "；已有值，留空则不改" : ""}`}>
                <input data-testid="account-api-key" type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} autoComplete="off" className={FIELD} />
              </Field>
              {hostOf(baseUrl) && (
                <p data-testid="account-api-risk" className="rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-foreground">
                  用这个账号时，你的提示词、代码片段和这个 Key 都会发往 <span className="font-mono font-medium">{hostOf(baseUrl)}</span>；
                  Key 会明文写进 <span className="font-mono">{`${editing?.configDir ?? defaultConfigDir(group.kind, effectiveId || "…")}/${group.kind === "codex" ? "config.toml" : "settings.json"}`}</span>（只有你自己能读）。只接你信任的服务。
                </p>
              )}
              <Field label="模型">
                <input data-testid="account-model" value={model} onChange={(event) => setModel(event.target.value)} className={FIELD} />
              </Field>
              <Field label="密钥变量名">
                <input data-testid="account-token-var" value={tokenVar} onChange={(event) => setTokenVar(event.target.value)} placeholder="DEEPSEEK_API_KEY" className={cn(FIELD, "font-mono")} />
              </Field>
            </fieldset>
          )}

          {!editing && (
            <Field label="启动参数" hint="沿用主账号命令里的参数，可以改">
              <input data-testid="account-extra-args" value={extraArgs} onChange={(event) => setExtraArgs(event.target.value)} className={cn(FIELD, "font-mono")} />
            </Field>
          )}

          {!editing && (
            planFacts && previewReady ? (
              <CreatePlan preview={previewReady} facts={planFacts} />
            ) : (
              <p data-testid="account-preview-state" className="rounded-md border px-3 py-2 text-xs text-muted-foreground">
                {previewError ? `没法预览要写的内容，暂不能创建：${previewError}` : idError || apiError ? "填好上面几项后，这里会逐条列出创建时要做的事。" : "正在计算创建时要做的事…"}
              </p>
            )
          )}
        </div>
        <footer className="flex items-center justify-end gap-2 border-t px-5 py-3">
          {apiError && <span className="mr-auto text-xs text-muted-foreground">{apiError}</span>}
          <button type="button" onClick={onClose} className="rounded-md border px-3 py-1.5 text-sm hover:bg-accent">取消</button>
          <button
            type="button"
            data-testid="account-save"
            disabled={!canSave}
            onClick={() => void submit()}
            className="rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-40"
          >
            {saving ? "保存中…" : editing ? "保存" : "创建"}
          </button>
        </footer>
      </div>
    </div>
  );
}
