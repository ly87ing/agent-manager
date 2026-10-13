import { t } from "../lib/i18n";
import type { ProgramElement } from "../lib/programs";
import { cn } from "../lib/utils";

export interface DependencyChip {
  program: string;
  name: string;
  missing: boolean;
}

/** 某个技能或 MCP 服务依赖的程序（从 programs:status 反查）。 */
export function dependenciesOf(programs: ProgramElement[], kind: "skill" | "mcp", name: string): DependencyChip[] {
  return programs
    .filter((program) => program.dependents.some((dep) => dep.kind === kind && dep.name === name))
    .map((program) => ({
      program: program.program,
      name: program.bins[0] ?? program.name.split("/").pop() ?? program.name,
      missing: program.problems.some((problem) => ["missing", "missing-no-install", "runner-missing"].includes(problem.code)),
    }));
}

/** 行上的依赖标签：缺的标出来，点一下到程序页那一行。 */
export function DependencyChips({ chips, onOpen }: { chips: DependencyChip[]; onOpen?: (program: string) => void }) {
  if (chips.length === 0) return null;
  return (
    <>
      {chips.map((chip) => (
        <button
          key={chip.program}
          type="button"
          data-testid={`dependency-${chip.program}`}
          onClick={() => onOpen?.(chip.program)}
          title={chip.missing ? t("需要的程序没装，点这里去程序页处理") : t("需要的程序，点这里看版本与管理方式")}
          className={cn(
            "whitespace-nowrap rounded px-1.5 py-0.5 text-[11px] hover:underline",
            chip.missing ? "bg-primary/10 text-primary" : "bg-muted text-muted-foreground",
          )}
        >
          {chip.missing ? t("缺") : t("需要")} {chip.name}
        </button>
      ))}
    </>
  );
}
