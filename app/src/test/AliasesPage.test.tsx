import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AliasesPage from "../pages/AliasesPage";
import type { Envelope } from "../lib/contract";
import { makeAliasStatus, makeApplyResult, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

const CLAUDE = 'TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" claude --dangerously-skip-permissions';

function aliasRoutes(
  aliases: Record<string, string> = {},
  options: Parameters<typeof makeAliasStatus>[1] = {},
): Record<string, Envelope> {
  return {
    "aliases:status": makeEnvelope(makeAliasStatus({ claude: CLAUDE, ...aliases }, options), { command: "aliases.status" }),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
    "aliases:discard-legacy": makeEnvelope(
      { domain: "aliases", discarded: "legacy-0123456789abcdef", source_removed: true },
      { command: "aliases.discard-legacy" },
    ),
  };
}

type Call = { command: string; args: string[] };

function applyArgs(calls: Call[]): string[] {
  const call = calls.find((item) => item.command === "aliases:apply");
  if (!call) throw new Error("没有 aliases:apply 调用");
  return call.args;
}

function valueOf(args: string[], flag: string): string | undefined {
  const index = args.indexOf(flag);
  return index >= 0 ? args[index + 1] : undefined;
}

describe("终端快捷命令页", () => {
  it("按条列出终端配置文件里实际的那一行，标出在哪些文件里", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes());
    const row = await screen.findByTestId("alias-row-claude");
    expect(row).toHaveTextContent(`alias claude='${CLAUDE}'`);
    expect(row).toHaveTextContent("~/.zshrc");
    expect(row).toHaveTextContent("~/.zprofile");
    // 没有「立即同步」与全局时区：每次编辑都直接写回文件
    expect(screen.queryByTestId("apply-aliases-button")).toBeNull();
    expect(screen.queryByTestId("global-timezone-select")).toBeNull();
  });

  it("新增命令：按时区与扩展参数生成取值，保存即写回两个文件并带上各自的基准", async () => {
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

    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const args = applyArgs(calls);
    expect(valueOf(args, "--zshrc-base")).toBe("base-aliases:zshrc");
    expect(valueOf(args, "--zprofile-base")).toBe("base-aliases:zprofile");
    expect(valueOf(args, "--set")).toBe("codex");
    // 写进文件的是字面量，不再有 {timezone}/{lang} 占位
    expect(valueOf(args, "--value")).toBe('TZ="Asia/Shanghai" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" codex --full-auto');
    expect(args).not.toContain("--rename-from");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("新增时预填内置默认的时区与语言（字面量），没有「跟随全局」", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByTestId("alias-timezone-select")).toHaveValue("America/Los_Angeles");
    expect(within(dialog).getByTestId("alias-lang-select")).toHaveValue("en_US.UTF-8");
    expect(dialog).not.toHaveTextContent("跟随全局");
    expect(dialog).toHaveTextContent("保存即写入");
  });

  it("编辑既有命令时回填时区、语言与扩展参数", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("edit-alias-claude"));
    const dialog = await screen.findByRole("dialog");
    // 名字可改：改名是常见需求，早先锁死导致只能删了重建
    expect(within(dialog).getByTestId("alias-name-input")).not.toBeDisabled();
    expect(within(dialog).getByTestId("alias-command-input")).toHaveValue("claude");
    expect(within(dialog).getByTestId("alias-extra-args-input")).toHaveValue("--dangerously-skip-permissions");
    expect(within(dialog).getByTestId("alias-timezone-select")).toHaveValue("America/Los_Angeles");
    expect(within(dialog).getByTestId("alias-lang-select")).toHaveValue("en_US.UTF-8");
  });

  it("改名保存是一次写入：带上旧名，由引擎在同一位置替换，旧名不残留", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("edit-alias-claude"));
    const dialog = await screen.findByRole("dialog");
    const nameInput = within(dialog).getByTestId("alias-name-input");
    await userEvent.clear(nameInput);
    await userEvent.type(nameInput, "cc");
    await userEvent.click(within(dialog).getByRole("button", { name: "保存" }));

    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const args = applyArgs(calls);
    expect(valueOf(args, "--set")).toBe("cc");
    expect(valueOf(args, "--rename-from")).toBe("claude");
    expect(calls.filter((call) => call.command === "aliases:apply")).toHaveLength(1);
  });

  it("写入被拒（文件已在外部修改）时编辑框保留，不假装成功", async () => {
    const routes = aliasRoutes();
    routes["aliases:apply"] = makeEnvelope(
      {},
      {
        ok: false,
        command: "aliases.apply",
        errors: [{ code: "ALIAS_ERROR", element: null, message: "~/.zshrc 已在外部修改（与打开时读到的内容不同），没有写入；请刷新后重新操作" }],
      },
    );
    const { calls } = renderWithEngine(<AliasesPage />, routes);
    await userEvent.click(await screen.findByTestId("edit-alias-claude"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-extra-args-input"), " --verbose");
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    expect(await screen.findByText(/已在外部修改/)).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("复制一条：名字自动避让，内容照搬等着改", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("duplicate-alias-claude"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByTestId("alias-name-input")).toHaveValue("claude-copy");
    expect(within(dialog).getByTestId("alias-extra-args-input")).toHaveValue("--dangerously-skip-permissions");
  });

  it("chrome 这类包装写法：原文编辑，写进去的就是原文", async () => {
    const chrome = 'open --env TZ="America/Los_Angeles" --env LANG="en_US.UTF-8" -n -a "Google Chrome" --args --lang=en-US';
    renderWithEngine(<AliasesPage />, aliasRoutes({ chrome }));
    expect(await screen.findByTestId("alias-row-chrome")).toHaveTextContent('TZ="America/Los_Angeles"');

    await userEvent.click(screen.getByTestId("edit-alias-chrome"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByTestId("alias-raw-input")).toHaveValue(chrome);
    expect(dialog).toHaveTextContent("写进去的就是这里的内容");
  });

  it("非常见写法的命令自动切到原文编辑，不改写用户写法", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes({ chrome: 'open --env TZ="UTC" -n -a "Google Chrome"' }));
    await userEvent.click(await screen.findByTestId("edit-alias-chrome"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByTestId("alias-raw-input")).toHaveValue('open --env TZ="UTC" -n -a "Google Chrome"');
    expect(within(dialog).queryByTestId("alias-command-input")).toBeNull();
  });

  it("区块里不是本工具写法的行只能删除，不给编辑", async () => {
    const routes = aliasRoutes();
    const data = makeAliasStatus({ claude: CLAUDE });
    data.aliases = [
      ...(data.aliases ?? []),
      { name: "ll", value: null, line: 'alias ll="ls -l"', targets: ["aliases:zshrc"], consistent: true },
    ];
    routes["aliases:status"] = makeEnvelope(data, { command: "aliases.status" });
    renderWithEngine(<AliasesPage />, routes);
    const row = await screen.findByTestId("alias-row-ll");
    expect(row).toHaveTextContent('alias ll="ls -l"');
    expect(row).toHaveTextContent("不是本工具的写法");
    expect(screen.queryByTestId("edit-alias-ll")).toBeNull();
    expect(screen.getByTestId("remove-alias-ll")).toBeInTheDocument();
    expect(screen.getByTestId("alias-missing-in-ll")).toHaveTextContent("~/.zprofile");
  });

  it("时区可以自定义输入，不受下拉列表限制", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-name-input"), "mytool");
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "mytool");
    await userEvent.selectOptions(within(dialog).getByTestId("alias-timezone-select"), "__custom__");
    await userEvent.type(within(dialog).getByTestId("alias-timezone-input"), "Pacific/Chatham");
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    expect(valueOf(applyArgs(calls), "--value")).toBe('TZ="Pacific/Chatham" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" mytool');
  });

  it("可以添加自定义环境变量，并放在命令之前", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-name-input"), "mytool");
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "mytool");
    await userEvent.click(within(dialog).getByTestId("alias-env-add"));
    await userEvent.type(within(dialog).getByTestId("alias-env-key-0"), "HTTPS_PROXY");
    await userEvent.type(within(dialog).getByTestId("alias-env-value-0"), "http://127.0.0.1:7890");
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    expect(valueOf(applyArgs(calls), "--value")).toBe(
      'HTTPS_PROXY="http://127.0.0.1:7890" TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" mytool',
    );
  });

  it("编辑既有命令时原有的其它环境变量可见可改", async () => {
    renderWithEngine(
      <AliasesPage />,
      aliasRoutes({ proxied: 'HTTPS_PROXY="http://127.0.0.1:7890" TZ="UTC" LANG="C" LC_ALL="C" proxied' }),
    );
    await userEvent.click(await screen.findByTestId("edit-alias-proxied"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByTestId("alias-env-key-0")).toHaveValue("HTTPS_PROXY");
    expect(within(dialog).getByTestId("alias-env-value-0")).toHaveValue("http://127.0.0.1:7890");
  });

  it("非法的环境变量名被拦下", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-name-input"), "mytool");
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "mytool");
    await userEvent.click(within(dialog).getByTestId("alias-env-add"));
    await userEvent.type(within(dialog).getByTestId("alias-env-key-0"), "1BAD");
    await userEvent.type(within(dialog).getByTestId("alias-env-value-0"), "x");
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));
    expect(await within(dialog).findByTestId("alias-form-error")).toHaveTextContent("不合法");
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
  });

  it("按命令给出常用参数：点一下加上全自动，再点去掉，危险后果明确写出", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-name-input"), "cc");
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "claude");

    const preset = within(dialog).getByTestId("alias-preset-claude-yolo");
    expect(preset).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(preset);
    expect(within(dialog).getByTestId("alias-extra-args-input")).toHaveValue("--dangerously-skip-permissions");
    expect(within(dialog).getByTestId("alias-preset-warning")).toHaveTextContent("不再逐次征求确认");

    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    expect(valueOf(applyArgs(calls), "--value")).toContain("--dangerously-skip-permissions");
  });

  it("Codex 有两档：完全放开与自动审查，未知命令没有预设", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "codex");
    expect(within(dialog).getByTestId("alias-preset-codex-yolo")).toBeInTheDocument();
    const sandboxed = within(dialog).getByTestId("alias-preset-codex-auto-sandboxed");
    await userEvent.click(sandboxed);
    expect(within(dialog).getByTestId("alias-extra-args-input")).toHaveValue("--approve-for-me");
    // 保留沙箱的那档不是危险项，不弹警示
    expect(within(dialog).queryByTestId("alias-preset-warning")).toBeNull();

    await userEvent.clear(within(dialog).getByTestId("alias-command-input"));
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "some-tool");
    expect(within(dialog).queryByTestId("alias-preset-codex-yolo")).toBeNull();
  });

  it("Claude 提供安全检查版 Auto，不必只能选择完全放开", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "claude");
    await userEvent.click(within(dialog).getByTestId("alias-preset-claude-auto"));
    expect(within(dialog).getByTestId("alias-extra-args-input")).toHaveValue("--permission-mode auto");
    expect(within(dialog).queryByTestId("alias-preset-warning")).toBeNull();
  });

  it("保存前拦住会破坏配置行的取值", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-name-input"), "bad");
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "echo 'x'");
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));
    expect(await within(dialog).findByTestId("alias-form-error")).toHaveTextContent("不能包含单引号");
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
  });

  it("重名的新增被拦下", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByTestId("alias-name-input"), "claude");
    await userEvent.type(within(dialog).getByTestId("alias-command-input"), "claude");
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));
    expect(await within(dialog).findByTestId("alias-form-error")).toHaveTextContent("已有同名快捷命令");
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
  });

  it("移除命令需确认，确认后按名从两个文件里去掉这一条", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes({ codex: "codex" }));
    await userEvent.click(await screen.findByTestId("remove-alias-codex"));
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
    await userEvent.click(await screen.findByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const args = applyArgs(calls);
    expect(valueOf(args, "--remove")).toBe("codex");
    expect(valueOf(args, "--zshrc-base")).toBe("base-aliases:zshrc");
  });

  it("内置推荐不自动写入，逐条确认后才添加", async () => {
    const { calls } = renderWithEngine(
      <AliasesPage />,
      aliasRoutes({}, { recommendations: [{ name: "grok", value: "grok", line: "alias grok='grok'" }] }),
    );
    const item = await screen.findByTestId("alias-recommend-grok");
    expect(item).toHaveTextContent("alias grok='grok'");
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
    await userEvent.click(screen.getByTestId("add-recommended-grok"));
    const modal = await screen.findByRole("dialog");
    expect(modal).toHaveTextContent("alias grok='grok'");
    await userEvent.click(within(modal).getByRole("button", { name: "添加" }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const args = applyArgs(calls);
    expect(valueOf(args, "--recommended")).toBe("grok");
    expect(args).not.toContain("--value");
  });

  it("旧版本保存、还没生效的命令：可加入终端配置或丢弃（只删库）", async () => {
    const legacy = {
      id: "legacy-0123456789abcdef",
      name: "claude",
      value: 'TZ="Asia/Shanghai" claude',
      line: `alias claude='TZ="Asia/Shanghai" claude'`,
      actual: `alias claude='${CLAUDE}'`,
    };
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes({}, { legacy: [legacy] }));
    const item = await screen.findByTestId("alias-legacy-claude");
    expect(item).toHaveTextContent('TZ="Asia/Shanghai"');
    expect(item).toHaveTextContent("加入会替换它");

    await userEvent.click(screen.getByTestId("add-legacy-claude"));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "加入终端配置" }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    expect(valueOf(applyArgs(calls), "--legacy")).toBe(legacy.id);

    await userEvent.click(screen.getByTestId("discard-legacy-claude"));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "丢弃" }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:discard-legacy")).toBe(true));
    expect(calls.find((call) => call.command === "aliases:discard-legacy")?.args).toEqual(["--legacy", legacy.id]);
  });

  it("区块外用户自己的 alias 只读列出，标为其他来源", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes({}, { external: { gp: "git push" } }));
    const item = await screen.findByTestId("alias-external-aliases:zshrc-gp");
    expect(item).toHaveTextContent("alias gp='git push'");
    const section = screen.getByTestId("alias-external");
    expect(section).toHaveTextContent("其他来源（不由本工具写入）");
    expect(within(section).queryByRole("button")).toBeNull();
  });

  it("读写不了的文件显示原因，写入只带能写的那个文件的基准", async () => {
    const { calls } = renderWithEngine(
      <AliasesPage />,
      aliasRoutes({ codex: "codex" }, { blockedZprofile: "目标路径是符号链接，拒绝操作以保证安全" }),
    );
    expect(await screen.findByTestId("alias-target-aliases:zprofile")).toHaveTextContent("符号链接");
    await userEvent.click(screen.getByTestId("remove-alias-codex"));
    await userEvent.click(await screen.findByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const args = applyArgs(calls);
    expect(args).toContain("--zshrc-base");
    expect(args).not.toContain("--zprofile-base");
  });
});
