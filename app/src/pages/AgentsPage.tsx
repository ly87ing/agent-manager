import { useEffect, useState } from "react";
import { toast } from "sonner";
import { AliasEditorDialog } from "../components/AliasEditorDialog";
import { ProfileCard } from "../components/ProfileCards";
import { ProfileFormDialog } from "../components/ProfileFormDialog";
import { ConfirmModal } from "../components/ConfirmModal";
import {
  ConnectionFormDialog,
  type ConnectionProfileImpact,
  type ConnectionSubmission,
} from "../components/ConnectionFormDialog";
import { ConnectionWorkbench } from "../components/ConnectionWorkbench";
import { ConnectionImportDialog } from "../components/ConnectionImportDialog";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageLoading } from "../components/PageState";
import type { EngineElement, Envelope, PolicyData, StatusData } from "../lib/contract";
import { parseAliasValue } from "../lib/aliasCommand";
import {
  buildProfileDefinition,
  defaultConfigDir,
  fixLaunchCommandTilde,
  fixLaunchCommandCredentialClear,
  isThirdPartyProfile,
  launchCommandsForProfile,
  parseProfileDefinition,
  profileKindOf,
  profileLaunchValue,
  connectionToProfileInput,
  instancesUsingConnection,
  profileConnectionUpdate,
  profileConnectionReadbackMatches,
  type ProfileFormInput,
} from "../lib/profileDefinition";
import { runEngine } from "../lib/engine";
import { formatBalance, type BalanceItem } from "../lib/balance";
import { toastWithUndo } from "../lib/undo";
import {
  connectionsOf,
  duplicateConnectionDraft,
  moveAgentConnection,
  parseInitialToFormValues,
  serializeConnectionBundle,
  removeAgentConnection,
  runtimeRequiresYoloConfirmation,
  upsertAgentConnection,
  type ConnectionEntry,
} from "../lib/agentDefinitionForm";
import { agentManagementOf, connectionKindOf } from "../lib/agentManagement";
import {
  backfillNotice,
  configureArgs,
  configureBaseOf,
  configureRefusalOf,
  currentConfigOf,
  currentConnectionOf,
  liveRefusalDescription,
  type ConfigureRequest,
} from "../lib/agentCurrent";
import { runtimePatchApplied } from "../lib/agentRuntime";
import type { AgentsSpec, OfficialAuthStatus } from "../lib/observed";
import { useEngineQuery } from "../lib/queries";
import { DEFAULT_LANG, DEFAULT_TIMEZONE, useAliasSource } from "../lib/useAliasSource";
import { useDirectApply } from "../lib/useDirectApply";
import { useExecutions } from "../lib/execution";
import { seedArgs, summarizeSeed, type SeedItem, type SeedResult } from "../lib/profileSeed";
import { isOfficial, switchBackEffects } from "../lib/defaultEnvironment";
import { DEFAULT_SECRETS_PATH, secretSourceOf } from "../lib/secretSource";

/** 用定义覆盖实例实际配置前的说明：实例文件自己设了、定义没声明的凭据相关变量会被置空，点名说出来。 */
function overwriteDescription(element: EngineElement): string {
  const fields = ((element.observed as { fields?: { field: string; state: string; purpose?: string }[] }).fields ?? []);
  const blanked = fields
    .filter((field) => field.purpose === "isolation" && field.state === "differs")
    .map((field) => field.field.replace(/^env\./, ""));
  const base = "实例目录里的实际配置与实例定义不同。继续会按定义写入这几项（你自己加的其他设置保留）；想保留实际配置请改用「以实际配置更新定义」。";
  return blanked.length > 0
    ? `${base}注意：配置文件里自己设的 ${blanked.join("、")} 定义里没有，会被置空；要保留请先编辑定义把它们加进去。`
    : base;
}

/** 引擎回读的 JSON 对象键顺序可能与表单不同，按值比较完整字段。 */
function sameJson(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left) && Array.isArray(right) && left.length === right.length
      && left.every((item, index) => sameJson(item, right[index]));
  }
  if (left && right && typeof left === "object" && typeof right === "object") {
    const a = left as Record<string, unknown>;
    const b = right as Record<string, unknown>;
    return Object.keys(a).length === Object.keys(b).length
      && Object.keys(a).every((key) => Object.prototype.hasOwnProperty.call(b, key) && sameJson(a[key], b[key]));
  }
  return false;
}

export default function AgentsPage({
  onManageSkills,
  onOpenSecrets,
  profileTarget,
}: {
  onManageSkills?: (agentId: string) => void;
  onOpenSecrets?: () => void;
  profileTarget?: { agentId: string; profileId: string } | null;
} = {}) {
  const query = useEngineQuery<StatusData>("agents:status");
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const definitionOps = useDefinitionOps({ section: "agents", domain: "agents" });
  // 实例启动命令与「终端快捷命令」页共用同一份实际配置（rc 文件受管区块）。
  const aliasSource = useAliasSource();
  // 实例（同一助手的多套独立配置）：定义与 agents 共用，状态与写入走 profiles 域
  const profilesQuery = useEngineQuery<StatusData>("profiles:status");
  // 只读：API Key 写到哪、变量有没有值，给接入表单说明用
  const secretsQuery = useEngineQuery<StatusData>("secrets:status");
  const profileApply = useDirectApply();
  const { execute } = useExecutions();
  const [profileForm, setProfileForm] = useState<ProfileFormInput | "new" | null>(null);
  /** 复制实例：表单带着预填内容，但名称与配置目录仍要能改（否则两个实例指同一套配置）。 */
  const [profileDuplicating, setProfileDuplicating] = useState(false);
  const [profileRemoving, setProfileRemoving] = useState<string | null>(null);
  // 实际配置与实例定义不同时，用定义覆盖实际配置前先确认
  const [profileOverwriting, setProfileOverwriting] = useState<EngineElement | null>(null);
  /** 日常接入工作台当前查看的主助手；null 时跟随列表里的第一个。 */
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [connectionForm, setConnectionForm] = useState<{
    agent: EngineElement;
    initial?: ConnectionEntry;
    variant?: "edit" | "duplicate";
    preserveUnknownFrom?: string;
    /** 从「新建环境」进来：保存接入后接着建成独立环境是主路径。 */
    intent?: "environment";
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
  /** 「新建环境 → 另一个官方账号」时预先选中的助手；null 按列表第一个。 */
  const [profileNewAgent, setProfileNewAgent] = useState<string | null>(null);

  const [aliasEditing, setAliasEditing] = useState<
    { bin: string; name: string; value: string | null; presetName?: string } | null
  >(null);
  const [launchRemoving, setLaunchRemoving] = useState<string | null>(null);
  const [runtimeSaving, setRuntimeSaving] = useState(false);
  /** 高风险运行模式先确认，再写入客户端；取消必须保持零写入。 */
  const [pendingRuntimeSave, setPendingRuntimeSave] = useState<{
    agent: EngineElement;
    patch: Record<string, unknown>;
  } | null>(null);
  /**
   * 切换前的回填被拒：实际令牌还没保存，或实际配置无法完整保存。由用户显式选择出路，
   * 确认后带对应的放弃标志重试；取消则文件与库都不动。
   */
  const [pendingRefusal, setPendingRefusal] = useState<{
    kind: "live-token" | "unrepresentable";
    message: string;
    request: Omit<ConfigureRequest, "base">;
    success?: ConfigureSuccess;
  } | null>(null);
  /** 正在写入哪一个助手/接入；非空时禁止并发切换。 */
  const [configuringTarget, setConfiguringTarget] = useState<string | null>(null);
  useEffect(() => {
    if (profileTarget) setSelectedAgentId(`agents:${profileTarget.agentId}`);
  }, [profileTarget]);
  useEffect(() => {
    if (!profileTarget || selectedAgentId !== `agents:${profileTarget.agentId}` || !profilesQuery.data?.ok) return;
    const timer = window.setTimeout(() => {
      const card = document.getElementById(`profile-card-${profileTarget.profileId}`);
      card?.scrollIntoView?.({ behavior: "smooth", block: "center" });
      card?.focus({ preventScroll: true });
    }, 0);
    return () => window.clearTimeout(timer);
  }, [profileTarget, selectedAgentId, profilesQuery.data]);
  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="助手状态读取失败" error={query.error} />;

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
  const elementOf = (agentId: string) => elements.find((element) => element.id === `agents:${agentId}`);
  const baseBinOf = (agentId: string) =>
    (elements.find((element) => element.id === `agents:${agentId}`)?.spec as AgentsSpec | undefined)?.bin ??
    (agentId === "codex" ? "codex" : "claude");

  /**
   * 写助手自己的配置文件：切换接入、编辑当前接入、改运行参数或保存当前实际配置。
   *
   * 每次都带 status 给出的写入基准：本工具以外改过就被引擎拒绝，界面刷新后让用户重来。
   * 降安全等级（--allow-yolo）与放弃实际配置（--discard-live / --discard-live-token）都要用户点头；
   * 引擎拒绝时文件与库一个字节都不会动。成功后以 status 回读确认，不靠本地乐观改状态。
   */
  const runConfigure = async (
    request: Omit<ConfigureRequest, "base">,
    success: ConfigureSuccess = {},
  ): Promise<boolean> => {
    const agentId = request.agentId;
    const element = elementOf(agentId);
    const base = element ? configureBaseOf(element) : null;
    if (!base) {
      toast.error("客户端配置文件暂不可读，没有写入", {
        description: "到“运行设置”底部直接编辑配置文件查看原因；修好之后再试。",
      });
      return false;
    }
    const target = request.saveCurrent
      ? `${agentId}/save-current`
      : request.useDefault
        ? `${agentId}/default`
        : `${agentId}/settings`;
    setConfiguringTarget(target);
    try {
      const envelope: Envelope = await runEngine("agents:configure", configureArgs({ ...request, base }));
      if (envelope.ok) {
        const refreshed = await query.refetch();
        const refreshedAgent = refreshed.data?.data.elements.find(
          (candidate) => candidate.id === `agents:${agentId}`,
        );
        const current = refreshedAgent ? currentConnectionOf(refreshedAgent) : null;
        const management = refreshedAgent ? agentManagementOf(refreshedAgent) : null;
        const verified = request.runtime
          ? Boolean(management && refreshedAgent && runtimePatchApplied(
            management.runtime_fields,
            request.runtime,
            currentConfigOf(refreshedAgent),
          ))
          : request.useDefault
            ? isOfficial(current)
            : Boolean(current?.profile);
        if (!verified) {
          toast.error("写入已经返回成功，但状态回读没有确认生效", {
            description: "请到“运行设置”查看客户端当前的值后再决定是否重试。",
          });
          return false;
        }
        const notice = backfillNotice(envelope.data);
        toast.success(success.message ?? `已写入 ${agentId} 的配置`, {
          description: notice
            ? `${notice}。助手下次启动生效。`
            : "助手下次启动生效；已经开着的要重启它。",
        });
        return true;
      }
      const message = envelope.errors[0]?.message ?? "未知错误";
      const refusal = configureRefusalOf(envelope);
      if (refusal === "changed") {
        await query.refetch();
        toast.error("配置文件已在外部修改，没有写入", {
          description: "已刷新为客户端当前的实际配置，请确认后重新操作。",
        });
        return false;
      }
      if (refusal === "live-token" || refusal === "unrepresentable") {
        setPendingRefusal({ kind: refusal, message, request, success });
        return false;
      }
      toast.error(`没有写入：${message}`, {
        description: "助手的配置文件与本工具保存的内容都没动。",
      });
      return false;
    } catch (err) {
      toast.error(`没有写入：${String(err)}`);
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
  /** 实例写入基准：status 给出的实际内容摘要（文件不存在为 missing）。 */
  const profileBaseOf = (element: EngineElement): string =>
    ((element.observed as unknown as { base?: string | null }).base ?? "missing");

  const writeProfile = (element: EngineElement) =>
    profileApply.apply({
      command: "profiles:apply",
      label: `写入实例配置 ${element.id.replace(/^profiles:/, "")}`,
      args: ["--only", element.id, "--base", profileBaseOf(element)],
    });

  const runCheckAll = async (agentId: string, profiles: string[]): Promise<void> => {
    if (checkingAllAgentId || profiles.length === 0) return;
    setCheckingAllAgentId(agentId);
    try {
      await Promise.all(profiles.map((profile) => runCheck(agentId, profile)));
    } finally {
      setCheckingAllAgentId(null);
    }
  };

  const affectedProfilesForConnection = (element: EngineElement, connection: ConnectionEntry): ConnectionProfileImpact[] => {
    const agentId = element.id.replace(/^agents:/, "");
    const definitions = ((policyQuery.data?.ok
      ? (policyQuery.data.data.effective as Record<string, any> | undefined)?.agents?.definitions
      : undefined) ?? {}) as Record<string, unknown>;
    return instancesUsingConnection(definitions, agentId, connection).flatMap(({ id, label }) => {
      const definition = definitions[id];
      const status = profilesQuery.data?.ok
        ? profilesQuery.data.data.elements.find((item) => item.id === `profiles:${id}`)
        : undefined;
      if (!definition || typeof definition !== "object") return [];
      const observed = status?.observed as { base?: string | null; definition_base?: string | null } | undefined;
      const source = ((definition as Record<string, any>).profile as Record<string, unknown> | undefined)?.source_connection;
      const matches = source ? 1 : parseInitialToFormValues(agentId, element.spec as Record<string, unknown>).connections
        .filter((candidate) => instancesUsingConnection({ [id]: definition }, agentId, candidate).length > 0).length;
      return [{
        id, label, definition: definition as Record<string, unknown>,
        state: status?.state ?? "unsupported",
        base: observed?.base ?? null,
        definitionBase: observed?.definition_base ?? null,
        blockedReason: matches > 1 ? "旧环境同时匹配多个 Provider，不能自动判断来源；请先在环境卡片核对。" : undefined,
      }];
    });
  };

  /** 创建/编辑 Provider 只保存定义与可选凭据；应用是卡片上的独立操作。 */
  const saveConnection = async (element: EngineElement, submission: ConnectionSubmission) => {
    if (connectionSaving) return;
    const agentId = element.id.replace(/^agents:/, "");
    setConnectionSaving(true);
    setConnectionFormError(null);
    try {
      const selectedIds = submission.profileUpdates ?? [];
      const previous = connectionForm?.variant === "edit" ? connectionForm.initial : undefined;
      const kind = connectionKindOf(element);
      const impacts = previous ? affectedProfilesForConnection(element, previous) : [];
      const updates = selectedIds.map((id) => {
        const impact = impacts.find((item) => item.id === id);
        const update = impact && previous && kind
          ? profileConnectionUpdate(id, impact.definition, previous, submission.connection, kind,
            submission.profileOverrides?.[id] ?? [])
          : null;
        return impact && update ? { impact, update } : null;
      });
      if (updates.some((item) => !item || item.impact.blockedReason || !item.impact.base || !item.impact.definitionBase
        || (item.impact.state !== "managed-ok" && item.impact.state !== "not-materialized"))) {
        setConnectionFormError("受影响环境的定义或写入基准已变化，请刷新后重新选择");
        return;
      }
      let sourceElement = element;
      if (updates.length > 0) {
        const [agentsNow, profilesNow] = await Promise.all([
          runEngine<StatusData>("agents:status", []),
          runEngine<StatusData>("profiles:status", []),
        ]);
        const currentElement = agentsNow.ok
          ? agentsNow.data.elements.find((item) => item.id === element.id)
          : undefined;
        sourceElement = currentElement ?? element;
        const fresh = Boolean(currentElement) && agentsNow.ok && profilesNow.ok
          && sameJson(sourceElement.spec, element.spec)
          && updates.every((entry) => {
            if (!entry) return false;
            const now = profilesNow.data.elements.find((item) => item.id === `profiles:${entry.impact.id}`);
            const observed = now?.observed as { base?: string | null; definition_base?: string | null } | undefined;
            return now?.state === entry.impact.state
              && observed?.base === entry.impact.base
              && observed?.definition_base === entry.impact.definitionBase;
          });
        if (!fresh) {
          void query.refetch();
          void profilesQuery.refetch();
          setConnectionFormError("Provider 或环境在表单打开后发生变化，未保存；请刷新后重新选择");
          return;
        }
      }
      const definition = upsertAgentConnection(
        sourceElement.spec as Record<string, unknown>,
        submission.connection,
        connectionForm?.preserveUnknownFrom,
      );
      // 主配置只放官方账号：编辑接入只保存定义，不写助手配置（包括主配置里旧版本留下的那套）
      const saved = await definitionOps.saveDefinition({ name: agentId, definition, isNew: false });
      if (!saved) {
        setConnectionFormError("接入没有保存，请根据提示修正后重试");
        return;
      }

      if (submission.secretValue) {
        const source = secretSourceOf(secretsQuery.data?.ok ? secretsQuery.data.data : null);
        if (source && !source.configured) {
          // 第一次填 API Key 还没有密钥文件：在默认位置新建一个（仅本人可读），不用先去建文件、改权限
          const created = await runEngine("secrets:source", ["--path", DEFAULT_SECRETS_PATH, "--create"]);
          if (!created.ok) {
            await query.refetch();
            setConnectionFormError(
              `接入已保存，但没能新建密钥文件 ${DEFAULT_SECRETS_PATH}：${created.errors[0]?.message ?? "未知原因"}。可以到“设置 → 密钥”选一个已有的文件。`,
            );
            return;
          }
          toast.success(`已新建密钥文件 ${DEFAULT_SECRETS_PATH}`, {
            description: "只有你能读；API Key 只存在这里，不进本应用的数据库。",
          });
        }
        const secret = await runEngine("secrets:set", [
          "--var",
          submission.connection.tokenVar,
          "--value",
          submission.secretValue,
        ]);
        if (!secret.ok) {
          await query.refetch();
          setConnectionFormError(
            `接入已保存，但凭据没有写入外部文件：${secret.errors[0]?.message ?? "请先在“设置 → 密钥”选择外部文件，或从进程环境提供这个变量"}`,
          );
          return;
        }
        void secretsQuery.refetch();
      }

      if (updates.length > 0) {
        let applied = 0;
        const failed: string[] = [];
        for (const entry of updates) {
          if (!entry) continue;
          const result = await execute({
            command: "profiles:apply-definition",
            label: `更新独立环境 ${entry.impact.id}`,
            args: [
              "--only", `profiles:${entry.impact.id}`,
              "--base", entry.impact.base!,
              "--definition-base", entry.impact.definitionBase!,
              "--candidate", JSON.stringify(entry.update.candidate),
            ],
            silent: true,
          });
          if (!result?.ok) {
            failed.push(entry.impact.id);
            continue;
          }
          try {
            const [status, policy] = await Promise.all([
              runEngine<StatusData>("profiles:status", []),
              runEngine<PolicyData>("policy:get", []),
            ]);
            const now = status.ok
              ? status.data.elements.find((item) => item.id === `profiles:${entry.impact.id}`)
              : undefined;
            const savedProfile = policy.ok
              ? ((policy.data.effective as Record<string, any>)?.agents?.definitions?.[entry.impact.id]?.profile as Record<string, unknown> | undefined)
              : undefined;
            const keys = kind === "codex"
              ? ["base_url", "env_key", "wire_api", "model", "source_connection"]
              : ["env", "model", "source_connection"];
            const saved = savedProfile && keys.every((key) =>
              sameJson(savedProfile[key], entry.update.candidate[key]));
            const observed = now?.observed as { fields?: { field: string; state: string; definition: string | null }[] } | undefined;
            if (now?.state === "managed-ok"
              && observed?.fields
              && kind
              && profileConnectionReadbackMatches(kind, entry.update.candidate, observed.fields)
              && saved) {
              applied += 1;
            } else {
              failed.push(`${entry.impact.id}（写入后回读未确认）`);
            }
          } catch {
            failed.push(`${entry.impact.id}（写入后回读失败）`);
          }
        }
        void profilesQuery.refetch();
        if (failed.length > 0) {
          toast.warning(`Provider 已保存，${applied} 个环境已更新，${failed.length} 个未确认`, {
            description: `${failed.join("、")}。请到对应环境卡片查看实际配置后再操作。`,
          });
        } else {
          toast.success(`Provider 已保存，${applied} 个独立环境已更新并回读`);
        }
      }
      setConnectionForm(null);
      await query.refetch();
      if (submission.thenMakeProfile) makeProfileFromConnection(element, submission.connection);
    } catch (err) {
      setConnectionFormError(`保存接入失败:${String(err)}`);
    } finally {
      setConnectionSaving(false);
    }
  };

  /**
   * 用表单里的地址与凭据变量问接口能用哪些模型（接入与独立环境表单共用）。只在点按钮时才联网、
   * 只读；凭据取值由引擎从密钥来源解析，不经界面传递。失败时返回原因。
   */
  const fetchModelsForDraft = async ({ kind, baseUrl, tokenVar }: { kind: "claude" | "codex"; baseUrl: string; tokenVar: string }) => {
    try {
      const envelope = await runEngine<{ models?: string[] }>("agents:models", [
        "--draft",
        JSON.stringify({ kind, base_url: baseUrl, token_var: tokenVar }),
      ]);
      return envelope.ok ? envelope.data?.models ?? [] : envelope.errors[0]?.message ?? "没拿到模型列表";
    } catch (err) {
      return String(err);
    }
  };

  /** 查余额（接入卡片与独立环境卡片共用）：只在点按钮时联网、只读；返回一行结果或失败原因。 */
  const fetchBalance = async (args: string[]): Promise<string> => {
    try {
      const envelope = await runEngine<{ items?: BalanceItem[] }>("agents:balance", args);
      return envelope.ok ? formatBalance(envelope.data?.items ?? []) : envelope.errors[0]?.message ?? "没查到余额";
    } catch (err) {
      return String(err);
    }
  };

  /** 删除非当前接入：只删定义里的目标 profile，不清理可能被其他接入复用的凭据变量。 */
  const removeConnection = async (element: EngineElement, connection: ConnectionEntry) => {
    if (connectionRemovingBusy) return;
    const definition = element.spec as Record<string, unknown>;
    if (currentConnectionOf(element)?.profile === connection.name) {
      toast.error("主配置正在使用这套接入，不能删除；请先切回官方账号");
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
        description: "凭据变量没有删除；仍在使用时不会受影响，可在“设置 → 密钥”单独处理。",
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

  /**
   * 把接入建成独立环境：打开预填好的新建表单，保存后照常写定义、配置与启动命令。凭据仍引用同一个
   * 变量名；第三方凭据只进新环境的目录，主配置留给官方账号。
   */
  const makeProfileFromConnection = (element: EngineElement, connection: ConnectionEntry) => {
    const kind = connectionKindOf(element);
    if (!kind) return;
    const made = connectionToProfileInput(
      { id: element.id.replace(/^agents:/, ""), kind },
      connection,
      elements.map((item) => item.id.replace(/^agents:/, "")),
    );
    if (!made) {
      toast.error("这套接入引用主配置 config.toml 里的接入方式，没法搬进独立目录");
      return;
    }
    if (made.dropped.length > 0) {
      toast.warning(`独立环境不支持：${made.dropped.join("、")}，没有带过去`);
    }
    setProfileDuplicating(true);
    setProfileForm(made.input);
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

  /** 运行参数只发改动过的字段；完全放开先确认，取消时文件与库都不动。 */
  const saveRuntimeSettings = (element: EngineElement, patch: Record<string, unknown>) => {
    if (runtimeSaving) return;
    const management = agentManagementOf(element);
    if (!management) {
      toast.error("当前助手没有可用的运行设置适配器");
      return;
    }
    if (runtimeRequiresYoloConfirmation(patch, management.runtime_fields)) {
      setPendingRuntimeSave({ agent: element, patch });
      return;
    }
    void persistRuntimeSettings(element, patch);
  };

  const persistRuntimeSettings = async (
    element: EngineElement,
    patch: Record<string, unknown>,
    allowYolo = false,
  ) => {
    const agentId = element.id.replace(/^agents:/, "");
    setRuntimeSaving(true);
    try {
      await runConfigure(
        { agentId, runtime: patch, allowYolo },
        { message: `已写入 ${agentId} 的运行参数` },
      );
    } finally {
      setRuntimeSaving(false);
    }
  };

  /**
   * 新环境的目标文件还不存在时先写好配置，再写启动命令：
   * - 第三方环境只有启动命令而目录空着，一运行助手就会引导登录官方账号，之后第三方凭据就写不进
   *   这个目录了；
   * - Claude 官方账号环境要先写好置空的凭据变量，否则第一次启动就会被 shell 里给别的账号或中转
   *   导出的变量压过这个目录里的登录。
   * 目标文件已存在时不自动覆盖，交给卡片上的逐字段对比由用户选择。返回 false 表示应该写却没写成。
   */
  const writeNewProfileConfig = async (id: string): Promise<boolean> => {
    let element: EngineElement | undefined;
    try {
      const status = await runEngine<StatusData>("profiles:status", []);
      element = status.ok ? status.data.elements.find((item) => item.id === `profiles:${id}`) : undefined;
    } catch {
      // 查不到状态就不知道目标文件在不在：按没写成处理，由调用方提示并先不写启动命令
      return false;
    }
    if (!element || profileBaseOf(element) !== "missing") return true;
    const applied = await profileApply.apply({
      command: "profiles:apply",
      label: `写入独立环境 ${id} 的配置`,
      args: ["--only", element.id, "--base", "missing"],
    });
    return applied?.ok === true;
  };

  /**
   * 从默认环境把勾选的设置带进独立环境（只补环境里没有的，凭据不带）。返回逐项结果或失败原因；
   * 执行记录照常留在任务记录里。
   */
  const seedProfile = async (id: string, items: SeedItem[]): Promise<SeedResult[] | string> => {
    const envelope = await execute({
      command: "profiles:seed",
      args: seedArgs(id, items),
      label: `从默认环境带设置到 ${id}`,
      silent: true,
    });
    if (!envelope) return "调用失败，详见任务记录";
    if (!envelope.ok) return envelope.errors[0]?.message ?? "没有带上，详见任务记录";
    return (envelope.data as { results?: SeedResult[] } | null)?.results ?? [];
  };

  const toastSeed = (id: string, outcome: SeedResult[] | string) => {
    if (typeof outcome === "string") {
      toast.error(`没有从默认环境带上设置到 ${id}：${outcome}`, {
        description: "可以稍后在环境卡片上点「从默认环境带上」再试。",
      });
      return;
    }
    const summary = summarizeSeed(outcome);
    (summary.failed ? toast.warning : toast.success)(summary.title, {
      description: summary.description || undefined,
    });
  };

  /**
   * 保存实例：写定义、（第三方环境与 Claude 环境）写好配置，再把启动命令存成同名快捷命令；新建时
   * 最后按勾选从默认环境带上设置。
   */
  const saveProfile = async (input: ProfileFormInput, seedItems: SeedItem[] = []) => {
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
    if ((isThirdPartyProfile(input) || input.kind === "claude") && !(await writeNewProfileConfig(input.id))) {
      toast.error(`已保存独立环境 ${input.id}，但配置没有写入，启动命令也先不写`, {
        description: "原因见任务记录；处理后在实例卡片上写入配置，再新增启动命令。",
      });
      setProfileForm(null);
      setProfileDuplicating(false);
      void profilesQuery.refetch();
      return;
    }
    if (aliasSource.source) {
      const ok = await aliasSource.setAlias(input.id, profileLaunchValue(input, baseBinOf(input.baseAgent)), {
        label: `写入独立环境 ${input.id} 的启动命令`,
        silent: true,
      });
      if (ok) {
        toast.success(`已保存独立环境 ${input.id} 与它的启动命令`, { description: "新开一个终端窗口后生效。" });
      } else {
        toast.error(`已保存独立环境 ${input.id}，但启动命令没有写入终端配置`, {
          description: "原因见任务记录；可以在实例卡片上重新新增启动命令。",
        });
      }
    } else {
      toast.success(`已保存独立环境 ${input.id}`);
    }
    setProfileForm(null);
    setProfileDuplicating(false);
    if (seedItems.length > 0) toastSeed(input.id, await seedProfile(input.id, seedItems));
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
      await aliasSource.removeAlias(id, { label: `移除实例 ${id} 的启动命令`, silent: true });
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
          if (removedAlias) {
            await aliasSource.setAlias(id, removedAlias, { label: `恢复实例 ${id} 的启动命令`, silent: true });
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

  /** 保存一条启动命令（直接写进终端配置）；改了名字即重命名，旧名那条一并去掉。 */
  const saveLaunchAlias = (name: string, value: string, renamedFrom?: string) =>
    aliasSource.setAlias(name, value, { renameFrom: renamedFrom });

  /** 删除一条启动命令：已确认过，直接从终端配置里去掉（写进文件的动作不给撤销）。 */
  const removeLaunchAlias = async (name: string) => {
    const ok = await aliasSource.removeAlias(name, { label: `删除启动命令 ${name}` });
    if (ok) setLaunchRemoving(null);
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
      {query.data.errors.length > 0 && <ErrorPanel title="助手状态读取失败" errors={query.data.errors} />}
      <ConnectionWorkbench
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
        onResetDefault={setConnectionResetting}
        aliases={aliasSource.source?.aliases ?? {}}
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
        onMakeProfile={(element, connection) => makeProfileFromConnection(element, connection)}
        instancesOf={(agentId, connection) =>
          instancesUsingConnection(
            ((policyQuery.data?.ok ? (policyQuery.data.data.effective as Record<string, any> | undefined)?.agents?.definitions : undefined) ??
              {}) as Record<string, unknown>,
            agentId,
            connection,
          )
        }
        onShowInstances={() =>
          document.querySelector('[data-testid="independent-environments"]')?.scrollIntoView?.({ behavior: "smooth", block: "start" })
        }
        onNewEnvironment={(element, kind) => {
          if (kind === "third-party") {
            setConnectionFormError(null);
            setConnectionForm({ agent: element, intent: "environment" });
          } else {
            setProfileNewAgent(element.id.replace(/^agents:/, ""));
            setProfileDuplicating(false);
            setProfileForm("new");
          }
        }}
        onBalance={(element, connection) => fetchBalance(["--only", element.id, "--use", connection.name])}
        onExport={(element, connection) => void exportConnection(element, connection)}
        onRemove={(element, connection) => setConnectionRemoving({ agent: element, connection })}
        onMove={(element, connection, direction) => void moveConnection(element, connection, direction)}
        runtimeSaving={runtimeSaving}
        onSaveRuntime={(element, patch) => saveRuntimeSettings(element, patch)}
        onSaveCurrent={(element) =>
          void runConfigure(
            { agentId: element.id.replace(/^agents:/, ""), saveCurrent: true },
            { message: "已把当前实际配置保存为接入配置" },
          )
        }
        onFixStatusline={(element) =>
          void runConfigure(
            { agentId: element.id.replace(/^agents:/, ""), runtime: { statusline: "builtin" } },
            { message: "已修复状态栏" },
          )
        }
        onClearEnvOverrides={(element) =>
          void runConfigure(
            { agentId: element.id.replace(/^agents:/, ""), runtime: { claude_clear_env_overrides: true } },
            { message: "已清理 env 里的单次会话覆盖变量" },
          )
        }
        onCheckAuth={async (element) => {
          try {
            const envelope = await runEngine<OfficialAuthStatus>("agents:auth-status", ["--only", element.id]);
            return envelope.ok ? envelope.data : envelope.errors[0]?.message ?? "未知错误";
          } catch (err) {
            return String(err);
          }
        }}
        renderInstances={() => (
          <section data-testid="independent-environments" className="space-y-4 rounded-xl border bg-card p-5 shadow-sm">
        <div className="flex items-center justify-between gap-3">
          <div>
            <h3 className="text-base font-semibold text-foreground">
              独立环境{visibleProfileElements.length > 0 ? `（${visibleProfileElements.length}）` : ""}
            </h3>
            <p className="mt-0.5 text-xs text-muted-foreground">
              各有自己的配置目录、登录与会话，和默认环境可以同时开着；第三方厂商只在这里用，也可以在这里登录另一个官方账号。
            </p>
          </div>
          <button
            type="button"
            data-testid="add-profile-button"
            disabled={profileCapableAgents.length === 0}
            onClick={() => {
              setProfileNewAgent(effectiveAgentName);
              setProfileForm("new");
            }}
            className="shrink-0 rounded-md border border-primary/30 bg-primary/10 px-3 py-1.5 text-xs text-primary hover:bg-primary/15 disabled:opacity-50"
          >
            新建独立环境
          </button>
        </div>
        {visibleProfileElements.length === 0 ? (
          <EmptyState
            compact
            testId="profiles-empty"
            title="还没有独立环境"
            description="用右上角「新建环境」：选第三方厂商会建成一个用它的独立环境；选另一个官方账号会建一个单独登录的目录。下方保存过的第三方接入也可以一键建成。"
          />
        ) : (
          <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
            {visibleProfileElements.map((element) => {
              const id = element.id.replace(/^profiles:/, "");
              return (
                <ProfileCard
                  key={element.id}
                  element={element}
                  onCheckAuth={async () => {
                    try {
                      const envelope = await runEngine<OfficialAuthStatus>("agents:auth-status", ["--only", element.id]);
                      return envelope.ok ? envelope.data : envelope.errors[0]?.message ?? "未知错误";
                    } catch (err) {
                      return String(err);
                    }
                  }}
                  onCheckBalance={() => fetchBalance(["--only", element.id])}
                  onSeed={(items) => seedProfile(id, items)}
                  onCheckEndpoint={async () => {
                    try {
                      const envelope = await runEngine<{ ok?: boolean; detail?: string; latency_ms?: number }>(
                        "agents:check",
                        ["--only", element.id],
                      );
                      return {
                        ok: envelope.ok === true && envelope.data?.ok === true,
                        detail: envelope.ok ? envelope.data?.detail ?? "测完了" : envelope.errors[0]?.message ?? "没测成",
                        latencyMs: envelope.data?.latency_ms ?? 0,
                      };
                    } catch (err) {
                      return { ok: false, detail: String(err), latencyMs: 0 };
                    }
                  }}
                  bin={baseBinOf((element.spec as unknown as { base_agent?: string }).base_agent ?? id)}
                  busy={profileApply.executing}
                  onApply={() => {
                    if (element.state === "drifted") {
                      setProfileOverwriting(element);
                      return;
                    }
                    void writeProfile(element);
                  }}
                  onWriteCredential={() => void writeProfile(element)}
                  onAdopt={() =>
                    void profileApply.apply({
                      command: "profiles:adopt",
                      label: `以实际配置更新实例定义 ${id}`,
                      args: ["--only", element.id, "--base", profileBaseOf(element)],
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
                    const command = launchCommandsOf(element).find((item) => item.name === name);
                    if (!command) return;
                    let fixed = command.needsTildeFix ? fixLaunchCommandTilde(command.value) : command.value;
                    if (command.needsCredentialClear) fixed = fixLaunchCommandCredentialClear(fixed);
                    void saveLaunchAlias(name, fixed);
                  }}
                />
              );
            })}
          </div>
        )}
          </section>
        )}
      />

      {connectionForm && connectionKindOf(connectionForm.agent) && (
        <ConnectionFormDialog
          agentName={(connectionForm.agent.spec as AgentsSpec).display_name ?? connectionForm.agent.id.replace(/^agents:/, "")}
          kind={connectionKindOf(connectionForm.agent)!}
          existingNames={connectionsOf(connectionForm.agent.spec as Record<string, unknown>).map((item) => item.name)}
          initialConnection={connectionForm.initial}
          affectedProfiles={connectionForm.variant === "edit" && connectionForm.initial
            ? affectedProfilesForConnection(connectionForm.agent, connectionForm.initial)
            : []}
          variant={connectionForm.variant}
          intent={connectionForm.intent}
          secretSource={secretSourceOf(secretsQuery.data?.ok ? secretsQuery.data.data : null)}
          busy={connectionSaving}
          error={connectionFormError}
          onOpenSecrets={onOpenSecrets}
          onFetchModels={fetchModelsForDraft}
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
          description="只删除本应用保存的这套接入，不会删除凭据变量。主配置正在使用的那套要先切回官方账号。"
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
          title={`把 ${(connectionResetting.spec as AgentsSpec).display_name ?? connectionResetting.id.replace(/^agents:/, "")} 的主配置切回官方账号`}
          description="主配置改回只用官方账号。改动范围如下："
          risk="write-local"
          riskLabel="切回官方账号"
          confirmLabel="确认切回"
          executing={configuringTarget === `${connectionResetting.id.replace(/^agents:/, "")}/default`}
          onConfirm={() => {
            const agentId = connectionResetting.id.replace(/^agents:/, "");
            setConnectionResetting(null);
            void runConfigure({ agentId, useDefault: true }, { message: `已把 ${agentId} 的主配置切回官方账号` });
          }}
          onClose={() => setConnectionResetting(null)}
        >
          {(() => {
            const effects = switchBackEffects(connectionKindOf(connectionResetting) ?? "claude");
            return (
              <dl data-testid="switch-back-effects" className="space-y-2 text-sm">
                <div>
                  <dt className="font-medium text-foreground">会撤掉</dt>
                  <dd className="text-muted-foreground">{effects.removes}</dd>
                </div>
                <div>
                  <dt className="font-medium text-foreground">切之前</dt>
                  <dd className="text-muted-foreground">{effects.before}</dd>
                </div>
                <div>
                  <dt className="font-medium text-foreground">不会动</dt>
                  <dd className="text-muted-foreground">{effects.keeps.join("；")}</dd>
                </div>
                <div>
                  <dt className="font-medium text-foreground">之后</dt>
                  <dd className="text-muted-foreground">{effects.after}</dd>
                </div>
              </dl>
            );
          })()}
        </ConfirmModal>
      )}

      {profileForm && (
        <ProfileFormDialog
          baseAgents={
            profileForm === "new" && profileNewAgent
              ? [...profileCapableAgents].sort((a, b) => Number(b.id === profileNewAgent) - Number(a.id === profileNewAgent))
              : profileCapableAgents
          }
          initial={profileForm === "new" ? undefined : profileForm}
          isEdit={profileForm !== "new" && !profileDuplicating}
          existingIds={elements.map((element) => element.id.replace(/^agents:/, ""))}
          baseBinOf={baseBinOf}
          onFetchModels={fetchModelsForDraft}
          secretSource={secretSourceOf(secretsQuery.data?.ok ? secretsQuery.data.data : null)}
          onSubmit={(input, options) => void saveProfile(input, options.seedItems)}
          onClose={() => {
            setProfileForm(null);
            setProfileDuplicating(false);
          }}
        />
      )}

      {profileOverwriting && (
        <ConfirmModal
          title={`用定义覆盖 ${profileOverwriting.id.replace(/^profiles:/, "")} 的实际配置`}
          description={overwriteDescription(profileOverwriting)}
          risk="write-local"
          confirmLabel="确认覆盖"
          executing={profileApply.executing}
          onConfirm={() => {
            const target = profileOverwriting;
            setProfileOverwriting(null);
            void writeProfile(target);
          }}
          onClose={() => setProfileOverwriting(null)}
        />
      )}

      {profileRemoving && (
        <ConfirmModal
          title={`移除独立环境 ${profileRemoving}`}
          description="只停止管理这个独立环境并删掉它的启动命令；配置目录、登录状态与里面的内容都原样保留。官方登录按目录保存（macOS 在钥匙串里按目录路径区分），不再使用时请先用实例卡片上的退出命令在终端退出，再自行删除目录。"
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
          defaults={{ timezone: DEFAULT_TIMEZONE, lang: DEFAULT_LANG }}
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
          description="只删这条命令，实例的配置目录与登录状态不受影响。确认后直接从 ~/.zshrc 与 ~/.zprofile 里本工具维护的那一段中去掉。"
          risk="write-local"
          confirmLabel="确认删除"
          executing={aliasSource.busy}
          onConfirm={() => void removeLaunchAlias(launchRemoving)}
          onClose={() => setLaunchRemoving(null)}
        />
      )}
      {definitionOps.modals}

      {pendingRefusal && (
        <ConfirmModal
          title={pendingRefusal.kind === "live-token" ? "当前实际令牌还没有保存" : "当前实际配置无法完整保存"}
          description={liveRefusalDescription(pendingRefusal.kind, pendingRefusal.message)}
          risk="write-local"
          riskLabel={pendingRefusal.kind === "live-token" ? "放弃令牌" : "放弃实际配置"}
          confirmLabel={pendingRefusal.kind === "live-token" ? "放弃这个令牌并继续" : "放弃当前实际配置继续切换"}
          secondaryLabel={pendingRefusal.kind === "live-token" && onOpenSecrets ? "去选择密钥文件" : undefined}
          onSecondary={pendingRefusal.kind === "live-token" && onOpenSecrets
            ? () => {
                setPendingRefusal(null);
                onOpenSecrets();
              }
            : undefined}
          executing={configuringTarget !== null}
          onConfirm={() => {
            const pending = pendingRefusal;
            setPendingRefusal(null);
            void runConfigure(
              pending.kind === "live-token"
                ? { ...pending.request, discardLiveToken: true }
                : { ...pending.request, discardLive: true },
              pending.success,
            );
          }}
          onClose={() => {
            setPendingRefusal(null);
            toast.message("没有写入", { description: "助手的配置文件与本工具保存的内容都没动。" });
          }}
        />
      )}
      {pendingRuntimeSave && (
        <ConfirmModal
          title="完全放开助手权限"
          description="这会写入客户端：关闭权限询问，同时允许助手脱离工作区沙箱操作。只应在可信且已经隔离的环境中使用。"
          risk="write-local"
          riskLabel="完全放开"
          confirmLabel="确认写入"
          executing={runtimeSaving}
          onConfirm={() => {
            const pending = pendingRuntimeSave;
            setPendingRuntimeSave(null);
            void persistRuntimeSettings(pending.agent, pending.patch, true);
          }}
          onClose={() => {
            setPendingRuntimeSave(null);
            toast.message("已取消", { description: "客户端配置与本工具保存的内容都没有修改。" });
          }}
        />
      )}
    </div>
  );
}

/** 写入成功后的提示文案。 */
interface ConfigureSuccess {
  message?: string;
}
