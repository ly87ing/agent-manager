import { useState } from "react";
import { Plus } from "lucide-react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import { COL_MIDDLE, COL_NARROW, DataTable, ROW, TableHeader } from "../components/DataTable";
import { EmptyState } from "../components/EmptyState";
import { PageError, PageLoading } from "../components/PageState";
import { RemoveCollectionDialog } from "../components/RemoveCollectionDialog";
import { AddSourceDialog, RemoveSourceDialog, useRuleSources, type UserSource } from "../components/RuleSources";
import { SkillCollectionFormDialog } from "../components/SkillCollectionFormDialog";
import type { EngineElement, PolicyData, SkillRecommendation, StatusData } from "../lib/contract";
import { runEngine } from "../lib/engine";
import type { SkillCollectionObserved, SkillCollectionSpec, SkillSpec } from "../lib/observed";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { dismissRecommendation, loadDismissed } from "../lib/recommendDismiss";
import { availableSkills, fullSkillDefinition, isCollection } from "../lib/skillCollections";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

const TEXT_BUTTON = "rounded px-2 py-1 text-xs text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-40";

/**
 * 来源：技能集合（git 仓库或本机目录，提供技能、规则与推荐命令）与「我的规则来源」放在一处，
 * 添加、更新、编辑、停用、移除都是看得见的文字按钮。各页里的同名入口照旧可用。
 */
export default function SourcesPage() {
  const skills = useEngineQuery<StatusData>("skills:status");
  const rules = useEngineQuery<StatusData>("rules:status");
  const policy = useEngineQuery<PolicyData>("policy:get");
  const userSources = useRuleSources();
  const directApply = useDirectApply();
  const queryClient = useQueryClient();
  const definitionOps = useDefinitionOps({ section: "skills", domain: "skills" });
  const [adding, setAdding] = useState<"collection" | "rule" | null>(null);
  const [editing, setEditing] = useState<EngineElement | null>(null);
  const [removingCollection, setRemovingCollection] = useState<EngineElement | null>(null);
  const [removingSource, setRemovingSource] = useState<UserSource | null>(null);
  const [dismissed, setDismissed] = useState(loadDismissed);
  const [draft, setDraft] = useState<SkillRecommendation | null>(null);

  if (skills.isPending) return <PageLoading />;
  if (skills.isError) return <PageError title="来源读取失败" error={skills.error} />;

  const elements = skills.data.ok ? skills.data.data.elements ?? [] : [];
  const collections = elements.filter(isCollection);
  const recommendations = skills.data.ok ? skills.data.data.recommendations ?? [] : [];
  const sourceRules = rules.data?.ok ? rules.data.data.source_rules ?? [] : [];
  const layers = policy.data?.ok ? policy.data.data.layers : null;
  const nameOf = (collection: EngineElement) => collection.id.replace(/^skill:/, "");
  const skillCount = (name: string) => elements.filter((element) => (element.spec as SkillSpec).collection === name).length;
  const ruleCount = (name: string) => sourceRules.filter((rule) => rule.collection === name).length;
  const agentIds = Array.from(
    new Set(elements.flatMap((element) => Object.keys(((element.observed as { agents?: Record<string, unknown> }).agents ?? {})))),
  ).sort();

  /** 停用或移除的是推荐来源：视为不想用了，之后不再推荐。 */
  const dismissIfRecommended = (collection: EngineElement) => {
    const spec = collection.spec as SkillCollectionSpec;
    const match = recommendations.find((item) => item.name === nameOf(collection) || (spec.url && item.url === spec.url));
    if (match) setDismissed(dismissRecommendation(match.name));
  };
  const suggested = recommendations.filter((item) => !item.added && !dismissed.includes(item.name));
  const collectionAction = (collection: EngineElement, action: "update" | "enable" | "disable") => {
    const verb = { update: "更新", enable: "启用", disable: "停用" }[action];
    if (action === "disable") dismissIfRecommended(collection);
    void directApply
      .apply({ command: "skills:apply", label: `${verb}来源 ${nameOf(collection)}`, args: ["--only", collection.id, "--action", action] })
      .then((envelope) => {
        // 拉完代码顺手重查一次最新提交，否则还挂着「可更新」
        if (action === "update" && envelope?.ok) {
          void runEngine("system:refresh-upstream", ["--only", collection.id])
            .catch(() => undefined)
            .finally(() => void queryClient.invalidateQueries({ queryKey: engineKeys.all }));
        }
      });
  };

  const collectionStatus = (collection: EngineElement): { text: string; tone: "ok" | "warn" | "muted" | "info" } => {
    const observed = collection.observed as SkillCollectionObserved;
    const spec = collection.spec as SkillCollectionSpec;
    if ((collection.policy as { enabled?: boolean }).enabled === false) return { text: "已停用", tone: "muted" };
    if (!observed.repo_cached) return { text: spec.source_path ? "目录不存在" : "尚未下载", tone: "warn" };
    if (collection.freshness.state === "behind") return { text: "可更新", tone: "info" };
    return { text: "正常", tone: "ok" };
  };
  const TONE = { ok: "text-success", warn: "text-warning", muted: "text-muted-foreground", info: "text-info" } as const;
  const total = collections.length + userSources.sources.length;

  return (
    <div className="space-y-3">
      <h1 className="sr-only">来源</h1>
      <DataTable label="来源" testId="sources-page">
        <TableHeader title={`来源 · ${total}`} middle="提供" narrow="状态">
          <button type="button" data-testid="source-add-collection" onClick={() => setAdding("collection")} className="inline-flex items-center gap-1 text-primary hover:underline">
            <Plus className="h-3.5 w-3.5" aria-hidden /> git 仓库或目录
          </button>
          <button type="button" data-testid="source-add-rule" onClick={() => setAdding("rule")} className="inline-flex items-center gap-1 text-primary hover:underline">
            <Plus className="h-3.5 w-3.5" aria-hidden /> 规则文件或目录
          </button>
        </TableHeader>
        {suggested.map((item) => (
          <div key={item.name} data-testid={`source-suggest-${item.name}`} className={cn(ROW, "border-b bg-primary/5")}>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2 text-sm font-medium">
                {item.name}
                <span className="rounded bg-primary/15 px-1.5 py-0.5 text-[11px] font-normal text-primary">推荐</span>
              </div>
              <div className="truncate text-xs text-muted-foreground">{item.description}</div>
            </div>
            <div className="flex w-60 shrink-0 items-center justify-end gap-1">
              <button type="button" data-testid={`source-suggest-dismiss-${item.name}`} onClick={() => setDismissed(dismissRecommendation(item.name))} className={TEXT_BUTTON}>
                不用了
              </button>
              <button type="button" data-testid={`source-suggest-add-${item.name}`} onClick={() => setDraft(item)} className="rounded-md bg-primary px-2.5 py-1 text-xs text-primary-foreground hover:bg-primary/90">
                添加…
              </button>
            </div>
          </div>
        ))}
        {total === 0 ? (
          <EmptyState compact title="还没有来源" description="添加一个 git 仓库或本机目录（提供技能、规则和推荐的启动命令），或一个规则文件、规则目录。" testId="sources-empty" />
        ) : (
          <ul className="divide-y">
            {collections.map((collection) => {
              const name = nameOf(collection);
              const spec = collection.spec as SkillCollectionSpec;
              const observed = collection.observed as SkillCollectionObserved;
              const enabled = (collection.policy as { enabled?: boolean }).enabled !== false;
              const status = collectionStatus(collection);
              return (
                <li key={collection.id} data-testid={`source-row-${name}`} className={ROW}>
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium">{name}</div>
                    <div className="truncate font-mono text-xs text-muted-foreground" title={spec.source_path ?? spec.url ?? undefined}>
                      {spec.source_path ? `本机目录 · ${spec.source_path}` : `git · ${spec.url ?? ""}`}
                    </div>
                  </div>
                  <span className={cn(COL_MIDDLE, "text-sm text-muted-foreground")}>
                    {skillCount(name)} 个技能 · {ruleCount(name)} 条规则
                  </span>
                  <span className={cn(COL_NARROW, "text-xs", TONE[status.tone])}>{status.text}</span>
                  <div className="flex w-60 shrink-0 items-center justify-end gap-1">
                    {(observed.actions ?? []).includes("update") && enabled && (
                      <button type="button" data-testid={`source-update-${name}`} disabled={directApply.executing} onClick={() => collectionAction(collection, "update")} className={cn(TEXT_BUTTON, status.tone === "info" || status.tone === "warn" ? "text-primary" : "")}>
                        {spec.source_path ? "重新扫描" : "更新"}
                      </button>
                    )}
                    <button type="button" data-testid={`source-edit-${name}`} onClick={() => setEditing(collection)} className={TEXT_BUTTON}>
                      编辑
                    </button>
                    <button
                      type="button"
                      data-testid={`source-toggle-${name}`}
                      disabled={directApply.executing}
                      onClick={() => collectionAction(collection, enabled ? "disable" : "enable")}
                      title={enabled ? "停用：各账号里它的技能链接先移除，规则暂不出现；重新启用即恢复" : "启用：按之前的分配重新链接"}
                      className={TEXT_BUTTON}
                    >
                      {enabled ? "停用" : "启用"}
                    </button>
                    <button type="button" data-testid={`source-remove-${name}`} onClick={() => setRemovingCollection(collection)} className={cn(TEXT_BUTTON, "hover:bg-destructive/10 hover:text-destructive")}>
                      移除
                    </button>
                  </div>
                </li>
              );
            })}
            {userSources.sources.map((source) => {
              const count = ruleCount(source.name);
              return (
                <li key={`rule:${source.name}`} data-testid={`source-row-${source.name}`} className={ROW}>
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium">{source.name}</div>
                    <div className="truncate font-mono text-xs text-muted-foreground" title={source.path}>规则 · {source.path}</div>
                  </div>
                  <span className={cn(COL_MIDDLE, "text-sm text-muted-foreground")}>{source.enabled ? `${count} 条规则` : "—"}</span>
                  <span className={cn(COL_NARROW, "text-xs", !source.enabled ? TONE.muted : count ? TONE.ok : TONE.warn)}>
                    {!source.enabled ? "已停用" : count ? "正常" : "没读到"}
                  </span>
                  <div className="flex w-60 shrink-0 items-center justify-end gap-1">
                    <button
                      type="button"
                      data-testid={`source-toggle-${source.name}`}
                      disabled={userSources.busy}
                      onClick={() => void userSources.setEnabled(source, !source.enabled)}
                      title={source.enabled ? "停用：它的规则暂不出现，已写进账号的规则文件不动" : "启用"}
                      className={TEXT_BUTTON}
                    >
                      {source.enabled ? "停用" : "启用"}
                    </button>
                    <button type="button" data-testid={`source-remove-${source.name}`} onClick={() => setRemovingSource(source)} className={cn(TEXT_BUTTON, "hover:bg-destructive/10 hover:text-destructive")}>
                      移除
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </DataTable>
      <p className="px-1 text-xs text-muted-foreground">
        git 仓库或本机目录提供技能，也可以带规则（默认在 rules/ 里）和推荐的启动命令；规则文件或目录只提供规则。停用可以随时再启用，移除后想用可以重新添加。
      </p>

      {(adding === "collection" || editing || draft) && (
        <SkillCollectionFormDialog
          agentIds={agentIds}
          draft={draft ? { name: draft.name, definition: draft.definition } : undefined}
          initial={editing ? { name: nameOf(editing), definition: editing.spec } : undefined}
          fullDefinition={editing ? fullSkillDefinition(layers, nameOf(editing)) : null}
          available={editing ? availableSkills(editing.observed as SkillCollectionObserved) : undefined}
          repoCached={editing ? (editing.observed as SkillCollectionObserved).repo_cached : undefined}
          onClose={() => {
            setAdding(null);
            setEditing(null);
            setDraft(null);
          }}
          onSubmit={(name, definition, complete) => {
            const isNew = !editing;
            setAdding(null);
            setEditing(null);
            setDraft(null);
            void definitionOps.saveDefinition({ name, definition, isNew, complete }).then((ok) => {
              if (ok && isNew) toast.info(`来源 ${name} 已添加，点「更新」取回内容。`);
            });
          }}
        />
      )}
      {definitionOps.modals}
      {adding === "rule" && (
        <AddSourceDialog
          taken={userSources.taken}
          busy={userSources.busy}
          onClose={() => setAdding(null)}
          onSubmit={async (name, path) => {
            if (await userSources.add(name, path)) setAdding(null);
          }}
        />
      )}
      {removingCollection && (
        <RemoveCollectionDialog
          name={nameOf(removingCollection)}
          local={Boolean((removingCollection.spec as SkillCollectionSpec).source_path)}
          executing={directApply.executing}
          onConfirm={(purge) => {
            const collection = removingCollection;
            setRemovingCollection(null);
            dismissIfRecommended(collection);
            void directApply.apply({
              command: "skills:apply",
              label: `移除来源 ${nameOf(collection)}`,
              args: ["--only", collection.id, "--action", "remove", ...(purge ? ["--purge"] : [])],
            });
          }}
          onClose={() => setRemovingCollection(null)}
        />
      )}
      {removingSource && (
        <RemoveSourceDialog
          source={removingSource}
          busy={userSources.busy}
          onConfirm={async () => {
            if (await userSources.remove(removingSource)) setRemovingSource(null);
          }}
          onClose={() => setRemovingSource(null)}
        />
      )}
    </div>
  );
}
