import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import { ConfigFileEditor } from "../components/ConfigFileEditor";
import type { Envelope } from "../lib/contract";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

const BASE = "a".repeat(64);
const CONTENT = '{\n  "env": {\n    "ANTHROPIC_AUTH_TOKEN": "tok-plain-1"\n  },\n  "theme": "dark"\n}\n';

function file(overrides: Record<string, unknown> = {}) {
  return makeEnvelope(
    {
      element: "agents:claude-code",
      path: "~/.claude/settings.json",
      format: "json",
      exists: true,
      content: CONTENT,
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

describe("直接编辑配置文件", () => {
  it("显示文件原文（含令牌），改动后才能写入，写入带打开时的整文件基准与原文", async () => {
    const { calls } = renderWithEngine(<ConfigFileEditor agentId="claude-code" />, {
      "agents:config-file": file(),
      "agents:edit-config": makeEnvelope({ changed: ["theme"], base: "b".repeat(64) }, { command: "agents.edit-config" }),
    });
    const textarea = await screen.findByTestId("config-file-text");
    expect(textarea).toHaveValue(CONTENT);
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
    // 写入后重新读取，拿到新的基准
    await waitFor(() => expect(calls.filter((call) => call.command === "agents:config-file")).toHaveLength(2));
  });

  it("放弃改动回到文件原文，不发写入", async () => {
    const { calls } = renderWithEngine(<ConfigFileEditor agentId="claude-code" />, { "agents:config-file": file() });
    const textarea = await screen.findByTestId("config-file-text");
    fireEvent.change(textarea, { target: { value: "{}" } });
    await userEvent.click(screen.getByTestId("config-file-reset"));
    expect(screen.getByTestId("config-file-text")).toHaveValue(CONTENT);
    expect(edits(calls)).toHaveLength(0);
  });

  it("不是合法 JSON 时不发写入", async () => {
    const { calls } = renderWithEngine(<ConfigFileEditor agentId="claude-code" />, { "agents:config-file": file() });
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
    renderWithEngine(<ConfigFileEditor agentId="claude-code" />, routes);
    const textarea = await screen.findByTestId("config-file-text");
    fireEvent.change(textarea, { target: { value: CONTENT.replace('"dark"', '"light"') } });
    await userEvent.click(screen.getByTestId("config-file-submit"));
    expect(await screen.findByTestId("config-file-error")).toHaveTextContent("已在外部修改");
    await waitFor(() => expect(screen.getByTestId("config-file-text")).toHaveValue(CONTENT.replace('"dark"', '"solarized"')));
  });

  it("不是文本的文件只说明原因，不给编辑框", async () => {
    renderWithEngine(<ConfigFileEditor agentId="claude-code" />, {
      "agents:config-file": file({ content: null, blocked_reason: "~/.claude/settings.json 不是 UTF-8 文本，不能在这里编辑" }),
    });
    expect(await screen.findByTestId("config-file-blocked")).toHaveTextContent("不是 UTF-8 文本");
    expect(screen.queryByTestId("config-file-text")).toBeNull();
    expect(screen.getByTestId("config-file-submit")).toBeDisabled();
  });
});
