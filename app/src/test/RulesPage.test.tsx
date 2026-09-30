import { describe, expect, it, vi } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import { toast } from "sonner";
import RulesPage from "../pages/RulesPage";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode } from "./fixtures";
import type { Envelope } from "../lib/contract";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

const SOURCE_BODY = "- Use Chinese (Simplified).\n- Prefer the simplest local change.\n";
const SOURCE_RULE = { id: "agent-skills/core", collection: "agent-skills", name: "core", body: SOURCE_BODY, revision: "rev-core", local: false };
const CLAUDE_BODY = "# Claude 规则\n- 回答用简体中文。\n";
const CLAUDE_BASE = "b".repeat(64);
const CODEX_BASE = "c".repeat(64);
const WRITE = [{ id: "write", risk: "write-local" as const, needs_lock: true, offline_ok: true }];

function ruleElement(
  id: string,
  name: string,
  target: string,
  observed: Record<string, unknown>,
  state: "managed-ok" | "missing" | "unmanaged-detected" | "disabled",
  writable: boolean,
) {
  return makeElement({
    id,
    domain: "rules",
    spec: { name, group: id.startsWith("rules:profile:") ? "profile-rules" : "global-rules", kind: id.includes("codex") ? "codex" : "claude", profile_id: null, target },
    policy: { enabled: state !== "disabled", agent_id: "claude-code" },
    observed: { target: `/tmp/home/${target.slice(2)}`, target_exists: null, content: null, base: null, matched_template: null, matched_source_rule: null, source_follow: null, blocked_reason: null, ...observed },
    state,
    freshness: makeFreshness(),
    actions: writable ? WRITE : [],
  });
}

/** 规则新契约的一组典型目标：与来源规则一致、有文件、缺文件的实例、阻塞与停用的实例。 */
function rulesRoutes(): Record<string, Envelope> {
  return {
    "rules:status": makeEnvelope(
      {
        domain: "rules",
        elements: [
          ruleElement("rules:codex_global", "Codex 全局规则", "~/.codex/AGENTS.md",
            { target_exists: true, content: SOURCE_BODY, base: CODEX_BASE, matched_source_rule: { id: SOURCE_RULE.id, name: "core", collection: "agent-skills" } }, "managed-ok", true),
          ruleElement("rules:claude_global", "Claude Code 全局规则", "~/.claude/CLAUDE.md",
            { target_exists: true, content: CLAUDE_BODY, base: CLAUDE_BASE, matched_template: { id: "tpl-team", name: "团队规则" } }, "managed-ok", true),
          ruleElement("rules:profile:cc-work", "Claude · work 实例规则", "~/.claude-work/CLAUDE.md",
            { target_exists: false, base: "missing" }, "missing", true),
          ruleElement("rules:profile:cc-link", "Claude · link 实例规则", "~/.claude-link/CLAUDE.md",
            { blocked_reason: "目标路径是符号链接，拒绝操作以保证安全" }, "unmanaged-detected", false),
          ruleElement("rules:profile:cc-off", "Claude · off 实例规则", "~/.claude-off/CLAUDE.md", {}, "disabled", false),
        ],
        templates: [
          { id: "legacy-claude", name: "旧规则来源（Claude）", body: "旧内容\n", revision: "rev-legacy", origin: "legacy", created_at: "2026-09-01T00:00:00Z", updated_at: "2026-09-01T00:00:00Z" },
          { id: "tpl-team", name: "团队规则", body: CLAUDE_BODY, revision: "rev-team", origin: "user", created_at: "2026-09-10T00:00:00Z", updated_at: "2026-09-10T00:00:00Z" },
          { id: "tpl-backup", name: "备份 · Codex 全局规则 · 2026-09-21 10:00:00", body: "老备份\n", revision: "rev-backup", origin: "backup", created_at: "2026-09-21T02:00:00Z", updated_at: "2026-09-21T02:00:00Z" },
        ],
        source_rules: [SOURCE_RULE],
        suggested_sources: [],
      },
      { command: "rules.status" },
    ),
    "rules:apply": makeEnvelope(
      { domain: "rules", steps: [{ name: "rules:write rules:claude_global", ok: true, exit_code: 0 }], skipped: [] },
      { command: "rules.apply" },
    ),
    "rules:save-template": makeEnvelope({ domain: "rules", template: {} }, { command: "rules.save-template" }),
    "rules:delete-template": makeEnvelope({ domain: "rules", deleted: "tpl-team" }, { command: "rules.delete-template" }),
  };
}

describe("RulesPage：规则来源与补齐", () => {
  it("补齐：只在还没有规则文件的账号行上给「从主号补齐」，从同类主账号复制；已有内容、阻塞与停用的不给", async () => {
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());
    const fill = await screen.findByTestId("rules-fill-rules:profile:cc-work");
    expect(screen.getAllByTestId(/^rules-fill-/)).toHaveLength(1);
    await userEvent.click(fill);
    await waitFor(() => expect(calls.filter((call) => call.command === "rules:apply")).toHaveLength(1));
    expect(calls.find((call) => call.command === "rules:apply")!.args).toEqual([
      "--only", "rules:profile:cc-work", "--base", "missing", "--from", "rules:claude_global",
    ]);
  });

  it("添加规则来源：直接指定一个文件或目录，只登记位置，不改任何规则文件", async () => {
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...rulesRoutes(),
      "policy:get": makeEnvelope({ layers: null, effective: { skills: { repositories: { "agent-skills": {} } } }, attribution: null, layer: null, key: null, value: null, path: null }, { command: "policy.get" }),
      "policy:set": makeEnvelope({}, { command: "policy.set" }),
    };
    const { calls } = renderWithEngine(<RulesPage />, routes);
    await userEvent.click(await screen.findByTestId("rule-source-add"));
    await userEvent.type(screen.getByTestId("rule-source-name"), "agent-skills");
    await userEvent.type(screen.getByTestId("rule-source-path"), "~/notes/CLAUDE.md");
    // 与技能集合同名会撞规则 id：拦下
    expect(screen.getByTestId("rule-source-save")).toBeDisabled();
    await userEvent.clear(screen.getByTestId("rule-source-name"));
    await userEvent.type(screen.getByTestId("rule-source-name"), "my-rules");
    await userEvent.click(screen.getByTestId("rule-source-save"));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const set = calls.find((call) => call.command === "policy:set")!;
    expect(set.args.slice(0, 4)).toEqual(["--layer", "local", "--key", "rules.sources.my-rules"]);
    expect(JSON.parse(set.args[set.args.indexOf("--value") + 1])).toEqual({ path: "~/notes/CLAUDE.md", _delete: false, _replace: true });
    expect(calls.some((call) => call.command === "rules:apply")).toBe(false);
  });

  it("规则库里来自技能集合的规则可以「移除来源」：同一个确认框，输入名称后移除这个集合并清理下载", async () => {
    const { calls } = renderWithEngine(<RulesPage />, {
      ...rulesRoutes(),
      "skills:apply": makeEnvelope({ domain: "skills", steps: [], skipped: [] }, { command: "skills.apply" }),
    });
    await userEvent.click(await screen.findByTestId("rules-remove-collection-agent-skills/core"));
    const dialog = await screen.findByRole("dialog", { name: "移除来源 agent-skills" });
    expect(dialog).toHaveTextContent("规则文件内容保留");
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
    await userEvent.type(within(dialog).getByTestId("confirm-modal-input"), "agent-skills");
    await userEvent.click(within(dialog).getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")!.args).toEqual(["--only", "skill:agent-skills", "--action", "remove", "--purge"]);
  });

  it("我的规则来源在规则库里：没读到规则的也列出来；移除先确认，只删登记，不改任何规则文件", async () => {
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...rulesRoutes(),
      "policy:get": makeEnvelope({ layers: null, effective: { rules: { sources: { "my-rules": { path: "~/notes/rules" } } } }, attribution: null, layer: null, key: null, value: null, path: null }, { command: "policy.get" }),
      "policy:set": makeEnvelope({}, { command: "policy.set" }),
    };
    const { calls } = renderWithEngine(<RulesPage />, routes);
    const row = await screen.findByTestId("rule-source-my-rules");
    expect(row).toHaveTextContent("没读到规则");
    expect(row).toHaveTextContent("规则文件 · ~/notes/rules");
    await userEvent.click(within(row).getByRole("button", { name: "移除规则来源 my-rules" }));
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    await userEvent.click(await screen.findByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const set = calls.find((call) => call.command === "policy:set")!;
    expect(set.args.slice(0, 4)).toEqual(["--layer", "local", "--key", "rules.sources.my-rules"]);
    expect(JSON.parse(set.args[set.args.indexOf("--value") + 1])).toEqual({ _delete: true });
    expect(calls.some((call) => call.command === "rules:apply")).toBe(false);
  });

  it("规则文件可展开查看实际全文，只读展示、不发起任何写入或 plan", async () => {
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());

    const card = await screen.findByTestId("rules-target-rules:claude_global");
    await userEvent.click(within(card).getByRole("button", { name: /^查看 .*全文$/ }));
    expect(screen.getByTestId("rules-content-rules:claude_global").textContent).toBe(CLAUDE_BODY);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "rules:apply")).toBe(false);
  });

  it("rules:status 返回 ok:false 且 data:{} 时显示 DOMAIN_NOT_IMPLEMENTED 且不抛异常", async () => {
    const notImplementedRoutes = {
      "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
      "rules:status": makeEnvelope(
        {} as unknown as { domain: string; elements: [] },
        {
          command: "rules.status",
          ok: false,
          errors: [{ code: "DOMAIN_NOT_IMPLEMENTED", element: null, message: "Rules 域尚未实现" }],
        },
      ),
      "policy:get": makeEnvelope(
        { layers: { sqlite: null } },
        { command: "policy.get" },
      ),
    };

    renderWithEngine(<RulesPage />, notImplementedRoutes);
    expect(await screen.findByRole("heading", { name: "规则" })).toBeInTheDocument();
    expect(screen.getByText("DOMAIN_NOT_IMPLEMENTED")).toBeInTheDocument();
    expect(screen.getByText("Rules 域尚未实现")).toBeInTheDocument();
  });
});

describe("RulesPage：实际文件为准", () => {
  it("逐目标展示路径、实际状态与一致提示；不可写目标没有操作按钮；不再有同步与接管入口", async () => {
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());

    const claude = await screen.findByTestId("rules-target-rules:claude_global");
    expect(within(claude).getByText("~/.claude/CLAUDE.md")).toBeInTheDocument();
    // 使用的规则一栏：规则名 · 来源 · 状态
    expect(claude).toHaveTextContent("团队规则· 本机模板· 一致");
    expect(within(claude).getByRole("button", { name: "编辑 Claude Code 全局规则" })).toBeInTheDocument();

    const codex = screen.getByTestId("rules-target-rules:codex_global");
    expect(codex).toHaveTextContent("core· agent-skills· 一致");

    const missing = screen.getByTestId("rules-target-rules:profile:cc-work");
    expect(within(missing).getByText("还没有规则文件")).toBeInTheDocument();
    // 文件不存在也可以新建，但没有可复制出去的内容
    expect(within(missing).getByRole("button", { name: "编辑 Claude · work 实例规则" })).toBeInTheDocument();
    expect(within(missing).queryByRole("button", { name: /复制/ })).toBeNull();

    const blocked = screen.getByTestId("rules-target-rules:profile:cc-link");
    expect(within(blocked).getByText("不可用")).toBeInTheDocument();
    expect(within(blocked).getByText("目标路径是符号链接，拒绝操作以保证安全")).toBeInTheDocument();
    expect(within(blocked).queryByRole("button")).toBeNull();

    const disabled = screen.getByTestId("rules-target-rules:profile:cc-off");
    expect(within(disabled).getByText("已停用")).toBeInTheDocument();
    expect(within(disabled).queryByRole("button")).toBeNull();

    expect(screen.queryByRole("button", { name: "立即同步" })).toBeNull();
    expect(screen.queryByText("同步此目标")).toBeNull();
    expect(screen.queryByText("接管原内容")).toBeNull();
    // 规则键已退出来源编辑：规则页不再读 sources:get
    expect(calls.some((call) => call.command === "sources:get")).toBe(false);
  });

  it("编辑保存：直接 rules:apply --only --base --value（带 gui- run id），成功后关闭弹窗", async () => {
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());

    await userEvent.click(await screen.findByRole("button", { name: "编辑 Claude Code 全局规则" }));
    const dialog = screen.getByRole("dialog", { name: "编辑 Claude Code 全局规则" });
    const field = within(dialog).getByLabelText("规则全文");
    expect(field).toHaveValue(CLAUDE_BODY);
    fireEvent.change(field, { target: { value: "---\n新的全文\n" } });
    await userEvent.click(within(dialog).getByRole("button", { name: "保存到文件" }));

    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));
    const applyCalls = calls.filter((call) => call.command === "rules:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "rules:claude_global", "--base", CLAUDE_BASE, "--value", "---\n新的全文\n"]);
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("文件已在外部修改：保存被拒时弹窗内说明并刷新状态，不关闭弹窗", async () => {
    const routes = rulesRoutes();
    routes["rules:apply"] = makeEnvelope({}, {
      command: "rules.apply",
      ok: false,
      errors: [{ code: "RULE_ERROR", element: null, message: "Claude Code 全局规则 已在外部修改，没有写入" }],
    });
    const { calls } = renderWithEngine(<RulesPage />, routes);

    await userEvent.click(await screen.findByRole("button", { name: "编辑 Claude Code 全局规则" }));
    const statusCallsBefore = calls.filter((call) => call.command === "rules:status").length;
    await userEvent.click(screen.getByRole("button", { name: "保存到文件" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("已在外部修改");
    expect(alert).toHaveTextContent("页面已刷新为文件的最新内容");
    expect(screen.getByRole("dialog", { name: "编辑 Claude Code 全局规则" })).toBeInTheDocument();
    await waitFor(() =>
      expect(calls.filter((call) => call.command === "rules:status").length).toBeGreaterThan(statusCallsBefore),
    );
  });

  it("应用模板：先展示现有内容到模板正文的差异，确认后发出 --template；现有内容另存为备份时给出提示", async () => {
    const infoSpy = vi.spyOn(toast, "info");
    const routes = rulesRoutes();
    routes["rules:apply"] = makeEnvelope(
      { domain: "rules", steps: [
        { name: "rules:backup rules:codex_global", ok: true, exit_code: 0 },
        { name: "rules:write rules:codex_global", ok: true, exit_code: 0 },
      ], skipped: [] },
      { command: "rules.apply" },
    );
    const { calls } = renderWithEngine(<RulesPage />, routes);

    await userEvent.click(await screen.findByRole("button", { name: "更换 Codex 全局规则 的规则" }));
    const dialog = screen.getByRole("dialog", { name: "更换 Codex 全局规则 的规则" });
    const sourceSelect = within(dialog).getByRole("combobox", { name: "内容来源" });
    // 来源先列来源规则，再列全部已保存模板
    expect(within(sourceSelect).getAllByRole("option").map((option) => option.getAttribute("value"))).toEqual([
      "rule:agent-skills/core",
      "template:legacy-claude",
      "template:tpl-team",
      "template:tpl-backup",
    ]);
    await userEvent.selectOptions(sourceSelect, "template:tpl-team");
    const diff = within(dialog).getByTestId("rules-diff");
    // 每行是「行号 · 标记 · 正文」三格，只比正文
    const linesOf = (kind: string) => Array.from(diff.querySelectorAll(`[data-kind="${kind}"]`)).map((line) => line.lastElementChild?.textContent);
    expect(linesOf("del")).toEqual(["- Use Chinese (Simplified).", "- Prefer the simplest local change."]);
    expect(linesOf("add")).toEqual(["# Claude 规则", "- 回答用简体中文。"]);
    expect(within(dialog).getByText(/会先另存为备份模板/)).toBeInTheDocument();

    await userEvent.click(within(dialog).getByRole("button", { name: "写入" }));
    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));
    const applyCall = calls.find((call) => call.command === "rules:apply")!;
    expect(applyCall.args).toEqual(["--only", "rules:codex_global", "--base", CODEX_BASE, "--template", "tpl-team"]);
    expect(applyCall.options?.runId).toMatch(/^gui-/);
    await waitFor(() => expect(infoSpy).toHaveBeenCalledWith("原内容已另存为备份模板"));
  });

  it("内容与所选来源相同时不提供写入；切换到来源规则后发出 --source-rule", async () => {
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());

    // Codex 现在就是来源规则：选它时没有差异，写入按钮不可用
    await userEvent.click(await screen.findByRole("button", { name: "更换 Codex 全局规则 的规则" }));
    let dialog = screen.getByRole("dialog", { name: "更换 Codex 全局规则 的规则" });
    await userEvent.selectOptions(within(dialog).getByRole("combobox", { name: "内容来源" }), "rule:agent-skills/core");
    expect(within(dialog).getByTestId("rules-diff-same")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "写入" })).toBeDisabled();
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));

    await userEvent.click(screen.getByRole("button", { name: "更换 Claude Code 全局规则 的规则" }));
    dialog = screen.getByRole("dialog", { name: "更换 Claude Code 全局规则 的规则" });
    await userEvent.selectOptions(within(dialog).getByRole("combobox", { name: "内容来源" }), "rule:agent-skills/core");
    expect(within(dialog).getByTestId("rules-diff")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "写入" }));

    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));
    expect(calls.find((call) => call.command === "rules:apply")!.args).toEqual([
      "--only", "rules:claude_global", "--base", CLAUDE_BASE, "--source-rule", "agent-skills/core",
    ]);
  });

  it("来源规则区：可预览正文，「应用到…」选目标后发出 --source-rule", async () => {
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());

    const section = await screen.findByRole("region", { name: "规则库" });
    expect(section).toHaveTextContent("agent-skills · git");
    expect(section).toHaveTextContent("1 个账号在用");
    await userEvent.click(within(section).getByRole("button", { name: "查看 core" }));
    expect(screen.getByTestId("rules-source-body-agent-skills/core").textContent).toBe(SOURCE_BODY);
    await userEvent.click(screen.getByRole("button", { name: "应用规则「core」（来自 agent-skills）到…" }));
    const dialog = screen.getByRole("dialog", { name: "应用规则「core」（来自 agent-skills）" });
    // 只列可写目标：阻塞与停用的实例不在其中
    const targetSelect = within(dialog).getByRole("combobox", { name: "写入目标" });
    // 与账号页同序：Claude 主号、Claude 其他账号、Codex
    expect(within(targetSelect).getAllByRole("option").map((option) => option.getAttribute("value"))).toEqual([
      "rules:claude_global",
      "rules:profile:cc-work",
      "rules:codex_global",
    ]);
    await userEvent.selectOptions(targetSelect, "rules:profile:cc-work");
    await userEvent.click(within(dialog).getByRole("button", { name: "写入" }));

    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));
    expect(calls.find((call) => call.command === "rules:apply")!.args).toEqual([
      "--only", "rules:profile:cc-work", "--base", "missing", "--source-rule", "agent-skills/core",
    ]);
  });

  it("文件与来源规则一致但还没跟随：点「跟随来源更新」只记下跟随，不弹差异", async () => {
    const infoSpy = vi.spyOn(toast, "info");
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());

    const card = await screen.findByTestId("rules-target-rules:codex_global");
    expect(card).toHaveTextContent("core· agent-skills· 一致");
    await userEvent.click(within(card).getByTestId("rules-follow-rules:codex_global"));
    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls.find((call) => call.command === "rules:apply")!.args).toEqual([
      "--only", "rules:codex_global", "--base", CODEX_BASE, "--source-rule", "agent-skills/core",
    ]);
    await waitFor(() => expect(infoSpy).toHaveBeenCalledWith(expect.stringContaining("已跟随规则「core」")));
  });

  it("来源出了新版且文件没改过：标出「来源有新版本」，一键打开差异后更新", async () => {
    const routes = rulesRoutes();
    const status = routes["rules:status"].data as { elements: Array<{ id: string; observed: Record<string, unknown> }> };
    const codex = status.elements.find((element) => element.id === "rules:codex_global")!;
    codex.observed.content = "- 旧版\n";
    codex.observed.matched_source_rule = null;
    codex.observed.source_follow = { rule: "agent-skills/core", applied_revision: "rev-old", current_revision: "rev-core", file_unchanged: true, update_available: true };
    const { calls } = renderWithEngine(<RulesPage />, routes);

    expect(await screen.findByTestId("rules-update-available-rules:codex_global")).toHaveTextContent("有新版本");
    expect(screen.queryByTestId("rules-follow-rules:codex_global")).toBeNull();
    await userEvent.click(screen.getByTestId("rules-update-from-source-rules:codex_global"));
    const dialog = screen.getByRole("dialog", { name: "用规则「core」（来自 agent-skills）更新 Codex 全局规则" });
    expect(within(dialog).getByTestId("rules-diff")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "写入" }));
    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));
    expect(calls.find((call) => call.command === "rules:apply")!.args).toEqual([
      "--only", "rules:codex_global", "--base", CODEX_BASE, "--source-rule", "agent-skills/core",
    ]);
  });

  it("文件在跟随后被改过：只说明情况，不标「可更新」，仍可看差异再决定", async () => {
    const routes = rulesRoutes();
    const status = routes["rules:status"].data as { elements: Array<{ id: string; observed: Record<string, unknown> }> };
    const codex = status.elements.find((element) => element.id === "rules:codex_global")!;
    codex.observed.content = "- 我改过\n";
    codex.observed.matched_source_rule = null;
    codex.observed.source_follow = { rule: "agent-skills/core", applied_revision: "rev-old", current_revision: "rev-core", file_unchanged: false, update_available: false };
    renderWithEngine(<RulesPage />, routes);

    expect(await screen.findByTestId("rules-changed-since-follow-rules:codex_global")).toHaveTextContent("被改过");
    expect(screen.queryByTestId("rules-update-available-rules:codex_global")).toBeNull();
    expect(screen.getByTestId("rules-update-from-source-rules:codex_global")).toHaveTextContent("查看新版本");
  });

  it("git 来源可在规则页直接「更新来源」，发出与技能页相同的更新动作", async () => {
    const { calls } = renderWithEngine(<RulesPage />, {
      ...rulesRoutes(),
      "skills:apply": makeEnvelope({ domain: "skills", steps: [], skipped: [] }, { command: "skills.apply" }),
    });
    await userEvent.click(await screen.findByTestId("rules-fetch-source-agent-skills"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")!.args).toEqual([
      "--only", "skill:agent-skills", "--action", "update",
    ]);
  });

  it("还没有来源规则：已登记的推荐来源给「取回」，没登记的引导去技能页添加", async () => {
    const routes = rulesRoutes();
    const data = routes["rules:status"].data as Record<string, unknown>;
    data.source_rules = [];
    data.suggested_sources = [
      { name: "agent-skills", description: "作者维护的技能与常驻规则", url: "https://github.com/ly87ing/agent-skills.git", collection: null },
    ];
    const onOpenSkills = vi.fn();
    const first = renderWithEngine(<RulesPage onOpenSkills={onOpenSkills} />, routes);
    const empty = await screen.findByTestId("rules-no-source");
    expect(empty).toHaveTextContent("添加规则文件，或添加带规则的仓库");
    await userEvent.click(within(empty).getByRole("button", { name: "技能页" }));
    expect(onOpenSkills).toHaveBeenCalled();
    first.unmount();

    (data.suggested_sources as Array<Record<string, unknown>>)[0].collection = "agent-skills";
    const { calls } = renderWithEngine(<RulesPage />, {
      ...routes,
      "skills:apply": makeEnvelope({ domain: "skills", steps: [], skipped: [] }, { command: "skills.apply" }),
    });
    await userEvent.click(await screen.findByRole("button", { name: "取回 agent-skills" }));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")!.args).toEqual([
      "--only", "skill:agent-skills", "--action", "update",
    ]);
  });

  it("复制到另一目标：目的方用它自己的 base，来源用 --from", async () => {
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());

    await userEvent.click(await screen.findByRole("button", { name: "复制 Claude Code 全局规则 到…" }));
    const dialog = screen.getByRole("dialog", { name: "复制 Claude Code 全局规则 的内容" });
    const targetSelect = within(dialog).getByRole("combobox", { name: "写入目标" });
    expect(within(targetSelect).queryByRole("option", { name: "Claude Code 全局规则" })).toBeNull();
    await userEvent.selectOptions(targetSelect, "rules:codex_global");
    expect(within(dialog).getByTestId("rules-diff")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "写入" }));

    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));
    expect(calls.find((call) => call.command === "rules:apply")!.args).toEqual([
      "--only", "rules:codex_global", "--base", CODEX_BASE, "--from", "rules:claude_global",
    ]);
  });

  it("只差文件末尾换行时照实说明且仍可写入（按字节判断，不按逐行差异）", async () => {
    const routes = rulesRoutes();
    const status = routes["rules:status"].data as { templates: { body: string }[] };
    status.templates[2].body = CLAUDE_BODY.slice(0, -1);
    const { calls } = renderWithEngine(<RulesPage />, routes);

    await userEvent.click(await screen.findByRole("button", { name: "更换 Claude Code 全局规则 的规则" }));
    const dialog = screen.getByRole("dialog", { name: "更换 Claude Code 全局规则 的规则" });
    await userEvent.selectOptions(within(dialog).getByRole("combobox", { name: "内容来源" }), "template:tpl-backup");
    expect(within(dialog).getByTestId("rules-diff-eol")).toBeInTheDocument();
    const write = within(dialog).getByRole("button", { name: "写入" });
    expect(write).toBeEnabled();
    await userEvent.click(write);
    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));
    expect(calls.find((call) => call.command === "rules:apply")!.args).toEqual([
      "--only", "rules:claude_global", "--base", CLAUDE_BASE, "--template", "tpl-backup",
    ]);
  });
});

describe("RulesPage：规则模板只写库", () => {
  it("新建模板发出 rules:save-template --name --value，不触发任何 rules:apply", async () => {
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());

    await userEvent.click(await screen.findByRole("button", { name: "新建模板" }));
    const dialog = screen.getByRole("dialog", { name: "新建模板" });
    await userEvent.type(within(dialog).getByLabelText("模板名称"), "个人规则");
    fireEvent.change(within(dialog).getByLabelText("模板正文"), { target: { value: "- 简洁\n" } });
    await userEvent.click(within(dialog).getByRole("button", { name: "保存模板" }));

    await waitFor(() => expect(calls.some((call) => call.command === "rules:save-template")).toBe(true));
    const saveCall = calls.find((call) => call.command === "rules:save-template")!;
    expect(saveCall.args).toEqual(["--name", "个人规则", "--value", "- 简洁\n"]);
    expect(saveCall.options?.runId).toMatch(/^gui-/);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls.some((call) => call.command === "rules:apply")).toBe(false);
  });

  it("编辑模板带 --template 与 --revision；保存失败在弹窗内显示原因", async () => {
    const routes = rulesRoutes();
    routes["rules:save-template"] = makeEnvelope({}, {
      command: "rules.save-template",
      ok: false,
      errors: [{ code: "RULE_ERROR", element: null, message: "已有同名模板" }],
    });
    const { calls } = renderWithEngine(<RulesPage />, routes);

    await userEvent.click(await screen.findByRole("button", { name: "编辑模板 团队规则" }));
    const dialog = screen.getByRole("dialog", { name: "编辑模板 团队规则" });
    expect(within(dialog).getByLabelText("模板正文")).toHaveValue(CLAUDE_BODY);
    await userEvent.clear(within(dialog).getByLabelText("模板名称"));
    await userEvent.type(within(dialog).getByLabelText("模板名称"), "团队规则 v2");
    await userEvent.click(within(dialog).getByRole("button", { name: "保存模板" }));

    await waitFor(() => expect(calls.some((call) => call.command === "rules:save-template")).toBe(true));
    expect(calls.find((call) => call.command === "rules:save-template")!.args).toEqual([
      "--template", "tpl-team", "--revision", "rev-team", "--name", "团队规则 v2", "--value", CLAUDE_BODY,
    ]);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("已有同名模板");
    expect(screen.getByRole("dialog", { name: "编辑模板 团队规则" })).toBeInTheDocument();
  });

  it("删除模板先确认，确认后发出 rules:delete-template --template", async () => {
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());

    const row = await screen.findByTestId("rule-template-tpl-team");
    expect(row).toHaveTextContent("本机模板 · 自建");
    expect(screen.getByTestId("rule-template-tpl-backup")).toHaveTextContent("备份");
    expect(screen.getByTestId("rule-template-legacy-claude")).toHaveTextContent("旧来源");
    await userEvent.click(within(row).getByRole("button", { name: "删除模板 团队规则" }));
    expect(calls.some((call) => call.command === "rules:delete-template")).toBe(false);
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.some((call) => call.command === "rules:delete-template")).toBe(true));
    expect(calls.find((call) => call.command === "rules:delete-template")!.args).toEqual(["--template", "tpl-team"]);
    expect(calls.some((call) => call.command === "rules:apply")).toBe(false);
  });
});
