import { useState } from "react";
import { ChevronDown, ChevronRight, Inbox, Pencil, Plus, RefreshCw, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { DriftExplanationPanel } from "../components/DriftExplanationPanel";
import { ErrorPanel } from "../components/ErrorPanel";
import { FreshnessBadge } from "../components/FreshnessBadge";
import { MatrixCell } from "../components/MatrixCell";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SkillCollectionFormDialog } from "../components/SkillCollectionFormDialog";
import { StateBadge } from "../components/StateBadge";
import { Switch } from "../components/Switch";
import type { EngineElement, PolicyData, StatusData } from "../lib/contract";
import { runEngine } from "../lib/engine";
import {
  agentCellsOf,
  type SkillCollectionObserved,
  type SkillCollectionSpec,
  type SkillObserved,
  type SkillSpec,
} from "../lib/observed";
import { getDefaultPolicyLayer } from "../lib/policyLayer";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { useElementToggle } from "../lib/useElementToggle";
import { OVERLAY_COPY } from "../lib/ui-copy";
import { useEngineQuery } from "../lib/queries";
import { useExecuteApply, usePlanPreview } from "../lib/usePlanPreview";
import { cn } from "../lib/utils";

function isCollection(element: EngineElement): boolean {
  return element.id.startsWith("skill:") && !element.id.includes("/");
}

function shortCommit(value: string | null): string {
  if (!value) return "?";
  return value.length > 12 ? value.slice(0, 7) : value;
}

function CollectionCard({
  element,
  inboxCount,
  onToggle,
  onRemove,
  onEdit,
  onUpdate,
}: {
  element: EngineElement;
  inboxCount: number;
  onToggle: () => void;
  onRemove: () => void;
  onEdit: () => void;
  onUpdate: () => void;
}) {
  const spec = element.spec as SkillCollectionSpec;
  const observed = element.observed as SkillCollectionObserved;
  const name = element.id.replace(/^skill:/, "");
  const enabled = (element.policy as { enabled?: boolean }).enabled !== false;
  return (
    <section data-testid={`collection-card-${name}`} className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 className="text-base font-semibold text-slate-800">{name}</h3>
          <p className="mt-0.5 font-mono text-xs break-all text-slate-400">{spec.url ?? "（无 url）"}</p>
        </div>
        <div className="flex items-center gap-2">
          <Switch
            checked={enabled}
            testId={`toggle-${element.id}`}
            label={`${enabled ? "停用" : "启用"}集合 ${name}`}
            onChange={onToggle}
          />
          <StateBadge state={element.state} />
          <button
            type="button"
            data-testid={`remove-collection-${name}`}
            onClick={onRemove}
            title={OVERLAY_COPY.removeCollection}
            className="rounded p-1 text-slate-300 hover:bg-red-50 hover:text-red-500"
          >
            <Trash2 className="h-3.5 w-3.5" aria-hidden />
          </button>
        </div>
      </div>

      <div className="mt-3 flex flex-wrap gap-1.5 text-xs">
        <span className="rounded border border-slate-200 bg-slate-50 px-1.5 py-0.5 text-slate-600">
          {spec.matrix_mode ? "按助手逐个授权" : "整包分发"}
        </span>
        <span className="rounded border border-slate-200 bg-slate-50 px-1.5 py-0.5 text-slate-600">
          新技能默认:{spec.new_skill_default === "enabled" ? "启用" : "不启用"}
        </span>
        <span
          className={cn(
            "rounded border px-1.5 py-0.5",
            observed.owner_marked
              ? "border-indigo-200 bg-indigo-50 text-indigo-700"
              : "border-slate-200 bg-slate-50 text-slate-500",
          )}
        >
          {observed.owner_marked ? "自研" : "第三方"}
        </span>
        <span
          className={cn(
            "rounded border px-1.5 py-0.5",
            observed.repo_cached
              ? "border-emerald-200 bg-emerald-50 text-emerald-700"
              : "border-amber-200 bg-amber-50 text-amber-700",
          )}
          title={observed.repo_cache}
        >
          {observed.repo_cached ? "已下载到本地" : "尚未下载"}
        </span>
        {inboxCount > 0 && (
          <span
            data-testid={`inbox-count-${name}`}
            className="rounded border border-sky-200 bg-sky-50 px-1.5 py-0.5 font-medium text-sky-700"
          >
            新发现 {inboxCount}
          </span>
        )}
      </div>

      <div className="mt-3 flex items-center justify-between border-t border-slate-100 pt-3 text-xs text-slate-500">
        <span>
          已收录 {observed.discovered?.length ?? 0} 个技能 · 官方最新提交{" "}
          <code className="font-mono">{shortCommit(element.freshness.upstream)}</code>
        </span>
        <FreshnessBadge freshness={element.freshness} className="items-end" />
      </div>
      <div className="mt-3 flex gap-2">
        <button type="button" onClick={onEdit} data-testid={`edit-collection-${name}`} className="inline-flex items-center gap-1 rounded border px-3 py-2 text-xs text-slate-600"><Pencil className="h-3.5 w-3.5" aria-hidden />编辑集合</button>
        <button type="button" onClick={onUpdate} data-testid={`update-collection-${name}`} className="inline-flex items-center gap-1 rounded border px-3 py-2 text-xs text-indigo-600"><RefreshCw className="h-3.5 w-3.5" aria-hidden />更新集合</button>
      </div>
      {(observed.extra_links?.length ?? 0) > 0 && (
        <p className="mt-2 text-xs text-amber-700">
          发现计划外的链接:{observed.extra_links!.join(", ")}
        </p>
      )}
    </section>
  );
}

export default function SkillsPage() {
  const query = useEngineQuery<StatusData>("skills:status");
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [showAddForm, setShowAddForm] = useState(false);
  const [editing, setEditing] = useState<EngineElement | null>(null);
  const preview = usePlanPreview();
  const executeApply = useExecuteApply();
  const elementToggle = useElementToggle();
  const definitionOps = useDefinitionOps({ section: "skills", domain: "skills" });

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="skills:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  const collections = elements.filter(isCollection);
  const skills = elements.filter((element) => !isCollection(element));
  const inboxSkills = skills.filter((element) => (element.observed as SkillObserved).discovery_inbox === true);
  const matrixSkills = skills.filter((element) => (element.observed as SkillObserved).discovery_inbox !== true);
  const agentColumns = Array.from(
    new Set(matrixSkills.flatMap((element) => Object.keys(agentCellsOf(element)))),
  ).sort();
  const layers = policyQuery.data?.data.layers ?? null;

  /** 策略写入 + plan 预览编排的公共尾部：skills:plan 过滤到本 skill 及 agent → skills:apply。 */
  const openApplyPreview = (skill: EngineElement, label: string, agent?: string) => {
    const args = agent ? ["--only", skill.id, "--agent", agent] : ["--only", skill.id];
    preview.open({
      title: label,
      subtitle: "仅同步此技能链接（执行前自动备份）。",
      planCommands: [{ command: "skills:plan", args }],
      confirmLabel: "立即同步",
      execute: executeApply({ command: "skills:apply", label, extraArgs: args }),
    });
  };

  /** 矩阵点击切换：policy.set（skills.<repo>.skills.<name>.agents.<agent> bool）→ plan 预览 → apply。 */
  const toggleSkillCell = async (skill: EngineElement, agent: string, currentState: string) => {
    const spec = skill.spec as SkillSpec;
    const next = currentState === "disabled";
    const layer = getDefaultPolicyLayer();
    const key = `skills.${spec.collection}.skills.${spec.name}.agents.${agent}`;
    let envelope;
    try {
      envelope = await runEngine<PolicyData>("policy:set", ["--layer", layer, "--key", key, "--value", next ? "true" : "false"]);
    } catch (err) {
      toast.error(`保存配置失败:${err instanceof Error ? err.message : String(err)}`);
      return;
    }
    if (!envelope.ok) {
      toast.error(`保存被拒绝:${envelope.errors[0]?.message ?? "未知错误"}`);
      return;
    }
    toast.success(`已${next ? "启用" : "停用"} ${spec.name} @ ${agent}（写入${layer === "synced" ? "共享层" : "仅本机层"}）`);
    openApplyPreview(skill, `${next ? "启用" : "停用"} ${spec.name} @ ${agent}`, agent);
  };

  /** 收件箱启用：agents 置 null = 跟随集合目标集（引擎 resolve_skill_agent_switch 语义）。 */
  const enableInboxSkill = async (skill: EngineElement) => {
    const spec = skill.spec as SkillSpec;
    const layer = getDefaultPolicyLayer();
    const key = `skills.${spec.collection}.skills.${spec.name}.agents`;
    let envelope;
    try {
      envelope = await runEngine<PolicyData>("policy:set", ["--layer", layer, "--key", key, "--value", "null"]);
    } catch (err) {
      toast.error(`保存配置失败:${err instanceof Error ? err.message : String(err)}`);
      return;
    }
    if (!envelope.ok) {
      toast.error(`保存被拒绝:${envelope.errors[0]?.message ?? "未知错误"}`);
      return;
    }
    toast.success(`已启用新技能 ${spec.name}（将分发给集合内全部助手，保存到「${layer === "synced" ? "多设备共享" : "仅这台电脑"}」）`);
    openApplyPreview(skill, `启用 ${spec.name}`);
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Skills"
        description="技能集合、新发现的技能、以及每个技能在各助手中的启用状态。矩阵里点格子即可开关（会先预览再执行）。"
      />
      {query.data.errors.length > 0 && <ErrorPanel title="skills:status 错误" errors={query.data.errors} />}

      <div className="flex justify-end">
        <button
          type="button"
          data-testid="add-collection-button"
          onClick={() => setShowAddForm(true)}
          className="inline-flex items-center gap-1.5 rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700"
        >
          <Plus className="h-4 w-4" aria-hidden /> {OVERLAY_COPY.addCollection}
        </button>
      </div>

      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        {collections.map((collection) => (
          <CollectionCard
            key={collection.id}
            element={collection}
            inboxCount={(collection.observed as SkillCollectionObserved).discovery_inbox?.length ?? 0}
            onToggle={() => {
              const repo = collection.id.replace(/^skill:/, "");
              void elementToggle.toggle({
                elementId: collection.id,
                enabledKey: `skills.${repo}.enabled`,
                enabled: (collection.policy as { enabled?: boolean }).enabled !== false,
                domain: "skills",
                name: `集合 ${repo}`,
              });
            }}
            onRemove={() => definitionOps.requestRemove(collection.id.replace(/^skill:/, ""), layers)}
            onEdit={() => setEditing(collection)}
            onUpdate={() => openApplyPreview(collection, `更新 ${collection.id.replace(/^skill:/, "")}`)}
          />
        ))}
      </div>

      <section data-testid="discovery-inbox" className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold text-slate-700">
          <Inbox className="h-4 w-4 text-sky-600" aria-hidden />
          新发现的技能
          <span data-testid="inbox-total" className="rounded-full bg-sky-100 px-2 py-0.5 text-xs font-medium text-sky-700">
            {inboxSkills.length}
          </span>
        </h3>
        {inboxSkills.length === 0 ? (
          <p className="text-sm text-slate-400">集合里没有新发现的技能。</p>
        ) : (
          <table className="w-full text-left text-sm">
            <thead>
              <tr className="border-b border-slate-200 text-xs text-slate-500">
                <th className="py-2 pr-4 font-medium">名称</th>
                <th className="py-2 pr-4 font-medium">来源</th>
                <th className="py-2 pr-4 font-medium">说明</th>
                <th className="py-2 font-medium">操作</th>
              </tr>
            </thead>
            <tbody>
              {inboxSkills.map((skill) => {
                const spec = skill.spec as SkillSpec;
                return (
                  <tr key={skill.id} className="border-b border-slate-100 last:border-0">
                    <td className="py-2 pr-4 font-mono text-xs text-slate-700">{spec.name ?? skill.id}</td>
                    <td className="py-2 pr-4 font-mono text-xs break-all text-slate-500">{spec.source_path ?? spec.collection}</td>
                    <td className="py-2 pr-4 text-xs text-slate-600">集合里新出现的技能；启用后才会分发到助手</td>
                    <td className="py-2">
                      <button
                        type="button"
                        data-testid={`enable-inbox-${skill.id}`}
                        onClick={() => void enableInboxSkill(skill)}
                        className="rounded border border-indigo-200 px-2 py-0.5 text-xs text-indigo-600 hover:bg-indigo-50"
                      >
                        启用
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </section>

      <section className="overflow-x-auto rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <h3 className="mb-3 text-sm font-semibold text-slate-700">每个技能 × 每个助手</h3>
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-slate-200 text-xs text-slate-500">
              <th className="py-2 pr-4 font-medium">Skill</th>
              {agentColumns.map((agent) => (
                <th key={agent} className="py-2 pr-4 font-mono font-medium">
                  {agent}
                </th>
              ))}
              <th className="py-2 font-medium">同步状态</th>
            </tr>
          </thead>
          <tbody>
            {matrixSkills.map((skill) => {
              const spec = skill.spec as SkillSpec;
              const cells = agentCellsOf(skill);
              const isExpanded = skill.id === expandedId;
              return (
                <FragmentRow
                  key={skill.id}
                  skill={skill}
                  name={spec.name ?? skill.id}
                  cells={cells}
                  agentColumns={agentColumns}
                  isExpanded={isExpanded}
                  onToggle={() => setExpandedId(isExpanded ? null : skill.id)}
                  onToggleCell={(agent, state) => void toggleSkillCell(skill, agent, state)}
                />
              );
            })}
          </tbody>
        </table>
        {matrixSkills.length === 0 && <p className="py-4 text-center text-sm text-slate-400">无已声明 skill</p>}
      </section>

      {(showAddForm || editing) && (
        <SkillCollectionFormDialog
          agentIds={agentColumns}
          initial={editing ? { name: editing.id.replace(/^skill:/, ""), definition: editing.spec } : undefined}
          onClose={() => { setShowAddForm(false); setEditing(null); }}
          onSubmit={(name, definition) => {
            setShowAddForm(false);
            setEditing(null);
            void definitionOps.saveAndApply({ name, definition, isNew: !editing, elementId: `skill:${name}` });
          }}
        />
      )}

      {preview.modal}
      {elementToggle.modal}
      {definitionOps.modals}
    </div>
  );
}

function FragmentRow({
  skill,
  name,
  cells,
  agentColumns,
  isExpanded,
  onToggle,
  onToggleCell,
}: {
  skill: EngineElement;
  name: string;
  cells: Record<string, { state: string }>;
  agentColumns: string[];
  isExpanded: boolean;
  onToggle: () => void;
  onToggleCell: (agent: string, currentState: string) => void;
}) {
  return (
    <>
      <tr className={cn("border-b border-slate-100", !isExpanded && "last:border-0")}>
        <td className="py-2 pr-4">
          <button
            type="button"
            onClick={onToggle}
            aria-expanded={isExpanded}
            className="inline-flex items-center gap-1 font-mono text-xs text-indigo-600 hover:text-indigo-800"
          >
            {isExpanded ? <ChevronDown className="h-3 w-3" aria-hidden /> : <ChevronRight className="h-3 w-3" aria-hidden />}
            {name}
          </button>
        </td>
        {agentColumns.map((agent) => {
          const cell = cells[agent];
          return (
            <td key={agent} className="py-2 pr-4">
              {cell ? (
                <button
                  type="button"
                  data-testid={`skill-cell-${skill.id}-${agent}`}
                  onClick={() => onToggleCell(agent, cell.state)}
                  title="点击切换启用（会先预览再执行）"
                  className="cursor-pointer rounded hover:ring-2 hover:ring-indigo-300"
                >
                  <MatrixCell state={cell.state} kind="skills" title={`${agent}: ${cell.state}`} />
                </button>
              ) : (
                <span className="text-xs text-slate-300">—</span>
              )}
            </td>
          );
        })}
        <td className="py-2">
          <StateBadge state={skill.state} />
        </td>
      </tr>
      {isExpanded && (
        <tr className="border-b border-slate-100 last:border-0">
          <td colSpan={agentColumns.length + 2} className="py-3">
            <DriftExplanationPanel element={skill} planCommand="skills:plan" cellKind="skills" />
          </td>
        </tr>
      )}
    </>
  );
}
