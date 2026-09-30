import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import AliasesPage from "../pages/AliasesPage";
import type { Envelope } from "../lib/contract";
import { agentManagementFixture, makeAliasStatus, makeApplyResult, makeElement, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

const CLAUDE = 'TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" claude --dangerously-skip-permissions';

function aliasRoutes(
  aliases: Record<string, string> = {},
  options: Parameters<typeof makeAliasStatus>[1] = {},
): Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> {
  return {
    "aliases:status": makeEnvelope(makeAliasStatus({ claude: CLAUDE, ...aliases }, options), { command: "aliases.status" }),
    "aliases:preview": async (args) => {
      const candidate: Record<string, string> = { claude: CLAUDE, ...aliases };
      const name = valueOf(args, "--set");
      if (name) {
        const rename = valueOf(args, "--rename-from");
        if (rename) delete candidate[rename];
        candidate[name] = valueOf(args, "--value") ?? "";
      }
      const removed = valueOf(args, "--remove");
      if (removed) delete candidate[removed];
      const recommended = options.recommendations?.find((item) => item.name === valueOf(args, "--recommended"));
      if (recommended) candidate[recommended.name] = recommended.value;
      const legacy = options.legacy?.find((item) => item.id === valueOf(args, "--legacy"));
      if (legacy) candidate[legacy.name] = legacy.value;
      return makeEnvelope({
        files: ["aliases:zshrc", ...(options.blockedZprofile ? [] : ["aliases:zprofile"])].map((target) => ({
          target,
          path: `/isolated/${target.replace("aliases:", ".")}`,
          exists: target === "aliases:zshrc",
          content: `# 保留用户注释\n${Object.entries(candidate).map(([key, value]) => `alias ${key}='${value}'`).join("\n")}\n`,
        })),
        skipped: [],
      }, { command: "aliases.preview" });
    },
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
  it("每条命令标出它启动的是哪个账号：直接调用 claude 的是 Claude 主号，与账号无关的不标", async () => {
    renderWithEngine(<AliasesPage />, {
      ...aliasRoutes({ gs: "git status" }),
      "agents:status": makeEnvelope({
        domain: "agents",
        elements: [makeElement({ id: "agents:claude-code", domain: "agents", spec: { display_name: "Claude Code", bin: "claude" }, observed: { management: agentManagementFixture("claude") } })],
      }, { command: "agents.status" }),
      "profiles:status": makeEnvelope({ domain: "profiles", elements: [] }, { command: "profiles.status" }),
    });
    expect(await screen.findByTestId("alias-row-claude")).toHaveTextContent("Claude · 主号");
    expect(screen.getByTestId("alias-row-gs")).toHaveTextContent("不是启动助手的命令");
  });

  it("按条列出终端配置文件里实际的那一行，标出在哪些文件里", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes());
    const row = await screen.findByTestId("alias-row-claude");
    // 行里显示取值，终端配置文件里完整的那一行在悬停提示里
    expect(row).toHaveTextContent(CLAUDE);
    expect(within(row).getByTitle(`alias claude='${CLAUDE}'`)).toBeInTheDocument();
    // 两个文件里都有：写「都有」，完整路径在悬停提示里
    expect(within(row).getByTitle("~/.zshrc、~/.zprofile")).toHaveTextContent("都有");
    // 没有「立即同步」与全局时区：每次编辑都直接写回文件
    expect(screen.queryByTestId("apply-aliases-button")).toBeNull();
    expect(screen.queryByTestId("global-timezone-select")).toBeNull();
  });

  it("新增命令：按时区与扩展参数生成取值，保存即写回两个文件并带上各自的基准", async () => {
    const { calls } = renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    // 一次粘贴而不是逐字输入：用例不依赖逐字输入，逐字输入在全量并发时会慢到超时
    await userEvent.click(within(dialog).getByTestId("alias-name-input"));
    await userEvent.paste("codex");
    await userEvent.click(within(dialog).getByTestId("alias-command-input"));
    await userEvent.paste("codex");
    await userEvent.selectOptions(within(dialog).getByTestId("alias-timezone-select"), "Asia/Shanghai");
    await userEvent.click(within(dialog).getByTestId("alias-extra-args-input"));
    await userEvent.paste("--full-auto");
    expect(within(dialog).getByTestId("alias-preview")).toHaveTextContent(
      `alias codex='TZ="Asia/Shanghai" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" codex --full-auto'`,
    );
    // 预览有防抖、全量并发时可能超过默认 1 秒才出来：这几处等待放宽到 5 秒
    const slow = { timeout: 5000 };
    expect(await within(dialog).findByTestId("alias-file-preview-aliases:zshrc", {}, slow)).toHaveTextContent("# 保留用户注释");
    await waitFor(() => expect(within(dialog).getByTestId("alias-form-submit")).not.toBeDisabled(), slow);
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true), slow);
    const args = applyArgs(calls);
    expect(valueOf(args, "--zshrc-base")).toBe("base-aliases:zshrc");
    expect(valueOf(args, "--zprofile-base")).toBe("base-aliases:zprofile");
    expect(valueOf(args, "--set")).toBe("codex");
    // 写进文件的是字面量，不再有 {timezone}/{lang} 占位
    expect(valueOf(args, "--value")).toBe('TZ="Asia/Shanghai" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" codex --full-auto');
    expect(args).not.toContain("--rename-from");
    // 放行保存的是最后一次预览；负载高时防抖可能先按输入到一半的内容预览过一次
    expect(calls.filter((call) => call.command === "aliases:preview").pop()?.args).toEqual(args);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull(), slow);
  }, 15000);

  it("新增时预填内置默认的时区与语言（字面量），没有「跟随全局」", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes());
    await userEvent.click(await screen.findByTestId("add-alias-button"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByTestId("alias-timezone-select")).toHaveValue("America/Los_Angeles");
    expect(within(dialog).getByTestId("alias-lang-select")).toHaveValue("en_US.UTF-8");
    expect(dialog).not.toHaveTextContent("跟随全局");
    expect(dialog).toHaveTextContent("保存即写入");
  });

  it("预览失败时禁止保存，不会写终端配置", async () => {
    const routes = aliasRoutes();
    let statusReads = 0;
    routes["aliases:status"] = async () => makeEnvelope(
      makeAliasStatus({ claude: statusReads++ === 0 ? CLAUDE : "claude --external" }),
      { command: "aliases.status" },
    );
    routes["aliases:preview"] = makeEnvelope({}, {
      command: "aliases.preview", ok: false,
      errors: [{ code: "ALIAS_ERROR", element: null, message: "~/.zshrc 已在外部修改" }],
    });
    const { calls } = renderWithEngine(<AliasesPage />, routes);
    await userEvent.click(await screen.findByTestId("edit-alias-claude"));
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("已在外部修改");
    expect(within(dialog).getByTestId("alias-form-submit")).toBeDisabled();
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
    await userEvent.click(within(dialog).getByRole("button", { name: "关闭并重新读取" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(screen.getByTestId("alias-row-claude")).toHaveTextContent("claude --external"));
    expect(statusReads).toBeGreaterThan(1);
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
  });

  it("输入变化后旧文件预览立即失效", async () => {
    const routes = aliasRoutes();
    let finish: ((envelope: Envelope) => void) | undefined;
    routes["aliases:preview"] = async (args) => {
      const changed = (valueOf(args, "--value") ?? "").includes("--verbose");
      if (changed) return new Promise<Envelope>((resolve) => { finish = resolve; });
      return makeEnvelope({ files: [{ target: "aliases:zshrc", path: "/isolated/.zshrc", exists: true, content: "第一次的文件" }], skipped: [] });
    };
    renderWithEngine(<AliasesPage />, routes);
    await userEvent.click(await screen.findByTestId("edit-alias-claude"));
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByTestId("alias-file-preview-aliases:zshrc")).toHaveTextContent("第一次的文件");
    await userEvent.type(within(dialog).getByTestId("alias-extra-args-input"), " --verbose");
    expect(within(dialog).queryByText("第一次的文件")).toBeNull();
    expect(within(dialog).getByTestId("alias-form-submit")).toBeDisabled();
    await waitFor(() => expect(finish).toBeDefined());
    finish?.(makeEnvelope({ files: [{ target: "aliases:zshrc", path: "/isolated/.zshrc", exists: true, content: "第二次的文件" }], skipped: [] }));
    expect(await within(dialog).findByTestId("alias-file-preview-aliases:zshrc")).toHaveTextContent("第二次的文件");
    expect(within(dialog).getByTestId("alias-form-submit")).not.toBeDisabled();
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
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "保存" })).not.toBeDisabled());
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
    await waitFor(() => expect(within(dialog).getByTestId("alias-form-submit")).not.toBeDisabled());
    await userEvent.click(within(dialog).getByTestId("alias-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    // 通知说人话：出了什么事、怎么办；原文留在任务记录里
    expect(await screen.findByText(/被别处改过，这次没有写入/)).toBeInTheDocument();
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
    await waitFor(() => expect(within(dialog).getByTestId("alias-form-submit")).not.toBeDisabled());
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
    await waitFor(() => expect(within(dialog).getByTestId("alias-form-submit")).not.toBeDisabled());
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

    await waitFor(() => expect(within(dialog).getByTestId("alias-form-submit")).not.toBeDisabled());
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
    const dialog = await screen.findByRole("dialog");
    const file = await within(dialog).findByTestId("alias-confirm-file-aliases:zshrc");
    expect(file).toHaveTextContent("# 保留用户注释");
    expect(file).toHaveTextContent("alias claude=");
    expect(file).not.toHaveTextContent("alias codex=");
    await userEvent.click(within(dialog).getByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const args = applyArgs(calls);
    expect(valueOf(args, "--remove")).toBe("codex");
    expect(valueOf(args, "--zshrc-base")).toBe("base-aliases:zshrc");
    expect(calls.find((call) => call.command === "aliases:preview")?.args).toEqual(args);
  });

  it("移除预览失败时不能确认；关闭重读后不写文件", async () => {
    const routes = aliasRoutes({ codex: "codex" });
    let statusReads = 0;
    routes["aliases:status"] = async () => makeEnvelope(
      makeAliasStatus({ claude: CLAUDE, codex: statusReads++ === 0 ? "codex" : "codex --external" }),
      { command: "aliases.status" },
    );
    routes["aliases:preview"] = makeEnvelope({}, {
      command: "aliases.preview", ok: false,
      errors: [{ code: "ALIAS_ERROR", element: null, message: "~/.zshrc 已在外部修改" }],
    });
    const { calls } = renderWithEngine(<AliasesPage />, routes);
    await userEvent.click(await screen.findByTestId("remove-alias-codex"));
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("已在外部修改");
    expect(within(dialog).getByRole("button", { name: "确认移除" })).toBeDisabled();
    await userEvent.click(within(dialog).getByRole("button", { name: "关闭并重新读取" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(screen.getByTestId("alias-row-codex")).toHaveTextContent("codex --external"));
    expect(statusReads).toBeGreaterThan(1);
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
  });

  it("来源推荐不自动写入，逐条确认后才添加，并标出来自哪个来源", async () => {
    const { calls } = renderWithEngine(
      <AliasesPage />,
      aliasRoutes({}, { recommendations: [{ name: "grok", value: "grok", line: "alias grok='grok'", source: "agent-skills" }] }),
    );
    const item = await screen.findByTestId("alias-recommend-grok");
    expect(within(item).getByTitle("alias grok='grok'")).toHaveTextContent("grok");
    expect(item).toHaveTextContent("来自 agent-skills");
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
    await userEvent.click(screen.getByTestId("add-recommended-grok"));
    const modal = await screen.findByRole("dialog");
    expect(modal).toHaveTextContent("alias grok='grok'");
    expect(await within(modal).findByTestId("alias-confirm-file-aliases:zshrc")).toHaveTextContent("alias grok='grok'");
    await userEvent.click(within(modal).getByRole("button", { name: "添加" }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const args = applyArgs(calls);
    expect(valueOf(args, "--recommended")).toBe("grok");
    expect(args).not.toContain("--value");
    expect(calls.find((call) => call.command === "aliases:preview")?.args).toEqual(args);
  });

  it("还没有来源别名：已登记的推荐来源给「取回」，没登记的引导去技能页添加", async () => {
    const suggested = { name: "agent-skills", description: "作者维护的技能与快捷命令", url: "https://github.com/ly87ing/agent-skills.git", collection: null as string | null };
    const onOpenSkills = vi.fn();
    const first = renderWithEngine(<AliasesPage onOpenSkills={onOpenSkills} />, aliasRoutes({}, { suggestedSources: [suggested] }));
    const section = await screen.findByTestId("alias-suggested-sources");
    expect(section).toHaveTextContent("不随程序发布");
    await userEvent.click(within(section).getByRole("button", { name: "技能页" }));
    expect(onOpenSkills).toHaveBeenCalled();
    first.unmount();

    const { calls } = renderWithEngine(
      <AliasesPage />,
      {
        ...aliasRoutes({}, { suggestedSources: [{ ...suggested, collection: "agent-skills" }] }),
        "skills:apply": makeEnvelope({ domain: "skills", steps: [], skipped: [] }, { command: "skills.apply" }),
      },
    );
    await userEvent.click(await screen.findByTestId("alias-fetch-source-agent-skills"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")!.args).toEqual([
      "--only", "skill:agent-skills", "--action", "update",
    ]);
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
    const modal = await screen.findByRole("dialog");
    expect(await within(modal).findByTestId("alias-confirm-file-aliases:zshrc")).toHaveTextContent(`alias claude='${legacy.value}'`);
    await userEvent.click(within(modal).getByRole("button", { name: "加入终端配置" }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    expect(valueOf(applyArgs(calls), "--legacy")).toBe(legacy.id);
    expect(calls.find((call) => call.command === "aliases:preview")?.args).toEqual(applyArgs(calls));

    await userEvent.click(screen.getByTestId("discard-legacy-claude"));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "丢弃" }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:discard-legacy")).toBe(true));
    expect(calls.find((call) => call.command === "aliases:discard-legacy")?.args).toEqual(["--legacy", legacy.id]);
  });

  it("用户自己写的 alias 收在「更多」里只读列出", async () => {
    renderWithEngine(<AliasesPage />, aliasRoutes({}, { external: { gp: "git push" } }));
    // 次要内容收在「更多」里
    await userEvent.click(await screen.findByTestId("aliases-more"));
    const item = await screen.findByTestId("alias-external-aliases:zshrc-gp");
    expect(within(item).getByTitle("alias gp='git push'")).toHaveTextContent("git push");
    const section = screen.getByTestId("alias-external");
    expect(section).toHaveTextContent("你自己写的");
    expect(section).toHaveTextContent("只读");
    expect(within(section).queryByRole("button")).toBeNull();
  });

  it("读写不了的文件显示原因，写入只带能写的那个文件的基准", async () => {
    const { calls } = renderWithEngine(
      <AliasesPage />,
      aliasRoutes({ codex: "codex" }, { blockedZprofile: "目标路径是符号链接，拒绝操作以保证安全" }),
    );
    // 有问题时终端配置文件直接摊开
    expect(await screen.findByTestId("alias-target-aliases:zprofile")).toHaveTextContent("符号链接");
    expect(screen.queryByTestId("aliases-more")).toBeNull();
    await userEvent.click(screen.getByTestId("remove-alias-codex"));
    const modal = await screen.findByRole("dialog");
    expect(await within(modal).findByTestId("alias-confirm-file-aliases:zshrc")).toHaveTextContent("# 保留用户注释");
    expect(within(modal).queryByTestId("alias-confirm-file-aliases:zprofile")).toBeNull();
    await userEvent.click(within(modal).getByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const args = applyArgs(calls);
    expect(args).toContain("--zshrc-base");
    expect(args).not.toContain("--zprofile-base");
  });
});
