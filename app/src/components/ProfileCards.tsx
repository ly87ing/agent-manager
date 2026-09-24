import { useState } from "react";
import { Activity, Copy, FolderTree, Loader2, PlayCircle, Plus, Sparkles, Terminal, Trash2, Wrench } from "lucide-react";
import type { EngineElement } from "../lib/contract";
import type { OfficialAuthStatus } from "../lib/observed";
import type { ProfileObserved, ProfileSpec } from "../lib/contract";
import { AgentUsageBar } from "./AgentUsageBar";
import { OfficialAuthPanel } from "./OfficialAuthPanel";
import type { ProfileLaunchCommand } from "../lib/profileDefinition";
import { StateBadge } from "./StateBadge";
import { cn } from "../lib/utils";
import { overrideEffect } from "../lib/overrideEffects";
import { balanceSupported } from "../lib/balance";

/** 实例状态 → 一句话说明用户现在处于什么位置。 */
const STATE_NOTE: Record<string, string> = {
  "not-materialized": "还没写入这套配置，点「写入配置」后就能用这个名字启动。",
  "managed-ok": "配置已写入，终端里用它的名字即可启动这个独立环境。",
  drifted: "实例定义与实际配置不同：可以用定义写入覆盖实际配置，也可以以实际配置更新定义。",
  disabled: "已停用，不再维护这套配置。",
  unsupported: "定义有问题，先修好再写入。",
};

export function ProfileCard({
  element,
  bin,
  busy,
  launchCommands,
  onApply,
  onAdopt,
  onEdit,
  onDuplicate,
  onRemove,
  onManageSkills,
  onAddLaunch,
  onEditLaunch,
  onDuplicateLaunch,
  onRemoveLaunch,
  onFixLaunch,
  onCheckAuth,
  onCheckEndpoint,
  onCheckBalance,
}: {
  element: EngineElement;
  bin: string;
  busy: boolean;
  /** 指向这个实例配置目录的全部启动命令（按配置目录反查，可以有多条）。 */
  launchCommands: ProfileLaunchCommand[];
  /** 用实例定义写入实际配置（不一致时由调用方先确认）。 */
  onApply: () => void;
  /** 以实际配置更新实例定义（只改库，不动实例文件）。 */
  onAdopt: () => void;
  onEdit: () => void;
  onDuplicate: () => void;
  onRemove: () => void;
  onManageSkills: () => void;
  onAddLaunch: () => void;
  onEditLaunch: (name: string) => void;
  onDuplicateLaunch: (name: string) => void;
  onRemoveLaunch: (name: string) => void;
  onFixLaunch: (name: string) => void;
  /** 用官方命令查一次登录状态；失败时返回原因。 */
  onCheckAuth: () => Promise<OfficialAuthStatus | string>;
  /** 第三方环境：按实例定义发一次最小真实请求，确认地址、凭据与模型能用。 */
  onCheckEndpoint?: () => Promise<{ ok: boolean; detail: string; latencyMs: number }>;
  /** 查这个环境凭据的余额（只对有官方余额接口的厂商）；返回一行结果或失败原因。 */
  onCheckBalance?: () => Promise<string>;
}) {
  const [balance, setBalance] = useState<string | null>(null);
  const [balanceBusy, setBalanceBusy] = useState(false);
  const [endpointCheck, setEndpointCheck] = useState<{ ok: boolean; detail: string; latencyMs: number } | null>(null);
  const [checkingEndpoint, setCheckingEndpoint] = useState(false);
  const checkEndpoint = async () => {
    if (!onCheckEndpoint) return;
    setCheckingEndpoint(true);
    try {
      setEndpointCheck(await onCheckEndpoint());
    } finally {
      setCheckingEndpoint(false);
    }
  };
  const spec = element.spec as unknown as ProfileSpec;
  const observed = element.observed as unknown as ProfileObserved;
  const id = element.id.replace(/^profiles:/, "");
  const blocked = observed.blocked_reason;
  const missingVars = (observed.env_refs ?? []).filter((ref) => ref.resolved === false);
  const allDiffers = (observed.fields ?? []).filter((field) => field.state !== "same");
  // 取消 shell 继承的隔离项合并成一句说明，其余差异逐条列出
  // 实际里还没有（shell 导出的会压过来）与实例文件自己设了值（定义没声明、写入会置空）分开说
  const isolationGaps = allDiffers.filter((field) => field.purpose === "isolation" && field.state === "missing");
  const isolationSet = allDiffers.filter((field) => field.purpose === "isolation" && field.state !== "missing");
  const differs = allDiffers.filter((field) => field.purpose !== "isolation");
  const canAdopt = element.actions.some((action) => action.id === "adopt");
  const canApply = element.actions.some((action) => action.id === "apply");
  const thirdParty = observed.third_party === true;
  const systemOverrides = observed.system_overrides ?? [];
  if (observed.invalid) {
    // 库里的定义按现在的规则解析不了（如旧版本存下的写法）：只给原因，以及编辑修正或移除
    return (
      <section data-testid={`profile-card-${id}`} className="rounded-lg border bg-card p-5 shadow-sm">
        <div className="flex items-start justify-between gap-3">
          <h3 className="min-w-0 text-base font-semibold text-foreground">
            {spec.display_name}
            <span className="ml-2 font-mono text-xs font-normal text-muted-foreground">{id}</span>
          </h3>
          <StateBadge state={element.state} />
        </div>
        <p
          data-testid={`profile-invalid-${id}`}
          role="alert"
          className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
        >
          这套独立环境的定义现在无法使用：{blocked ?? "未知原因"}。编辑修正后保存，或移除它；配置目录与里面的内容不受影响。
        </p>
        <div className="mt-3 flex flex-wrap items-center gap-2 border-t pt-3">
          <button type="button" data-testid={`profile-edit-${id}`} onClick={onEdit} className="rounded border px-2.5 py-1 text-xs">
            编辑
          </button>
          <button
            type="button"
            data-testid={`profile-remove-${id}`}
            onClick={onRemove}
            className="ml-auto inline-flex items-center gap-1 rounded border border-destructive/30 px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10"
          >
            <Trash2 className="h-3 w-3" aria-hidden /> 移除环境
          </button>
        </div>
      </section>
    );
  }
  return (
    <section
      data-testid={`profile-card-${id}`}
      className="rounded-lg border bg-card p-5 shadow-sm transition-shadow hover:shadow-md"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-base font-semibold text-foreground">
            {spec.display_name}
            <span className="ml-2 font-mono text-xs font-normal text-muted-foreground">{id}</span>
          </h3>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {spec.base_agent} 的独立环境{spec.base_url ? ` · ${spec.base_url}` : ""}
          </p>
        </div>
        <StateBadge state={element.state} />
      </div>

      <p className="mt-2 text-xs text-muted-foreground">{blocked ?? STATE_NOTE[element.state] ?? ""}</p>

      {/* 实例多半是另一个账号，额度各算各的 */}
      {observed.usage && (
        <div className="mt-3 rounded-md border px-3 py-2">
          <AgentUsageBar usage={observed.usage} testId={`profile-usage-${id}`} />
        </div>
      )}

      <div className="mt-3">
        <OfficialAuthPanel
          kind={observed.kind === "codex" ? "codex" : "claude"}
          bin={bin}
          configDir={observed.config_dir}
          thirdParty={thirdParty}
          testIds={{
            root: `profile-auth-status-${id}`,
            text: `profile-auth-text-${id}`,
            check: `profile-${id}-auth-check`,
            mixed: `profile-auth-mixed-${id}`,
            commands: `profile-${id}`,
          }}
          onCheck={onCheckAuth}
        />
      </div>

      {systemOverrides.length > 0 && (
        <div
          role="status"
          data-testid={`profile-system-overrides-${id}`}
          className="mt-2 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning"
        >
          <p className="font-medium">这些系统级设置对每个配置目录都生效，这个环境挡不住：</p>
          <ul className="mt-1 list-disc space-y-0.5 pl-5">
            {systemOverrides.map((item) => (
              <li key={`${item.source}/${item.name}`} className="break-all">
                <span className="font-mono">{item.source}</span> 里的 <span className="font-mono">{item.name}</span>：
                {overrideEffect(item.name, thirdParty ? "third-party-instance" : "official-instance")}
              </li>
            ))}
          </ul>
        </div>
      )}

      <div className="mt-3 space-y-1.5 border-t pt-3 text-[11px] text-muted-foreground">
        <p className="flex items-start gap-1.5 break-all">
          <FolderTree className="mt-0.5 h-3 w-3 shrink-0" aria-hidden />
          <span className="font-mono">{observed.config_dir}</span>
          {!observed.config_dir_exists && <span className="shrink-0 text-warning">（尚未创建）</span>}
        </p>
      </div>

      <div className="mt-3 border-t pt-3">
        <div className="flex items-center justify-between gap-2">
          <h4 className="flex items-center gap-1.5 text-xs font-medium text-foreground">
            <Terminal className="h-3 w-3" aria-hidden /> 启动命令
          </h4>
          <button
            type="button"
            data-testid={`profile-add-launch-${id}`}
            onClick={onAddLaunch}
            className="inline-flex items-center gap-1 rounded border px-2 py-0.5 text-[11px] text-muted-foreground hover:bg-accent"
          >
            <Plus className="h-3 w-3" aria-hidden /> 新增一条
          </button>
        </div>
        {launchCommands.length === 0 ? (
          <p data-testid={`profile-launch-empty-${id}`} className="mt-2 text-[11px] text-muted-foreground">
            还没有指向这套配置的启动命令，新增一条后在终端里用它的名字启动。
          </p>
        ) : (
          <ul className="mt-2 space-y-1.5">
            {launchCommands.map((cmd) => (
              <li key={cmd.name} data-testid={`profile-launch-${id}-${cmd.name}`} className="rounded border px-2.5 py-1.5">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="font-mono text-[11px] font-semibold text-foreground">{cmd.name}</span>
                  <div className="flex shrink-0 items-center gap-1">
                    <button
                      type="button"
                      data-testid={`profile-launch-edit-${id}-${cmd.name}`}
                      onClick={() => onEditLaunch(cmd.name)}
                      className="rounded border px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-accent"
                    >
                      编辑
                    </button>
                    <button
                      type="button"
                      data-testid={`profile-launch-copy-${id}-${cmd.name}`}
                      onClick={() => onDuplicateLaunch(cmd.name)}
                      className="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-accent"
                    >
                      <Copy className="h-3 w-3" aria-hidden /> 复制
                    </button>
                    <button
                      type="button"
                      data-testid={`profile-launch-remove-${id}-${cmd.name}`}
                      onClick={() => onRemoveLaunch(cmd.name)}
                      className="inline-flex items-center gap-1 rounded border border-destructive/30 px-1.5 py-0.5 text-[11px] text-destructive hover:bg-destructive/10"
                    >
                      <Trash2 className="h-3 w-3" aria-hidden /> 删除
                    </button>
                  </div>
                </div>
                {/* 同助手卡片：命令按空格才有意义，横向滚动比从中间断开好读 */}
                <p
                  title={`alias ${cmd.name}='${cmd.value}'`}
                  className="mt-1 overflow-x-auto whitespace-pre font-mono text-[11px] text-muted-foreground"
                >
                  alias {cmd.name}=&#39;{cmd.value}&#39;
                </p>
                {cmd.needsTildeFix && (
                  <div
                    data-testid={`profile-launch-fix-${id}-${cmd.name}`}
                    className="mt-1.5 flex flex-wrap items-center gap-2 rounded border border-warning/30 bg-warning/10 px-2 py-1 text-[11px] text-warning"
                  >
                    <span>
                      配置目录写成了引号里的 ~，终端不会把它当成主目录，这条命令会在当前目录下另建一套配置。
                    </span>
                    <button
                      type="button"
                      onClick={() => onFixLaunch(cmd.name)}
                      className="inline-flex shrink-0 items-center gap-1 rounded border border-warning/40 px-1.5 py-0.5 font-medium hover:bg-warning/15"
                    >
                      <Wrench className="h-3 w-3" aria-hidden /> 订正
                    </button>
                  </div>
                )}
                {cmd.needsCredentialClear && (
                  <div
                    data-testid={`profile-launch-clear-${id}-${cmd.name}`}
                    className="mt-1.5 flex flex-wrap items-center gap-2 rounded border border-warning/30 bg-warning/10 px-2 py-1 text-[11px] text-warning"
                  >
                    <span>
                      这条命令没有清空 CODEX_API_KEY / CODEX_ACCESS_TOKEN：shell 里有它们时，Codex 会优先用这份凭据，而不是这个环境自己的登录或接入。
                    </span>
                    <button
                      type="button"
                      onClick={() => onFixLaunch(cmd.name)}
                      className="inline-flex shrink-0 items-center gap-1 rounded border border-warning/40 px-1.5 py-0.5 font-medium hover:bg-warning/15"
                    >
                      <Wrench className="h-3 w-3" aria-hidden /> 订正
                    </button>
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>

      {missingVars.length > 0 && (
        <p data-testid={`profile-missing-vars-${id}`} className="mt-2 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
          这些环境变量当前取不到值，写入会被拒绝：{missingVars.map((ref) => ref.variable ?? ref.name).join("、")}
        </p>
      )}

      {element.state === "drifted" && isolationGaps.length > 0 && (
        <p data-testid={`profile-isolation-gaps-${id}`} className="mt-2 rounded-md border border-warning/30 bg-warning/5 px-3 py-2 text-xs text-warning">
          还没挡住 shell 里导出的 {isolationGaps.length} 个凭据相关变量（
          <span className="font-mono">{isolationGaps.map((field) => field.field.replace(/^env\./, "")).join("、")}</span>
          ）：它们会压过{thirdParty ? "这个环境自己的第三方凭据" : "这个环境自己的登录"}。写入配置后这些变量在本环境里一律置空。
        </p>
      )}

      {element.state === "drifted" && isolationSet.length > 0 && (
        <p data-testid={`profile-isolation-set-${id}`} className="mt-2 rounded-md border border-warning/30 bg-warning/5 px-3 py-2 text-xs text-warning">
          这个环境的配置文件自己设了定义里没有的{" "}
          <span className="font-mono">{isolationSet.map((field) => field.field.replace(/^env\./, "")).join("、")}</span>
          ：写入配置会把它们置空。要保留，请先编辑定义把它们加进去。
        </p>
      )}

      {element.state === "drifted" && differs.length > 0 && (
        <div data-testid={`profile-differs-${id}`} className="mt-2 rounded-md border border-warning/30 bg-warning/5 px-3 py-2 text-xs">
          <p className="font-medium text-warning">实际配置与实例定义不同</p>
          <ul className="mt-1 space-y-0.5 text-muted-foreground">
            {differs.map((field) => (
              <li key={field.field} className="break-all">
                <span className="font-mono text-foreground">{field.field}</span>
                {"：定义 "}
                <span className="font-mono">{field.definition ?? "（无）"}</span>
                {" · 实际 "}
                <span className="font-mono">
                  {field.state === "missing" ? "（没有）" : field.state === "unresolved" ? "（变量取不到值，无法比较）" : field.live ?? "（无）"}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {balance && (
        <p data-testid={`profile-balance-${id}`} className="mt-2 text-xs text-muted-foreground">
          {balance}
        </p>
      )}
      {endpointCheck && (
        <p
          data-testid={`profile-check-result-${id}`}
          className={cn("mt-2 text-xs", endpointCheck.ok ? "text-success" : "text-destructive")}
        >
          {endpointCheck.ok && endpointCheck.latencyMs > 0
            ? `${endpointCheck.detail}（${endpointCheck.latencyMs}ms）`
            : endpointCheck.detail}
        </p>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2 border-t pt-3">
        {thirdParty && onCheckEndpoint && (
          <button
            type="button"
            data-testid={`profile-check-${id}`}
            disabled={checkingEndpoint}
            onClick={() => void checkEndpoint()}
            title="按这个环境的地址、凭据与模型发一次最小请求；只读，不改配置"
            className="inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            {checkingEndpoint ? <Loader2 className="h-3 w-3 animate-spin" aria-hidden /> : <Activity className="h-3 w-3" aria-hidden />}
            {checkingEndpoint ? "正在测…" : "测一下"}
          </button>
        )}
        {thirdParty && onCheckBalance && balanceSupported(spec.base_url) && (
          <button
            type="button"
            data-testid={`profile-balance-button-${id}`}
            disabled={balanceBusy}
            title="用这个环境的凭据查账户余额；只在点它时联网，不改配置"
            onClick={async () => {
              setBalanceBusy(true);
              try {
                setBalance(await onCheckBalance());
              } finally {
                setBalanceBusy(false);
              }
            }}
            className="inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            {balanceBusy ? "正在查…" : "查余额"}
          </button>
        )}
        <button
          type="button"
          data-testid={`profile-apply-${id}`}
          disabled={busy || !canApply}
          onClick={onApply}
          className={cn(
            "inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs",
            element.state === "drifted"
              ? "border-warning/40 bg-warning/10 text-warning hover:bg-warning/15"
              : "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15",
            (busy || !canApply) && "cursor-not-allowed opacity-50",
          )}
        >
          {busy ? <Loader2 className="h-3 w-3 animate-spin" aria-hidden /> : <PlayCircle className="h-3 w-3" aria-hidden />}
          {busy ? "执行中…" : element.state === "drifted" ? "用定义覆盖实际配置" : "写入配置"}
        </button>
        {canAdopt && (
          <button
            type="button"
            data-testid={`profile-adopt-${id}`}
            disabled={busy}
            onClick={onAdopt}
            className="inline-flex items-center gap-1 rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10 disabled:opacity-50"
          >
            以实际配置更新定义
          </button>
        )}
        <button type="button" data-testid={`profile-edit-${id}`} onClick={onEdit} className="rounded border px-2.5 py-1 text-xs">
          编辑
        </button>
        <button
          type="button"
          data-testid={`profile-duplicate-${id}`}
          onClick={onDuplicate}
          className="inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent"
        >
          <Copy className="h-3 w-3" aria-hidden /> 复制
        </button>
        <button
          type="button"
          data-testid={`profile-skills-${id}`}
          onClick={onManageSkills}
          className="inline-flex items-center gap-1 rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15"
        >
          <Sparkles className="h-3 w-3" aria-hidden /> 管理技能
        </button>
        <button
          type="button"
          data-testid={`profile-remove-${id}`}
          onClick={onRemove}
          className="ml-auto inline-flex items-center gap-1 rounded border border-destructive/30 px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10"
        >
          <Trash2 className="h-3 w-3" aria-hidden /> 移除环境
        </button>
      </div>
    </section>
  );
}
