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

function routes(options: { previewVersion?: () => number } = {}): Record<string, Route> {
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
          observed: { management: agentManagementFixture("claude"), current_config: { state: "ready", file: "~/.claude/settings.json", current: { profile: null, default: true, official: true } } },
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
      { element: valueOf(args, "--only"), state: "logged-in", logged_in: true, method: "claude.ai", third_party: false, mixed: false, reason: null },
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
    expect(await within(main).findByText("官方 · 已登录")).toBeInTheDocument();
    const ds = screen.getByTestId("account-row-profiles:ds");
    expect(within(ds).getByText("ds")).toBeInTheDocument();
    expect(within(ds).getByText("API · api.deepseek.com")).toBeInTheDocument();
  });

  it("新建官方账号：预览通过后依次登记、写目录、写启动命令（沿用主账号时区与参数）、带上全部共用设置", async () => {
    const { calls } = renderWithEngine(<AccountsPage />, routes());
    const drawer = await openCreate();
    await userEvent.type(within(drawer).getByTestId("account-name"), "Work");
    expect(within(drawer).getByTestId("account-id")).toHaveValue("work");
    expect(within(drawer).getByTestId("account-extra-args")).toHaveValue("--dangerously-skip-permissions");
    expect(await within(drawer).findByText(/将写入 2 个文件/)).toBeInTheDocument();
    await userEvent.click(within(drawer).getByTestId("account-save"));
    await waitFor(() => expect(screen.queryByTestId("account-drawer")).toBeNull());

    const writes = calls.filter((call) => ["policy:set", "profiles:apply", "aliases:apply", "profiles:seed", "skills:apply"].includes(call.command));
    expect(writes.map((call) => call.command)).toEqual(["policy:set", "profiles:apply", "aliases:apply", "profiles:seed", "skills:apply"]);
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
  });

  it("预览之后目标文件变了：不登记、不写任何文件", async () => {
    let version = 1;
    const { calls } = renderWithEngine(<AccountsPage />, routes({ previewVersion: () => version }));
    const drawer = await openCreate();
    await userEvent.type(within(drawer).getByTestId("account-name"), "Work");
    await within(drawer).findByText(/将写入 2 个文件/);
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
    await within(drawer).findByText(/将写入 2 个文件/);
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
});
