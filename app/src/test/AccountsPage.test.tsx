import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AccountsPage from "../pages/AccountsPage";
import type { Envelope } from "../lib/contract";
import { agentManagementFixture, makeAliasStatus, makeApplyResult, makeElement, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

type Route = Envelope | ((args: string[]) => Promise<Envelope>);

const MAIN_ALIAS = 'TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" claude --dangerously-skip-permissions';
const DS_DEFINITION = {
  display_name: "DeepSeek",
  bin: "claude",
  profile: {
    base_agent: "claude-code",
    kind: "claude",
    config_dir: "~/.claude-ds",
    env: { ANTHROPIC_BASE_URL: "https://api.deepseek.com/anthropic", ANTHROPIC_AUTH_TOKEN: "${DEEPSEEK_API_KEY}" },
    model: "deepseek-v4",
  },
};

function valueOf(args: string[], flag: string): string | undefined {
  const index = args.indexOf(flag);
  return index >= 0 ? args[index + 1] : undefined;
}

function profileElement(id: string, configDir: string, observed: Record<string, unknown>) {
  return makeElement({
    id: `profiles:${id}`,
    domain: "profiles",
    spec: { display_name: id, base_agent: "claude-code", kind: "claude", config_dir: configDir, base_url: null, model: null },
    observed: { config_dir: configDir, blocked_reason: null, ...observed },
    state: "managed-ok",
  });
}

function routes(options: { previewVersion?: () => number; statusline?: string } = {}): Record<string, Route> {
  /** 登记成功后状态里才有新账号，目录文件还不存在（base = missing）。 */
  const registered: string[] = [];
  return {
    "agents:status": makeEnvelope({
      domain: "agents",
      elements: [
        makeElement({
          id: "agents:claude-code",
          domain: "agents",
          spec: { display_name: "Claude Code", bin: "claude" },
          observed: { management: agentManagementFixture("claude"), current_config: { state: "ready", file: "~/.claude/settings.json", base: "main-base", config: { runtime: { statusline: options.statusline ?? "builtin" } }, current: { profile: null, default: true, official: true } } },
        }),
        makeElement({ id: "agents:ds", domain: "agents", spec: DS_DEFINITION, observed: {} }),
      ],
    }, { command: "agents.status" }),
    "profiles:status": async () => makeEnvelope({
      domain: "profiles",
      elements: [...registered.map((id) => profileElement(id, `~/.claude-${id}`, { base: "missing" })), makeElement({
        id: "profiles:ds",
        domain: "profiles",
        spec: { display_name: "DeepSeek", base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-ds", base_url: "https://api.deepseek.com/anthropic", model: "deepseek-v4" },
        observed: { config_dir: "~/.claude-ds", third_party: true, base: "file-base", definition_base: "def-base", blocked_reason: null },
        state: "managed-ok",
      })],
    }, { command: "profiles.status" }),
    "aliases:status": makeEnvelope(makeAliasStatus({ claude: MAIN_ALIAS, ds: 'CLAUDE_CONFIG_DIR="$HOME/.claude-ds" claude' }), { command: "aliases.status" }),
    "aliases:preview": async (args) => makeEnvelope({
      files: [{ target: "aliases:zshrc", path: "/isolated/.zshrc", exists: true, content: `alias ${valueOf(args, "--set")}='${valueOf(args, "--value")}'\n` }],
      skipped: [],
    }, { command: "aliases.preview" }),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
    "secrets:status": makeEnvelope({
      domain: "secrets",
      elements: [makeElement({ id: "secrets:mcp-env", domain: "secrets", observed: { source: { configured: true, ready: true, path: "~/.config/agent-manager/secrets.env", names: ["DEEPSEEK_API_KEY"] } } })],
    }, { command: "secrets.status" }),
    "secrets:set": makeEnvelope({}, { command: "secrets.set" }),
    "agents:auth-status": async (args) => makeEnvelope(
      {
        element: valueOf(args, "--only"), state: "logged-in", logged_in: true, method: "claude.ai",
        account: valueOf(args, "--only") === "agents:claude-code" ? { email: "YOUR_EMAIL@example.com", org: "YOUR_ORG", plan: "Max 5x" } : null,
        third_party: false, mixed: false, reason: null,
      },
      { command: "agents.auth-status" },
    ),
    "profiles:preview": async (args) => makeEnvelope({
      path: `/isolated/${valueOf(args, "--only")}/settings.json`,
      content: `{"v":${options.previewVersion?.() ?? 1}}`,
      exists: false,
      file_base: "preview-file-base",
      content_base: "preview-content-base",
    }, { command: "profiles.preview" }),
    "policy:set": async (args) => {
      const id = valueOf(args, "--key")?.replace("agents.definitions.", "");
      if (id && valueOf(args, "--expect-definition") === "null") registered.push(id);
      return makeEnvelope({}, { command: "policy.set" });
    },
    "profiles:apply": makeEnvelope(makeApplyResult({ domain: "profiles" }), { command: "profiles.apply" }),
    "profiles:apply-definition": makeEnvelope(makeApplyResult({ domain: "profiles" }), { command: "profiles.apply-definition" }),
    "profiles:seed": makeEnvelope({ results: [{ item: "permissions", state: "copied", detail: "" }] }, { command: "profiles.seed" }),
    // 在用技能 alpha：主账号已链接；新登记的账号还没分配。beta 从没被选用，不自动加。
    "skills:status": async () => makeEnvelope({
      domain: "skills",
      elements: ["alpha", "beta"].map((name) => makeElement({
        id: `skill:repo/${name}`,
        domain: "skills",
        spec: { collection: "repo", name },
        observed: {
          agents: {
            "claude-code": name === "alpha" ? { state: "linked", assigned: true, enabled: true, actions: ["disable"] } : { state: "not-assigned", assigned: false, actions: ["add"] },
            ...Object.fromEntries(registered.map((id) => [id, { state: "not-assigned", assigned: false, actions: ["add"] }])),
          },
        },
      })),
    }, { command: "skills.status" }),
    "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
    // 只有 ds 账号上启用着的 MCP：新 Claude 账号从同格式的 ds 复制过来
    "mcp:status": async () => makeEnvelope({
      domain: "mcp",
      targets: ["claude-code", "ds", ...registered].map((agent) => ({ agent, path: `/${agent}/.claude.json`, format: "claude-json", exists: true, agent_enabled: true, disable_mode: "snapshot", error: null })),
      elements: [makeElement({
        id: "mcp:only-ds",
        domain: "mcp",
        observed: { agents: { ds: { state: "enabled", source: "file", base: "ds-base", actions: ["edit", "copy", "disable", "remove"] } } },
      })],
    }, { command: "mcp.status" }),
    "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
    "profiles:shared-status": async () => makeEnvelope({
      items: [{ item: "statusline", state: options.statusline === "builtin" || !options.statusline ? "same" : "different", source: "", target: "", changed_fields: [], field_diffs: [], source_base: "src-base", target_base: "dst-base", blocked_reason: null }],
    }, { command: "profiles.shared-status" }),
    "profiles:shared-apply": makeEnvelope(makeApplyResult({ domain: "profiles" }), { command: "profiles.shared-apply" }),
    "agents:preview-runtime": makeEnvelope({ path: "~/.claude/settings.json", content: "{}", file_base: "preview-main" }, { command: "agents.preview-runtime" }),
    "agents:configure": makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.configure" }),
  };
}

async function openCreate() {
  await userEvent.click(await screen.findByTestId("accounts-add"));
  return screen.findByTestId("account-drawer");
}

describe("账号页", () => {
  it("主账号与独立账号并列，一行一个，复制的是启动命令", async () => {
    renderWithEngine(<AccountsPage />, routes());
    const main = await screen.findByTestId("account-row-agents:claude-code");
    expect(within(main).getByText("主账号")).toBeInTheDocument();
    // 认得出是哪个账号：邮箱与档位，组织名在悬停提示里
    const identity = await within(main).findByText("官方 · YOUR_EMAIL@example.com · Max 5x");
    expect(identity).toHaveAttribute("title", "组织：YOUR_ORG");
    const ds = screen.getByTestId("account-row-profiles:ds");
    expect(within(ds).getByText("ds")).toBeInTheDocument();
    expect(within(ds).getByText("API · api.deepseek.com")).toBeInTheDocument();
  });

  it("打开页面只看本机登录记录；说明不了时显示「未确认」，点「确认」才带 --probe 用官方命令查", async () => {
    const base = routes();
    const { calls } = renderWithEngine(<AccountsPage />, {
      ...base,
      "agents:auth-status": async (args) =>
        args.includes("--probe")
          ? makeEnvelope({ element: "agents:claude-code", state: "logged-out", logged_in: false, method: null, account: null, third_party: false, mixed: false, reason: null }, { command: "agents.auth-status" })
          : makeEnvelope({ element: "agents:claude-code", state: "unknown", logged_in: null, method: null, account: null, third_party: false, mixed: false, reason: "本机登录记录里没有账号" }, { command: "agents.auth-status" }),
    });
    const main = await screen.findByTestId("account-row-agents:claude-code");
    expect(await within(main).findByText("官方 · 未确认")).toBeInTheDocument();
    expect(calls.filter((call) => call.command === "agents:auth-status").every((call) => !call.args.includes("--probe"))).toBe(true);
    await userEvent.click(within(main).getByTestId("account-probe-agents:claude-code"));
    expect(await within(main).findByText("官方 · 未登录")).toBeInTheDocument();
    expect(calls.find((call) => call.args.includes("--probe"))?.args).toEqual(["--only", "agents:claude-code", "--probe"]);
  });

  it("准备情况：逐项核对登录、启动命令、技能、MCP、规则，缺的就地补齐", async () => {
    const { calls } = renderWithEngine(<AccountsPage />, routes());
    const main = await screen.findByTestId("account-row-agents:claude-code");
    await userEvent.click(within(main).getByRole("button", { name: /更多/ }));
    await userEvent.click(await screen.findByTestId("account-menu-readiness-agents:claude-code"));
    const panel = await screen.findByTestId("account-readiness-agents:claude-code");
    expect(await within(panel).findByText("已登录 YOUR_EMAIL@example.com")).toBeInTheDocument();
    expect(within(panel).getByTestId("readiness-skills")).toHaveTextContent("在用的技能都有了");
    // 只在 ds 上启用着的服务：主账号缺，给补齐
    expect(await within(panel).findByText("缺 1 个在用的服务")).toBeInTheDocument();
    await userEvent.click(within(panel).getByTestId("readiness-action-mcp"));
    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));
    expect(calls.find((call) => call.command === "mcp:apply")!.args).toEqual([
      "--only", "mcp:only-ds", "--agent", "ds", "--to", "claude-code", "--action", "copy", "--base", "ds-base",
    ]);
  });

  it("新建官方账号：预览通过后依次登记、写目录、写启动命令（沿用主账号时区与参数）、带上全部共用设置", async () => {
    const { calls } = renderWithEngine(<AccountsPage />, routes());
    const drawer = await openCreate();
    await userEvent.type(within(drawer).getByTestId("account-name"), "Work");
    expect(within(drawer).getByTestId("account-id")).toHaveValue("work");
    expect(within(drawer).getByTestId("account-extra-args")).toHaveValue("--dangerously-skip-permissions");
    // 点「创建」前逐条列出要做的事：目录、文件、启动命令、技能与 MCP，以及不会动什么
    const plan = await within(drawer).findByTestId("account-create-plan");
    expect(plan).toHaveTextContent("新建目录 ~/.claude-work");
    expect(plan).toHaveTextContent("另从其他账号复制在用的 MCP：only-ds");
    expect(plan).toHaveTextContent("链接 1 个在用的技能");
    expect(plan).toHaveTextContent(`alias work='TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude --dangerously-skip-permissions'`);
    expect(plan).toHaveTextContent("不会改动主账号 ~/.claude");
    expect(plan).toHaveTextContent('CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude auth login');
    await userEvent.click(within(drawer).getByTestId("account-save"));
    await waitFor(() => expect(screen.queryByTestId("account-drawer")).toBeNull());

    const writes = calls.filter((call) => ["policy:set", "profiles:apply", "aliases:apply", "profiles:seed", "skills:apply", "mcp:apply"].includes(call.command));
    expect(writes.map((call) => call.command)).toEqual(["policy:set", "profiles:apply", "aliases:apply", "profiles:seed", "skills:apply", "mcp:apply"]);
    expect(writes[5].args).toEqual(["--only", "mcp:only-ds", "--agent", "ds", "--to", "work", "--action", "copy", "--base", "ds-base"]);
    // 只把在用的 alpha 链接给新账号
    expect(writes[4].args).toEqual(["--only", "skill:repo/alpha", "--action", "add", "--agent", "work"]);
    const definition = JSON.parse(valueOf(writes[0].args, "--value")!);
    expect(definition.profile).toMatchObject({ base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-work" });
    expect(valueOf(writes[0].args, "--expect-definition")).toBe("null");
    expect(valueOf(writes[1].args, "--base")).toBe("missing");
    expect(valueOf(writes[2].args, "--set")).toBe("work");
    expect(valueOf(writes[2].args, "--value")).toBe(
      'TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude --dangerously-skip-permissions',
    );
    expect(valueOf(writes[3].args, "--items")).toBe("permissions,statusline,hooks,rules,mcp");
    // 建好后直接给出新账号的准备情况
    expect(await screen.findByTestId("account-readiness-profiles:work")).toHaveTextContent("新账号「");
  });

  it("预览之后目标文件变了：不登记、不写任何文件", async () => {
    let version = 1;
    const { calls } = renderWithEngine(<AccountsPage />, routes({ previewVersion: () => version }));
    const drawer = await openCreate();
    await userEvent.type(within(drawer).getByTestId("account-name"), "Work");
    await within(drawer).findByTestId("account-create-plan");
    version = 2;
    await userEvent.click(within(drawer).getByTestId("account-save"));
    expect(await screen.findByText(/目标文件在预览后变了/)).toBeInTheDocument();
    expect(calls.some((call) => ["policy:set", "profiles:apply", "aliases:apply", "secrets:set"].includes(call.command))).toBe(false);
  });

  it("新建 API 账号：API Key 只进密钥文件，登记里只有变量引用", async () => {
    const { calls } = renderWithEngine(<AccountsPage />, routes());
    const drawer = await openCreate();
    await userEvent.click(within(drawer).getByTestId("account-type-choice-api"));
    await userEvent.type(within(drawer).getByTestId("account-name"), "Relay");
    await userEvent.type(within(drawer).getByTestId("account-base-url"), "https://relay.example.com");
    await userEvent.type(within(drawer).getByTestId("account-token-var"), "RELAY_KEY");
    await userEvent.type(within(drawer).getByTestId("account-api-key"), "sk-secret-value");
    await within(drawer).findByTestId("account-create-plan");
    await userEvent.click(within(drawer).getByTestId("account-save"));
    await waitFor(() => expect(screen.queryByTestId("account-drawer")).toBeNull());

    const secret = calls.findIndex((call) => call.command === "secrets:set");
    const policy = calls.findIndex((call) => call.command === "policy:set");
    expect(secret).toBeGreaterThanOrEqual(0);
    expect(secret).toBeLessThan(policy);
    expect(calls[secret].args).toEqual(["--var", "RELAY_KEY", "--value", "sk-secret-value"]);
    const value = valueOf(calls[policy].args, "--value")!;
    expect(value).not.toContain("sk-secret-value");
    expect(JSON.parse(value).profile.env).toMatchObject({ ANTHROPIC_BASE_URL: "https://relay.example.com", ANTHROPIC_AUTH_TOKEN: "${RELAY_KEY}" });
  });

  it("编辑 API 账号的模型：登记与目录文件一起写，带文件和登记的基准", async () => {
    const { calls } = renderWithEngine(<AccountsPage />, routes());
    const row = await screen.findByTestId("account-row-profiles:ds");
    await userEvent.click(within(row).getByRole("button", { name: "更多操作" }));
    await userEvent.click(within(row).getByRole("menuitem", { name: "编辑" }));
    const drawer = await screen.findByTestId("account-drawer");
    const model = within(drawer).getByTestId("account-model");
    await userEvent.clear(model);
    await userEvent.type(model, "deepseek-v5");
    await userEvent.click(within(drawer).getByTestId("account-save"));
    await waitFor(() => expect(screen.queryByTestId("account-drawer")).toBeNull());

    const apply = calls.find((call) => call.command === "profiles:apply-definition")!;
    expect(apply.args.slice(0, 6)).toEqual(["--only", "profiles:ds", "--base", "file-base", "--definition-base", "def-base"]);
    expect(JSON.parse(valueOf(apply.args, "--candidate")!)).toMatchObject({ model: "deepseek-v5" });
    expect(valueOf(apply.args, "--preview-file-base")).toBe("preview-file-base");
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("移除账号：只删登记与启动命令，不碰目录", async () => {
    const { calls } = renderWithEngine(<AccountsPage />, routes());
    const row = await screen.findByTestId("account-row-profiles:ds");
    await userEvent.click(within(row).getByRole("button", { name: "更多操作" }));
    await userEvent.click(within(row).getByRole("menuitem", { name: "移除" }));
    await userEvent.click(await screen.findByRole("button", { name: "移除" }));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));

    const policy = calls.find((call) => call.command === "policy:set")!;
    expect(JSON.parse(valueOf(policy.args, "--value")!)).toEqual({ _delete: true });
    expect(JSON.parse(valueOf(policy.args, "--expect-definition")!)).toEqual(DS_DEFINITION);
    expect(valueOf(calls.find((call) => call.command === "aliases:apply")!.args, "--remove")).toBe("ds");
    expect(calls.some((call) => call.command.startsWith("profiles:apply"))).toBe(false);
  });

  it("主账号的状态栏已是本工具的、各账号也一致：不提示", async () => {
    renderWithEngine(<AccountsPage />, routes());
    await screen.findByTestId("account-row-profiles:ds");
    expect(screen.queryByTestId("accounts-quota-notice")).toBeNull();
  });

  it("开启实时额度：先按完整文件预览的基准写主账号，再把状态栏同步给其余 Claude 账号", async () => {
    const { calls } = renderWithEngine(<AccountsPage />, routes({ statusline: "custom" }));
    await userEvent.click(await screen.findByTestId("accounts-enable-live-quota"));
    const dialog = await screen.findByRole("dialog", { name: "为全部 Claude 账号开启实时额度" });
    expect(dialog).toHaveTextContent("主账号现在的自定义状态栏会被替换");
    expect(calls.some((call) => call.command === "agents:configure")).toBe(false);
    await userEvent.click(within(dialog).getByRole("button", { name: "开启" }));
    await waitFor(() => expect(calls.some((call) => call.command === "profiles:shared-apply")).toBe(true));

    const writes = calls.filter((call) => ["agents:configure", "profiles:shared-apply"].includes(call.command));
    expect(writes.map((call) => call.command)).toEqual(["agents:configure", "profiles:shared-apply"]);
    expect(writes[0].args).toEqual(["--only", "agents:claude-code", "--base", "main-base", "--preview-file-base", "preview-main", "--runtime", '{"statusline":"builtin"}']);
    expect(writes[1].args).toEqual(["--only", "profiles:ds", "--item", "statusline", "--source-base", "src-base", "--target-base", "dst-base"]);
  });

  it("账号的启动命令：主账号列出直接调用 claude 的命令（区块外的只读），独立账号只列指向自己目录的", async () => {
    const r = routes();
    r["aliases:status"] = makeEnvelope(makeAliasStatus(
      { cc: 'claude --dangerously-skip-permissions', ds: 'CLAUDE_CONFIG_DIR="$HOME/.claude-ds" claude', "ds-yolo": 'CLAUDE_CONFIG_DIR="$HOME/.claude-ds" claude --dangerously-skip-permissions' },
      { external: { claude: MAIN_ALIAS } },
    ), { command: "aliases.status" });
    renderWithEngine(<AccountsPage />, r);
    const main = await screen.findByTestId("account-row-agents:claude-code");
    await userEvent.click(within(main).getByRole("button", { name: "更多操作" }));
    await userEvent.click(within(main).getByRole("menuitem", { name: "启动命令" }));
    let drawer = await screen.findByTestId("launch-commands-drawer");
    expect(within(drawer).getByTestId("launch-command-cc")).toBeInTheDocument();
    expect(within(drawer).getByTestId("launch-command-claude")).toHaveTextContent("你自己写的，只读");
    expect(within(drawer).queryByTestId("launch-edit-claude")).toBeNull();
    expect(within(drawer).queryByTestId("launch-command-ds")).toBeNull();
    await userEvent.click(within(drawer).getByRole("button", { name: "关闭" }));

    const ds = screen.getByTestId("account-row-profiles:ds");
    await userEvent.click(within(ds).getByRole("button", { name: "更多操作" }));
    await userEvent.click(within(ds).getByRole("menuitem", { name: "启动命令" }));
    drawer = await screen.findByTestId("launch-commands-drawer");
    expect(within(drawer).getByTestId("launch-command-ds")).toBeInTheDocument();
    expect(within(drawer).getByTestId("launch-command-ds-yolo")).toBeInTheDocument();
    expect(within(drawer).queryByTestId("launch-command-cc")).toBeNull();
  });

  it("删除一条启动命令：先看完整文件预览，确认后只删这一条", async () => {
    const { calls } = renderWithEngine(<AccountsPage />, routes());
    const ds = await screen.findByTestId("account-row-profiles:ds");
    await userEvent.click(within(ds).getByRole("button", { name: "更多操作" }));
    await userEvent.click(within(ds).getByRole("menuitem", { name: "启动命令" }));
    const drawer = await screen.findByTestId("launch-commands-drawer");
    await userEvent.click(within(drawer).getByTestId("launch-remove-ds"));
    const confirm = await screen.findByRole("button", { name: "确认删除" });
    await waitFor(() => expect(confirm).toBeEnabled());
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
    await userEvent.click(confirm);
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    const apply = calls.find((call) => call.command === "aliases:apply")!;
    expect(valueOf(apply.args, "--remove")).toBe("ds");
    expect(calls.find((call) => call.command === "aliases:preview")?.args).toEqual(apply.args);
  });

  it("开始使用：列出还没做的几步，就地去做；全做完或点关掉就不再出现", async () => {
    localStorage.clear();
    const r = routes({ statusline: "custom" });
    r["skills:status"] = makeEnvelope({ domain: "skills", elements: [], recommendations: [] }, { command: "skills.status" });
    let opened = 0;
    renderWithEngine(<AccountsPage onOpenSources={() => opened++} />, r);
    const card = await screen.findByTestId("getting-started");
    await waitFor(() => expect(screen.getByTestId("start-step-login")).toHaveAttribute("data-done", "true"));
    // 添加账号不是必须的，不在引导里
    expect(screen.queryByTestId("start-step-account")).toBeNull();
    expect(screen.getByTestId("start-step-source")).toHaveAttribute("data-done", "false");
    expect(card).toHaveTextContent("1/2");
    await userEvent.click(screen.getByTestId("start-action-source"));
    expect(opened).toBe(1);
    await userEvent.click(screen.getByTestId("getting-started-hide"));
    expect(screen.queryByTestId("getting-started")).toBeNull();
  });

  it("开始使用：技能来源可以不用，点「不用」就算这一步做完，不写任何配置", async () => {
    localStorage.clear();
    const r = routes();
    r["skills:status"] = makeEnvelope({ domain: "skills", elements: [], recommendations: [] }, { command: "skills.status" });
    const { calls } = renderWithEngine(<AccountsPage />, r);
    await waitFor(() => expect(screen.getByTestId("start-step-login")).toHaveAttribute("data-done", "true"));
    expect(screen.getByTestId("start-step-source")).toHaveTextContent("可选");
    await userEvent.click(screen.getByTestId("start-skip-source"));
    // 登录那一步已做完：整张清单收起，下次也不再催
    expect(screen.queryByTestId("getting-started")).toBeNull();
    expect(localStorage.getItem("agent-manager:getting-started-skipped")).toBe('["source"]');
    expect(calls.some((call) => /:(apply|set)$/.test(call.command))).toBe(false);
  });

  it("几步都做完时不显示开始使用", async () => {
    localStorage.clear();
    const r = routes();
    r["skills:status"] = makeEnvelope({ domain: "skills", elements: [makeElement({ id: "skill:agent-skills", domain: "skills" })], recommendations: [] }, { command: "skills.status" });
    renderWithEngine(<AccountsPage />, r);
    await screen.findByTestId("accounts-list");
    await waitFor(() => expect(screen.getByTestId("account-type-agents:claude-code")).toHaveTextContent("Max 5x"));
    expect(screen.queryByTestId("getting-started")).toBeNull();
  });
});
