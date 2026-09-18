import { useMemo, useState } from "react";
import { Bot, FolderSymlink, Link2Off, PackageCheck, RefreshCw, Server, ShieldCheck, Sparkles } from "lucide-react";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { ErrorPanel } from "../components/ErrorPanel";
import type {
  DiscoverAgentCandidate,
  DiscoverData,
  DiscoverMcpCandidate,
  DiscoverSkillGroup,
  DiscoverSkillItem,
} from "../lib/contract";
import { definitionKeyPrefix } from "../lib/definitionOverlay";
import { candidateToDefinition } from "../lib/mcpImport";
import { useEngineQuery } from "../lib/queries";
import { useTakeover, type TakeoverTask } from "../lib/useTakeover";
import { cn } from "../lib/utils";

/** 助手来源 → 展示文案：官方安装器/Homebrew/npm 全局/其他路径。 */
const AGENT_ORIGIN_LABEL: Record<string, string> = {
  native: "官方安装器",
  brew: "Homebrew",
  npm: "npm 全局",
  system: "其他路径",
};

function Badge({ tone, children }: { tone: "info" | "success" | "warning" | "muted"; children: React.ReactNode }) {
  const toneClass = {
    info: "border-info/30 bg-info/10 text-info",
    success: "border-success/30 bg-success/10 text-success",
    warning: "border-warning/30 bg-warning/10 text-warning",
    muted: "border bg-muted text-muted-foreground",
  }[tone];
  return <span className={cn("rounded border px-1 py-0.5 text-[10px]", toneClass)}>{children}</span>;
}

function SectionCard({
  icon: Icon,
  title,
  description,
  children,
  testId,
}: {
  icon: typeof Bot;
  title: string;
  description: string;
  children: React.ReactNode;
  testId: string;
}) {
  return (
    <section data-testid={testId} className="rounded-lg border bg-card p-5 shadow-sm">
      <h3 className="flex items-center gap-2 text-sm font-semibold text-foreground">
        <Icon className="h-4 w-4 text-primary" aria-hidden />
        {title}
      </h3>
      <p className="mt-1 text-xs text-muted-foreground">{description}</p>
      <div className="mt-3 space-y-2">{children}</div>
    </section>
  );
}

function SelectableRow({
  checked,
  onToggle,
  testId,
  children,
}: {
  checked: boolean;
  onToggle: () => void;
  testId: string;
  children: React.ReactNode;
}) {
  return (
    <label
      data-testid={testId}
      className="flex cursor-pointer items-start gap-3 rounded-md border px-3 py-2 hover:bg-accent/40"
    >
      <input type="checkbox" checked={checked} onChange={onToggle} className="mt-1 h-3.5 w-3.5" />
      <div className="min-w-0 flex-1">{children}</div>
    </label>
  );
}

/** 同名 MCP 候选按服务名聚合：一条定义（agents 为全部来源助手），逐助手各写一次目标配置。 */
function groupMcpCandidates(candidates: DiscoverMcpCandidate[]): { name: string; agents: string[]; entries: DiscoverMcpCandidate[] }[] {
  const byName = new Map<string, DiscoverMcpCandidate[]>();
  for (const candidate of candidates) {
    const list = byName.get(candidate.name) ?? [];
    list.push(candidate);
    byName.set(candidate.name, list);
  }
  return [...byName.entries()].map(([name, entries]) => ({
    name,
    agents: entries.map((entry) => entry.agent),
    entries,
  }));
}

/**
 * 接管中心：一次看清本机已有的助手、技能与 MCP 资源，按来源分类后批量纳入管理。
 *
 * 分类即处理方式：官方路径与原生安装方案（Claude 同步目录、Homebrew、官方安装器等）
 * 只标识、继续用它们各自的原生管理方式；其余外部资源用软链接方案纳管
 * （技能登记为本地来源集合后直链源目录，内容仍归外部所有）。
 */
export default function TakeoverPage() {
  const query = useEngineQuery<DiscoverData>("system:discover");
  const takeover = useTakeover();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirming, setConfirming] = useState(false);

  const data = query.data?.ok ? query.data.data : null;
  const agents = data?.agents ?? [];
  const skillItems = data?.skills?.items ?? [];
  const skillGroups = data?.skills?.groups ?? [];
  const mcpGroups = useMemo(() => groupMcpCandidates(data?.mcp ?? []), [data?.mcp]);

  const extraLinks = skillItems.filter((item) => item.manage === "collection");
  const nativeItems = skillItems.filter((item) => item.manage === "native");
  const deadLinks = skillItems.filter((item) => item.manage === "cleanup");

  const tasks = useMemo<Map<string, TakeoverTask>>(() => {
    const map = new Map<string, TakeoverTask>();
    for (const agent of agents) {
      map.set(`agent:${agent.id}`, {
        key: `agent:${agent.id}`,
        label: `助手 ${agent.display_name}`,
        // 登记定义即可：安装/升级/卸载仍由来源渠道决定，本步不执行任何命令。
        steps: [
          { kind: "policy", key: `${definitionKeyPrefix("agents")}.${agent.id}`, definition: agent.suggested_definition },
        ],
      });
    }
    for (const group of skillGroups) {
      const steps: TakeoverTask["steps"] = [];
      if (!group.existing_collection) {
        steps.push({
          kind: "policy",
          key: `${definitionKeyPrefix("skills")}.${group.name}`,
          definition: group.suggested_definition as unknown as Record<string, unknown>,
        });
      }
      for (const skill of group.skills) {
        for (const agent of skill.agents) {
          steps.push({
            kind: "apply",
            command: "skills:apply",
            args: ["--only", `${group.name}/${skill.name}`, "--agent", agent, "--action", "add"],
          });
        }
      }
      map.set(`skill-group:${group.name}`, { key: `skill-group:${group.name}`, label: `技能集合 ${group.name}`, steps });
    }
    for (const item of extraLinks) {
      if (!item.collection) continue;
      const key = `skill-extra:${item.collection}/${item.name}@${item.agent}`;
      map.set(key, {
        key,
        label: `技能 ${item.name} @ ${item.agent}`,
        // 非受管链接经既有显式接管通道替换（真实文件/目录仍 fail-closed）。
        steps: [
          {
            kind: "apply",
            command: "skills:apply",
            args: ["--only", `${item.collection}/${item.name}`, "--agent", item.agent, "--action", "add", "--allow-takeover"],
          },
        ],
      });
    }
    for (const item of deadLinks) {
      const key = `skill-cleanup:${item.name}@${item.agent}`;
      map.set(key, {
        key,
        label: `清理链接 ${item.name} @ ${item.agent}`,
        // 只删链接本身：真实目录与有分配记录的条目引擎侧一律拒绝
        steps: [
          {
            kind: "apply",
            command: "skills:apply",
            args: ["--action", "remove-link", "--agent", item.agent, "--link", item.name],
          },
        ],
      });
    }
    for (const group of mcpGroups) {
      const definition = {
        ...candidateToDefinition(group.entries[0]),
        agents: [...new Set(group.agents)],
      };
      const steps: TakeoverTask["steps"] = [
        { kind: "policy", key: `${definitionKeyPrefix("mcp")}.${group.name}`, definition },
      ];
      for (const entry of group.entries) {
        steps.push({
          kind: "apply",
          command: "mcp:apply",
          args: ["--only", `mcp:${group.name}`, "--agent", entry.agent, "--action", "add", "--allow-takeover"],
        });
      }
      map.set(`mcp:${group.name}`, { key: `mcp:${group.name}`, label: `MCP ${group.name}`, steps });
    }
    return map;
  }, [agents, skillGroups, extraLinks, deadLinks, mcpGroups]);

  const toggle = (key: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const selectableKeys = [...tasks.keys()];
  const selectedKeys = selectableKeys.filter((key) => selected.has(key));
  const takeoverCount = selectedKeys.length;
  /** 携带 --allow-takeover 的任务数：确认文案要显式说明会替换助手里的同名未受管条目。 */
  const replacingCount = selectedKeys.filter((key) =>
    tasks.get(key)?.steps.some((step) => step.kind === "apply" && step.args.includes("--allow-takeover")),
  ).length;
  /** 选中的清理项数量：确认文案要分开说，它删的是链接而不是纳管。 */
  const cleanupCount = selectedKeys.filter((key) => key.startsWith("skill-cleanup:")).length;

  if (query.isPending) return <PageLoading text="正在扫描本机资源…" />;
  if (query.isError) return <PageError title="扫描失败" error={query.error} />;

  const runTakeover = async () => {
    const list = selectedKeys.map((key) => tasks.get(key)).filter((task): task is TakeoverTask => Boolean(task));
    setConfirming(false);
    const outcome = await takeover.run(list);
    if (outcome) {
      setSelected(new Set());
      void query.refetch();
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <PageHeader
          title="接管中心"
          description="扫描本机已有的助手、技能与 MCP 资源并按来源分类。官方路径与原生安装方案只做标识，继续用它们自己的方式升级卸载；其余资源用软链接纳入管理，内容仍归原处所有。"
        />
        <div className="flex items-center gap-2">
          <button
            type="button"
            data-testid="takeover-refresh"
            onClick={() => void query.refetch()}
            className="inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent"
          >
            <RefreshCw className="h-3.5 w-3.5" aria-hidden /> 重新扫描
          </button>
          <button
            type="button"
            data-testid="takeover-select-all"
            onClick={() => setSelected(new Set(selectableKeys))}
            disabled={selectableKeys.length === 0}
            className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
          >
            全选可接管项
          </button>
          <button
            type="button"
            data-testid="takeover-run"
            onClick={() => setConfirming(true)}
            disabled={takeoverCount === 0 || takeover.executing}
            className={cn(
              "inline-flex items-center gap-1 rounded border px-3 py-1 text-xs",
              takeoverCount === 0 || takeover.executing
                ? "cursor-not-allowed border bg-muted text-muted-foreground"
                : "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15",
            )}
          >
            <PackageCheck className="h-3.5 w-3.5" aria-hidden />
            {takeover.executing ? "正在接管…" : `一键接管所选（${takeoverCount}）`}
          </button>
        </div>
      </div>

      {query.data && !query.data.ok && <ErrorPanel title="扫描被拒绝" errors={query.data.errors} />}

      <SectionCard
        icon={Bot}
        testId="takeover-agents"
        title="助手"
        description="本机已安装、但还没有管理定义的助手 CLI。登记只保存定义，不执行任何安装命令；只有能可靠反查到 npm 包名的才由本工具接管升级卸载，其余保持原生管理方式。"
      >
        {agents.length === 0 ? (
          <EmptyState compact title="没有发现未登记的助手" description="已安装的助手都已在「助手」页管理。" testId="takeover-agents-empty" />
        ) : (
          agents.map((agent: DiscoverAgentCandidate) => (
            <SelectableRow
              key={agent.id}
              testId={`takeover-agent-${agent.id}`}
              checked={selected.has(`agent:${agent.id}`)}
              onToggle={() => toggle(`agent:${agent.id}`)}
            >
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="text-xs font-semibold text-foreground">{agent.display_name}</span>
                <Badge tone="muted">{AGENT_ORIGIN_LABEL[agent.origin] ?? agent.origin}</Badge>
                {agent.managed_after_takeover ? (
                  <Badge tone="success">登记后可由本工具升级</Badge>
                ) : (
                  <Badge tone="info">仅登记标识，保持原生管理</Badge>
                )}
                {agent.version && <Badge tone="muted">{agent.version}</Badge>}
              </div>
              <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{agent.bin_path}</p>
            </SelectableRow>
          ))
        )}
      </SectionCard>

      <SectionCard
        icon={FolderSymlink}
        testId="takeover-skill-groups"
        title="技能：外部目录（软链接接管）"
        description="来自第三方工具或自建目录的技能。接管会登记为本地来源集合，助手目录里的链接直接指向源目录——内容仍由原处维护并随其更新，停用只删链接。"
      >
        {skillGroups.length === 0 ? (
          <EmptyState compact title="没有发现可接管的外部技能" testId="takeover-skill-groups-empty" />
        ) : (
          skillGroups.map((group: DiscoverSkillGroup) => (
            <SelectableRow
              key={group.name}
              testId={`takeover-skill-group-${group.name}`}
              checked={selected.has(`skill-group:${group.name}`)}
              onToggle={() => toggle(`skill-group:${group.name}`)}
            >
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="font-mono text-xs font-semibold text-foreground">{group.name}</span>
                <Badge tone="info">软链接接管</Badge>
                {group.existing_collection && <Badge tone="muted">并入已有集合</Badge>}
                <Badge tone="muted">{group.skills.length} 个技能</Badge>
                <Badge tone="muted">{group.agents.join(" / ")}</Badge>
              </div>
              <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{group.source_path}</p>
              <p className="mt-1 text-[11px] text-muted-foreground">
                {group.skills.map((skill) => skill.name).join("、")}
              </p>
            </SelectableRow>
          ))
        )}
      </SectionCard>

      <SectionCard
        icon={Sparkles}
        testId="takeover-skill-extras"
        title="技能：已定义集合的计划外链接"
        description="链接名命中已有集合，但没有分配记录。接管走集合通道：纳入管理后由集合统一维护版本。"
      >
        {extraLinks.length === 0 ? (
          <EmptyState compact title="没有计划外链接" testId="takeover-skill-extras-empty" />
        ) : (
          extraLinks.map((item: DiscoverSkillItem) => {
            const key = `skill-extra:${item.collection}/${item.name}@${item.agent}`;
            return (
              <SelectableRow key={key} testId={`takeover-skill-extra-${item.name}-${item.agent}`} checked={selected.has(key)} onToggle={() => toggle(key)}>
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="font-mono text-xs font-semibold text-foreground">{item.name}</span>
                  <Badge tone="muted">{item.agent}</Badge>
                  <Badge tone="warning">替换外部同名链接</Badge>
                  <Badge tone="muted">集合 {item.collection}</Badge>
                </div>
                <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{item.link_path}</p>
              </SelectableRow>
            );
          })
        )}
      </SectionCard>

      <SectionCard
        icon={Server}
        testId="takeover-mcp"
        title="MCP 服务"
        description="各助手配置里尚未纳入管理的 MCP 条目。接管会把定义导入服务库（密钥全程只以变量引用形式入库，本工具拿不到明文），并在对应助手配置里把该条目标记为由本工具管理。"
      >
        {mcpGroups.length === 0 ? (
          <EmptyState compact title="没有发现未纳管的 MCP 条目" testId="takeover-mcp-empty" />
        ) : (
          mcpGroups.map((group) => (
            <SelectableRow
              key={group.name}
              testId={`takeover-mcp-${group.name}`}
              checked={selected.has(`mcp:${group.name}`)}
              onToggle={() => toggle(`mcp:${group.name}`)}
            >
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="font-mono text-xs font-semibold text-foreground">{group.name}</span>
                <Badge tone="muted">{group.entries[0]?.transport === "remote" ? "远程" : "stdio"}</Badge>
                <Badge tone="muted">{group.agents.join(" / ")}</Badge>
                {group.entries.some((entry) => entry.origin === "previously-managed") && (
                  <Badge tone="info">此前由本工具管理</Badge>
                )}
                <Badge tone="warning">替换同名外部条目</Badge>
              </div>
            </SelectableRow>
          ))
        )}
      </SectionCard>

      <SectionCard
        icon={ShieldCheck}
        testId="takeover-native"
        title="原生管理（只标识，不接管）"
        description="位于助手官方技能目录内的真实目录，由助手自身或其安装方案维护。本工具不接管、不覆盖，升级与卸载继续走它们原来的方式。"
      >
        {nativeItems.length === 0 ? (
          <EmptyState compact title="没有原生安装的技能目录" testId="takeover-native-empty" />
        ) : (
          nativeItems.map((item) => (
            <div key={`${item.agent}:${item.name}`} data-testid={`takeover-native-${item.name}-${item.agent}`} className="rounded-md border px-3 py-2">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="font-mono text-xs font-semibold text-foreground">{item.name}</span>
                <Badge tone="muted">{item.agent}</Badge>
                <Badge tone="success">原生管理</Badge>
              </div>
              <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{item.link_path}</p>
            </div>
          ))
        )}
      </SectionCard>

      {deadLinks.length > 0 && (
        <SectionCard
          icon={Link2Off}
          testId="takeover-dead-links"
          title="可清理的残留链接"
          description="断链或旧体系留下的链接。清理只删链接本身，不会跟随链接删掉源目录；真实目录与仍有分配记录的条目会被拒绝。"
        >
          {deadLinks.map((item) => {
            const key = `skill-cleanup:${item.name}@${item.agent}`;
            return (
              <SelectableRow
                key={`${item.agent}:${item.name}`}
                testId={`takeover-dead-${item.name}-${item.agent}`}
                checked={selected.has(key)}
                onToggle={() => toggle(key)}
              >
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="font-mono text-xs font-semibold text-foreground">{item.name}</span>
                  <Badge tone="muted">{item.agent}</Badge>
                  <Badge tone="warning">{item.origin === "dead-link" ? "目标已丢失" : "旧体系残留"}</Badge>
                </div>
                <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{item.link_path}</p>
              </SelectableRow>
            );
          })}
        </SectionCard>
      )}

      {confirming && (
        <ConfirmModal
          title={cleanupCount === takeoverCount && takeoverCount > 0 ? `清理所选 ${takeoverCount} 项` : `接管所选 ${takeoverCount} 项`}
          description={[
            "将保存定义并对可接管项执行真实操作。外部源目录与原生安装的资源不会被删除或改写。",
            replacingCount > 0
              ? `其中 ${replacingCount} 项会替换助手里的同名外部条目（接管后由本工具统一维护）。`
              : "",
            cleanupCount > 0 ? `另有 ${cleanupCount} 条残留链接会被删除（只删链接本身，不动源目录）。` : "",
          ]
            .filter(Boolean)
            .join("")}
          risk="write-local"
          confirmLabel="开始接管"
          executing={takeover.executing}
          onConfirm={() => void runTakeover()}
          onClose={() => setConfirming(false)}
        />
      )}
    </div>
  );
}
