import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import ProgramsPage from "../pages/ProgramsPage";
import { ProgramActionDialog } from "../components/ProgramActionDialog";
import type { Envelope } from "../lib/contract";
import type { ProgramElement, ProgramsData } from "../lib/programs";
import { makeElement, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

const unknown = { state: "unknown", current: null, upstream: null, checked_at: null, probe: null, cache_age_sec: null } as const;

function program(overrides: Partial<ProgramElement>): ProgramElement {
  return {
    id: `programs:${overrides.program}`,
    program: "npm:x",
    kind: "npm",
    name: "x",
    bins: [],
    all_bins: [],
    mode: "delegated",
    owner: null,
    version: null,
    installs: [],
    install: [],
    runners: [],
    dependents: [],
    problems: [],
    commands: {},
    managed_versions: [],
    managed_current: null,
    freshness: unknown,
    ...overrides,
  } as ProgramElement;
}

function data(): ProgramsData {
  return {
    domain: "programs",
    elements: [
      program({
        program: "npm:@wecom/cli",
        name: "@wecom/cli",
        bins: ["wecom-cli"],
        all_bins: ["wecom-cli"],
        mode: "delegated",
        owner: "npm-global",
        version: "1.3.4",
        installs: [{ path: "~/.npm-global/bin/wecom-cli", real: "~/.npm-global/lib/node_modules/@wecom/cli/bin/wecom.js", owner: "npm-global", program: "npm:@wecom/cli", version: "1.3.4", app: null, package_dir: "~/.npm-global/lib/node_modules/@wecom/cli" }],
        dependents: [{ kind: "skill", name: "wecomcli-doc", agents: ["claude-code"], source: "declared", bins: ["wecom-cli"], any: false }],
        commands: { upgrade: "npm install -g @wecom/cli@latest" },
        freshness: { ...unknown, state: "behind", current: "1.3.4", upstream: "1.3.5" },
      }),
      program({
        program: "npm:json",
        name: "json",
        bins: ["json"],
        mode: "missing",
        dependents: [{ kind: "skill", name: "jsontool", agents: ["claude-code"], source: "declared", bins: ["json"], any: false }],
        problems: [{ code: "missing", bins: ["json"] }],
      }),
      program({
        program: "git:example.com/mcp/code",
        kind: "git",
        name: "example.com/mcp/code",
        mode: "runner",
        runners: [{ server: "example-code", agent: "codex", runner: "uvx", version: "main", pinned: false }],
        dependents: [{ kind: "mcp", name: "example-code", agents: ["codex"], source: "mcp", bins: [], any: false }],
        problems: [{ code: "unpinned", server: "example-code", agent: "codex", version: "main" }],
        freshness: { ...unknown, upstream: "7a2968335f2198b966864de1ce3655c8e485a653" },
      }),
    ],
    suggestions: [{ skill: "xiaoyu-bug-loop", bin: "playwright-cli", program: null, line: "command -v playwright-cli", found: true }],
    user_path: { dirs: ["~/.local/bin", "~/.npm-global/bin"], entry_candidates: ["~/.local/bin", "~/.npm-global/bin"], error: null },
    entry_dir: null,
    allow_scripts: [],
    links: {},
    backups: [{ id: "20260930T010101000-npm__wecom_cli", program: "npm:@wecom/cli", version: "1.3.3", reason: "upgrade", created_at: "2026-09-30T01:01:01Z" }],
  };
}

const PLAN = {
  program: "npm:@wecom/cli",
  action: "upgrade",
  target_version: "1.3.5",
  steps: [
    { kind: "move", from: "~/.npm-global/lib/node_modules/@wecom/cli", to: "/data/programs/backups/x/package" },
    { kind: "exec", command: "npm install -g --prefix ~/.npm-global --ignore-scripts @wecom/cli@1.3.5" },
  ],
  notes: ["改动前把 ~/.npm-global/lib/node_modules/@wecom/cli 整个移到备份，失败或回退时原样移回"],
  scripts_ignored: true,
  backup: "x",
  manual: ["npm install -g --prefix ~/.npm-global --ignore-scripts @wecom/cli@1.3.5"],
};

function routes(): Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> {
  return {
    "programs:status": makeEnvelope(data(), { command: "programs.status" }),
    "programs:preview": async (args) => makeEnvelope({ ...PLAN, action: args[args.indexOf("--action") + 1] }, { command: "programs.preview" }),
    "programs:apply": makeEnvelope({ ...PLAN, scripts_skipped: [] }, { command: "programs.apply" }),
    "programs:declare": makeEnvelope({ declared: "xiaoyu-bug-loop" }, { command: "programs.declare" }),
    "mcp:status": makeEnvelope({
      domain: "mcp",
      targets: [],
      elements: [makeElement({
        id: "mcp:example-code",
        domain: "mcp",
        observed: { agents: { codex: { state: "enabled", source: "file", base: "b".repeat(64), actions: ["edit"], entry: { command: "uvx", args: ["--from", "git+https://example.com/mcp/code@main", "example-code", "start"], env: { K: "${K}" } } } } },
      })],
    }, { command: "mcp.status" }),
    "mcp:preview": makeEnvelope({ files: [{ path: "~/.codex/config.toml", exists: true, original: "args = [\"@main\"]\n", content: "args = [\"@7a29683\"]\n" }] }, { command: "mcp.preview" }),
    "mcp:apply": makeEnvelope({ domain: "mcp", steps: [], skipped: [] }, { command: "mcp.apply" }),
  };
}

describe("程序页：谁需要、装在哪、什么版本；动作先看计划，也能自己执行", () => {
  it("未知安装方式和缺启动器直达依赖配置；多份命令可查看完整位置，不猜测安装", async () => {
    const contents = data();
    contents.elements = [
      program({ program: "bin:helper", kind: "bin", name: "helper", mode: "missing", problems: [{ code: "missing-no-install", bins: ["helper"] }], dependents: [{ kind: "skill", name: "helper-skill", agents: ["codex"], source: "declared", bins: ["helper"], any: false }] }),
      program({ program: "git:example.com/mcp/missing", kind: "git", name: "missing", mode: "runner", runners: [{ server: "missing-server", agent: "codex", runner: "uvx", version: "main", pinned: false }], problems: [{ code: "runner-missing", bin: "uvx" }], dependents: [{ kind: "mcp", name: "missing-server", agents: ["codex"], source: "mcp", bins: [], any: false }] }),
      program({ program: "bin:duplicate", kind: "bin", name: "duplicate", mode: "observed", problems: [{ code: "multiple", bin: "duplicate", paths: ["/isolated/a/duplicate", "/isolated/b/duplicate"] }] }),
    ];
    const onOpen = vi.fn();
    const { calls } = renderWithEngine(<ProgramsPage onOpen={onOpen} />, { ...routes(), "programs:status": makeEnvelope(contents) });
    await userEvent.click(await screen.findByTestId("program-check-source-bin:helper"));
    expect(onOpen).toHaveBeenLastCalledWith("skills:helper-skill");
    await userEvent.click(screen.getByTestId("program-check-runner-git:example.com/mcp/missing"));
    expect(onOpen).toHaveBeenLastCalledWith("mcp:missing-server");
    expect(screen.queryByTestId("program-pin-git:example.com/mcp/missing")).toBeNull();
    const duplicate = screen.getByTestId("program-row-bin:duplicate");
    await userEvent.click(within(duplicate).getByText("查看 duplicate 的位置"));
    expect(duplicate).toHaveTextContent("/isolated/a/duplicate");
    expect(duplicate).toHaveTextContent("/isolated/b/duplicate");
    expect(calls.some((call) => call.command === "programs:preview" || call.command === "programs:apply" || call.command === "mcp:apply")).toBe(false);
  });

  it.each(["delegated", "managed"] as const)("%s 仅在确认有新版本时提供升级入口", async (mode) => {
    const states = [
      { state: "current", checked_at: "2026-10-08T00:00:00Z", label: "已是最新" },
      { state: "unknown", checked_at: null, label: "未检查" },
      { state: "unknown", checked_at: "2026-10-08T00:00:00Z", label: "版本未知" },
      { state: "ahead", checked_at: "2026-10-08T00:00:00Z", label: "本地较新" },
    ] as const;
    const contents = data();
    contents.elements = states.map((item, i) => program({
      program: `npm:state-${i}`, name: `state-${i}`, mode, version: "1.3.4",
      freshness: { ...unknown, state: item.state, current: "1.3.4", upstream: item.state === "unknown" ? null : item.state === "ahead" ? "1.3.3" : "1.3.4", checked_at: item.checked_at },
    }));
    const { calls } = renderWithEngine(<ProgramsPage />, { ...routes(), "programs:status": makeEnvelope(contents) });
    for (const [i, item] of states.entries()) {
      const row = await screen.findByTestId(`program-row-npm:state-${i}`);
      expect(row).toHaveTextContent(item.label);
      expect(within(row).queryByRole("button", { name: /升级/ })).toBeNull();
    }
    expect(calls.some((call) => ["programs:preview", "programs:apply"].includes(call.command))).toBe(false);
  });

  it("一行一个程序：管理方式、位置与版本、依赖方、统一状态标签；问题多的排前面", async () => {
    renderWithEngine(<ProgramsPage />, routes());
    const rows = await screen.findAllByTestId(/^program-row-/);
    expect(rows.map((row) => row.getAttribute("data-testid"))).toEqual([
      "program-row-npm:json",
      "program-row-npm:@wecom/cli",
      "program-row-git:example.com/mcp/code",
    ]);
    const wecom = screen.getByTestId("program-row-npm:@wecom/cli");
    expect(wecom).toHaveTextContent("代管");
    expect(wecom).toHaveTextContent("npm 全局 · ~/.npm-global/bin/wecom-cli");
    expect(wecom).toHaveTextContent("可更新");
    expect(within(wecom).getByRole("button", { name: "升级到 1.3.5" })).toBeInTheDocument();
    expect(wecom).toHaveTextContent("技能 wecomcli-doc");
    expect(screen.getByTestId("program-row-npm:json")).toHaveTextContent("缺 json");
    expect(screen.getByTestId("program-row-git:example.com/mcp/code")).toHaveTextContent("未固定版本");
  });

  it("代管升级：打开后先要计划（不写），列出步骤与自己执行的命令；确认后才执行", async () => {
    const { calls } = renderWithEngine(<ProgramsPage />, routes());
    await userEvent.click(await screen.findByTestId("program-upgrade-npm:@wecom/cli"));
    const dialog = await screen.findByTestId("program-action-dialog");
    await within(dialog).findByTestId("program-plan-steps");
    expect(calls.find((call) => call.command === "programs:preview")!.args).toEqual(["--program", "npm:@wecom/cli", "--action", "upgrade", "--version", "1.3.5"]);
    expect(within(dialog).getByTestId("program-manual")).toHaveTextContent("npm install -g --prefix ~/.npm-global --ignore-scripts @wecom/cli@1.3.5");
    expect(within(dialog).queryByTestId("program-entry-dir")).toBeNull();
    expect(calls.some((call) => call.command === "programs:apply")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "升级" }));
    await waitFor(() => expect(calls.some((call) => call.command === "programs:apply")).toBe(true));
    expect(calls.find((call) => call.command === "programs:apply")!.args).toEqual(["--program", "npm:@wecom/cli", "--action", "upgrade", "--version", "1.3.5"]);
  });

  it.each([
    { kind: "pypi", mode: "delegated", code: "PROGRAM_VERSION_UNVERIFIED" },
    { kind: "pypi", mode: "managed", code: "PROGRAM_VERSION_UNVERIFIED" },
    { kind: "pypi", mode: "managed", code: "PROGRAM_ENTRY_UNVERIFIED" },
    { kind: "npm", mode: "managed", code: "PROGRAM_VERSION_UNVERIFIED" },
    { kind: "npm", mode: "managed", code: "PROGRAM_ENTRY_UNVERIFIED" },
    { kind: "git", mode: "managed", code: "PROGRAM_VERSION_UNVERIFIED" },
    { kind: "git", mode: "managed", code: "PROGRAM_ENTRY_UNVERIFIED" },
  ] as const)("$kind $mode $code keeps inspection details visible without promising restoration", async ({ kind, mode, code }) => {
    const contents = data();
    const id = kind === "git" ? "git:example.invalid/owner/tool-a" : `${kind}:tool-a`;
    const current = kind === "git" ? "a".repeat(40) : "1.0.0", target = kind === "git" ? "b".repeat(40) : "2.0.0";
    contents.elements = [program({ program: id, kind, name: "tool-a", mode, owner: mode === "managed" ? "managed" : "uv-tool", version: current, freshness: { ...unknown, state: "behind", current, upstream: target } })];
    const plan = { ...PLAN, program: id, target_version: target, notes: ["The unverified candidate is retained for inspection."], manual: [kind === "git" ? `uv --no-python-downloads tool install --force git+https://example.invalid/owner/tool-a@${target}` : kind === "npm" ? "npm install tool-a@2.0.0" : "uv --no-python-downloads tool install --force tool-a==2.0.0"] };
    const diagnostic = kind === "git" ? `Managed Git ${code === "PROGRAM_ENTRY_UNVERIFIED" ? "command" : "source/commit"} verification before completion failed. The candidate is retained for inspection.` : kind === "npm" ? `Managed npm ${code === "PROGRAM_ENTRY_UNVERIFIED" ? "command" : "identity/version"} verification before completion failed. The candidate is retained for inspection.` : code === "PROGRAM_ENTRY_UNVERIFIED"
      ? "Managed PyPI command verification before completion failed. The candidate at ~/programs/tool-a/2.0.0 is retained. Inspect its bin entries."
      : "uv finished, but the installed version is unknown. No automatic restoration was attempted. Inspect ~/uv/data/tools/tool-a and the backup.";
    const { calls } = renderWithEngine(<ProgramsPage />, {
      ...routes(), "programs:status": makeEnvelope(contents),
      "programs:preview": makeEnvelope(plan),
      "programs:apply": makeEnvelope(plan, { ok: false, errors: [{ code, element: null, message: diagnostic }] }),
    });
    await userEvent.click(await screen.findByTestId(`program-upgrade-${id}`));
    await screen.findByText(kind === "git" ? /Git 依赖会核对来源、完整提交/ : kind === "npm" ? /自管 npm 会在完成标记和当前版本选择前核验包身份/ : mode === "managed" ? /自管 PyPI 会在完成标记和当前版本选择前核验版本/ : /uv 版本无法核验时保留现场，不自动恢复/);
    await screen.findByText(plan.notes[0]);
    expect(screen.queryByText(/执行失败时尝试恢复/)).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "升级" }));
    await screen.findAllByText(kind === "git" ? /Managed Git.*verification before completion failed/ : kind === "npm" ? /Managed npm.*verification before completion failed/ : code === "PROGRAM_ENTRY_UNVERIFIED" ? /Managed PyPI command verification before completion failed/ : /uv finished, but the installed version is unknown/);
    expect(screen.getByTestId("program-action-dialog")).toBeInTheDocument();
    expect(document.querySelector('[data-sonner-toast][data-type="warning"]')).toBeInTheDocument();
    expect(screen.queryByText(/可以复制上面的命令自己执行/)).toBeNull();
    expect(within(screen.getByTestId("confirm-modal-feedback")).getByRole("alert")).toHaveClass("text-warning");
    expect(screen.getByTestId("confirm-modal-feedback")).toHaveAttribute("tabindex", "0");
    expect(calls.filter((call) => call.command === "programs:apply")).toHaveLength(1);
  });

  it.each(["install", "upgrade", "switch", "adopt"] as const)("npm %s explains retained candidates before confirmation", async action => {
    const { calls } = renderWithEngine(<ProgramActionDialog
      program={program({ program: "npm:tool-a", kind: "npm", name: "tool-a", mode: action === "install" ? "missing" : action === "adopt" ? "delegated" : "managed" })}
      action={action} version="2.0.0" entryCandidates={["~/bin"]} entryDir="~/bin" onClose={vi.fn()}
    />, { ...routes(), "programs:preview": makeEnvelope({ ...PLAN, program: "npm:tool-a", action, target_version: "2.0.0" }) });
    await screen.findByText(/自管 npm 会在完成标记和当前版本选择前核验包身份/);
    expect(screen.queryByText(/执行失败时尝试恢复/)).toBeNull();
    expect(calls.some(call => call.command === "programs:apply")).toBe(false);
  });

  it("安装原本没有的程序：选入口目录（默认 ~/.local/bin），安装脚本默认不执行、勾选后重新算计划", async () => {
    const { calls } = renderWithEngine(<ProgramsPage />, routes());
    await userEvent.click(await screen.findByTestId("program-install-npm:json"));
    const dialog = await screen.findByTestId("program-action-dialog");
    expect(within(dialog).getByTestId("program-entry-dir")).toHaveAttribute("data-value", "~/.local/bin");
    await waitFor(() => expect(calls.filter((call) => call.command === "programs:preview")).toHaveLength(1));
    expect(calls.find((call) => call.command === "programs:preview")!.args).toEqual(["--program", "npm:json", "--action", "install", "--entry-dir", "~/.local/bin"]);
    await userEvent.click(within(dialog).getByTestId("program-allow-scripts"));
    await waitFor(() => expect(calls.filter((call) => call.command === "programs:preview")).toHaveLength(2));
    expect(calls.filter((call) => call.command === "programs:preview")[1].args).toContain("--allow-scripts");
  });

  it("回退在更多操作里，按备份选版本", async () => {
    const { calls } = renderWithEngine(<ProgramsPage />, routes());
    await userEvent.click(await screen.findByRole("button", { name: "@wecom/cli 的更多操作" }));
    await userEvent.click(await screen.findByTestId("program-restore-20260930T010101000-npm__wecom_cli"));
    await screen.findByTestId("program-action-dialog");
    await waitFor(() => expect(calls.some((call) => call.command === "programs:preview")).toBe(true));
    expect(calls.find((call) => call.command === "programs:preview")!.args).toEqual(["--program", "npm:@wecom/cli", "--action", "restore", "--backup", "20260930T010101000-npm__wecom_cli"]);
  });

  it("按需拉取的 MCP：固定版本改的是条目参数里版本那一段，env 原样；先预览改动，确认后逐个账号写入", async () => {
    const { calls } = renderWithEngine(<ProgramsPage />, routes());
    await userEvent.click(await screen.findByTestId("program-pin-git:example.com/mcp/code"));
    const dialog = await screen.findByTestId("program-pin-dialog");
    expect(within(dialog).getByTestId("program-pin-version")).toHaveValue("7a2968335f2198b966864de1ce3655c8e485a653");
    await waitFor(() => expect(calls.some((call) => call.command === "mcp:preview")).toBe(true));
    const preview = calls.find((call) => call.command === "mcp:preview")!;
    const spec = JSON.parse(preview.args[preview.args.indexOf("--spec") + 1]);
    expect(spec).toEqual({ command: "uvx", args: ["--from", "git+https://example.com/mcp/code@7a2968335f2198b966864de1ce3655c8e485a653", "example-code", "start"], env: { K: "${K}" } });
    expect(preview.args.slice(0, 8)).toEqual(["--only", "mcp:example-code", "--agent", "codex", "--action", "edit", "--base", "b".repeat(64)]);
    expect(calls.some((call) => call.command === "mcp:apply")).toBe(false);
    await userEvent.click(await screen.findByRole("button", { name: "写入" }));
    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));
  });

  it("没声明的命令作为候选；确认后记为这个技能的依赖（只写本工具的库）", async () => {
    const { calls } = renderWithEngine(<ProgramsPage />, routes());
    const row = await screen.findByTestId("program-suggestion-xiaoyu-bug-loop-playwright-cli");
    expect(row).toHaveTextContent("command -v playwright-cli");
    await userEvent.click(within(row).getByRole("button", { name: "确认依赖" }));
    await waitFor(() => expect(calls.some((call) => call.command === "programs:declare")).toBe(true));
    expect(calls.find((call) => call.command === "programs:declare")!.args).toEqual(["--skill", "xiaoyu-bug-loop", "--bins", "[\"playwright-cli\"]"]);
  });

  it("依赖方点过去：技能与 MCP 各到自己的页面", async () => {
    const onOpen = vi.fn();
    renderWithEngine(<ProgramsPage onOpen={onOpen} />, routes());
    await userEvent.click(await screen.findByTestId("program-dependent-npm:@wecom/cli-wecomcli-doc"));
    expect(onOpen).toHaveBeenLastCalledWith("skills:wecomcli-doc");
    await userEvent.click(screen.getByTestId("program-dependent-git:example.com/mcp/code-example-code"));
    expect(onOpen).toHaveBeenLastCalledWith("mcp:example-code");
  });

  it("检查新版本进行中：升级、安装、固定版本都先不能点，结束后恢复", async () => {
    let release: (() => void) | null = null;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const r = routes();
    r["system:refresh-upstream"] = async () => {
      await gate;
      return makeEnvelope({ elements: [], summary: { probed: 0, current: 0, behind: 0, ahead: 0, unknown: 0 } }, { command: "system.refresh-upstream" });
    };
    renderWithEngine(<ProgramsPage />, r);
    await userEvent.click(await screen.findByTestId("programs-check-updates"));
    expect(await screen.findByTestId("programs-checking")).toBeInTheDocument();
    expect(screen.getByTestId("program-upgrade-npm:@wecom/cli")).toBeDisabled();
    expect(screen.getByTestId("program-install-npm:json")).toBeDisabled();
    expect(screen.getByTestId("program-pin-git:example.com/mcp/code")).toBeDisabled();
    expect(screen.queryByRole("button", { name: "@wecom/cli 的更多操作" })).toBeNull();
    release!();
    await waitFor(() => expect(screen.queryByTestId("programs-checking")).toBeNull());
    expect(screen.getByTestId("program-upgrade-npm:@wecom/cli")).toBeEnabled();
  });

  it("升级完成后只重新检查这一项，显示的版本与实际一致", async () => {
    const r = routes();
    r["system:refresh-upstream"] = makeEnvelope({ elements: [], summary: { probed: 1, current: 1, behind: 0, ahead: 0, unknown: 0 } }, { command: "system.refresh-upstream" });
    const { calls } = renderWithEngine(<ProgramsPage />, r);
    await userEvent.click(await screen.findByTestId("program-upgrade-npm:@wecom/cli"));
    await within(await screen.findByTestId("program-action-dialog")).findByTestId("program-plan-steps");
    await userEvent.click(screen.getByRole("button", { name: "升级" }));
    await waitFor(() => expect(calls.some((call) => call.command === "system:refresh-upstream")).toBe(true));
    expect(calls.find((call) => call.command === "system:refresh-upstream")!.args).toEqual(["--only", "programs:npm:@wecom/cli"]);
  });

  it("已固定的 MCP 可以取消固定：改回 @latest，写入后记下「跟最新版本」，以后不再报未固定版本", async () => {
    const r = routes();
    const status = r["programs:status"] as Envelope<ProgramsData>;
    status.data.elements.push(program({
      program: "npm:chrome-devtools-mcp",
      name: "chrome-devtools-mcp",
      mode: "runner",
      runners: [{ server: "chrome-devtools", agent: "codex", runner: "npx", version: "1.9.0", pinned: true }],
      dependents: [{ kind: "mcp", name: "chrome-devtools", agents: ["codex"], source: "mcp", bins: [], any: false }],
      freshness: { ...unknown, state: "behind", current: "1.9.0", upstream: "1.10.1" },
    }));
    const mcp = r["mcp:status"] as Envelope<{ elements: ReturnType<typeof makeElement>[] }>;
    mcp.data.elements.push(makeElement({
      id: "mcp:chrome-devtools",
      domain: "mcp",
      observed: { agents: { codex: { state: "enabled", source: "file", base: "c".repeat(64), actions: ["edit"], entry: { command: "npx", args: ["-y", "chrome-devtools-mcp@1.9.0", "--isolated"] } } } },
    }));
    const { calls } = renderWithEngine(<ProgramsPage />, r);
    expect(await screen.findByTestId("program-pin-npm:chrome-devtools-mcp")).toHaveTextContent("升级到 1.10.1");
    await userEvent.click(screen.getByRole("button", { name: "chrome-devtools-mcp 的更多操作" }));
    await userEvent.click(await screen.findByTestId("program-unpin-npm:chrome-devtools-mcp"));
    const dialog = await screen.findByTestId("program-pin-dialog");
    expect(within(dialog).queryByTestId("program-pin-version")).toBeNull();
    await waitFor(() => expect(calls.some((call) => call.command === "mcp:preview")).toBe(true));
    const preview = calls.find((call) => call.command === "mcp:preview")!;
    expect(JSON.parse(preview.args[preview.args.indexOf("--spec") + 1]).args).toEqual(["-y", "chrome-devtools-mcp@latest", "--isolated"]);
    await userEvent.click(await screen.findByRole("button", { name: "写入" }));
    await waitFor(() => expect(calls.some((call) => call.command === "programs:declare")).toBe(true));
    expect(calls.find((call) => call.command === "programs:declare")!.args).toEqual(["--server", "chrome-devtools", "--follow-latest", "true"]);
  });
});
