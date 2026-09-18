import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AliasesPage from "../pages/AliasesPage";
import { makeApplyResult, makeElement, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

function aliasRoutes(aliases: Record<string, string> = {}) {
  return {
    "sources:get": makeEnvelope(
      {
        sources: [
          {
            key: "aliases.shell",
            label: "Shell Aliases",
            format: "json",
            revision: "rev-1",
            value: {
              _comment: "保留字段",
              timezone: "America/Los_Angeles",
              lang: "en_US.UTF-8",
              aliases: {
                claude: 'TZ="{timezone}" LANG="{lang}" LC_ALL="{lang}" claude --dangerously-skip-permissions',
                ...aliases,
              },
            },
          },
        ],
      },
      { command: "sources.get" },
    ),
    "sources:set": makeEnvelope({ key: "aliases.shell", revision: "rev-2" }, { command: "sources.set" }),
    "aliases:status": makeEnvelope(
      {
        domain: "aliases",
        elements: [makeElement({ id: "aliases:zshrc", domain: "aliases", state: "managed-ok", observed: {}, spec: {} })],
      },
      { command: "aliases.status" },
    ),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
  };
}

describe("终端快捷命令页", () => {
  it("按条列出命令并展示同步后实际生效的那一行", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes());
    const row = await screen.findByTestId("alias-row-claude");
    expect(row).toHaveTextContent(
      `alias claude='TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" claude --dangerously-skip-permissions'`,
    );
  });

  it("新增命令：按时区与扩展参数生成取值，保存带当前版本号且保留未展示字段", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-name-input"), "codex");
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "codex");
    await userEvent.selectOptions(within(dialog).getByTestId("alias-timezone-select"), "Asia/Shanghai");
    await userEvent.type(within(dialog).getByTestId("alias-extra-args-input"), "--full-auto");
    expect(within(dialog).getByTestId("alias-preview")).toHaveTextContent(
      `alias codex='TZ="Asia/Shanghai" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" codex --full-auto'`,
    );
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "sources:set")).toBe(true));
    const write = calls.find((call) => call.command === "sources:set")!;
    expect(write.args[write.args.indexOf("--key") + 1]).toBe("aliases.shell");
    const payload = JSON.parse(write.args[write.args.indexOf("--value") + 1]);
    expect(payload.revision).toBe("rev-1");
    expect(payload.value.aliases.codex).toBe('TZ="Asia/Shanghai" LANG="{lang}" LC_ALL="{lang}" codex --full-auto');
    // 既有条目与未展示字段不丢
    expect(payload.value.aliases.claude).toContain("--dangerously-skip-permissions");
    expect(payload.value._comment).toBe("保留字段");
    // 保存不写 shell 配置
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
  });

  it("编辑既有命令时回填时区、语言与扩展参数", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("edit-alias-claude"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByTestId("alias-name-input")).toBeDisabled();
    expect(within(dialog).getByTestId("alias-command-input")).toHaveValue("claude");
    expect(within(dialog).getByTestId("alias-extra-args-input")).toHaveValue("--dangerously-skip-permissions");
    // 跟随全局的取值回填为「跟随全局」选项
    expect(within(dialog).getByTestId("alias-timezone-select")).toHaveValue("{timezone}");
    expect(within(dialog).getByTestId("alias-lang-select")).toHaveValue("{lang}");
  });

  it("非常见写法的命令自动切到原文编辑，不改写用户写法", async () => {
    renderWithEngine(
      <AliasesPage />,
      aliasRoutes({ chrome: 'open --env TZ="{timezone}" -n -a "Google Chrome"' }),
    );
    await userEvent.click(await screen.findByTestId("edit-alias-chrome"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByTestId("alias-raw-input")).toHaveValue('open --env TZ="{timezone}" -n -a "Google Chrome"');
    expect(within(dialog).queryByTestId("alias-command-input")).toBeNull();
  });

  it("保存前拦住会破坏配置行的取值", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-name-input"), "bad");
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "echo 'x'");
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));
    expect(await within(dialog).findByTestId("alias-form-error")).toHaveTextContent("不能包含单引号");
    expect(calls.some((call) => call.command === "sources:set")).toBe(false);
  });

  it("重名的新增被拦下", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-name-input"), "claude");
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "claude");
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));
    expect(await within(dialog).findByTestId("alias-form-error")).toHaveTextContent("已有同名快捷命令");
    expect(calls.some((call) => call.command === "sources:set")).toBe(false);
  });

  it("改全局时区即保存，并且不顺带执行同步", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.selectOptions(await screen.findByTestId("global-timezone-select"), "UTC");
    await waitFor(() => expect(calls.some((call) => call.command === "sources:set")).toBe(true));
    const write = calls.find((call) => call.command === "sources:set")!;
    const payload = JSON.parse(write.args[write.args.indexOf("--value") + 1]);
    expect(payload.value.timezone).toBe("UTC");
    expect(payload.value.aliases.claude).toContain("{timezone}");
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
  });

  it("移除命令需确认，确认后写回不含该条的配置", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes({ codex: "codex" }));
    await userEvent.click(await screen.findByTestId("remove-alias-codex"));
    await userEvent.click(await screen.findByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(calls.some((call) => call.command === "sources:set")).toBe(true));
    const write = calls.find((call) => call.command === "sources:set")!;
    const payload = JSON.parse(write.args[write.args.indexOf("--value") + 1]);
    expect(payload.value.aliases.codex).toBeUndefined();
    expect(payload.value.aliases.claude).toBeDefined();
  });

  it("立即同步触发 aliases:apply", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("apply-aliases-button"));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
  });
});
