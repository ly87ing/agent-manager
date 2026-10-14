import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { SkillAdoptDialog, type AdoptPreview } from "../components/SkillAdoptDialog";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function localPreview(): AdoptPreview {
  return { name: "mine", kind: "adopt", locations: [], removes: ["~/.claude/skills/mine"],
    source: { repository: "https://example.invalid/YOUR_REPOSITORY.git", path: "skills/mine", version: null, installer: null },
    local: { root: "~/skills/my-skills", dest: "~/skills/my-skills/mine", dest_exists: false, collection: "my-skills", digest: "a".repeat(64), accounts: ["codex"], from: "~/.claude/skills/mine", variants: 1 } };
}

describe("Local-first skill adoption", () => {
  it("can preserve local content before requesting any remote comparison", async () => {
    const confirm = vi.fn(async (_args: string[], _label: string, _run: string) => true);
    const { calls } = renderWithEngine(<SkillAdoptDialog name="mine" nameOf={(name) => name} onConfirm={confirm} onClose={() => {}} />, {
      "skills:adopt-preview": makeEnvelope(localPreview()),
    });
    await waitFor(() => expect(screen.getByTestId("adopt-confirm")).toBeEnabled());
    expect(calls.filter((call) => call.command === "skills:adopt-preview")).toHaveLength(1);
    expect(calls[0].args).toEqual(["--name", "mine", "--local-only"]);
    await userEvent.click(screen.getByTestId("adopt-confirm"));
    expect(confirm.mock.calls[0][0]).toContain("--keep-local");
  });

  it("requests the remote source explicitly and uses its verified plan", async () => {
    const confirm = vi.fn(async (_args: string[], _label: string, _run: string) => true);
    const { calls } = renderWithEngine(<SkillAdoptDialog name="mine" nameOf={(name) => name} onConfirm={confirm} onClose={() => {}} />, {
      "skills:adopt-preview": async (args) => {
        const preview = localPreview();
        if (!args.includes("--local-only")) preview.source_check = { same: true, path: "skills/mine", adopt_at: { name: "main", label: "main", commit: "b".repeat(40) }, latest: { name: "main", label: "main", commit: "b".repeat(40) }, ref: null, locations: [], accounts: ["codex"], digest: "c".repeat(64) };
        return makeEnvelope(preview);
      },
    });
    await userEvent.click(await screen.findByRole("button", { name: "比对远程来源" }));
    await screen.findByTestId("adopt-compare");
    const previews = calls.filter((call) => call.command === "skills:adopt-preview");
    expect(previews).toHaveLength(2);
    expect(previews[1].args).not.toContain("--local-only");
    await userEvent.click(screen.getByTestId("adopt-confirm"));
    expect(confirm.mock.calls[0][0]).toContain("--path");
    expect(confirm.mock.calls[0][0]).not.toContain("--keep-local");
  });

  it("keeps the local plan usable after a failed remote comparison", async () => {
    renderWithEngine(<SkillAdoptDialog name="mine" nameOf={(name) => name} onConfirm={async () => true} onClose={() => {}} />, {
      "skills:adopt-preview": async (args) => makeEnvelope({ ...localPreview(), ...(!args.includes("--local-only") ? { source_check: { error: "Remote unavailable" } } : {}) }),
    });
    await userEvent.click(await screen.findByRole("button", { name: "比对远程来源" }));
    await screen.findByText("Remote unavailable");
    expect(screen.getByTestId("adopt-confirm")).toBeEnabled();
    expect(screen.getByTestId("adopt-choice-local")).toBeChecked();
  });
});
