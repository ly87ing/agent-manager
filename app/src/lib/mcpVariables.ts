import type { EngineElement } from "./contract";
import type { McpObserved } from "./observed";

function variableName(placeholder: string): string {
  return /^\$\{([^}]+)\}$/.exec(placeholder)?.[1] ?? placeholder;
}

/** 条目里还没取到值的变量名（去重）。 */
export function missingVariables(element: EngineElement): string[] {
  const cells = Object.values((element.observed as McpObserved).agents ?? {});
  const names = cells.flatMap((cell) =>
    Object.values(cell.vars ?? {})
      .filter((resolution) => resolution.resolved === false)
      .map((resolution) => variableName(resolution.placeholder)),
  );
  return [...new Set(names)];
}
