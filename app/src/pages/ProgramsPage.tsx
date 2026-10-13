import { t } from "../lib/i18n";
import { useEffect, useRef, useState } from "react";
import { RefreshCw, ScanSearch } from "lucide-react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import { ActionMenu } from "../components/AccountGrid";
import { EmptyState } from "../components/EmptyState";
import { PageError, PageLoading } from "../components/PageState";
import { ProgramActionDialog, type ProgramActionKind } from "../components/ProgramActionDialog";
import { ProgramPinDialog } from "../components/ProgramPinDialog";
import { StateBadge } from "../components/StateBadge";
import { runEngine } from "../lib/engine";
import { useExecutions } from "../lib/execution";
import type { InboxState } from "../lib/inbox";
import { MODE_HINT, MODE_LABEL, displayName, ownerLabel, upgradeTarget, type ProgramElement, type ProgramsData } from "../lib/programs";
import { engineKeys, useCheckingUpdates, useEngineQuery, useRefreshUpstream } from "../lib/queries";
import { cn } from "../lib/utils";

const TEXT_BUTTON = "rounded px-2 py-1 text-xs text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-40";
const PRIMARY = "min-w-0 break-words [overflow-wrap:anywhere] rounded-md border px-2.5 py-1.5 text-xs font-medium hover:bg-accent disabled:opacity-40";
const GRID = "grid grid-cols-2 items-center gap-x-4 gap-y-3 md:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)_minmax(0,1fr)_minmax(9rem,1fr)]";

/** 程序的问题 → 统一状态标签。 */
function problemBadges(program: ProgramElement): { state: InboxState; text: string }[] {
  const out: { state: InboxState; text: string }[] = [];
  for (const problem of program.problems) {
    if (problem.code === "missing") out.push({ state: "missing", text: t("缺 {0}", (problem.bins ?? []).join("、")) });
    if (problem.code === "missing-no-install") out.push({ state: "missing", text: t("未提供安装方式") });
    if (problem.code === "runner-missing") out.push({ state: "broken", text: t("找不到 {0}", problem.bin) });
    if (problem.code === "multiple") out.push({ state: "drifted", text: t("{0} 有多份", problem.bin) });
    if (problem.code === "unused") out.push({ state: "cleanup", text: t("无扩展依赖") });
  }
  if (program.problems.some((problem) => problem.code === "unpinned")) out.push({ state: "update", text: t("未固定版本") });
  else if (program.runners.some((runner) => runner.follow_latest && !runner.pinned)) out.push({ state: "unmanaged", text: t("跟最新版本") });
  else if (program.freshness?.state === "behind") out.push({ state: "update", text: t("可更新") });
  return out;
}

function severity(program: ProgramElement): number {
  const states = problemBadges(program).map((badge) => badge.state);
  const order: InboxState[] = ["broken", "drifted", "missing", "update", "cleanup", "unmanaged"];
  const ranks = states.map((state) => order.indexOf(state));
  return ranks.length ? Math.min(...ranks) : order.length;
}

async function copy(text: string, what: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(t("已复制{0}", what));
  } catch (reason) {
    toast.error(t("没能复制：{0}", reason instanceof Error ? reason.message : String(reason)));
  }
}

/**
 * 程序：技能与 MCP 依赖的命令行程序（docs/PROGRAM-LIFECYCLE.md）。一行一个程序：谁需要它、装在哪、
 * 什么版本、有什么问题；动作按管理方式给——代管的调用原包管理器升级、回退，本工具安装的换版本、删除，
 * 按需拉取的改 MCP 条目里的版本号，只观察的给命令。每个动作都先看计划，也都能复制命令自己执行。
 * 智能体本身（Claude Code、Codex）不在这里。
 */
export default function ProgramsPage({ focus, onOpen }: { focus?: string | null; onOpen?: (target: string) => void } = {}) {
  const query = useEngineQuery<ProgramsData>("programs:status");
  const refresh = useRefreshUpstream();
  // 检查新版本进行中（本页或命令面板发起）：可升级到的版本还没定，先不让改动
  const checking = useCheckingUpdates();
  const { execute } = useExecutions();
  const queryClient = useQueryClient();
  const [dialog, setDialog] = useState<{ program: ProgramElement; action: ProgramActionKind; version?: string; backup?: string } | null>(null);
  const [pin, setPin] = useState<{ program: ProgramElement; mode: "pin" | "unpin" } | null>(null);
  const [rescanning, setRescanning] = useState(false);
  const focused = useRef(false);

  useEffect(() => {
    if (!focus || focused.current || !query.data) return;
    focused.current = true;
    document.querySelector(`[data-program-anchor="${CSS.escape(focus)}"]`)?.scrollIntoView?.({ block: "center" });
  }, [focus, query.data]);

  if (query.isPending) return <PageLoading text={t("正在查找技能与 MCP 用到的程序…")} />;
  if (query.isError) return <PageError title={t("依赖程序读取失败")} error={query.error} />;
  if (!query.data.ok) return <PageError title={t("依赖程序读取失败")} error={new Error(query.data.errors[0]?.message ?? t("读取失败"))} />;
  const data = query.data.data;
  const programs = [...data.elements].sort((a, b) => severity(a) - severity(b) || a.name.localeCompare(b.name));

  const rescan = async () => {
    setRescanning(true);
    try {
      await runEngine("programs:status", ["--rescan"]);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    } finally {
      setRescanning(false);
    }
  };

  const declare = async (skill: string, bin: string) => {
    const existing = data.elements.flatMap((program) => program.dependents.filter((dep) => dep.kind === "skill" && dep.name === skill && dep.source === "user").flatMap((dep) => dep.bins));
    const bins = [...new Set([...existing, bin])];
    await execute({ command: "programs:declare", label: t("记下 {0} 需要 {1}", skill, bin), args: ["--skill", skill, "--bins", JSON.stringify(bins)] });
    await queryClient.invalidateQueries({ queryKey: engineKeys.all });
  };

  const actionsOf = (program: ProgramElement) => {
    const menu: { label: string; testId: string; danger?: boolean; onSelect: () => void }[] = [];
    let primary: { label: string; testId: string; run: () => void } | null = null;
    const backups = data.backups.filter((backup) => backup.program === program.program);
    const target = upgradeTarget(program);
    const installable = ["npm", "pypi", "git"].includes(program.kind);
    switch (program.mode) {
      case "missing":
        if (installable) primary = { label: t("安装依赖"), testId: `program-install-${program.program}`, run: () => setDialog({ program, action: "install" }) };
        break;
      case "delegated":
        if (target) primary = { label: t("升级到 {0}", target), testId: `program-upgrade-${program.program}`, run: () => setDialog({ program, action: "upgrade", version: target ?? undefined }) };
        for (const backup of backups.filter((item) => item.reason !== "adopt")) {
          menu.push({ label: t("回退到 {0}", backup.version ?? t("上一版")), testId: `program-restore-${backup.id}`, onSelect: () => setDialog({ program, action: "restore", backup: backup.id }) });
        }
        if (program.kind === "npm" && program.owner === "npm-global") {
          menu.push({ label: t("改由本工具安装"), testId: `program-adopt-${program.program}`, onSelect: () => setDialog({ program, action: "adopt" }) });
        }
        break;
      case "managed":
        if (target) primary = { label: t("升级到 {0}", target), testId: `program-upgrade-${program.program}`, run: () => setDialog({ program, action: "upgrade", version: target ?? undefined }) };
        for (const version of program.managed_versions.filter((item) => item !== program.managed_current)) {
          menu.push({ label: t("切换到 {0}", version), testId: `program-switch-${version}`, onSelect: () => setDialog({ program, action: "switch", version }) });
          menu.push({ label: t("删除旧版本 {0}", version), testId: `program-remove-version-${version}`, danger: true, onSelect: () => setDialog({ program, action: "remove", version }) });
        }
        for (const backup of backups.filter((item) => item.reason === "adopt")) {
          menu.push({ label: t("撤回：改回原来的安装"), testId: `program-unadopt-${program.program}`, onSelect: () => setDialog({ program, action: "unadopt", backup: backup.id }) });
        }
        menu.push({ label: t("移除依赖程序"), testId: `program-remove-${program.program}`, danger: true, onSelect: () => setDialog({ program, action: "remove" }) });
        break;
      case "runner":
        if (program.runners.length) {
          const unpinned = program.runners.some((runner) => !runner.pinned);
          primary = {
            label: unpinned ? t("固定版本") : target ? t("升级到 {0}", target) : t("修改版本"),
            testId: `program-pin-${program.program}`,
            run: () => setPin({ program, mode: "pin" }),
          };
          // 已固定的可以取消固定、改回跟最新版本（检查新版本照常提示）
          if (program.runners.some((runner) => runner.pinned)) {
            menu.push({ label: t("取消固定并使用最新版本"), testId: `program-unpin-${program.program}`, onSelect: () => setPin({ program, mode: "unpin" }) });
          }
        }
        break;
    }
    for (const [key, label] of [["install", t("安装命令")], ["upgrade", t("升级命令")], ["uninstall", t("卸载命令")]] as const) {
      const command = program.commands[key];
      if (command && program.mode !== "runner") menu.push({ label: t("复制{0}", label), testId: `program-copy-${key}-${program.program}`, onSelect: () => void copy(command, label) });
    }
    return { primary, menu };
  };

  return (
    <div className="space-y-3">
      <h1 className="text-lg font-semibold">{(t("依赖程序") + " ")}<span className="ml-1 text-sm font-normal text-muted-foreground">{programs.length}</span></h1>
      <div className="flex flex-wrap items-center justify-end gap-2">
        <div className="mr-auto min-w-0 break-all text-xs text-muted-foreground">{t("入口目录：")}<span className="font-mono">{data.entry_dir ?? t("首次安装时选择")}</span>
          {data.user_path.error && <details className="mt-1 text-warning"><summary className="w-fit cursor-pointer">{t("PATH 读取失败")}</summary><p className="mt-1">{data.user_path.error}</p></details>}
        </div>
        <button type="button" data-testid="programs-rescan" disabled={rescanning} onClick={() => void rescan()} className="inline-flex items-center gap-1 rounded-md border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40">
          <ScanSearch className="h-4 w-4" aria-hidden />{(" " + t("重新扫描"))}</button>
        <button type="button" data-testid="programs-check-updates" disabled={checking} onClick={() => refresh.mutate(undefined)} className="inline-flex items-center gap-1 rounded-md border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40">
          <RefreshCw className={cn("h-4 w-4", checking && "animate-spin")} aria-hidden /> {checking ? t("正在检查…") : t("检查新版本")}
        </button>
      </div>

      {checking && (
        <p data-testid="programs-checking" className="rounded-md border border-info/30 bg-info/5 px-3 py-2 text-xs text-info">{t("检查中，暂不可安装或改版本。")}</p>
      )}

      {programs.length === 0 ? (
        <EmptyState testId="programs-empty" title={t("暂无依赖程序")} description={t("技能和 MCP 的依赖会显示在这里。")} />
      ) : (
        <section aria-label={t("程序")} data-testid="programs-list" className="overflow-clip rounded-lg border bg-card">
          <div className={cn(GRID, "hidden border-b bg-muted/50 px-4 py-2.5 text-xs text-muted-foreground md:grid")}>
            <span>{t("程序 / 来源")}</span>
            <span>{t("依赖此程序")}</span>
            <span>{t("版本与状态")}</span>
            <span className="text-right">{t("操作")}</span>
          </div>
          <ul className="divide-y">
            {programs.map((program) => {
              const { primary, menu } = actionsOf(program);
              const install = program.installs[0];
              const runners = [...new Set(program.runners.map((runner) => runner.runner))].join(" / ");
              // 第二行小字：提供的命令与装在哪（按需拉取的写运行方式与完整来源）
              const where = program.mode === "runner"
                ? `${runners} · ${program.name}`
                : [program.all_bins.length ? t("命令 {0}", program.all_bins.join("、")) : null, install ? `${ownerLabel(install)} · ${install.path}` : null].filter(Boolean).join(" · ");
              const current = program.mode === "runner"
                ? [...new Set(program.runners.map((runner) => runner.version || t("未指定版本")))].join("、")
                : program.version ?? program.managed_current;
              return (
                <li
                  key={program.program}
                  data-testid={`program-row-${program.program}`}
                  data-program-anchor={program.program}
                  className={cn(GRID, "px-4 py-3 hover:bg-accent/30", focus === program.program && "ring-2 ring-inset ring-primary/40")}
                >
                  <div className="col-span-2 min-w-0 space-y-1.5 md:col-span-1">
                    <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                      <span className="break-words [overflow-wrap:anywhere] text-sm font-medium" title={program.program}>{displayName(program)}</span>
                      <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground" title={MODE_HINT[program.mode]}>{MODE_LABEL[program.mode]}</span>
                    </div>
                    <div className="break-words [overflow-wrap:anywhere] font-mono text-xs text-muted-foreground" title={install?.real ?? where}>{where || "—"}</div>
                  </div>
                  <div
                    className={"col-span-2 flex min-w-0 flex-col items-start gap-1 text-xs text-muted-foreground md:col-span-1"}
                    title={program.dependents.map((dep) => `${dep.kind === "skill" ? t("技能") : "MCP"} ${dep.name}`).join("\n")}
                  >
                    {program.dependents.length === 0 && t("无扩展依赖")}
                    {program.dependents.map((dep) => (
                      <span className="max-w-full" key={`${dep.kind}:${dep.name}`}>
                        <button
                          type="button"
                          data-testid={`program-dependent-${program.program}-${dep.name}`}
                          onClick={() => onOpen?.(dep.kind === "skill" ? `skills:${dep.name}` : `mcp:${dep.name}`)}
                          title={`${dep.kind === "skill" ? t("技能") : "MCP"} ${dep.name}`}
                          className="block max-w-full break-words [overflow-wrap:anywhere] text-left hover:text-foreground hover:underline"
                        >
                          {dep.kind === "skill" ? (t("技能") + " ") : "MCP "}
                          {dep.name}
                        </button>
                      </span>
                    ))}
                  </div>
                  <div className="min-w-0 space-y-1.5">
                    <div className="break-words [overflow-wrap:anywhere] font-mono text-xs" title={current || undefined}>{current || "—"}</div>
                    <div className="flex flex-wrap gap-1">
                      {["delegated", "managed"].includes(program.mode) && !upgradeTarget(program) && (
                        <span className="text-xs text-muted-foreground">
                          {program.freshness.state === "current" ? t("已是最新") : program.freshness.state === "ahead" ? t("本地较新") : program.freshness.checked_at ? t("版本未知") : t("未检查")}
                        </span>
                      )}
                      {problemBadges(program).map((badge) => (
                        <StateBadge key={badge.text} state={badge.state}>{badge.text}</StateBadge>
                      ))}
                    </div>
                  </div>
                  <div className="flex items-center justify-end gap-1">
                    {primary && (
                      <button
                        type="button"
                        data-testid={primary.testId}
                        disabled={checking}
                        title={checking ? t("正在检查新版本，完成后再操作") : undefined}
                        onClick={primary.run}
                        className={PRIMARY}
                      >
                        {primary.label}
                      </button>
                    )}
                    {menu.length > 0 && !checking ? <ActionMenu label={t("{0} 的更多操作", displayName(program))} items={menu} /> : <span className="w-7" />}
                  </div>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      {data.suggestions.length > 0 && (
        <section aria-label={t("可能用到但没声明的程序")} data-testid="programs-suggestions" className="overflow-clip rounded-lg border bg-card">
          <div className="space-y-1 border-b bg-muted/30 px-4 py-3 text-xs text-muted-foreground">
            <h2 className="text-sm font-medium text-foreground">{(t("待确认的依赖 ·") + " ")}{data.suggestions.length}</h2>
            <p>{t("仅记录依赖，不修改技能文件。")}</p>
          </div>
          <ul className="divide-y">
            {data.suggestions.map((item) => (
              <li key={`${item.skill}:${item.bin}`} data-testid={`program-suggestion-${item.skill}-${item.bin}`} className="flex flex-wrap items-start gap-3 px-4 py-3 text-sm">
                <div className="min-w-0 flex-1 basis-48">
                  <div className="break-words">{(t("技能") + " ")}{item.skill}{(" " + t("用到") + " ")}<span className="font-mono">{item.bin}</span>
                    {!item.found && <span className="ml-2 text-xs text-muted-foreground">{t("（未找到命令）")}</span>}
                  </div>
                  <details className="mt-1.5 text-xs text-muted-foreground">
                    <summary className="w-fit cursor-pointer hover:text-foreground">{t("查看原文")}</summary>
                    <p className="mt-2 whitespace-pre-wrap break-words rounded-md bg-muted/50 p-2 font-mono">{item.line}</p>
                  </details>
                </div>
                <button type="button" data-testid={`program-declare-${item.skill}-${item.bin}`} onClick={() => void declare(item.skill, item.bin)} className={TEXT_BUTTON}>{t("确认依赖")}</button>
              </li>
            ))}
          </ul>
        </section>
      )}

      <details className="px-1 text-xs leading-relaxed text-muted-foreground">
        <summary className="w-fit cursor-pointer hover:text-foreground">{t("管理方式说明")}</summary>
        <p className="mt-2">{t("代管：原地升级，自动备份，失败恢复。工具安装：多版本保存，可切换。按需拉取：管理 MCP 启动版本。各操作均可复制命令。")}</p>
      </details>

      {dialog && (
        <ProgramActionDialog
          program={dialog.program}
          action={dialog.action}
          version={dialog.version}
          backup={dialog.backup}
          entryCandidates={data.user_path.entry_candidates}
          entryDir={data.entry_dir}
          onClose={() => setDialog(null)}
        />
      )}
      {pin && (
        <ProgramPinDialog
          program={pin.program}
          mode={pin.mode}
          initialVersion={upgradeTarget(pin.program) ?? pin.program.freshness?.upstream ?? pin.program.runners.find((runner) => runner.pinned)?.version ?? null}
          onClose={() => setPin(null)}
        />
      )}
    </div>
  );
}
