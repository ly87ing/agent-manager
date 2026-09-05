import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Toaster } from "sonner";
import { setEngineClient } from "../lib/engine";
import { createQueryClient } from "../lib/queries";
import type { Envelope } from "../lib/contract";
import { createFakeEngineClient } from "./engine-test-client";
import type { ReactNode } from "react";

export function renderWithEngine(ui: ReactNode, routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)>) {
  setEngineClient(createFakeEngineClient(routes));
  const queryClient = createQueryClient({ retry: false });
  return render(
    <QueryClientProvider client={queryClient}>
      {ui}
      <Toaster richColors position="top-right" />
    </QueryClientProvider>,
  );
}

export { screen, within, userEvent };
