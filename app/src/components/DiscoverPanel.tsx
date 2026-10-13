import { t } from "../lib/i18n";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { FolderSymlink, Link2Off, PackageCheck, RefreshCw, ShieldCheck, Sparkles } from "lucide-react";
import { ConfirmModal } from "./ConfirmModal";
import { PageError } from "./PageState";
import { ErrorPanel } from "./ErrorPanel";
import type {
  DiscoverData,
  DiscoverSkillGroup,
  DiscoverSkillItem,
} from "../lib/contract";
import { useEngineQuery } from "../lib/queries";
import { useTakeover, type TakeoverOutcome, type TakeoverTask } from "../lib/useTakeover";
import { cn } from "../lib/utils";

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
  icon: typeof FolderSymlink;
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

/** Discover external skill directories and removable links without scanning accounts. */
export function DiscoverPanel() {
  const query = useEngineQuery<DiscoverData>("system:discover", ["--only", "skills"]);
  const takeover = useTakeover();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirming, setConfirming] = useState<string[] | null>(null);
  const [failed, setFailed] = useState<TakeoverOutcome["failures"]>([]);

  const data = query.data?.ok ? query.data.data : null;
  const skillItems = data?.skills?.items ?? [];
  const skillGroups = data?.skills?.groups ?? [];

  const extraLinks = skillItems.filter((item) => item.manage === "collection");
  const nativeItems = skillItems.filter((item) => item.manage === "native");
  const deadLinks = skillItems.filter((item) => item.manage === "cleanup");

  const tasks = useMemo<Map<string, TakeoverTask>>(() => {
    const map = new Map<string, TakeoverTask>();
    for (const group of skillGroups) {
      const steps: TakeoverTask["steps"] = [];
      if (!group.existing_collection) {
        steps.push({
          kind: "policy",
          key: `skills.repositories.${group.name}`,
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
      map.set(`skill-group:${group.name}`, { key: `skill-group:${group.name}`, label: t("技能来源 {0}", group.name), steps });
    }
    for (const item of extraLinks) {
      if (!item.collection) continue;
      const key = `skill-extra:${item.collection}/${item.name}@${item.agent}`;
      map.set(key, {
        key,
        label: t("技能 {0} @ {1}", item.name, item.agent),
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
        label: t("清理链接 {0} @ {1}", item.name, item.agent),
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
    return map;
  }, [skillGroups, extraLinks, deadLinks]);

  const toggle = (key: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const selectableKeys = [...tasks.keys()];
  const takeoverKeys = selectableKeys.filter((key) => !key.startsWith("skill-cleanup:"));
  const selectedTakeoverKeys = takeoverKeys.filter((key) => selected.has(key));
  const selectedCleanupKeys = selectableKeys.filter((key) => key.startsWith("skill-cleanup:") && selected.has(key));
  const otherFailures = failed.filter((item) => !item.key.startsWith("skill-cleanup:"));
  const takeoverCount = selectedTakeoverKeys.length;
  /** 携带 --allow-takeover 的任务数：确认文案要显式说明会替换助手里的同名未受管条目。 */
  const replacingCount = (confirming ?? []).filter((key) =>
    tasks.get(key)?.steps.some((step) => step.kind === "apply" && step.args.includes("--allow-takeover")),
  ).length;
  const confirmingCleanup = confirming?.every((key) => key.startsWith("skill-cleanup:")) ?? false;
  // Hide the panel while scanning or when there is nothing to manage.
  if (query.isPending) return null;
  if (query.isError) return <PageError title={t("扫描本机已有内容失败")} error={query.error} />;
  if (tasks.size === 0 && query.data?.ok) return null;

  const runTakeover = async () => {
    const keys = confirming ?? [];
    const list = keys.map((key) => tasks.get(key)).filter((task): task is TakeoverTask => Boolean(task));
    const cleanup = keys.every((key) => key.startsWith("skill-cleanup:"));
    setConfirming(null);
    const outcome = await takeover.run(list, cleanup ? t("清理") : t("接管"));
    if (outcome) {
      const readback = await query.refetch();
      const failures = [...outcome.failures];
      if (readback.isError || !readback.data?.ok || (cleanup && !readback.data.data.skills)) {
        for (const key of outcome.succeededKeys) failures.push({ key, label: tasks.get(key)?.label ?? key, message: t("操作已执行，但重新扫描失败，尚未确认结果") });
      } else if (cleanup) {
        for (const key of outcome.succeededKeys) {
          const item = deadLinks.find((link) => `skill-cleanup:${link.name}@${link.agent}` === key);
          if (item && readback.data.data.skills?.items.some((link) => link.name === item.name && link.agent === item.agent)) {
            failures.push({ key, label: tasks.get(key)?.label ?? key, message: t("重新扫描后链接仍在，请检查后重试") });
          }
        }
      }
      const failedKeys = new Set(failures.map((item) => item.key));
      setFailed(failures);
      setSelected((previous) => new Set([...previous].filter((key) => !keys.includes(key) || failedKeys.has(key)).concat([...failedKeys])));
      const summary = t("{0}完成：成功 {1}，失败 {2}", cleanup ? t("清理") : t("接管"), list.length - failures.length, failures.length);
      if (failures.length) toast.error(summary);
      else toast.success(summary);
    }
  };

  return (
    <div data-testid="discover-skills" className="space-y-3 rounded-lg border bg-card p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="text-sm font-semibold">{t("外部技能目录与残留链接")}</h2>
          <p className="text-xs text-muted-foreground">{t("技能目标目录里本工具之外的链接：外部目录可登记为本机目录来源，计划外链接并回已有来源，残留链接可以清理。")}</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            data-testid="takeover-refresh"
            onClick={() => void query.refetch()}
            className="inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent"
          >
            <RefreshCw className="h-3.5 w-3.5" aria-hidden />{(" " + t("重新扫描"))}</button>
          <button
            type="button"
            data-testid="takeover-select-all"
            onClick={() => setSelected((previous) => new Set([...previous, ...takeoverKeys]))}
            disabled={takeoverKeys.length === 0}
            className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
          >{t("全选可接管项")}</button>
          <button
            type="button"
            data-testid="takeover-run"
            onClick={() => setConfirming(selectedTakeoverKeys)}
            disabled={takeoverCount === 0 || takeover.executing}
            className={cn(
              "inline-flex items-center gap-1 rounded border px-3 py-1 text-xs",
              takeoverCount === 0 || takeover.executing
                ? "cursor-not-allowed border bg-muted text-muted-foreground"
                : "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15",
            )}
          >
            <PackageCheck className="h-3.5 w-3.5" aria-hidden />
            {takeover.executing
              ? takeover.progress
                ? t("正在接管 {0}/{1}…", takeover.progress.done + 1, takeover.progress.total)
                : t("正在接管…")
              : t("一键接管所选（{0}）", takeoverCount)}
          </button>
        </div>
      </div>

      {takeover.progress && (
        <div data-testid="takeover-progress" className="rounded-lg border bg-card p-4 shadow-sm">
          <p className="text-xs text-foreground">{(t("正在处理第") + " ")}{takeover.progress.done + 1} / {takeover.progress.total}{(" " + t("项："))}{takeover.progress.current}
          </p>
          <div className="mt-2 h-1.5 w-full overflow-hidden rounded bg-muted">
            <div
              className="h-full rounded bg-primary transition-[width] duration-200"
              style={{ width: `${(takeover.progress.done / takeover.progress.total) * 100}%` }}
            />
          </div>
          <p className="mt-2 text-[11px] text-muted-foreground">{t("每项逐条执行，失败的那项会跳过、后面的继续；结束后重新扫描并显示未完成项。")}</p>
        </div>
      )}

      {query.data && !query.data.ok && <ErrorPanel title={t("扫描被拒绝")} errors={query.data.errors} />}
      {otherFailures.length > 0 && (
        <div role="alert" className="rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
          <p>{t("以下项目未完成，可保留选择后重试：")}</p>
          {otherFailures.map((item) => <p key={item.key} className="mt-1 break-words">{item.label}：{item.message}</p>)}
        </div>
      )}

      {skillGroups.length > 0 && (
        <SectionCard
          icon={FolderSymlink}
          testId="takeover-skill-groups"
          title={t("技能：外部目录（软链接接管）")}
          description={t("来自第三方工具或自建目录的技能。接管会登记为本地来源集合，助手目录里的链接直接指向源目录——内容仍由原处维护并随其更新，停用只删链接。")}
        >
          {skillGroups.map((group: DiscoverSkillGroup) => (
            <SelectableRow
              key={group.name}
              testId={`takeover-skill-group-${group.name}`}
              checked={selected.has(`skill-group:${group.name}`)}
              onToggle={() => toggle(`skill-group:${group.name}`)}
            >
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="font-mono text-xs font-semibold text-foreground">{group.name}</span>
                <Badge tone="info">{t("软链接接管")}</Badge>
                {group.existing_collection && <Badge tone="muted">{t("并入已有集合")}</Badge>}
                <Badge tone="muted">{group.skills.length}{(" " + t("个技能"))}</Badge>
                <Badge tone="muted">{group.agents.join(" / ")}</Badge>
              </div>
              <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{group.source_path}</p>
              <p className="mt-1 text-[11px] text-muted-foreground">
                {group.skills.map((skill) => skill.name).join("、")}
              </p>
            </SelectableRow>
          ))}
        </SectionCard>
      )}

      {extraLinks.length > 0 && (
        <SectionCard
          icon={Sparkles}
          testId="takeover-skill-extras"
          title={t("技能：已定义集合的计划外链接")}
          description={t("链接名命中已有集合，但没有分配记录。接管走集合通道：纳入管理后由集合统一维护版本。")}
        >
          {extraLinks.map((item: DiscoverSkillItem) => {
            const key = `skill-extra:${item.collection}/${item.name}@${item.agent}`;
            return (
              <SelectableRow key={key} testId={`takeover-skill-extra-${item.name}-${item.agent}`} checked={selected.has(key)} onToggle={() => toggle(key)}>
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="font-mono text-xs font-semibold text-foreground">{item.name}</span>
                  <Badge tone="muted">{item.agent}</Badge>
                  <Badge tone="warning">{t("替换外部同名链接")}</Badge>
                  <Badge tone="muted">{(t("集合") + " ")}{item.collection}</Badge>
                </div>
                <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{item.link_path}</p>
              </SelectableRow>
            );
          })}
        </SectionCard>
      )}

      {nativeItems.length > 0 && (
        <SectionCard
          icon={ShieldCheck}
          testId="takeover-native"
          title={t("原生管理（只标识，不接管）")}
          description={t("位于助手官方技能目录内的真实目录，由助手自身或其安装方案维护。本工具不接管、不覆盖，升级与卸载继续走它们原来的方式。")}
        >
          {nativeItems.map((item) => (
            <div key={`${item.agent}:${item.name}`} data-testid={`takeover-native-${item.name}-${item.agent}`} className="rounded-md border px-3 py-2">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="font-mono text-xs font-semibold text-foreground">{item.name}</span>
                <Badge tone="muted">{item.agent}</Badge>
                <Badge tone="success">{t("原生管理")}</Badge>
              </div>
              <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{item.link_path}</p>
            </div>
          ))}
        </SectionCard>
      )}

      {deadLinks.length > 0 && (
        <SectionCard
          icon={Link2Off}
          testId="takeover-dead-links"
          title={t("可清理的残留链接")}
          description={t("断链或旧体系留下的链接。清理只删链接本身，不会跟随链接删掉源目录；真实目录与仍有分配记录的条目会被拒绝。")}
        >
          <div className="flex justify-end">
            <button type="button" data-testid="cleanup-selected" disabled={selectedCleanupKeys.length === 0 || takeover.executing} onClick={() => setConfirming(selectedCleanupKeys)} className="rounded border border-primary/30 bg-primary/10 px-3 py-1 text-xs text-primary hover:bg-primary/15 disabled:opacity-40">
              {t("清理所选 {0} 项", selectedCleanupKeys.length)}
            </button>
          </div>
          {deadLinks.map((item) => {
            const key = `skill-cleanup:${item.name}@${item.agent}`;
            const failure = failed.find((entry) => entry.key === key);
            return (
              <div key={`${item.agent}:${item.name}`} data-testid={`takeover-dead-${item.name}-${item.agent}`} className="flex flex-wrap items-center gap-3 rounded-md border px-3 py-2 hover:bg-accent/40">
                <label className="flex min-w-0 flex-1 cursor-pointer items-start gap-3">
                  <input type="checkbox" checked={selected.has(key)} onChange={() => toggle(key)} disabled={takeover.executing} className="mt-1 h-3.5 w-3.5" />
                  <span className="min-w-0 flex-1">
                    <span className="flex flex-wrap items-center gap-1.5">
                      <span className="font-mono text-xs font-semibold text-foreground">{item.name}</span>
                      <Badge tone="muted">{item.agent}</Badge>
                      <Badge tone="warning">{item.origin === "dead-link" ? t("目标已丢失") : t("旧体系残留")}</Badge>
                    </span>
                    <span className="mt-1 block break-all font-mono text-[11px] text-muted-foreground">{item.link_path}</span>
                    {item.target && <span className="mt-1 block break-all font-mono text-[11px] text-muted-foreground">{t("原目标：{0}", item.target)}</span>}
                    {failure && <span role="alert" className="mt-1 block break-words text-xs text-warning">{failure.message}</span>}
                  </span>
                </label>
                <button type="button" data-testid={`cleanup-link-${item.name}-${item.agent}`} disabled={takeover.executing} onClick={() => setConfirming([key])} className="w-24 shrink-0 rounded border px-2 py-1 text-xs hover:bg-accent disabled:opacity-40">
                  {t("清理链接")}
                </button>
              </div>
            );
          })}
        </SectionCard>
      )}

      {confirming && (
        <ConfirmModal
          title={confirmingCleanup ? t("清理所选 {0} 项", confirming.length) : t("接管所选 {0} 项", confirming.length)}
          description={confirmingCleanup
            ? t("将删除所选的 {0} 条链接本身，不删除其原目标；真实文件或目录、有分配记录的链接会被拒绝。", confirming.length)
            : [
                t("将保存定义并对可接管项执行真实操作。外部源目录与原生安装的资源不会被删除或改写。"),
                replacingCount > 0 ? t("其中 {0} 项会替换助手里的同名外部条目（接管后由本工具统一维护）。", replacingCount) : "",
              ].filter(Boolean).join("")}
          risk="write-local"
          confirmLabel={confirmingCleanup ? t("清理链接") : t("开始接管")}
          executing={takeover.executing}
          onConfirm={() => void runTakeover()}
          onClose={() => setConfirming(null)}
        >
          {confirmingCleanup && (
            <ul className="mt-3 space-y-1 text-xs text-foreground">
              {confirming.map((key) => {
                const item = deadLinks.find((link) => `skill-cleanup:${link.name}@${link.agent}` === key);
                return item && <li key={key} className="break-all rounded border px-2 py-1"><strong>{item.name} · {item.agent}</strong><span className="block font-mono text-muted-foreground">{item.link_path}</span></li>;
              })}
            </ul>
          )}
        </ConfirmModal>
      )}
    </div>
  );
}
