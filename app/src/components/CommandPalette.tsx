import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Search } from "lucide-react";
import { toast } from "sonner";
import type { StatusData } from "../lib/contract";
import {
  backfillNotice,
  configureArgs,
  configureBaseOf,
  configureRefusalOf,
  currentConnectionOf,
  liveRefusalDescription,
} from "../lib/agentCurrent";
import { isOfficial } from "../lib/defaultEnvironment";
import { useExecutions } from "../lib/execution";
import { useEngineQuery } from "../lib/queries";
import { useRefreshUpstream } from "../lib/queries";
import { matchesKeyword } from "./SearchInput";
import { cn } from "../lib/utils";
import { ConfirmModal } from "./ConfirmModal";
import {
  publishTrayConnections,
  type TrayConnection,
} from "../lib/trayActions";

export interface CommandItem {
  id: string;
  label: string;
  /** 右侧说明：分类、所属助手或目标路径。 */
  hint?: string;
  /** 额外匹配词（英文名、命令名等）。 */
  keywords?: string;
  group: string;
  /** 当前状态行仍可搜索，但不会重复执行写入。 */
  disabled?: boolean;
  current?: boolean;
  run: () => void;
}

interface ConnectionSwitchRequest {
  agentId: string;
  agentName: string;
  profile: string;
  profileLabel: string;
}

const PAGES: { key: string; label: string; keywords: string }[] = [
  { key: "dashboard", label: "总览", keywords: "dashboard 状态 诊断" },
  { key: "agents", label: "助手", keywords: "agents claude codex 实例" },
  { key: "skills", label: "技能", keywords: "skills 技能矩阵" },
  { key: "mcp", label: "MCP", keywords: "mcp 服务器" },
  { key: "rules-hooks", label: "规则与 Hooks", keywords: "rules hooks 规则" },
  { key: "takeover", label: "导入现有配置", keywords: "takeover 接管 发现 导入 扫描" },
  { key: "usage", label: "用量", keywords: "usage 用量 token 花费 成本" },
  { key: "runs", label: "操作记录", keywords: "runs 任务 日志" },
  { key: "settings", label: "设置", keywords: "settings 偏好 终端快捷命令" },
  // 设置里的二级页面也要搜得到：密钥现在是要填东西的页，藏在二级菜单里不好找
  { key: "settings:secrets", label: "密钥", keywords: "secrets 密钥 变量 token api key 凭据" },
  { key: "settings:aliases", label: "终端快捷命令", keywords: "aliases alias 别名 启动命令 时区" },
];

function supportsConnectionSwitch(spec: Record<string, unknown>): boolean {
  const bin = typeof spec.bin === "string" ? spec.bin.trim().replace(/\.exe$/i, "").split(/[\\/]/).pop() : "";
  return bin === "claude" || bin === "codex";
}

/** 当前接入以实际配置为准：引擎按内容识别；官方账号看地址与凭据，指定了模型也仍是官方账号。 */
function isCurrentConnection(data: StatusData | undefined, agentId: string, profile: string): boolean {
  const element = data?.elements.find((item) => item.id === `agents:${agentId}`);
  const current = element ? currentConnectionOf(element) : null;
  if (!current) return false;
  return profile ? current.profile === profile : isOfficial(current);
}

/**
 * 命令面板（⌘K）：一个入口搜到所有资源、页面与常用动作，键盘直接完成。
 *
 * 资源只从已加载的查询缓存里取（面板打开时才发起首次查询），点中资源就跳到它所在的页面；
 * 动作走与页面上相同的执行通道，因此锁、审计与结果提示都一致。
 */
export function CommandPalette({
  open,
  scope = "all",
  initialKeyword,
  initialItemId,
  onClose,
  onNavigate,
  onOpenAgentSkills,
}: {
  open: boolean;
  /** 账号快切模式只保留各助手的「官方账号」项；执行路径与完整命令面板相同。 */
  scope?: "all" | "connections";
  /** 托盘目标会带精确查询，打开后直接把焦点落在对应接入。 */
  initialKeyword?: string | null;
  initialItemId?: string | null;
  onClose: () => void;
  onNavigate: (page: string) => void;
  /** 选中某个助手/实例的技能时，跳到技能页并预选它。 */
  onOpenAgentSkills: (agentId: string) => void;
}) {
  const [keyword, setKeyword] = useState("");
  const [targetItemId, setTargetItemId] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const [switchingTarget, setSwitchingTarget] = useState<string | null>(null);
  /** 切换前的回填被拒：由用户显式选择放弃，或先去选密钥文件。 */
  const [pendingSwitch, setPendingSwitch] = useState<{
    request: ConnectionSwitchRequest;
    kind: "live-token" | "unrepresentable";
    message: string;
  } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const switchBusyRef = useRef(false);
  const refreshUpstream = useRefreshUpstream();
  const { execute } = useExecutions();

  // 托盘快切在窗口隐藏时也要有可信的脱敏状态，因此这两项保持热缓存；
  // React Query 会与助手页共享请求，不会为同一状态另建数据源。
  const agents = useEngineQuery<StatusData>("agents:status", [], { enabled: true });
  const skills = useEngineQuery<StatusData>("skills:status", [], { enabled: open });
  const mcp = useEngineQuery<StatusData>("mcp:status", [], { enabled: open });
  const profiles = useEngineQuery<StatusData>("profiles:status", [], { enabled: true });
  const refetchAgents = agents.refetch;

  const switchConnection = useCallback(
    async (
      request: ConnectionSwitchRequest,
      discard: { discardLive?: boolean; discardLiveToken?: boolean } = {},
    ) => {
      if (switchBusyRef.current) return;
      const element = agents.data?.ok
        ? agents.data.data.elements.find((item) => item.id === `agents:${request.agentId}`)
        : undefined;
      const base = element ? configureBaseOf(element) : null;
      if (!base) {
        toast.error("客户端配置文件暂不可读，接入没有切换", { description: "到助手页“运行设置”底部直接编辑配置文件查看原因。" });
        return;
      }
      switchBusyRef.current = true;
      const target = `${request.agentId}/${request.profile || "default"}`;
      setSwitchingTarget(target);
      try {
        const envelope = await execute({
          command: "agents:configure",
          args: configureArgs({ agentId: request.agentId, base, useDefault: true, ...discard }),
          label: `切换 ${request.agentName} 到 ${request.profileLabel}`,
          silent: true,
        });
        if (!envelope) {
          toast.error("切换失败，请查看任务记录后重试");
          return;
        }
        if (!envelope.ok) {
          const message = envelope.errors[0]?.message ?? "未知错误";
          const refusal = configureRefusalOf(envelope);
          if (refusal === "changed") {
            await refetchAgents();
            toast.error("配置文件已在外部修改，接入没有切换", {
              description: "已刷新为客户端当前的实际配置，请确认后重新选择。",
            });
            return;
          }
          if (refusal === "live-token" || refusal === "unrepresentable") {
            setPendingSwitch({ request, kind: refusal, message });
            return;
          }
          toast.error(`接入没有切换：${message}`);
          return;
        }

        const refreshed = await refetchAgents();
        const confirmed = refreshed.data?.ok
          && isCurrentConnection(refreshed.data.data, request.agentId, request.profile);
        if (!confirmed) {
          toast.error("写入已经返回成功，但状态回读没有确认接入生效，请在助手页检查");
          return;
        }
        setPendingSwitch(null);
        const notice = backfillNotice(envelope.data);
        toast.success(`已切换 ${request.agentName} 到 ${request.profileLabel}`, {
          description: notice
            ? `${notice}。助手下次启动生效。`
            : "助手下次启动生效；已经开着的要重启它。",
        });
      } finally {
        switchBusyRef.current = false;
        setSwitchingTarget(null);
      }
    },
    [agents.data, execute, refetchAgents],
  );

  const trayConnections = useMemo<TrayConnection[]>(() => {
    const out: TrayConnection[] = [];
    const profileIds = new Set(
      (profiles.data?.ok ? profiles.data.data.elements : []).map((element) => element.id.replace(/^profiles:/, "")),
    );
    for (const element of agents.data?.ok ? agents.data.data.elements : []) {
      const agentId = element.id.replace(/^agents:/, "");
      if (profileIds.has(agentId) || !supportsConnectionSwitch(element.spec)) continue;
      const spec = element.spec as { display_name?: string };
      const agentName = spec.display_name ?? agentId;
      // 主配置只放官方账号：托盘只给「官方账号」一项，主配置里有第三方凭据时用它切回
      const options = [{ profile: "", label: "官方账号" }];
      for (const option of options) {
        out.push({
          agentId,
          agentName,
          profile: option.profile,
          profileLabel: option.label,
          current: Boolean(agents.data?.ok && isCurrentConnection(agents.data.data, agentId, option.profile)),
        });
      }
    }
    return out;
  }, [agents.data, profiles.data]);

  useEffect(() => {
    // 两份状态都成功后才替换原生菜单，避免启动竞态或瞬时读取失败清空上一次可用菜单。
    if (agents.data?.ok && profiles.data?.ok) void publishTrayConnections(trayConnections);
  }, [agents.data?.ok, profiles.data?.ok, trayConnections]);

  useEffect(() => {
    if (open) {
      setKeyword(initialKeyword ?? "");
      setTargetItemId(initialItemId ?? null);
      setActive(0);
      // 面板一打开就能直接敲字
      window.setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [initialItemId, initialKeyword, open]);

  const items = useMemo<CommandItem[]>(() => {
    const out: CommandItem[] = PAGES.map((page) => ({
      id: `page:${page.key}`,
      label: page.label,
      hint: "页面",
      keywords: page.keywords,
      group: "前往",
      run: () => onNavigate(page.key),
    }));

    const profileIds = new Set(
      (profiles.data?.ok ? profiles.data.data.elements : []).map((element) => element.id.replace(/^profiles:/, "")),
    );
    for (const element of agents.data?.ok ? agents.data.data.elements : []) {
      const id = element.id.replace(/^agents:/, "");
      const spec = element.spec as { display_name?: string; bin?: string };
      const displayName = spec.display_name ?? id;
      out.push({
        id: `agent:${id}`,
        label: displayName,
        hint: "助手",
        keywords: `${id} ${spec.bin ?? ""}`,
        group: "资源",
        run: () => onNavigate("agents"),
      });
      out.push({
        id: `agent-skills:${id}`,
        label: `${spec.display_name ?? id} 的技能`,
        hint: "技能分配",
        keywords: `${id} skills`,
        group: "资源",
        run: () => onOpenAgentSkills(id),
      });

      if (!profileIds.has(id) && supportsConnectionSwitch(element.spec)) {
        // 主配置只放官方账号，第三方接入在独立环境里用；这里只剩「切回官方账号」
        const options = [{ profile: "", label: "官方账号", baseUrl: "", model: "", note: "" }];
        for (const option of options) {
          const current = Boolean(
            agents.data?.ok && isCurrentConnection(agents.data.data, id, option.profile),
          );
          const target = `${id}/${option.profile || "default"}`;
          out.push({
            id: `connection:${id}:${option.profile || "default"}`,
            label: `${displayName} · ${option.label}`,
            hint: current
              ? "当前使用"
              : switchingTarget === target
                ? "正在切换…"
                : "切回官方账号",
            keywords: `${id} ${spec.bin ?? ""} ${option.label} 官方 账号 登录 provider 接入 切换`,
            group: "官方账号",
            disabled: current || switchingTarget !== null,
            current,
            run: () =>
              void switchConnection({
                agentId: id,
                agentName: displayName,
                profile: option.profile,
                profileLabel: option.label,
              }),
          });
        }
      }
    }
    for (const element of profiles.data?.ok ? profiles.data.data.elements : []) {
      const id = element.id.replace(/^profiles:/, "");
      const spec = element.spec as { display_name?: string; config_dir?: string };
      out.push({
        id: `profile:${id}`,
        label: spec.display_name ?? id,
        hint: `实例 · ${spec.config_dir ?? ""}`,
        keywords: `${id} profile 实例`,
        group: "资源",
        run: () => onNavigate("agents"),
      });
    }
    for (const element of skills.data?.ok ? skills.data.data.elements : []) {
      const id = element.id.replace(/^skill:/, "");
      if (id.includes("/")) continue;
      out.push({
        id: `skill:${id}`,
        label: id,
        hint: "技能集合",
        keywords: `${id} skills`,
        group: "资源",
        run: () => onNavigate("skills"),
      });
    }
    for (const element of mcp.data?.ok ? mcp.data.data.elements : []) {
      const id = element.id.replace(/^mcp:/, "");
      out.push({
        id: `mcp:${id}`,
        label: id,
        hint: "MCP 服务",
        keywords: `${id} mcp`,
        group: "资源",
        run: () => onNavigate("mcp"),
      });
    }

    out.push(
      {
        id: "action:refresh-upstream",
        label: "检查更新",
        hint: "对比官方最新版本",
        keywords: "update upstream 版本",
        group: "动作",
        run: () => refreshUpstream.mutate(undefined),
      },
      {
        id: "action:takeover",
        label: "扫描本机资源",
        hint: "扫描并导入本机已有配置",
        keywords: "discover 接管 扫描 导入 现有配置",
        group: "动作",
        run: () => onNavigate("takeover"),
      },
    );
    return out;
  }, [agents.data, skills.data, mcp.data, profiles.data, switchingTarget, refreshUpstream, switchConnection, onNavigate, onOpenAgentSkills]);

  const filtered = useMemo(() => {
    const scoped = scope === "connections"
      ? items.filter((item) => item.group === "官方账号")
      : items;
    return targetItemId
      ? scoped.filter((item) => item.id === targetItemId)
      : scoped.filter((item) => matchesKeyword(keyword, [item.label, item.hint, item.keywords]));
  }, [items, keyword, scope, targetItemId]);

  useEffect(() => {
    setActive((prev) => (prev >= filtered.length ? 0 : prev));
  }, [filtered.length]);

  if (!open && !pendingSwitch) return null;

  const runItem = (item: CommandItem) => {
    if (item.disabled) return;
    item.run();
    onClose();
  };

  const groups = filtered.reduce<Record<string, CommandItem[]>>((acc, item) => {
    (acc[item.group] ??= []).push(item);
    return acc;
  }, {});
  let renderIndex = -1;

  return (
    <>
      {open && (
        <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-slate-900/40 p-4 pt-[12vh] animate-fade-in"
      onClick={(event) => event.target === event.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={scope === "connections" ? "账号快切" : "命令面板"}
        data-testid="command-palette"
        className="w-full max-w-xl overflow-hidden rounded-xl border bg-card shadow-xl animate-zoom-in-95"
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setActive((prev) => (filtered.length === 0 ? 0 : (prev + 1) % filtered.length));
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            setActive((prev) => (filtered.length === 0 ? 0 : (prev - 1 + filtered.length) % filtered.length));
          } else if (event.key === "Enter") {
            event.preventDefault();
            const item = filtered[active];
            if (item) runItem(item);
          } else if (event.key === "Escape") {
            event.preventDefault();
            onClose();
          }
        }}
      >
        <div className="flex items-center gap-2 border-b px-4 py-3">
          <Search className="h-4 w-4 text-muted-foreground" aria-hidden />
          <input
            ref={inputRef}
            value={keyword}
            onChange={(event) => {
              setTargetItemId(null);
              setKeyword(event.target.value);
            }}
            data-testid="command-palette-input"
            aria-label={scope === "connections" ? "搜索助手" : "搜索资源、页面与动作"}
            placeholder={scope === "connections" ? "搜索助手名称…" : "搜索助手、技能、MCP、实例，或直接执行常用动作…"}
            className="w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
        </div>
        <div className="max-h-[50vh] overflow-y-auto p-2">
          {filtered.length === 0 ? (
            <p className="px-3 py-6 text-center text-sm text-muted-foreground">
              {scope === "connections" ? "没有匹配的助手。" : "没有匹配的资源或动作。"}
            </p>
          ) : (
            Object.entries(groups).map(([group, groupItems]) => (
              <div key={group} className="mb-2 last:mb-0">
                <p className="px-3 py-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">{group}</p>
                {groupItems.map((item) => {
                  renderIndex += 1;
                  const index = renderIndex;
                  return (
                    <button
                      key={item.id}
                      type="button"
                      data-testid={`command-item-${item.id}`}
                      disabled={item.disabled}
                      aria-current={item.current ? "true" : undefined}
                      onMouseEnter={() => setActive(index)}
                      onClick={() => runItem(item)}
                      className={cn(
                        "flex w-full items-center justify-between gap-3 rounded-md px-3 py-2 text-left text-sm",
                        item.current
                          ? "cursor-default bg-success/5 text-foreground"
                          : item.disabled
                            ? "cursor-wait text-muted-foreground"
                            : index === active
                              ? "bg-accent text-foreground"
                              : "text-foreground hover:bg-accent/60",
                      )}
                    >
                      <span className="truncate">{item.label}</span>
                      {item.hint && <span className="shrink-0 text-xs text-muted-foreground">{item.hint}</span>}
                    </button>
                  );
                })}
              </div>
            ))
          )}
        </div>
        <div className="border-t px-4 py-2 text-[11px] text-muted-foreground">
          ↑↓ 选择 · Enter 执行 · Esc 关闭
        </div>
      </div>
        </div>
      )}
      {pendingSwitch && (
        <ConfirmModal
          title={pendingSwitch.kind === "live-token" ? "当前实际令牌还没有保存" : "当前实际配置无法完整保存"}
          description={liveRefusalDescription(pendingSwitch.kind, pendingSwitch.message)}
          risk="write-local"
          riskLabel={pendingSwitch.kind === "live-token" ? "放弃令牌" : "放弃实际配置"}
          confirmLabel={pendingSwitch.kind === "live-token" ? "放弃这个令牌并切换" : "放弃当前实际配置继续切换"}
          secondaryLabel={pendingSwitch.kind === "live-token" ? "去选择密钥文件" : undefined}
          onSecondary={pendingSwitch.kind === "live-token"
            ? () => {
                setPendingSwitch(null);
                onNavigate("settings:secrets");
              }
            : undefined}
          executing={switchingTarget !== null}
          onConfirm={() => void switchConnection(
            pendingSwitch.request,
            pendingSwitch.kind === "live-token" ? { discardLiveToken: true } : { discardLive: true },
          )}
          onClose={() => setPendingSwitch(null)}
        />
      )}
    </>
  );
}
