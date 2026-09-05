import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ModeBanner } from "../components/ModeBanner";
import { setEngineClient, type EngineClient } from "../lib/engine";
import { createQueryClient } from "../lib/queries";
import { makeAuditData, makeEnvelope, makeMode } from "./fixtures";

function renderBanner(mode: { dry_run: boolean; offline: boolean }) {
  const client: EngineClient = {
    run: vi.fn(async () => makeEnvelope(makeAuditData(), { mode: makeMode(mode) })),
  };
  setEngineClient(client);
  const queryClient = createQueryClient({ retry: false });
  return {
    client,
    ...render(
      <QueryClientProvider client={queryClient}>
        <ModeBanner />
      </QueryClientProvider>,
    ),
  };
}

describe("ModeBanner", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("offline 模式：渲染离线标注", async () => {
    renderBanner({ offline: true, dry_run: false });
    expect(await screen.findByText(/离线模式/)).toBeInTheDocument();
    expect(screen.getByText(/缓存值/)).toBeInTheDocument();
  });

  it("dry-run 模式：渲染演练标注", async () => {
    renderBanner({ offline: false, dry_run: true });
    expect(await screen.findByText(/演练模式/)).toBeInTheDocument();
  });

  it("普通模式：不渲染横幅", async () => {
    const { client, container } = renderBanner({ offline: false, dry_run: false });
    await waitFor(() => expect(client.run).toHaveBeenCalled());
    await waitFor(() => expect(container.querySelector('[role="status"]')).toBeNull());
  });
});
