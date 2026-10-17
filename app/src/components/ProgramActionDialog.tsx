import { t } from "../lib/i18n";
import { useEffect, useState } from "react";
import { Copy } from "lucide-react";
import { notifyError, toast } from "../lib/notifications";
import { useQueryClient } from "@tanstack/react-query";
import { ConfirmModal } from "./ConfirmModal";
import { runEngine, engineErrorMessage } from "../lib/engine";
import { useExecutions } from "../lib/execution";
import type { ProgramElement, ProgramPlan, ProgramPlanStep } from "../lib/programs";
import { engineKeys } from "../lib/queries";
import { Select } from "./Select";

export type ProgramActionKind = "install" | "upgrade" | "switch" | "restore" | "remove" | "adopt" | "unadopt";

const TITLE: Record<ProgramActionKind, string> = {
  get install() { return t("安装"); },
  get upgrade() { return t("升级"); },
  get switch() { return t("切换版本"); },
  get restore() { return t("回退"); },
  get remove() { return t("删除"); },
  get adopt() { return t("改由本工具安装"); },
  get unadopt() { return t("撤回纳入"); },
};

/** 需要入口目录的动作（本工具安装的程序要在 PATH 上找得到）。 */
function needsEntryDir(program: ProgramElement, action: ProgramActionKind): boolean {
  if (action === "adopt") return true;
  if (action === "install") return true;
  return action === "upgrade" && (program.mode === "managed" || program.mode === "missing");
}

function stepText(step: ProgramPlanStep): string | null {
  switch (step.kind) {
    case "exec":
      return t("执行 {0}", step.command);
    case "move":
      return t("移动 {0} → {1}", step.from, step.to);
    case "link":
      return t("放链接 {0} → {1}", step.path, step.target);
    case "unlink":
      return t("删除链接 {0}", step.path);
    case "remove":
      return t("删除 {0}", step.path);
    case "current":
      return t("当前版本改为 {0}", step.version);
    case "entry-dir":
      return t("记下入口目录 {0}", step.dir);
    default:
      return null;
  }
}

/**
 * 程序的一个动作：先向引擎要计划（programs:preview，只算不做），列出会做的每一步与「自己在终端里做」的命令，
 * Explicit confirmation executes the plan; failures retain diagnostics and inspection locations.
 */
export function ProgramActionDialog({
  program,
  action,
  version,
  backup,
  entryCandidates,
  entryDir,
  onClose,
}: {
  program: ProgramElement;
  action: ProgramActionKind;
  version?: string;
  backup?: string;
  entryCandidates: string[];
  entryDir: string | null;
  onClose: () => void;
}) {
  const { execute } = useExecutions();
  const queryClient = useQueryClient();
  const withEntry = needsEntryDir(program, action);
  const verifiesUv = program.mode === "delegated" && program.owner === "uv-tool" && ["upgrade", "switch"].includes(action);
  const verifiesManagedPyPi = program.kind === "pypi" && ["managed", "missing"].includes(program.mode) && ["install", "upgrade", "switch"].includes(action);
  const [chosenEntry, setChosenEntry] = useState<string>(entryDir ?? (entryCandidates.includes("~/.local/bin") ? "~/.local/bin" : entryCandidates[0] ?? ""));
  const [allowScripts, setAllowScripts] = useState(false);
  const [plan, setPlan] = useState<ProgramPlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [executing, setExecuting] = useState(false);
  const [failed, setFailed] = useState<{ message: string; unverified: boolean } | null>(null);

  const args = [
    "--program", program.program, "--action", action,
    ...(version ? ["--version", version] : []),
    ...(backup ? ["--backup", backup] : []),
    ...(withEntry && chosenEntry ? ["--entry-dir", chosenEntry] : []),
    ...(allowScripts ? ["--allow-scripts", "true"] : []),
  ];
  const key = args.join(" ");

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setError(null);
    runEngine<ProgramPlan>("programs:preview", args)
      .then((envelope) => {
        if (!alive) return;
        if (envelope.ok) setPlan(envelope.data);
        else {
          setPlan(null);
          setError(envelope.errors[0]?.message ?? t("算不出计划"));
        }
      })
      .catch((reason) => alive && setError(engineErrorMessage(reason)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [key]);

  const confirm = async () => {
    setExecuting(true);
    setFailed(null);
    try {
      const target = plan?.target_version ? ` ${plan.target_version}` : "";
      const envelope = await execute({ command: "programs:apply", label: `${TITLE[action]} ${program.name}${target}`, args, silent: true });
      if (envelope?.ok) {
        const skipped = (envelope.data as ProgramPlan).scripts_skipped ?? [];
        toast.success(t("{0} {1}{2}：完成", TITLE[action], program.name, target), {
          description: skipped.length ? t("这些包的安装脚本没执行：{0}。程序用不了时可以允许后重装。", skipped.join("、")) : undefined,
        });
        onClose();
        // 版本变了：只重新检查这一项，显示的「当前 → 最新」跟实际一致
        void runEngine("system:refresh-upstream", ["--only", program.id])
          .catch(() => undefined)
          .finally(() => void queryClient.invalidateQueries({ queryKey: engineKeys.all }));
      } else {
        const message = envelope?.errors[0]?.message ?? t("没有完成，详见操作记录");
        const unverified = envelope?.errors.some((error) => ["PROGRAM_VERSION_UNVERIFIED", "PROGRAM_ENTRY_UNVERIFIED"].includes(error.code)) ?? false;
        setFailed({ message, unverified });
        if (unverified) {
          toast.warning(t("{0}未完成", TITLE[action]), { description: message });
        } else {
          notifyError(t("{0}失败", TITLE[action]), message);
        }
      }
    } finally {
      setExecuting(false);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

  const copyManual = async () => {
    if (!plan) return;
    try {
      await navigator.clipboard.writeText(plan.manual.join("\n"));
      toast.success(t("已复制命令"));
    } catch (reason) {
      notifyError(t("复制失败"), reason);
    }
  };

  const steps = (plan?.steps ?? []).map(stepText).filter((text): text is string => Boolean(text));
  return (
    <ConfirmModal
      title={`${TITLE[action]} ${program.name}${plan?.target_version ? ` ${plan.target_version}` : version ? ` ${version}` : ""}`}
      description={loading ? t("正在生成操作计划") : error ? "" : verifiesUv
        ? t("确认后执行下列步骤；uv 版本无法核验时保留现场，不自动恢复，并报告需要检查的位置。")
        : verifiesManagedPyPi ? t("确认后执行下列步骤；自管 PyPI 会在完成标记和当前版本选择前核验版本及声明命令入口。无法核验时保留候选目录，并报告检查位置。")
        : t("确认后执行下列步骤；执行失败时尝试恢复，并报告需要检查的备份。")}
      risk="write-local"
      riskLabel={t("改动本机程序")}
      confirmLabel={TITLE[action]}
      confirmDisabled={loading || Boolean(error) || !plan || (withEntry && !chosenEntry)}
      executing={executing}
      error={failed ? `${t("执行未完成，请核对当前状态：")} ${failed.message}${plan?.manual.length && !failed.unverified ? `\n${t("可以复制上面的命令自己执行。")}` : ""}` : null}
      errorTone={failed?.unverified ? "warning" : "error"}
      onConfirm={() => void confirm()}
      onClose={() => !executing && onClose()}
    >
      <div data-testid="program-action-dialog" className="mt-3 space-y-3 text-sm">
        {withEntry && (
          <label className="block space-y-1">
            <span className="text-xs text-muted-foreground">{t("入口目录（在 PATH 上，本工具只在这里放登记过的链接）")}</span>
            <Select
              label={t("命令入口放在哪个目录")}
              testId="program-entry-dir"
              value={chosenEntry}
              onChange={setChosenEntry}
              mono
              className="w-full rounded-md border bg-card px-2 py-1.5 text-xs"
              options={entryCandidates.length === 0
                ? [{ value: "", label: t("（PATH 上没有可用的目录）") }]
                : entryCandidates.map((dir) => ({ value: dir, label: dir }))}
            />
          </label>
        )}
        {program.kind === "npm" && ["install", "upgrade", "adopt"].includes(action) && (
          <label className="flex items-start gap-2 text-xs">
            <input type="checkbox" data-testid="program-allow-scripts" checked={allowScripts} onChange={(event) => setAllowScripts(event.target.checked)} className="mt-0.5" />
            <span>{t("允许安装脚本")}<span className="block text-muted-foreground">{t("npm 包可以带安装时自动执行的脚本；默认不执行（与 pnpm 10、npm 12 一致），程序装好却用不了时再允许")}</span>
            </span>
          </label>
        )}
        {error && <p data-testid="program-plan-error" className="rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">{error}</p>}
        {plan && (
          <>
            {plan.notes.length > 0 && (
              <ul className="list-disc space-y-0.5 pl-5 text-xs text-muted-foreground">
                {plan.notes.map((note) => (
                  <li key={note}>{note}</li>
                ))}
              </ul>
            )}
            <details open className="rounded-md border">
              <summary className="cursor-pointer px-3 py-1.5 text-xs text-muted-foreground">{t("会做的事（")}{steps.length}{(" " + t("步）"))}</summary>
              <ol data-testid="program-plan-steps" className="list-decimal space-y-0.5 border-t px-3 py-2 pl-8 font-mono text-[11px] break-all">
                {steps.map((text, index) => (
                  <li key={index}>{text}</li>
                ))}
              </ol>
            </details>
            {plan.manual.length > 0 && (
              <div className="space-y-1">
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("不想让本工具代办，也可以自己在终端里执行：")}</span>
                  <button type="button" data-testid="program-copy-manual" onClick={() => void copyManual()} className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 hover:bg-accent hover:text-foreground">
                    <Copy className="h-3 w-3" aria-hidden />{(" " + t("复制"))}</button>
                </div>
                <pre data-testid="program-manual" className="max-h-40 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono text-[11px]">{plan.manual.join("\n")}</pre>
              </div>
            )}
          </>
        )}
      </div>
    </ConfirmModal>
  );
}
