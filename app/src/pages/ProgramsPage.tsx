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
import { MODE_HINT, MODE_LABEL, displayName, ownerLabel, shortVersion, upgradeTarget, type ProgramElement, type ProgramsData } from "../lib/programs";
import { engineKeys, useCheckingUpdates, useEngineQuery, useRefreshUpstream } from "../lib/queries";
import { cn } from "../lib/utils";

const TEXT_BUTTON = "rounded px-2 py-1 text-xs text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-40";
const PRIMARY = "rounded-md bg-primary px-2.5 py-1 text-xs text-primary-foreground hover:bg-primary/90 disabled:opacity-40";

/** 程序的问题 → 统一状态标签。 */
function problemBadges(program: ProgramElement): { state: InboxState; text: string }[] {
  const out: { state: InboxState; text: string }[] = [];
  for (const problem of program.problems) {
    if (problem.code === "missing") out.push({ state: "missing", text: `缺 ${(problem.bins ?? []).join("、")}` });
    if (problem.code === "missing-no-install") out.push({ state: "missing", text: "不知道怎么装" });
    if (problem.code === "runner-missing") out.push({ state: "broken", text: `找不到 ${problem.bin}` });
    if (problem.code === "multiple") out.push({ state: "drifted", text: `${problem.bin} 有多份` });
    if (problem.code === "unused") out.push({ state: "cleanup", text: "没人用了" });
  }
  if (program.problems.some((problem) => problem.code === "unpinned")) out.push({ state: "update", text: "没固定版本" });
  else if (program.freshness?.state === "behind") out.push({ state: "update", text: `可更新到 ${shortVersion(program.freshness.upstream)}` });
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
    toast.success(`已复制${what}`);
  } catch (reason) {
    toast.error(`没能复制：${reason instanceof Error ? reason.message : String(reason)}`);
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
  const [pin, setPin] = useState<ProgramElement | null>(null);
  const [rescanning, setRescanning] = useState(false);
  const focused = useRef(false);

  useEffect(() => {
    if (!focus || focused.current || !query.data) return;
    focused.current = true;
    document.querySelector(`[data-program-anchor="${CSS.escape(focus)}"]`)?.scrollIntoView?.({ block: "center" });
  }, [focus, query.data]);

  if (query.isPending) return <PageLoading text="正在查找技能与 MCP 用到的程序…" />;
  if (query.isError) return <PageError title="程序读取失败" error={query.error} />;
  if (!query.data.ok) return <PageError title="程序读取失败" error={new Error(query.data.errors[0]?.message ?? "读取失败")} />;
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
    await execute({ command: "programs:declare", label: `记下 ${skill} 需要 ${bin}`, args: ["--skill", skill, "--bins", JSON.stringify(bins)] });
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
        if (installable) primary = { label: "安装…", testId: `program-install-${program.program}`, run: () => setDialog({ program, action: "install" }) };
        break;
      case "delegated":
        primary = { label: target ? `升级到 ${shortVersion(target)}…` : "升级…", testId: `program-upgrade-${program.program}`, run: () => setDialog({ program, action: "upgrade", version: target ?? undefined }) };
        for (const backup of backups.filter((item) => item.reason !== "adopt")) {
          menu.push({ label: `回退到 ${backup.version ?? "上一版"}`, testId: `program-restore-${backup.id}`, onSelect: () => setDialog({ program, action: "restore", backup: backup.id }) });
        }
        if (program.kind === "npm" && program.owner === "npm-global") {
          menu.push({ label: "改为本工具安装…", testId: `program-adopt-${program.program}`, onSelect: () => setDialog({ program, action: "adopt" }) });
        }
        break;
      case "managed":
        primary = { label: target ? `升级到 ${shortVersion(target)}…` : "升级…", testId: `program-upgrade-${program.program}`, run: () => setDialog({ program, action: "upgrade", version: target ?? undefined }) };
        for (const version of program.managed_versions.filter((item) => item !== program.managed_current)) {
          menu.push({ label: `换到 ${version}`, testId: `program-switch-${version}`, onSelect: () => setDialog({ program, action: "switch", version }) });
          menu.push({ label: `删除旧版本 ${version}`, testId: `program-remove-version-${version}`, danger: true, onSelect: () => setDialog({ program, action: "remove", version }) });
        }
        for (const backup of backups.filter((item) => item.reason === "adopt")) {
          menu.push({ label: "撤回：改回原来的安装", testId: `program-unadopt-${program.program}`, onSelect: () => setDialog({ program, action: "unadopt", backup: backup.id }) });
        }
        menu.push({ label: "删除这个程序…", testId: `program-remove-${program.program}`, danger: true, onSelect: () => setDialog({ program, action: "remove" }) });
        break;
      case "runner":
        if (program.runners.length) {
          primary = {
            label: program.runners.some((runner) => !runner.pinned) ? "固定版本…" : target ? `升级到 ${shortVersion(target)}…` : "改版本…",
            testId: `program-pin-${program.program}`,
            run: () => setPin(program),
          };
        }
        break;
    }
    for (const [key, label] of [["install", "安装命令"], ["upgrade", "升级命令"], ["uninstall", "卸载命令"]] as const) {
      const command = program.commands[key];
      if (command && program.mode !== "runner") menu.push({ label: `复制${label}`, testId: `program-copy-${key}-${program.program}`, onSelect: () => void copy(command, label) });
    }
    return { primary, menu };
  };

  return (
    <div className="space-y-3">
      <h1 className="sr-only">程序</h1>
      <div className="flex flex-wrap items-center justify-end gap-2">
        <span className="mr-auto text-xs text-muted-foreground">
          入口目录：<span className="font-mono">{data.entry_dir ?? "还没选（第一次由本工具安装时选）"}</span>
          {data.user_path.error && <span className="ml-2 text-warning">读取你的 PATH 失败：{data.user_path.error}</span>}
        </span>
        <button type="button" data-testid="programs-rescan" disabled={rescanning} onClick={() => void rescan()} className="inline-flex items-center gap-1 rounded-md border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40">
          <ScanSearch className="h-4 w-4" aria-hidden /> 重新扫描
        </button>
        <button type="button" data-testid="programs-check-updates" disabled={checking} onClick={() => refresh.mutate(undefined)} className="inline-flex items-center gap-1 rounded-md border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40">
          <RefreshCw className={cn("h-4 w-4", checking && "animate-spin")} aria-hidden /> {checking ? "正在检查…" : "检查新版本"}
        </button>
      </div>

      {checking && (
        <p data-testid="programs-checking" className="rounded-md border border-info/30 bg-info/5 px-3 py-2 text-xs text-info">
          正在检查新版本：完成前先不能升级、安装或改版本，免得按旧的检查结果操作。
        </p>
      )}

      {programs.length === 0 ? (
        <EmptyState testId="programs-empty" title="技能和 MCP 都没有声明要用的程序" description="技能在 SKILL.md 里声明了要用的命令、或 MCP 用 npx / uvx 启动时，会出现在这里。" />
      ) : (
        <section aria-label="程序" data-testid="programs-list" className="overflow-clip rounded-lg border bg-card">
          <div className="flex items-center gap-4 border-b bg-muted px-4 py-2 text-xs text-muted-foreground">
            <span className="min-w-0 flex-1 font-medium text-foreground">程序 · {programs.length}</span>
            <span className="w-56 shrink-0">装在哪 · 版本</span>
            <span className="w-56 shrink-0 text-right">状态与操作</span>
          </div>
          <ul className="divide-y">
            {programs.map((program) => {
              const { primary, menu } = actionsOf(program);
              const install = program.installs[0];
              return (
                <li
                  key={program.program}
                  data-testid={`program-row-${program.program}`}
                  data-program-anchor={program.program}
                  className={cn("flex items-start gap-4 px-4 py-2 hover:bg-accent/40", focus === program.program && "ring-2 ring-inset ring-primary/40")}
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2 text-sm font-medium">
                      <span className="font-mono">{displayName(program)}</span>
                      <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] font-normal text-muted-foreground" title={MODE_HINT[program.mode]}>{MODE_LABEL[program.mode]}</span>
                    </div>
                    {program.all_bins.length > 0 && (
                      <div className="truncate font-mono text-xs text-muted-foreground" title="这个包提供的全部命令">命令：{program.all_bins.join("、")}</div>
                    )}
                    <div className="mt-0.5 flex flex-wrap gap-1 text-[11px]">
                      {program.dependents.map((dep) => (
                        <button
                          key={`${dep.kind}:${dep.name}`}
                          type="button"
                          data-testid={`program-dependent-${program.program}-${dep.name}`}
                          onClick={() => onOpen?.(dep.kind === "skill" ? `skills:${dep.name}` : `mcp:${dep.name}`)}
                          className="rounded border px-1.5 py-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
                          title={dep.source === "user" ? "你确认的补充声明" : dep.source === "declared" ? "技能自己声明的" : "从 MCP 启动命令看出的"}
                        >
                          {dep.kind === "skill" ? "技能" : "MCP"} {dep.name}
                        </button>
                      ))}
                    </div>
                  </div>
                  <div className="w-56 shrink-0 text-xs text-muted-foreground">
                    {program.mode === "runner" ? (
                      <div>
                        {[...new Set(program.runners.map((runner) => `${runner.runner} · ${runner.version ? shortVersion(runner.version) : "没写版本"}`))].map((text) => (
                          <div key={text} className="font-mono">{text}</div>
                        ))}
                      </div>
                    ) : install ? (
                      <div className="truncate" title={install.real}>
                        {ownerLabel(install)} · <span className="font-mono">{install.path}</span>
                      </div>
                    ) : (
                      <div>—</div>
                    )}
                    {program.version && <div className="font-mono text-foreground">{shortVersion(program.version)}</div>}
                    {program.mode === "managed" && program.managed_versions.length > 1 && (
                      <div>已装：{program.managed_versions.join("、")}</div>
                    )}
                  </div>
                  <div className="flex w-56 shrink-0 flex-col items-end gap-1">
                    <div className="flex flex-wrap justify-end gap-1">
                      {problemBadges(program).map((badge) => (
                        <StateBadge key={badge.text} state={badge.state}>{badge.text}</StateBadge>
                      ))}
                    </div>
                    <div className="flex items-center gap-1">
                      {primary && (
                        <button
                          type="button"
                          data-testid={primary.testId}
                          disabled={checking}
                          title={checking ? "正在检查新版本，完成后再操作" : undefined}
                          onClick={primary.run}
                          className={PRIMARY}
                        >
                          {primary.label}
                        </button>
                      )}
                      {menu.length > 0 && !checking && <ActionMenu label={`${displayName(program)} 的更多操作`} items={menu} />}
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      {data.suggestions.length > 0 && (
        <section aria-label="可能用到但没声明的程序" data-testid="programs-suggestions" className="overflow-clip rounded-lg border bg-card">
          <div className="border-b bg-muted px-4 py-2 text-xs text-muted-foreground">
            <span className="font-medium text-foreground">技能说明里提到、但没声明的命令</span> · 确认后记为这个技能的依赖（只存在本工具里，不改技能文件）
          </div>
          <ul className="divide-y">
            {data.suggestions.map((item) => (
              <li key={`${item.skill}:${item.bin}`} data-testid={`program-suggestion-${item.skill}-${item.bin}`} className="flex items-center gap-3 px-4 py-2 text-sm">
                <div className="min-w-0 flex-1">
                  <div>
                    技能 {item.skill} 用到 <span className="font-mono">{item.bin}</span>
                    {!item.found && <span className="ml-2 text-xs text-muted-foreground">（PATH 上没找到）</span>}
                  </div>
                  <div className="truncate font-mono text-xs text-muted-foreground" title={item.line}>{item.line}</div>
                </div>
                <button type="button" data-testid={`program-declare-${item.skill}-${item.bin}`} onClick={() => void declare(item.skill, item.bin)} className={TEXT_BUTTON}>
                  确认是依赖
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      <p className="px-1 text-xs text-muted-foreground">
        代管：程序留在原处，升级时本工具调用原来的包管理器，改动前整包备份、失败原样恢复、可以回退。本工具安装：装在本工具目录，按版本各一份，换版本只切指针。
        按需拉取：MCP 启动时由 npx / uvx 按条目里的版本拉取，这里管理的是那个版本号。每个动作都能复制命令自己执行。
      </p>

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
      {pin && <ProgramPinDialog program={pin} initialVersion={upgradeTarget(pin) ?? pin.runners.find((runner) => runner.pinned)?.version ?? null} onClose={() => setPin(null)} />}
    </div>
  );
}
