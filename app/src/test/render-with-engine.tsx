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
  const base = createFakeEngineClient(routes);
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
          <Toaster richColors position="top-right" />
        </ThemeProvider>
      </ExecutionProvider>
    </QueryClientProvider>,
  );
  return { calls, ...view };
}

export { screen, within, userEvent };
