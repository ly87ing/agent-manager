import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import { HooksEditor } from "../components/HooksEditor";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

function routes() {
  return {
    "sources:get": makeEnvelope(
      {
        sources: [
          {
            key: "hooks.claude",
            label: "Claude Hooks",
            format: "json",
            revision: "rev-1",
            value: {
              removed_signatures: [{ event: "Stop", command_contains: "old.sh" }],
              hooks: {
                PreToolUse: [
                  { matcher: "Edit|Write", hooks: [{ type: "command", command: "./guard.sh", timeout: 30 }] },
                ],
              },
            },
          },
        ],
      },
      { command: "sources.get" },
    ),
    "sources:set": makeEnvelope({ key: "hooks.claude", revision: "rev-2" }, { command: "sources.set" }),
  };
}

describe("Hooks 结构化编辑", () => {
  it("按事件与命令逐行展示既有配置", async () => {
    renderWithEngine(<HooksEditor />, routes());
    const row = await screen.findByTestId("hooks-row-hooks.claude-0");
    expect(within(row).getByTestId("hooks-event-hooks.claude-0")).toHaveValue("PreToolUse");
    expect(within(row).getByTestId("hooks-matcher-hooks.claude-0")).toHaveValue("Edit|Write");
    expect(within(row).getByTestId("hooks-command-hooks.claude-0")).toHaveValue("./guard.sh");
    // 本工具不理解的字段如实说明会保留
    expect(row).toHaveTextContent("保留原有字段：timeout");
  });

  it("新增一条命令并保存：按原结构回填，未展示字段与版本号都带上", async () => {
    const { calls } = renderWithEngine(<HooksEditor />, routes());
    await userEvent.click(await screen.findByTestId("hooks-add-hooks.claude"));
    await userEvent.type(screen.getByTestId("hooks-event-hooks.claude-1"), "SessionStart");
    await userEvent.type(screen.getByTestId("hooks-command-hooks.claude-1"), "echo hi");
    await userEvent.click(screen.getByTestId("hooks-save-hooks.claude"));

    await waitFor(() => expect(calls.some((call) => call.command === "sources:set")).toBe(true));
    const write = calls.find((call) => call.command === "sources:set")!;
    expect(write.args[write.args.indexOf("--key") + 1]).toBe("hooks.claude");
    const payload = JSON.parse(write.args[write.args.indexOf("--value") + 1]);
    expect(payload.revision).toBe("rev-1");
    expect(payload.value.hooks.SessionStart).toEqual([{ hooks: [{ type: "command", command: "echo hi" }] }]);
    // 原有条目与本工具不展示的字段都不丢
    expect(payload.value.hooks.PreToolUse[0].hooks[0].timeout).toBe(30);
    expect(payload.value.removed_signatures).toHaveLength(1);
  });

  it("命令为空时拦在保存前，不发写请求", async () => {
    const { calls } = renderWithEngine(<HooksEditor />, routes());
    await userEvent.click(await screen.findByTestId("hooks-add-hooks.claude"));
    await userEvent.type(screen.getByTestId("hooks-event-hooks.claude-1"), "Stop");
    await userEvent.click(screen.getByTestId("hooks-save-hooks.claude"));
    expect(await screen.findByText(/命令不能为空/)).toBeInTheDocument();
    expect(calls.some((call) => call.command === "sources:set")).toBe(false);
  });

  it("移除一条命令后保存，剩余结构正确", async () => {
    const { calls } = renderWithEngine(<HooksEditor />, routes());
    await userEvent.click(await screen.findByTestId("hooks-remove-hooks.claude-0"));
    await userEvent.click(screen.getByTestId("hooks-save-hooks.claude"));

    await waitFor(() => expect(calls.some((call) => call.command === "sources:set")).toBe(true));
    const write = calls.find((call) => call.command === "sources:set")!;
    const payload = JSON.parse(write.args[write.args.indexOf("--value") + 1]);
    expect(payload.value.hooks).toEqual({});
    expect(payload.value.removed_signatures).toHaveLength(1);
  });
});
