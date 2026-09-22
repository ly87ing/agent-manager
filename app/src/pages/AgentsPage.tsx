import { useState } from "react";
import { toast } from "sonner";
import { AgentRuntimeDialog } from "../components/AgentRuntimeDialog";
import { AliasEditorDialog } from "../components/AliasEditorDialog";
import { ProfileCard } from "../components/ProfileCards";
import { ProfileFormDialog } from "../components/ProfileFormDialog";
import { ConfirmModal } from "../components/ConfirmModal";
import {
  ConnectionFormDialog,
  type ConnectionSubmission,
} from "../components/ConnectionFormDialog";
import { ConnectionWorkbench, connectionAlignment } from "../components/ConnectionWorkbench";
import { ConnectionImportDialog } from "../components/ConnectionImportDialog";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageLoading } from "../components/PageState";
import type { EngineElement, PolicyData, StatusData } from "../lib/contract";
import { parseAliasValue } from "../lib/aliasCommand";
import {
  buildProfileDefinition,
  defaultConfigDir,
  fixLaunchCommandTilde,
  launchCommandsForProfile,
  parseProfileDefinition,
  profileKindOf,
  profileLaunchValue,
  type ProfileFormInput,
} from "../lib/profileDefinition";
import { runEngine } from "../lib/engine";
import { toastWithUndo } from "../lib/undo";
import {
  activeConnectionOf,
  connectionsOf,
  duplicateConnectionDraft,
  moveAgentConnection,
  parseInitialToFormValues,
  serializeConnectionBundle,
  removeAgentConnection,
  runtimeRequiresYoloConfirmation,
  runtimeSettingsOf,
  upsertAgentConnection,
  updateAgentRuntimeSettings,
  type AgentRuntimeSettings,
  type ConnectionEntry,
} from "../lib/agentDefinitionForm";
import { agentManagementOf, connectionKindOf } from "../lib/agentManagement";
import type { AgentCurrentConfig, AgentsObserved, AgentsSpec } from "../lib/observed";
import { useEngineQuery } from "../lib/queries";
import { useAliasSource } from "../lib/useAliasSource";
import { useDirectApply } from "../lib/useDirectApply";

export default function AgentsPage({
  onManageSkills,
}: {
  onManageSkills?: (agentId: string) => void;
  onGoTakeover?: () => void;
} = {}) {
  const query = useEngineQuery<StatusData>("agents:status");
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const definitionOps = useDefinitionOps({ section: "agents", domain: "agents" });
  // 实例启动命令与「终端快捷命令」页共用同一份来源。
  const aliasSource = useAliasSource();
  const aliasApply = useDirectApply();
  // 实例（同一助手的多套独立配置）：定义与 agents 共用，状态与写入走 profiles 域
  const profilesQuery = useEngineQuery<StatusData>("profiles:status");
  const profileApply = useDirectApply();
  const [profileForm, setProfileForm] = useState<ProfileFormInput | "new" | null>(null);
  /** 复制实例：表单带着预填内容，但名称与配置目录仍要能改（否则两个实例指同一套配置）。 */
  const [profileDuplicating, setProfileDuplicating] = useState(false);
  const [profileRemoving, setProfileRemoving] = useState<string | null>(null);
  /** 日常接入工作台当前查看的主助手；null 时跟随列表里的第一个。 */
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [connectionForm, setConnectionForm] = useState<{
    agent: EngineElement;
    initial?: ConnectionEntry;
    variant?: "edit" | "duplicate";
    preserveUnknownFrom?: string;
  } | null>(null);
  const [connectionSaving, setConnectionSaving] = useState(false);
  const [connectionFormError, setConnectionFormError] = useState<string | null>(null);
  const [connectionImportAgent, setConnectionImportAgent] = useState<EngineElement | null>(null);
  const [connectionRemoving, setConnectionRemoving] = useState<{
    agent: EngineElement;
    connection: ConnectionEntry;
  } | null>(null);
  const [connectionRemovingBusy, setConnectionRemovingBusy] = useState(false);
  const [connectionResetting, setConnectionResetting] = useState<EngineElement | null>(null);
  /** 接入配置的检查结果：按「助手/配置名」存，卡片与弹窗都读它。 */
  const [checkResults, setCheckResults] = useState<
    Record<string, { ok: boolean; detail: string; latencyMs: number; pending?: boolean }>
  >({});
  const [checkingAllAgentId, setCheckingAllAgentId] = useState<string | null>(null);
  const [environmentView, setEnvironmentView] = useState<"default" | "independent">("default");

  const [aliasEditing, setAliasEditing] = useState<
    { bin: string; name: string; value: string | null; presetName?: string } | null
  >(null);
  const [launchRemoving, setLaunchRemoving] = useState<string | null>(null);
  const [runtimeFormAgent, setRuntimeFormAgent] = useState<EngineElement | null>(null);
  const [runtimeSaving, setRuntimeSaving] = useState(false);
  const [runtimeFormError, setRuntimeFormError] = useState<string | null>(null);
  /** 运行参数写入前要用户点头的那一步：降安全等级，或目标被外部改过。 */
  const [pendingConfigure, setPendingConfigure] = useState<
    { agentId: string; yolo: boolean; use?: string; reason: "yolo" | "takeover" } | null
  >(null);
  /** 正在写入哪一个助手/接入；非空时禁止并发切换。 */
  const [configuringTarget, setConfiguringTarget] = useState<string | null>(null);
  const configuring = configuringTarget !== null;
  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="agents:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  const profileElements = profilesQuery.data?.ok ? profilesQuery.data.data.elements : [];
  const profileIds = new Set(profileElements.map((element) => element.id.replace(/^profiles:/, "")));
  /** 主助手（非实例）：参与接入工作台与「新建实例」。 */
  const allPrimaryElements = elements.filter(
    (element) =>
      connectionKindOf(element) !== null &&
      !profileIds.has(element.id.replace(/^agents:/, "")),
  );
  const effectiveSelectedAgentId =
    selectedAgentId && allPrimaryElements.some((element) => element.id === selectedAgentId)
      ? selectedAgentId
      : allPrimaryElements[0]?.id ?? null;
  const effectiveAgentName = effectiveSelectedAgentId?.replace(/^agents:/, "") ?? null;
  const visibleProfileElements = profileElements.filter(
    (element) => (element.spec as unknown as { base_agent?: string }).base_agent === effectiveAgentName,
  );
  /** 可作为实例归属的助手：以后由后端适配器声明，不再按定义 ID 猜客户端类型。 */
  const profileCapableAgents = elements
    .map((element) => ({
      id: element.id.replace(/^agents:/, ""),
      spec: element.spec as AgentsSpec,
      management: agentManagementOf(element),
    }))
    .filter((item) => !profileIds.has(item.id) && item.management?.supports_profiles)
    .map((item) => ({
      id: item.id,
      displayName: item.spec.display_name ?? item.id,
      kind: item.management!.connection_kind as "claude" | "codex",
    }))
    .filter((item) => item.kind === "claude" || item.kind === "codex");
  const runtimeManagement = runtimeFormAgent ? agentManagementOf(runtimeFormAgent) : null;
  const baseBinOf = (agentId: string) =>
    (elements.find((element) => element.id === `agents:${agentId}`)?.spec as AgentsSpec | undefined)?.bin ??
    (agentId === "codex" ? "codex" : "claude");

  const takeoverAgent = pendingConfigure?.reason === "takeover"
    ? elements.find((element) => element.id === `agents:${pendingConfigure.agentId}`) ?? null
    : null;
  const takeoverKind = takeoverAgent ? connectionKindOf(takeoverAgent) : null;
  const takeoverCurrent = takeoverAgent
    ? (takeoverAgent.observed as AgentsObserved).current_config?.config ?? null
    : null;
  const takeoverConnection = takeoverAgent && pendingConfigure?.use
    ? parseInitialToFormValues(pendingConfigure.agentId, takeoverAgent.spec as Record<string, unknown>).connections
      .find((item) => item.name === pendingConfigure.use) ?? null
    : null;
  const takeoverDiffs = pendingConfigure?.reason === "takeover" && pendingConfigure.use !== undefined
    ? connectionDifferences(takeoverKind, takeoverConnection, takeoverCurrent)
    : [];

  /**
   * 把运行参数写进助手自己的配置文件。
   *
   * 两处需要用户点头，都由引擎强制、界面只负责问：降安全等级（--allow-yolo）、
   * 目标里我们那几项被外部改过（--allow-takeover）。引擎拒绝时一个字节都不会动。
   */
  const runConfigure = async (
    agentId: string,
    opts: { allowYolo?: boolean; allowTakeover?: boolean; use?: string } = {},
  ): Promise<boolean> => {
    const args = ["--only", `agents:${agentId}`];
    // 切换与写入是一条命令：库里的生效项和目标文件要么一起变，要么都不变
    if (opts.use !== undefined) args.push(...(opts.use ? ["--use", opts.use] : ["--use-default"]));
    if (opts.allowYolo) args.push("--allow-yolo");
    if (opts.allowTakeover) args.push("--allow-takeover");
    const target = opts.use !== undefined ? `${agentId}/${opts.use || "default"}` : `${agentId}/settings`;
    setConfiguringTarget(target);
    try {
      const envelope = await runEngine("agents:configure", args);
      if (envelope.ok) {
        // 以 status 的真实回读更新界面，再宣布成功；不靠本地乐观改状态。
        const refreshed = await query.refetch();
        if (opts.use !== undefined) {
          const refreshedAgent = refreshed.data?.data.elements.find(
            (element) => element.id === `agents:${agentId}`,
          );
          const actual = refreshedAgent
            ? activeConnectionOf(refreshedAgent.spec as Record<string, unknown>)
            : null;
          const expected = opts.use || null;
          const kind = refreshedAgent ? connectionKindOf(refreshedAgent) : null;
          const selectedConnection = refreshedAgent && expected
            ? parseInitialToFormValues(agentId, refreshedAgent.spec as Record<string, unknown>).connections
              .find((connection) => connection.name === expected) ?? null
            : null;
          const currentConfig = refreshedAgent
            ? (refreshedAgent.observed as AgentsObserved).current_config
            : undefined;
          const actualAligned = Boolean(
            kind && currentConfig
            && connectionAlignment(kind, selectedConnection, currentConfig) === "aligned",
          );
          if (!refreshedAgent || actual !== expected || !actualAligned) {
            toast.error("写入已经返回成功，但状态回读没有确认接入生效", {
              description: "界面保留纳管选择和客户端实际配置，请检查配置不一致提示后重试。",
            });
            return false;
          }
        }
        toast.success(opts.use !== undefined ? `已切换 ${agentId} 的接入配置` : `已写入 ${agentId} 的运行参数`, {
          description: "助手下次启动生效；已经开着的要重启它。",
        });
        return true;
      }
      const message = envelope.errors[0]?.message ?? "未知错误";
      // 外部改过：让用户看清再决定要不要用这里的内容替换
      if (!opts.allowTakeover && message.includes("拒绝覆盖")) {
        setPendingConfigure({ agentId, yolo: opts.allowYolo === true, use: opts.use, reason: "takeover" });
        return false;
      }
      toast.error(`运行参数没写进去:${message}`, {
        description: "定义已经存下了，助手的配置文件没动。",
      });
      return false;
    } catch (err) {
      toast.error(`运行参数没写进去:${String(err)}`);
      return false;
    } finally {
      setConfiguringTarget(null);
    }
  };

  /**
   * 测一下这套接入配置：发一个最小真实请求，确认地址、令牌与模型名都对。
   *
   * 这是全局唯一会主动联网的助手侧动作，只在点它时才发；只读，不碰配置文件，
   * 也不改当前生效的那套。
   */
  const runCheck = async (agentId: string, profile: string): Promise<void> => {
    const key = `${agentId}/${profile}`;
    setCheckResults((prev) => ({ ...prev, [key]: { ok: false, detail: "正在测…", latencyMs: 0, pending: true } }));
    try {
      const envelope = await runEngine<{ ok?: boolean; detail?: string; latency_ms?: number }>(
        "agents:check",
        ["--only", `agents:${agentId}`, "--use", profile],
      );
      const detail = envelope.ok
        ? envelope.data?.detail ?? "测完了"
        : envelope.errors[0]?.message ?? "没测成";
      setCheckResults((prev) => ({
        ...prev,
        [key]: {
          ok: envelope.ok === true && envelope.data?.ok === true,
          detail,
          latencyMs: envelope.data?.latency_ms ?? 0,
        },
      }));
    } catch (err) {
      setCheckResults((prev) => ({
        ...prev,
        [key]: { ok: false, detail: String(err), latencyMs: 0 },
      }));
    }
  };

  /** 用户显式触发后并发检查当前助手的全部接入；单项失败不阻断其他结果。 */
  const runCheckAll = async (agentId: string, profiles: string[]): Promise<void> => {
    if (checkingAllAgentId || profiles.length === 0) return;
    setCheckingAllAgentId(agentId);
    try {
      await Promise.all(profiles.map((profile) => runCheck(agentId, profile)));
    } finally {
      setCheckingAllAgentId(null);
    }
  };

  /** 创建/编辑 Provider 只保存定义与可选凭据；应用是卡片上的独立操作。 */
  const saveConnection = async (element: EngineElement, submission: ConnectionSubmission) => {
    if (connectionSaving) return;
    const agentId = element.id.replace(/^agents:/, "");
    setConnectionSaving(true);
    setConnectionFormError(null);
    try {
      const definition = upsertAgentConnection(
        element.spec as Record<string, unknown>,
        submission.connection,
        connectionForm?.preserveUnknownFrom,
      );
      const saved = await definitionOps.saveDefinition({ name: agentId, definition, isNew: false });
      if (!saved) {
        setConnectionFormError("接入没有保存，请根据提示修正后重试");
        return;
      }

      if (submission.secretValue) {
        const secret = await runEngine("secrets:set", [
          "--var",
          submission.connection.tokenVar,
          "--value",
          submission.secretValue,
        ]);
        if (!secret.ok) {
          await query.refetch();
          setConnectionFormError(
            `接入已保存，但凭据没有存下：${secret.errors[0]?.message ?? "请修正后重试，或从外部环境提供这个变量"}`,
          );
          return;
        }
      }

      setConnectionForm(null);
      await query.refetch();
    } catch (err) {
      setConnectionFormError(`保存接入失败:${String(err)}`);
    } finally {
      setConnectionSaving(false);
    }
  };

  /** 删除非当前接入：只删定义里的目标 profile，不清理可能被其他接入复用的凭据变量。 */
  const removeConnection = async (element: EngineElement, connection: ConnectionEntry) => {
    if (connectionRemovingBusy) return;
    const definition = element.spec as Record<string, unknown>;
    if (activeConnectionOf(definition) === connection.name) {
      toast.error("当前接入不能删除，请先切换到其他接入或助手默认账号");
      return;
    }
    const agentId = element.id.replace(/^agents:/, "");
    setConnectionRemovingBusy(true);
    try {
      const saved = await definitionOps.saveDefinition({
        name: agentId,
        definition: removeAgentConnection(definition, connection.name),
        isNew: false,
      });
      if (!saved) return;
      setConnectionRemoving(null);
      await query.refetch();
      toast.success(`已删除接入 ${connection.label || connection.name}`, {
        description: "凭据变量没有删除；仍在使用时不会受影响，可在密钥页单独处理。",
      });
    } finally {
      setConnectionRemovingBusy(false);
    }
  };

  /** Provider 排序只保存定义顺序，不改当前接入，也不触碰助手配置或认证文件。 */
  const moveConnection = async (
    element: EngineElement,
    connection: ConnectionEntry,
    direction: "up" | "down",
  ) => {
    if (connectionSaving) return;
    const agentId = element.id.replace(/^agents:/, "");
    setConnectionSaving(true);
    try {
      const definition = moveAgentConnection(
        element.spec as Record<string, unknown>,
        connection.name,
        direction,
      );
      const saved = await definitionOps.saveDefinition({ name: agentId, definition, isNew: false });
      if (!saved) return;
      await query.refetch();
    } finally {
      setConnectionSaving(false);
    }
  };

  /** 分享内容不含凭据取值；导入只保存定义，不启用、不写助手配置。 */
  const exportConnection = async (element: EngineElement, connection: ConnectionEntry) => {
    const kind = connectionKindOf(element);
    if (!kind) return;
    try {
      await navigator.clipboard.writeText(serializeConnectionBundle(kind, connection));
      toast.success(`已复制接入 ${connection.label || connection.name}`, {
        description: "只包含连接参数与凭据变量名，不包含密钥或官方登录信息。",
      });
    } catch {
      toast.error("复制失败，请检查剪贴板权限");
    }
  };

  /** 保存运行偏好后立刻写进助手配置；只改运行设置弹窗明确拥有的键。 */
  const saveRuntimeSettings = async (element: EngineElement, settings: AgentRuntimeSettings) => {
    if (runtimeSaving) return;
    const agentId = element.id.replace(/^agents:/, "");
    const management = agentManagementOf(element);
    if (!management) {
      setRuntimeFormError("当前助手没有可用的运行设置适配器");
      return;
    }
    setRuntimeSaving(true);
    setRuntimeFormError(null);
    try {
      const definition = updateAgentRuntimeSettings(element.spec as Record<string, unknown>, settings);
      const saved = await definitionOps.saveDefinition({ name: agentId, definition, isNew: false });
      if (!saved) {
        setRuntimeFormError("运行设置没有保存，请根据提示修正后重试");
        return;
      }
      setRuntimeFormAgent(null);
      await query.refetch();
      if (runtimeRequiresYoloConfirmation(settings, management.runtime_fields)) {
        setPendingConfigure({ agentId, yolo: true, reason: "yolo" });
        return;
      }
      await runConfigure(agentId);
    } catch (err) {
      setRuntimeFormError(`保存运行设置失败:${String(err)}`);
    } finally {
      setRuntimeSaving(false);
    }
  };

  /** 保存实例：写定义 + 把启动命令存成同名快捷命令；两步都只写配置，不落地。 */
  const saveProfile = async (input: ProfileFormInput) => {
    const definition = buildProfileDefinition(input, baseBinOf(input.baseAgent));
    let envelope;
    try {
      envelope = await runEngine("policy:set", [
        "--layer",
        "local",
        "--key",
        `agents.definitions.${input.id}`,
        "--value",
        JSON.stringify({ ...definition, _delete: false, _replace: true }),
      ]);
    } catch (err) {
      toast.error(`保存独立环境失败:${String(err)}`);
      return;
    }
    if (!envelope.ok) {
      toast.error(`保存独立环境被拒绝:${envelope.errors[0]?.message ?? "未知错误"}`);
      return;
    }
    if (aliasSource.source) {
      await aliasSource.save(
        {
          aliases: {
            ...aliasSource.source.aliases,
            [input.id]: profileLaunchValue(input, baseBinOf(input.baseAgent)),
          },
        },
        `已保存独立环境 ${input.id} 与它的启动命令`,
      );
    } else {
      toast.success(`已保存独立环境 ${input.id}`);
    }
    setProfileForm(null);
    setProfileDuplicating(false);
    void profilesQuery.refetch();
  };

  /** 移除实例定义：只停止管理这套配置，配置目录与里面的内容原样保留。 */
  const removeProfile = async (id: string) => {
    const snapshot = (policyQuery.data?.ok
      ? ((policyQuery.data.data.layers?.sqlite as Record<string, any> | null)?.agents?.definitions?.[id] ?? null)
      : null) as Record<string, unknown> | null;
    const removedAlias = aliasSource.source?.aliases[id];
    let envelope;
    try {
      envelope = await runEngine("policy:set", [
        "--layer",
        "local",
        "--key",
        `agents.definitions.${id}`,
        "--value",
        JSON.stringify({ _delete: true }),
      ]);
    } catch (err) {
      toast.error(`移除实例失败:${String(err)}`);
      return;
    }
    if (!envelope.ok) {
      toast.error(`移除实例被拒绝:${envelope.errors[0]?.message ?? "未知错误"}`);
      return;
    }
    if (aliasSource.source && aliasSource.source.aliases[id]) {
      const next = { ...aliasSource.source.aliases };
      delete next[id];
      await aliasSource.save({ aliases: next }, `已移除实例 ${id}`, { silent: true });
    }
    setProfileRemoving(null);
    void profilesQuery.refetch();
    if (snapshot) {
      toastWithUndo(`已移除实例 ${id}`, {
        description: "配置目录原样保留；撤销会把实例定义与启动命令加回来。",
        onUndo: async () => {
          const restored = await runEngine("policy:set", [
            "--layer",
            "local",
            "--key",
            `agents.definitions.${id}`,
            "--value",
            JSON.stringify({ ...snapshot, _delete: false, _replace: true }),
          ]);
          if (!restored.ok) {
            toast.error(`恢复实例被拒绝:${restored.errors[0]?.message ?? "未知错误"}`);
            return;
          }
          if (removedAlias && aliasSource.source) {
            await aliasSource.save(
              { aliases: { ...aliasSource.source.aliases, [id]: removedAlias } },
              `已恢复实例 ${id}`,
              { silent: true },
            );
          }
          toast.success(`已恢复实例 ${id}`);
          void profilesQuery.refetch();
        },
      });
    } else {
      toast.success(`已移除实例 ${id}`);
    }
  };

  /** 一个实例可以挂多条启动命令：按命令里的配置目录变量反查，别名仍只有别名域一个写者。 */
  const launchCommandsOf = (element: EngineElement) => {
    if (!aliasSource.source) return [];
    const spec = element.spec as unknown as { base_agent?: string; config_dir?: string };
    const observed = element.observed as unknown as { config_dir?: string };
    const dir = observed.config_dir ?? spec.config_dir ?? "";
    if (!dir) return [];
    return launchCommandsForProfile(
      aliasSource.source.aliases,
      profileKindOf(spec.base_agent ?? "claude-code"),
      dir,
      (value) => parseAliasValue(value).otherEnv,
    );
  };

  /** 保存一条启动命令；改了名字即重命名，旧名那条一并去掉。 */
  const saveLaunchAlias = async (name: string, value: string, renamedFrom?: string) => {
    if (!aliasSource.source) return false;
    const aliases = { ...aliasSource.source.aliases, [name]: value };
    if (renamedFrom && renamedFrom !== name) delete aliases[renamedFrom];
    return aliasSource.save(
      { aliases },
      renamedFrom && renamedFrom !== name
        ? `已改名为 ${name}，点「同步启动命令」写入配置`
        : `已保存 ${name}，点「同步启动命令」写入配置`,
    );
  };

  const removeLaunchAlias = async (name: string) => {
    if (!aliasSource.source) return;
    const removed = aliasSource.source.aliases[name];
    const aliases = { ...aliasSource.source.aliases };
    delete aliases[name];
    const ok = await aliasSource.save({ aliases }, `已删除 ${name}`, { silent: true });
    if (!ok) return;
    setLaunchRemoving(null);
    toastWithUndo(`已删除启动命令 ${name}`, {
      description: "还没同步到终端配置；撤销会把它加回来。",
      onUndo: () => void aliasSource.save({ aliases: { ...aliases, [name]: removed } }, `已恢复 ${name}`),
    });
  };

  /** 复制一条：名字自动避让已有条目，内容照搬等着改。 */
  const duplicateLaunchAlias = (name: string, bin: string) => {
    const existing = aliasSource.source?.aliases ?? {};
    let candidate = `${name}-copy`;
    let n = 2;
    while (existing[candidate] !== undefined) {
      candidate = `${name}-copy${n}`;
      n += 1;
    }
    setAliasEditing({ bin, name: "", value: existing[name] ?? null, presetName: candidate });
  };

  return (
    <div>
      <h2 className="sr-only">助手</h2>
      {query.data.errors.length > 0 && <ErrorPanel title="agents:status 错误" errors={query.data.errors} />}
      <div className="mb-3 inline-flex rounded-xl bg-muted p-1" role="group" aria-label="选择环境类型">
        <button type="button" data-testid="default-environment-button" aria-pressed={environmentView === "default"} onClick={() => setEnvironmentView("default")} className={`rounded-lg px-4 py-2 text-sm ${environmentView === "default" ? "bg-background font-medium shadow-sm" : "text-muted-foreground"}`}>默认环境</button>
        <button type="button" data-testid="manage-instances-button" aria-pressed={environmentView === "independent"} onClick={() => setEnvironmentView("independent")} className={`rounded-lg px-4 py-2 text-sm ${environmentView === "independent" ? "bg-background font-medium shadow-sm" : "text-muted-foreground"}`}>独立环境</button>
      </div>
      {environmentView === "default" && <ConnectionWorkbench
        agents={allPrimaryElements}
        selectedAgentId={effectiveSelectedAgentId}
        busyTarget={configuringTarget}
        checkingAllAgentId={checkingAllAgentId}
        checkResults={checkResults}
        onSelectAgent={setSelectedAgentId}
        onAdd={(element) => {
          setConnectionFormError(null);
          setConnectionForm({ agent: element });
        }}
        onImport={setConnectionImportAgent}
        onSwitch={(element, connection) =>
          void runConfigure(element.id.replace(/^agents:/, ""), { use: connection })
        }
        onResetDefault={setConnectionResetting}
        aliases={aliasSource.source?.aliases ?? {}}
        aliasGlobals={{
          timezone: aliasSource.source?.timezone ?? "America/Los_Angeles",
          lang: aliasSource.source?.lang ?? "en_US.UTF-8",
        }}
        aliasBusy={aliasApply.executing}
        onAddLaunch={(element) => {
          const agentId = element.id.replace(/^agents:/, "");
          const bin = (element.spec as AgentsSpec).bin ?? (connectionKindOf(element) === "codex" ? "codex" : "claude");
          const aliases = aliasSource.source?.aliases ?? {};
          let candidate = aliases[agentId] === undefined ? agentId : `${agentId}-2`;
          let n = 3;
          while (aliases[candidate] !== undefined) {
            candidate = `${agentId}-${n}`;
            n += 1;
          }
          setAliasEditing({ bin, name: "", value: null, presetName: candidate });
        }}
        onEditLaunch={(element, name) => {
          const bin = (element.spec as AgentsSpec).bin ?? (connectionKindOf(element) === "codex" ? "codex" : "claude");
          setAliasEditing({ bin, name, value: aliasSource.source?.aliases[name] ?? null });
        }}
        onRemoveLaunch={setLaunchRemoving}
        onSyncLaunch={() =>
          void aliasApply.apply({ command: "aliases:apply", label: "同步终端快捷命令" })
        }
        onCheck={(element, connection) =>
          void runCheck(element.id.replace(/^agents:/, ""), connection)
        }
        onCheckAll={(element, connections) =>
          void runCheckAll(
            element.id.replace(/^agents:/, ""),
            connections.map((connection) => connection.name),
          )
        }
        onEdit={(element, connection) => {
          setConnectionFormError(null);
          setConnectionForm({ agent: element, initial: connection, variant: "edit" });
        }}
        onDuplicate={(element, connection) => {
          const existingNames = connectionsOf(element.spec as Record<string, unknown>).map((item) => item.name);
          setConnectionFormError(null);
          setConnectionForm({
            agent: element,
            initial: duplicateConnectionDraft(connection, existingNames),
            variant: "duplicate",
            preserveUnknownFrom: connection.name,
          });
        }}
        onExport={(element, connection) => void exportConnection(element, connection)}
        onRemove={(element, connection) => setConnectionRemoving({ agent: element, connection })}
        onMove={(element, connection, direction) => void moveConnection(element, connection, direction)}
        onRuntimeSettings={(element) => {
          setRuntimeFormError(null);
          setRuntimeFormAgent(element);
        }}
      />}
      {environmentView === "independent" && (
          <section data-testid="independent-environments" className="space-y-4 rounded-xl border bg-card p-5 shadow-sm">
        <div aria-label="选择助手" className="inline-flex rounded-xl bg-muted p-1" role="group">
          {allPrimaryElements.map((element) => {
            const id = element.id.replace(/^agents:/, "");
            const chosen = element.id === effectiveSelectedAgentId;
            return <button key={element.id} type="button" data-testid={`independent-agent-${id}`} aria-pressed={chosen} onClick={() => setSelectedAgentId(element.id)} className={`rounded-lg px-5 py-2 text-sm font-medium ${chosen ? "bg-background text-foreground shadow-sm" : "text-muted-foreground"}`}>{(element.spec as AgentsSpec).display_name ?? id}</button>;
          })}
        </div>
        <div className="flex items-center justify-between gap-3">
          <div>
            <h3 className="text-base font-semibold text-foreground">独立环境</h3>
            <p className="mt-0.5 text-xs text-muted-foreground">
              为当前 Agent 创建另一套配置目录、登录与会话；它和默认环境可以同时运行，互不干扰。
            </p>
          </div>
          <button
            type="button"
            data-testid="add-profile-button"
            disabled={profileCapableAgents.length === 0}
            onClick={() => setProfileForm("new")}
            className="shrink-0 rounded-md border border-primary/30 bg-primary/10 px-3 py-1.5 text-xs text-primary hover:bg-primary/15 disabled:opacity-50"
          >
            新建独立环境
          </button>
        </div>
        {visibleProfileElements.length === 0 ? (
          <EmptyState
            compact
            testId="profiles-empty"
            title="当前 Agent 还没有独立环境"
            description="需要第二个账号或完全独立的配置时再创建；只添加 Provider 请回到默认环境。"
          />
        ) : (
          <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
            {visibleProfileElements.map((element) => {
              const id = element.id.replace(/^profiles:/, "");
              return (
                <ProfileCard
                  key={element.id}
                  element={element}
                  bin={baseBinOf((element.spec as unknown as { base_agent?: string }).base_agent ?? id)}
                  busy={profileApply.executing}
                  onApply={() =>
                    void profileApply.apply({
                      command: "profiles:apply",
                      label: `写入实例配置 ${id}`,
                      args: ["--only", element.id],
                    })
                  }
                  onTakeover={() =>
                    void profileApply.apply({
                      command: "profiles:apply",
                      label: `接管并写入实例配置 ${id}`,
                      args: ["--only", element.id, "--allow-takeover"],
                    })
                  }
                  onEdit={() => {
                    const definition = (policyQuery.data?.ok
                      ? ((policyQuery.data.data.layers?.sqlite as Record<string, any> | null)?.agents?.definitions?.[id] ?? null)
                      : null) as Record<string, unknown> | null;
                    setProfileForm(parseProfileDefinition(id, definition ?? {}));
                  }}
                  onRemove={() => setProfileRemoving(id)}
                  onDuplicate={() => {
                    const definition = (policyQuery.data?.ok
                      ? ((policyQuery.data.data.layers?.sqlite as Record<string, any> | null)?.agents?.definitions?.[id] ?? null)
                      : null) as Record<string, unknown> | null;
                    const base = parseProfileDefinition(id, definition ?? {});
                    const existing = new Set(elements.map((e) => e.id.replace(/^agents:/, "")));
                    let nextId = `${id}-copy`;
                    let n = 2;
                    while (existing.has(nextId)) {
                      nextId = `${id}-copy${n}`;
                      n += 1;
                    }
                    setProfileDuplicating(true);
                    setProfileForm({
                      ...base,
                      id: nextId,
                      displayName: `${base.displayName} 副本`,
                      // 必须换目录：两个实例指同一套配置就不是两个实例了
                      configDir: defaultConfigDir(base.kind, nextId),
                    });
                  }}
                  onManageSkills={() => onManageSkills?.(id)}
                  onSyncLaunch={() =>
                    void aliasApply.apply({ command: "aliases:apply", label: "同步终端快捷命令" })
                  }
                  launchCommands={launchCommandsOf(element)}
                  onAddLaunch={() => {
                    const bin = baseBinOf((element.spec as unknown as { base_agent?: string }).base_agent ?? id);
                    const existing = aliasSource.source?.aliases ?? {};
                    let candidate = `${id}-2`;
                    let n = 3;
                    while (existing[candidate] !== undefined) {
                      candidate = `${id}-${n}`;
                      n += 1;
                    }
                    // 以该实例已有的第一条为底稿：配置目录等前置变量照搬，只改名字和参数
                    const template = launchCommandsOf(element)[0]?.value ?? null;
                    setAliasEditing({ bin, name: "", value: template, presetName: candidate });
                  }}
                  onEditLaunch={(name) => {
                    const bin = baseBinOf((element.spec as unknown as { base_agent?: string }).base_agent ?? id);
                    setAliasEditing({ bin, name, value: aliasSource.source?.aliases[name] ?? null });
                  }}
                  onDuplicateLaunch={(name) =>
                    duplicateLaunchAlias(
                      name,
                      baseBinOf((element.spec as unknown as { base_agent?: string }).base_agent ?? id),
                    )
                  }
                  onRemoveLaunch={(name) => setLaunchRemoving(name)}
                  onFixLaunch={(name) => {
                    const current = aliasSource.source?.aliases[name];
                    if (!current) return;
                    void saveLaunchAlias(name, fixLaunchCommandTilde(current));
                  }}
                />
              );
            })}
          </div>
        )}
          </section>
      )}

      {connectionForm && connectionKindOf(connectionForm.agent) && (
        <ConnectionFormDialog
          agentName={(connectionForm.agent.spec as AgentsSpec).display_name ?? connectionForm.agent.id.replace(/^agents:/, "")}
          kind={connectionKindOf(connectionForm.agent)!}
          existingNames={connectionsOf(connectionForm.agent.spec as Record<string, unknown>).map((item) => item.name)}
          initialConnection={connectionForm.initial}
          variant={connectionForm.variant}
          busy={connectionSaving}
          error={connectionFormError}
          onSubmit={(submission) => void saveConnection(connectionForm.agent, submission)}
          onClose={() => {
            setConnectionForm(null);
            setConnectionFormError(null);
          }}
        />
      )}
      {connectionImportAgent && connectionKindOf(connectionImportAgent) && (
        <ConnectionImportDialog
          kind={connectionKindOf(connectionImportAgent)!}
          existingNames={connectionsOf(connectionImportAgent.spec as Record<string, unknown>).map((item) => item.name)}
          busy={connectionSaving}
          onImport={(connection) => {
            setConnectionImportAgent(null);
            void saveConnection(connectionImportAgent, { connection, secretValue: "" });
          }}
          onClose={() => setConnectionImportAgent(null)}
        />
      )}
      {connectionRemoving && (
        <ConfirmModal
          title={`删除接入 ${connectionRemoving.connection.label || connectionRemoving.connection.name}`}
          description="只删除本应用保存的这套接入，不会删除凭据变量。当前正在使用的接入必须先切换到其他接入或助手默认账号。"
          risk="write-local"
          riskLabel="删除接入"
          confirmLabel="确认删除"
          executing={connectionRemovingBusy}
          onConfirm={() => void removeConnection(connectionRemoving.agent, connectionRemoving.connection)}
          onClose={() => setConnectionRemoving(null)}
        />
      )}
      {connectionResetting && (
        <ConfirmModal
          title={`应用 ${(connectionResetting.spec as AgentsSpec).display_name ?? connectionResetting.id.replace(/^agents:/, "")} 的官方 Provider`}
          description="把官方默认地址与账号配置写入助手，解决当前选择与实际配置不一致。不会删除助手登录状态、密钥变量或其他 Provider 定义；已经打开的助手需要重启后生效。"
          risk="write-local"
          riskLabel="应用官方 Provider"
          confirmLabel="确认应用"
          executing={configuringTarget === `${connectionResetting.id.replace(/^agents:/, "")}/default`}
          onConfirm={() => {
            const agentId = connectionResetting.id.replace(/^agents:/, "");
            setConnectionResetting(null);
            void runConfigure(agentId, { use: "" });
          }}
          onClose={() => setConnectionResetting(null)}
        />
      )}

      {profileForm && (
        <ProfileFormDialog
          baseAgents={profileCapableAgents}
          initial={profileForm === "new" ? undefined : profileForm}
          isEdit={profileForm !== "new" && !profileDuplicating}
          existingIds={elements.map((element) => element.id.replace(/^agents:/, ""))}
          baseBinOf={baseBinOf}
          onSubmit={(input) => void saveProfile(input)}
          onClose={() => {
            setProfileForm(null);
            setProfileDuplicating(false);
          }}
        />
      )}

      {profileRemoving && (
        <ConfirmModal
          title={`移除独立环境 ${profileRemoving}`}
          description="只停止管理这个独立环境并删掉它的启动命令；配置目录、登录状态与里面的内容都原样保留，需要的话请自行删除目录。"
          risk="write-local"
          confirmLabel="确认移除"
          executing={false}
          onConfirm={() => void removeProfile(profileRemoving)}
          onClose={() => setProfileRemoving(null)}
        />
      )}

      {aliasEditing && aliasSource.source && (
        <AliasEditorDialog
          initialName={aliasEditing.value === null ? undefined : aliasEditing.name || undefined}
          initialValue={aliasEditing.value ?? undefined}
          defaultCommand={aliasEditing.bin}
          presetName={aliasEditing.presetName}
          globals={{ timezone: aliasSource.source.timezone, lang: aliasSource.source.lang }}
          existingNames={Object.keys(aliasSource.source.aliases)}
          onSubmit={(name, value) => {
            // value===null 表示新建，此时 name 只是预填的可执行名，不算改名
            const renamedFrom = aliasEditing.value === null ? undefined : aliasEditing.name || undefined;
            void saveLaunchAlias(name, value, renamedFrom).then((ok) => {
              if (ok) setAliasEditing(null);
            });
          }}
          onClose={() => setAliasEditing(null)}
        />
      )}
      {launchRemoving && (
        <ConfirmModal
          title={`删除启动命令 ${launchRemoving}`}
          description="只删这条命令，实例的配置目录与登录状态不受影响。下次同步会把它从终端配置里本工具维护的那一段中去掉。"
          risk="write-local"
          confirmLabel="确认删除"
          executing={false}
          onConfirm={() => void removeLaunchAlias(launchRemoving)}
          onClose={() => setLaunchRemoving(null)}
        />
      )}
      {runtimeFormAgent && runtimeManagement && (
        <AgentRuntimeDialog
          key={runtimeFormAgent.id}
          agentName={(runtimeFormAgent.spec as AgentsSpec).display_name ?? runtimeFormAgent.id.replace(/^agents:/, "")}
          adapter={runtimeManagement.adapter}
          fields={runtimeManagement.runtime_fields}
          initial={runtimeSettingsOf(runtimeFormAgent.spec as Record<string, unknown>)}
          currentConfig={(runtimeFormAgent.observed as AgentsObserved).current_config}
          busy={runtimeSaving}
          error={runtimeFormError}
          onSubmit={(settings) => void saveRuntimeSettings(runtimeFormAgent, settings)}
          onClose={() => {
            setRuntimeFormAgent(null);
            setRuntimeFormError(null);
          }}
        />
      )}
      {definitionOps.modals}

      {pendingConfigure && (
        <ConfirmModal
          title={pendingConfigure.reason === "yolo" ? "完全放开助手权限" : "选择如何处理配置差异"}
          description={
            pendingConfigure.reason === "yolo"
              ? "这会关闭权限询问，并允许助手脱离工作区沙箱操作；任何入口启动它都一样。只应在可信且已经隔离的环境中使用。"
              : "下面只列出本工具负责且与软件配置不一致的项目。可以使用软件配置覆盖这些项目，也可以保留客户端当前值并导入为新的 Provider。"
          }
          risk="write-local"
          confirmLabel={pendingConfigure.reason === "yolo" ? "确认完全放开" : "使用软件配置"}
          secondaryLabel={pendingConfigure.reason === "takeover" && pendingConfigure.use !== undefined && takeoverCurrent ? "保留并纳管当前配置" : undefined}
          onSecondary={pendingConfigure.reason === "takeover" && pendingConfigure.use !== undefined && takeoverAgent && takeoverCurrent
            ? () => {
                const draft = observedConnectionDraft(takeoverKind, takeoverCurrent, connectionsOf(takeoverAgent.spec as Record<string, unknown>).map((item) => item.name));
                setPendingConfigure(null);
                setConnectionFormError(null);
                setConnectionForm({ agent: takeoverAgent, initial: draft, variant: "duplicate" });
              }
            : undefined}
          executing={configuring}
          onConfirm={() => {
            const pending = pendingConfigure;
            setPendingConfigure(null);
            void runConfigure(pending.agentId, {
              allowYolo: pending.yolo,
              allowTakeover: pending.reason === "takeover",
              use: pending.use,
            });
          }}
          onClose={() => {
            setPendingConfigure(null);
            toast.message("运行参数没写进去", { description: "定义已经存下了，助手的配置文件没动。" });
          }}
        >
          {pendingConfigure.reason === "takeover" && (
            <div className="mt-4 overflow-hidden rounded-lg border" data-testid="takeover-differences">
              {takeoverDiffs.length > 0 ? (
                <table className="w-full table-fixed text-left text-xs">
                  <thead className="bg-muted/60 text-muted-foreground"><tr><th className="w-1/4 px-3 py-2">配置项</th><th className="px-3 py-2">软件配置</th><th className="px-3 py-2">客户端当前值</th></tr></thead>
                  <tbody>{takeoverDiffs.map((item) => <tr key={item.label} className="border-t"><th className="px-3 py-2 font-medium text-foreground">{item.label}</th><td className="break-all px-3 py-2 text-primary">{item.managed}</td><td className="break-all px-3 py-2 text-warning">{item.actual}</td></tr>)}</tbody>
                </table>
              ) : <p className="px-3 py-3 text-xs text-muted-foreground">客户端运行参数与软件记录的摘要不一致；请回到“自动化与权限”审查最终写入值。</p>}
            </div>
          )}
        </ConfirmModal>
      )}
    </div>
  );
}

function displayConfigValue(value: unknown): string {
  return value === undefined || value === null || value === "" ? "使用客户端默认值" : String(value);
}

function connectionDifferences(
  kind: "claude" | "codex" | null,
  connection: ConnectionEntry | null,
  current: AgentCurrentConfig | null,
): Array<{ label: string; managed: string; actual: string }> {
  if (!current) return [];
  const expected = connection ?? emptyObservedConnection();
  const fields: Array<[string, unknown, unknown]> = [
    ["接口地址", expected.baseUrl, current.base_url],
    ["模型", expected.model, current.model],
  ];
  if (kind === "claude") fields.push(
    ["小任务模型", expected.smallFastModel, current.small_fast_model],
    ["Opus 映射", expected.aliasOpus, current.model_aliases?.opus],
    ["Sonnet 映射", expected.aliasSonnet, current.model_aliases?.sonnet],
    ["Haiku 映射", expected.aliasHaiku, current.model_aliases?.haiku],
    ["请求超时", expected.timeoutMs, current.timeout_ms],
  );
  if (connection) {
    fields.push(["凭据", `${connection.auth} · ${connection.tokenVar}`, current.credential?.configured
      ? `${current.credential.kind ?? "已配置"}${current.credential.variable ? ` · ${current.credential.variable}` : ""}`
      : "未配置"]);
  } else if (current.credential?.configured) {
    fields.push(["凭据", "使用助手默认登录", `${current.credential.kind ?? "已配置"}${current.credential.variable ? ` · ${current.credential.variable}` : ""}`]);
  }
  return fields
    .filter(([, managed, actual]) => displayConfigValue(managed) !== displayConfigValue(actual))
    .map(([label, managed, actual]) => ({ label, managed: displayConfigValue(managed), actual: displayConfigValue(actual) }));
}

function emptyObservedConnection(): ConnectionEntry {
  return { name: "", label: "", baseUrl: "", auth: "token", tokenVar: "", model: "", smallFastModel: "", aliasOpus: "", aliasSonnet: "", aliasHaiku: "", timeoutMs: "" };
}

function observedConnectionDraft(kind: "claude" | "codex" | null, current: AgentCurrentConfig, existingNames: string[]): ConnectionEntry {
  let name = "client-current";
  let index = 2;
  while (existingNames.includes(name)) name = `client-current-${index++}`;
  return {
    name,
    label: "客户端当前配置",
    note: "从客户端现有配置导入，请核对后保存",
    baseUrl: current.base_url ?? "",
    auth: current.credential?.kind === "api_key" ? "api_key" : "token",
    tokenVar: current.credential?.variable ?? "",
    model: current.model ?? "",
    smallFastModel: kind === "claude" ? current.small_fast_model ?? "" : "",
    aliasOpus: kind === "claude" ? current.model_aliases?.opus ?? "" : "",
    aliasSonnet: kind === "claude" ? current.model_aliases?.sonnet ?? "" : "",
    aliasHaiku: kind === "claude" ? current.model_aliases?.haiku ?? "" : "",
    timeoutMs: current.timeout_ms === undefined || current.timeout_ms === null ? "" : String(current.timeout_ms),
  };
}
