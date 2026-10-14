import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { Toaster } from "sonner";
import { setEngineClient, type EngineRunOptions } from "../lib/engine";
import { ExecutionProvider } from "../lib/execution";
import { createQueryClient } from "../lib/queries";
import { ThemeProvider } from "../lib/theme";
import type { Envelope } from "../lib/contract";
import { createFakeEngineClient } from "./engine-test-client";
import { makeEnvelope } from "./fixtures";

export interface RecordedCall {
  command: string;
  args: string[];
  options?: EngineRunOptions;
}

export function renderWithEngine(
  ui: ReactNode,
  routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)>,
) {
  const calls: RecordedCall[] = [];
  const resourceTargets: { id: string; name: string; group: string; kind: string | null }[] = [];
  for (const domain of ["agents", "profiles"]) {
    const route = routes[domain + ":status"];
    if (!route || typeof route === "function") continue;
    const data = route.data as { elements?: { id: string; spec: Record<string, unknown> }[] };
    for (const item of data.elements ?? []) {
      const id = item.id.replace(/^(agents|profiles):/, "");
      const base = domain === "profiles" ? String(item.spec.base_agent ?? id) : id;
      resourceTargets.push({ id, name: String(item.spec.display_name ?? id), group: base,
        kind: base === "claude-code" ? "claude" : base === "codex" ? "codex" : null });
    }
  }
  const base = createFakeEngineClient({
    "launch:catalog": makeEnvelope({ platform: "macos", shell: "zsh", apps: [] }, { command: "launch:catalog" }),
    "launch:list": makeEnvelope({ items: [] }, { command: "launch:list" }),
    "environment:profiles": makeEnvelope({ items: [] }, { command: "environment:profiles" }),
    "resources:targets": { contract: "agent-manager/engine/v1", run_id: "targets-fixture", command: "resources.targets", ok: true,
      mode: { offline: true }, lock: { held: false, pid: null }, errors: [], warnings: [], data: { targets: resourceTargets } },
    ...routes,
  });
  setEngineClient({
    async run(command, args = [], options) {
      calls.push({ command, args: [...args], options });
      return base.run(command, args, options);
    },
  });
  const queryClient = createQueryClient({ retry: false });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <ExecutionProvider>
        <ThemeProvider>
          {ui}
          <Toaster richColors position="bottom-right" />
        </ThemeProvider>
      </ExecutionProvider>
    </QueryClientProvider>,
  );
  return { calls, queryClient, ...view };
}

export { screen, within, userEvent };

/** 打开的下拉弹出层里的选项（components/Select 渲染到 body）。 */
function openOptions(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('[role="listbox"] [role="option"]')];
}

/** 应用的下拉（components/Select）：打开后点值为 value 的一项。 */
export async function chooseOption(trigger: HTMLElement, value: string): Promise<void> {
  await userEvent.click(trigger);
  const option = openOptions().find((item) => item.getAttribute("data-value") === value);
  if (!option) throw new Error(`下拉里没有 ${value}：${openOptions().map((item) => item.getAttribute("data-value")).join(", ")}`);
  await userEvent.click(option);
}

/** 下拉里的全部选项（值与文字）；读完关掉，不改当前值。 */
export async function readOptions(trigger: HTMLElement): Promise<{ value: string; label: string }[]> {
  await userEvent.click(trigger);
  const options = openOptions().map((item) => ({ value: item.getAttribute("data-value") ?? "", label: item.textContent ?? "" }));
  await userEvent.click(trigger);
  return options;
}

