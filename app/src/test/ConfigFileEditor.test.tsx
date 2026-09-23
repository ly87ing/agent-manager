import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import { ConfigFileEditorDialog } from "../components/ConfigFileEditorDialog";
import type { Envelope } from "../lib/contract";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

const BASE = "a".repeat(64);
const CONTENT = '{\n  "env": {\n    "ANTHROPIC_AUTH_TOKEN": "<hidden>"\n  },\n  "theme": "dark"\n}\n';

function file(overrides: Record<string, unknown> = {}) {
  return makeEnvelope(
    {
      element: "agents:claude-code",
      path: "~/.claude/settings.json",
      format: "json",
      exists: true,
      content: CONTENT,
      hidden: 1,
      base: BASE,
      blocked_reason: null,
      ...overrides,
    },
    { command: "agents.config-file" },
  );
}

function edits(calls: { command: string; args: string[] }[]) {
  return calls.filter((call) => call.command === "agents:edit-config");
}

describe("编辑配置文件", () => {
  it("显示遮住凭据的原文，改动后才能写入，写入带打开时的整文件基准", async () => {
    const { calls } = renderWithEngine(
      <ConfigFileEditorDialog agentId="claude-code" agentName="Claude Code" onClose={() => {}} />,
      {
        "agents:config-file": file(),
        "agents:edit-config": makeEnvelope({ changed: ["theme"], base: "b".repeat(64) }, { command: "agents.edit-config" }),
      },
    );
    const textarea = await screen.findByTestId("config-file-text");
    expect(textarea).toHaveValue(CONTENT);
    expect(screen.getByTestId("config-file-hidden-note")).toHaveTextContent("<hidden>");
    expect(screen.getByTestId("config-file-submit")).toBeDisabled();

    const edited = CONTENT.replace('"dark"', '"light"');
    fireEvent.change(textarea, { target: { value: edited } });
    await userEvent.click(screen.getByTestId("config-file-mode-diff"));
    const diff = screen.getByTestId("config-file-diff");
    const added = [...diff.querySelectorAll('[data-kind="add"]')].map((line) => line.textContent?.trim());
    const removed = [...diff.querySelectorAll('[data-kind="del"]')].map((line) => line.textContent?.trim());
    expect(added).toEqual(['+   "theme": "light"']);
    expect(removed).toEqual(['-   "theme": "dark"']);

    await userEvent.click(screen.getByTestId("config-file-submit"));
    await waitFor(() => expect(edits(calls)).toHaveLength(1));
    expect(edits(calls)[0].args).toEqual([
      "--only", "agents:claude-code", "--file-base", BASE, "--value", edited,
    ]);
  });

  it("不是合法 JSON 时不发写入", async () => {
    const { calls } = renderWithEngine(
      <ConfigFileEditorDialog agentId="claude-code" agentName="Claude Code" onClose={() => {}} />,
      { "agents:config-file": file() },
    );
    const textarea = await screen.findByTestId("config-file-text");
    fireEvent.change(textarea, { target: { value: "{ broken" } });
    await userEvent.click(screen.getByTestId("config-file-submit"));
    expect(await screen.findByTestId("config-file-error")).toHaveTextContent("不是合法的 JSON");
    expect(edits(calls)).toHaveLength(0);
  });

  it("文件已在外部修改：写入被拒后重新读取最新内容并提示", async () => {
    let reads = 0;
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      "agents:config-file": async () => {
        reads += 1;
        return reads === 1 ? file() : file({ content: CONTENT.replace('"dark"', '"solarized"'), base: "c".repeat(64) });
      },
      "agents:edit-config": async () =>
        makeEnvelope({}, {
          command: "agents.edit-config",
          ok: false,
          errors: [{ code: "AGENT_CONFIG_CHANGED", element: null, message: "~/.claude/settings.json 已在外部修改" }],
        }),
    };
    renderWithEngine(<ConfigFileEditorDialog agentId="claude-code" agentName="Claude Code" onClose={() => {}} />, routes);
    const textarea = await screen.findByTestId("config-file-text");
    fireEvent.change(textarea, { target: { value: CONTENT.replace('"dark"', '"light"') } });
    await userEvent.click(screen.getByTestId("config-file-submit"));
    expect(await screen.findByTestId("config-file-error")).toHaveTextContent("已在外部修改");
    await waitFor(() => expect(screen.getByTestId("config-file-text")).toHaveValue(CONTENT.replace('"dark"', '"solarized"')));
  });

  it("读不懂的文件不给原文，只说明原因", async () => {
    renderWithEngine(
      <ConfigFileEditorDialog agentId="claude-code" agentName="Claude Code" onClose={() => {}} />,
      { "agents:config-file": file({ content: null, hidden: 0, blocked_reason: "settings.json 不是合法的 JSON 对象，请先在编辑器里修好再来编辑" }) },
    );
    expect(await screen.findByTestId("config-file-blocked")).toHaveTextContent("请先在编辑器里修好");
    expect(screen.queryByTestId("config-file-text")).toBeNull();
    expect(screen.getByTestId("config-file-submit")).toBeDisabled();
  });
});
