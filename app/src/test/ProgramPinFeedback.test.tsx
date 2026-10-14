import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { ProgramPinDialog } from "../components/ProgramPinDialog";
import type { ProgramElement } from "../lib/programs";
import { makeElement, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

describe("program version feedback", () => {
  it("does not claim completion when the MCP edit succeeds but its version preference fails", async () => {
    const program = { id: "programs:npm:example-mcp", program: "npm:example-mcp", name: "Example", kind: "npm",
      runners: [{ server: "example", agent: "codex", runner: "npx", follow_latest: true }],
      dependents: [{ kind: "mcp", name: "example", agents: ["codex"] }] } as ProgramElement;
    const onClose = vi.fn();
    const { calls } = renderWithEngine(<ProgramPinDialog program={program} initialVersion="2.0.0" onClose={onClose} />, {
      "mcp:status": makeEnvelope({ elements: [makeElement({ id: "mcp:example", observed: { agents: { codex: {
        actions: ["edit"], base: "a".repeat(64), entry: { command: "npx", args: ["example-mcp@1.0.0"] },
      } } } })] }),
      "mcp:preview": makeEnvelope({ files: [{ path: "/example/mcp.json", original: "old", content: "new" }] }),
      "mcp:apply": makeEnvelope({}),
      "programs:declare": makeEnvelope({}, { ok: false, errors: [{ code: "CONFLICT", element: null, message: "Definition changed" }] }),
      "system:refresh-upstream": makeEnvelope({}),
    });
    const confirm = await screen.findByTestId("confirm-modal-confirm");
    await waitFor(() => expect(confirm).toBeEnabled());
    await userEvent.click(confirm);
    await waitFor(() => expect(document.querySelector('[data-sonner-toast][data-type="warning"]')).toHaveTextContent("1 处没改成"));
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
    expect(calls.filter((call) => ["mcp:apply", "programs:declare"].includes(call.command)).map((call) => call.command)).toEqual(["mcp:apply", "programs:declare"]);
  });
});
