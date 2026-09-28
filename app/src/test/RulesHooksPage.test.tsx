import { describe, expect, it, vi } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import { toast } from "sonner";
import RulesHooksPage from "../pages/RulesHooksPage";
import RulesPage from "../pages/RulesPage";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode } from "./fixtures";
import type { Envelope } from "../lib/contract";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

function routes() {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "rules:status": makeEnvelope(
      { domain: "rules", elements: [] },
      { command: "rules.status" },
    ),
    "hooks:status": makeEnvelope(
      { domain: "hooks", elements: [] },
      { command: "hooks.status" },
    ),
    "policy:get": makeEnvelope(
      { layers: { sqlite: null } },
      { command: "policy.get" },
    ),
  };
}

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
    spec: { name, group: id.startsWith("rules:profile:") ? "profile-rules" : "global-rules", kind: "claude", profile_id: null, target },
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

describe("RulesHooksPage 规则与 Hooks 切换容器", () => {
  it("默认渲染规则 Tab，tabIndex 正确，选中状态明确", async () => {
    renderWithEngine(<RulesHooksPage />, routes());

    const rulesTab = screen.getByRole("tab", { name: "规则" });
    const hooksTab = screen.getByRole("tab", { name: "Hooks" });

    expect(rulesTab).toHaveAttribute("aria-selected", "true");
    expect(rulesTab).toHaveAttribute("tabindex", "0");
    expect(hooksTab).toHaveAttribute("aria-selected", "false");
    expect(hooksTab).toHaveAttribute("tabindex", "-1");

    // 渲染规则页内容
    expect(await screen.findByRole("region", { name: "规则文件" })).toBeInTheDocument();
  });

  it("支持鼠标点击切换 Tab 与对应视图内容", async () => {
    renderWithEngine(<RulesHooksPage />, routes());

    const hooksTab = screen.getByRole("tab", { name: "Hooks" });
    await userEvent.click(hooksTab);

    expect(hooksTab).toHaveAttribute("aria-selected", "true");
    expect(hooksTab).toHaveAttribute("tabindex", "0");
    expect(screen.getByRole("tab", { name: "规则" })).toHaveAttribute("aria-selected", "false");
    expect(screen.getByRole("tab", { name: "规则" })).toHaveAttribute("tabindex", "-1");

    // 渲染 Hooks 页内容
    expect(await screen.findByRole("heading", { name: "Hooks" })).toBeInTheDocument();

    // 切换回规则 Tab
    const rulesTab = screen.getByRole("tab", { name: "规则" });
    await userEvent.click(rulesTab);
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("region", { name: "规则文件" })).toBeInTheDocument();
  });

  it("支持标准键盘 ArrowRight / ArrowLeft / End / Home 切换焦点与内容", async () => {
    renderWithEngine(<RulesHooksPage />, routes());

    const rulesTab = screen.getByRole("tab", { name: "规则" });
    const hooksTab = screen.getByRole("tab", { name: "Hooks" });

    // 聚焦初始规则 Tab
    rulesTab.focus();
    expect(rulesTab).toHaveFocus();

    // 按下右方向键 -> 切换至 Hooks Tab
    await userEvent.keyboard("{ArrowRight}");
    expect(hooksTab).toHaveFocus();
    expect(hooksTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("heading", { name: "Hooks" })).toBeInTheDocument();

    // 按下左方向键 -> 切回 规则 Tab
    await userEvent.keyboard("{ArrowLeft}");
    expect(rulesTab).toHaveFocus();
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("region", { name: "规则文件" })).toBeInTheDocument();

    // 按下 End 键 -> 跳至最后一个 Tab (Hooks)
    await userEvent.keyboard("{End}");
    expect(hooksTab).toHaveFocus();
    expect(hooksTab).toHaveAttribute("aria-selected", "true");

    // 按下 Home 键 -> 跳至第一个 Tab (规则)
    await userEvent.keyboard("{Home}");
    expect(rulesTab).toHaveFocus();
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
  });

  it("规则文件可展开查看实际全文，只读展示、不发起任何写入或 plan", async () => {
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());

    const card = await screen.findByTestId("rules-target-rules:claude_global");
    await userEvent.click(within(card).getByRole("button", { name: /查看全文/ }));
    expect(screen.getByTestId("rules-content-rules:claude_global").textContent).toBe(CLAUDE_BODY);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "rules:apply")).toBe(false);
  });

  it("rules:status 与 hooks:status 返回 ok:false 且 data:{} 时，两 Tab 均显示 DOMAIN_NOT_IMPLEMENTED 且不抛异常", async () => {
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
      "hooks:status": makeEnvelope(
        {} as unknown as { domain: string; elements: [] },
        {
          command: "hooks.status",
          ok: false,
          errors: [{ code: "DOMAIN_NOT_IMPLEMENTED", element: null, message: "Hooks 域尚未实现" }],
        },
      ),
      "policy:get": makeEnvelope(
        { layers: { sqlite: null } },
        { command: "policy.get" },
      ),
    };

    renderWithEngine(<RulesHooksPage />, notImplementedRoutes);

    // 默认 Rules Tab: 展示 DOMAIN_NOT_IMPLEMENTED 与错误消息，不抛异常
    expect(await screen.findByRole("heading", { name: "规则" })).toBeInTheDocument();
    expect(screen.getByText("DOMAIN_NOT_IMPLEMENTED")).toBeInTheDocument();
    expect(screen.getByText("Rules 域尚未实现")).toBeInTheDocument();

    // 切换到 Hooks Tab: 不抛异常，展示 DOMAIN_NOT_IMPLEMENTED 与对应错误消息
    const hooksTab = screen.getByRole("tab", { name: "Hooks" });
    await userEvent.click(hooksTab);
    expect(hooksTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("heading", { name: "Hooks" })).toBeInTheDocument();
    expect(screen.getByText("DOMAIN_NOT_IMPLEMENTED")).toBeInTheDocument();
    expect(screen.getByText("Hooks 域尚未实现")).toBeInTheDocument();

    // 切回 Rules Tab: 依然正常
    const rulesTab = screen.getByRole("tab", { name: "规则" });
    await userEvent.click(rulesTab);
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("heading", { name: "规则" })).toBeInTheDocument();
    expect(screen.getByText("Rules 域尚未实现")).toBeInTheDocument();
  });
});

describe("RulesPage：实际文件为准", () => {
  it("逐目标展示路径、实际状态与一致提示；不可写目标没有操作按钮；不再有同步与接管入口", async () => {
    const { calls } = renderWithEngine(<RulesPage />, rulesRoutes());

    const claude = await screen.findByTestId("rules-target-rules:claude_global");
    expect(within(claude).getByText("~/.claude/CLAUDE.md")).toBeInTheDocument();
    expect(within(claude).getByText("已有文件")).toBeInTheDocument();
    expect(within(claude).getByText("与模板「团队规则」一致")).toBeInTheDocument();
    expect(within(claude).getByRole("button", { name: "编辑 Claude Code 全局规则" })).toBeInTheDocument();

    const codex = screen.getByTestId("rules-target-rules:codex_global");
    expect(within(codex).getByText("与来源规则「core」（agent-skills）一致")).toBeInTheDocument();

    const missing = screen.getByTestId("rules-target-rules:profile:cc-work");
    expect(within(missing).getByText("还没有文件")).toBeInTheDocument();
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

    await userEvent.click(await screen.findByRole("button", { name: "应用模板到 Codex 全局规则" }));
    const dialog = screen.getByRole("dialog", { name: "应用模板到 Codex 全局规则" });
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
    const linesOf = (kind: string) => Array.from(diff.querySelectorAll(`[data-kind="${kind}"]`)).map((line) => line.textContent);
    expect(linesOf("del")).toEqual(["- - Use Chinese (Simplified).", "- - Prefer the simplest local change."]);
    expect(linesOf("add")).toEqual(["+ # Claude 规则", "+ - 回答用简体中文。"]);
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
    await userEvent.click(await screen.findByRole("button", { name: "应用模板到 Codex 全局规则" }));
    let dialog = screen.getByRole("dialog", { name: "应用模板到 Codex 全局规则" });
    await userEvent.selectOptions(within(dialog).getByRole("combobox", { name: "内容来源" }), "rule:agent-skills/core");
    expect(within(dialog).getByTestId("rules-diff-same")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "写入" })).toBeDisabled();
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));

    await userEvent.click(screen.getByRole("button", { name: "应用模板到 Claude Code 全局规则" }));
    dialog = screen.getByRole("dialog", { name: "应用模板到 Claude Code 全局规则" });
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

    const section = await screen.findByRole("region", { name: "来源规则" });
    expect(section).toHaveTextContent("agent-skills · git 镜像");
    await userEvent.click(within(section).getByRole("button", { name: "预览" }));
    expect(screen.getByTestId("rules-source-body-agent-skills/core").textContent).toBe(SOURCE_BODY);
    await userEvent.click(screen.getByRole("button", { name: "应用来源规则「core」（agent-skills）到…" }));
    const dialog = screen.getByRole("dialog", { name: "应用来源规则「core」（agent-skills）" });
    // 只列可写目标：阻塞与停用的实例不在其中
    const targetSelect = within(dialog).getByRole("combobox", { name: "写入目标" });
    expect(within(targetSelect).getAllByRole("option").map((option) => option.getAttribute("value"))).toEqual([
      "rules:codex_global",
      "rules:claude_global",
      "rules:profile:cc-work",
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
    expect(card).toHaveTextContent("与来源规则「core」（agent-skills）一致");
    await userEvent.click(within(card).getByTestId("rules-follow-rules:codex_global"));
    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls.find((call) => call.command === "rules:apply")!.args).toEqual([
      "--only", "rules:codex_global", "--base", CODEX_BASE, "--source-rule", "agent-skills/core",
    ]);
    await waitFor(() => expect(infoSpy).toHaveBeenCalledWith(expect.stringContaining("已跟随来源规则「core」")));
  });

  it("来源出了新版且文件没改过：标出「来源有新版本」，一键打开差异后更新", async () => {
    const routes = rulesRoutes();
    const status = routes["rules:status"].data as { elements: Array<{ id: string; observed: Record<string, unknown> }> };
    const codex = status.elements.find((element) => element.id === "rules:codex_global")!;
    codex.observed.content = "- 旧版\n";
    codex.observed.matched_source_rule = null;
    codex.observed.source_follow = { rule: "agent-skills/core", applied_revision: "rev-old", current_revision: "rev-core", file_unchanged: true, update_available: true };
    const { calls } = renderWithEngine(<RulesPage />, routes);

    expect(await screen.findByTestId("rules-update-available-rules:codex_global")).toHaveTextContent("来源有新版本");
    expect(screen.queryByTestId("rules-follow-rules:codex_global")).toBeNull();
    await userEvent.click(screen.getByTestId("rules-update-from-source-rules:codex_global"));
    const dialog = screen.getByRole("dialog", { name: "用来源规则「core」（agent-skills）更新 Codex 全局规则" });
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
    expect(empty).toHaveTextContent("规则内容不随程序发布");
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

    await userEvent.click(await screen.findByRole("button", { name: "应用模板到 Claude Code 全局规则" }));
    const dialog = screen.getByRole("dialog", { name: "应用模板到 Claude Code 全局规则" });
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
    expect(within(row).getByText("自建")).toBeInTheDocument();
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

const HOOK_WRITE = [{ id: "write", risk: "write-local" as const, needs_lock: true, offline_ok: true }];
const CLAUDE_HOOKS_BASE = "d".repeat(64);
const OLD_BLOCK = "sh /repo/agent-rule/hooks/block-dist-edits.sh";

function hookElement(
  id: string,
  name: string,
  target: string,
  observed: Record<string, unknown>,
  state: "managed-ok" | "missing" | "unmanaged-detected" | "disabled",
  writable: boolean,
) {
  return makeElement({
    id,
    domain: "hooks",
    spec: { name, group: id.startsWith("hooks:profile:") ? "profile-hooks" : "global-hooks", kind: "claude", scope: target.endsWith("settings.json") ? "hooks-key" : "whole-file", profile_id: null, target },
    policy: { enabled: state !== "disabled", agent_id: "claude-code" },
    observed: { target: `/tmp/home/${target.slice(2)}`, target_exists: null, hooks: null, hook_count: 0, base: null, cleanup: [], legacy: [], blocked_reason: null, ...observed },
    state,
    freshness: makeFreshness(),
    actions: writable ? HOOK_WRITE : [],
  });
}

/** Hooks 新契约的一组典型目标：有实际 hooks 且带两类建议、缺文件、阻塞的实例。 */
function nativeHooksRoutes(): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "rules:status": makeEnvelope({ domain: "rules", elements: [] }, { command: "rules.status" }),
    "hooks:status": makeEnvelope(
      {
        domain: "hooks",
        elements: [
          hookElement("hooks:claude", "Claude Code Hooks", "~/.claude/settings.json", {
            target_exists: true,
            hooks: {
              PreToolUse: [{ matcher: "Edit|MultiEdit|Write", hooks: [{ type: "command", command: OLD_BLOCK }, { type: "command", command: "./guard.sh", timeout: 30 }] }],
              Stop: [{ hooks: [{ type: "http", url: "https://hooks.example.com/stop" }] }],
            },
            hook_count: 3,
            base: CLAUDE_HOOKS_BASE,
            cleanup: [{ id: "cleanup-0123456789abcdef", event: "PreToolUse", matcher: "Edit|MultiEdit|Write", command: OLD_BLOCK }],
            legacy: [{ id: "legacy-0123456789abcdef", source_key: "hooks.claude", event: "Stop", matcher: null, command: "echo saved-old" }],
          }, "managed-ok", true),
          hookElement("hooks:codex", "Codex Hooks", "~/.codex/hooks.json", { target_exists: false, hooks: {}, base: "missing" }, "missing", true),
          hookElement("hooks:profile:cc-link", "Claude link 实例 Hooks", "~/.claude-link/settings.json", { blocked_reason: "目标路径是符号链接，拒绝操作以保证安全" }, "unmanaged-detected", false),
        ],
      },
      { command: "hooks.status" },
    ),
    "hooks:apply": makeEnvelope(
      { domain: "hooks", steps: [{ name: "hooks:write hooks:claude", ok: true, exit_code: 0 }], skipped: [] },
      { command: "hooks.apply" },
    ),
    "hooks:preview": makeEnvelope(
      { path: "/tmp/home/.claude/settings.json", content: '{"theme":"dark","hooks":{"Stop":[]}}', exists: true, will_write: true },
      { command: "hooks.preview" },
    ),
    "hooks:discard-legacy": makeEnvelope({ domain: "hooks", discarded: "legacy-0123456789abcdef", source_removed: true }, { command: "hooks.discard-legacy" }),
    "policy:get": makeEnvelope({ layers: { sqlite: null } }, { command: "policy.get" }),
  };
}

async function openHooksTab(routesMap: Record<string, Envelope>) {
  const view = renderWithEngine(<RulesHooksPage />, routesMap);
  await userEvent.click(screen.getByRole("tab", { name: "Hooks" }));
  await screen.findByTestId("hooks-target-hooks:claude");
  return view;
}

describe("Hooks 页：实际配置为准", () => {
  it("编辑输入后显示完整目标文件，改动后只使用当前输入的预览", async () => {
    const { calls } = await openHooksTab(nativeHooksRoutes());
    await userEvent.click(screen.getByRole("button", { name: "编辑 Claude Code Hooks" }));
    const dialog = await screen.findByRole("dialog", { name: "编辑 Claude Code Hooks" });
    expect(await within(dialog).findByTestId("hooks-preview-content")).toHaveTextContent('"theme":"dark"');
    const first = calls.find((call) => call.command === "hooks:preview")!;
    expect(first.args.slice(0, 5)).toEqual(["--only", "hooks:claude", "--base", CLAUDE_HOOKS_BASE, "--value"]);
    await userEvent.clear(within(dialog).getByTestId("hooks-command-1"));
    expect(within(dialog).queryByTestId("hooks-preview-content")).toBeNull();
    await userEvent.type(within(dialog).getByTestId("hooks-command-1"), "echo new");
    await waitFor(() => expect(calls.filter((call) => call.command === "hooks:preview").length).toBeGreaterThan(1));
    const previews = calls.filter((call) => call.command === "hooks:preview");
    const latest = previews[previews.length - 1];
    expect(JSON.parse(latest.args[5]).PreToolUse[0].hooks[1].command).toBe("echo new");
    expect(calls.some((call) => call.command === "hooks:apply")).toBe(false);
  });

  it("逐目标展示实际 hooks 与两类建议；没有同步、接管与来源编辑入口", async () => {
    const { calls } = await openHooksTab(nativeHooksRoutes());
    const claude = screen.getByTestId("hooks-target-hooks:claude");
    expect(within(claude).getByText("./guard.sh")).toBeInTheDocument();
    expect(within(claude).getByText("3 条命令")).toBeInTheDocument();
    // command 以外的类型如实展示类型与内容
    expect(within(claude).getByText("http")).toBeInTheDocument();
    expect(within(claude).getByText('{"url":"https://hooks.example.com/stop"}')).toBeInTheDocument();
    expect(within(screen.getByTestId("hooks-cleanup-hooks:claude")).getByText(OLD_BLOCK)).toBeInTheDocument();
    expect(within(screen.getByTestId("hooks-legacy-hooks:claude")).getByText("echo saved-old")).toBeInTheDocument();
    expect(screen.getByText("旧版已保存的 hooks（未生效）")).toBeInTheDocument();

    const codex = screen.getByTestId("hooks-target-hooks:codex");
    expect(within(codex).getByText("还没有文件")).toBeInTheDocument();
    expect(within(codex).getByRole("button", { name: "编辑 Codex Hooks" })).toBeInTheDocument();

    const blocked = screen.getByTestId("hooks-target-hooks:profile:cc-link");
    expect(within(blocked).getByText(/符号链接/)).toBeInTheDocument();
    expect(within(blocked).queryByRole("button")).toBeNull();

    expect(screen.queryByTestId("apply-hooks-button")).toBeNull();
    expect(screen.queryByText("接管原内容")).toBeNull();
    expect(screen.queryByText("立即同步")).toBeNull();
    expect(calls.some((call) => call.command === "sources:get")).toBe(false);
    expect(calls.some((call) => call.command === "hooks:apply")).toBe(false);
  });

  it("编辑：保存直接写回实际文件，带 --only --base --value，不走来源保存", async () => {
    const { calls } = await openHooksTab(nativeHooksRoutes());
    await userEvent.click(screen.getByRole("button", { name: "编辑 Claude Code Hooks" }));
    const dialog = await screen.findByRole("dialog", { name: "编辑 Claude Code Hooks" });
    expect(within(dialog).getByTestId("hooks-command-1")).toHaveValue("./guard.sh");
    await userEvent.click(within(dialog).getByTestId("hooks-remove-0"));
    await userEvent.click(within(dialog).getByTestId("hooks-save"));

    await waitFor(() => expect(calls.some((call) => call.command === "hooks:apply")).toBe(true));
    const write = calls.find((call) => call.command === "hooks:apply")!;
    expect(write.args.slice(0, 5)).toEqual(["--only", "hooks:claude", "--base", CLAUDE_HOOKS_BASE, "--value"]);
    expect(JSON.parse(write.args[5])).toEqual({
      PreToolUse: [{ matcher: "Edit|MultiEdit|Write", hooks: [{ type: "command", command: "./guard.sh", timeout: 30 }] }],
      Stop: [{ hooks: [{ type: "http", url: "https://hooks.example.com/stop" }] }],
    });
    expect(write.options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command === "sources:set")).toBe(false);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("保存时文件已在外部修改：弹窗内说明并提示刷新，不关闭", async () => {
    const routesMap = nativeHooksRoutes();
    routesMap["hooks:apply"] = makeEnvelope({}, {
      command: "hooks.apply",
      ok: false,
      errors: [{ code: "HOOK_ERROR", element: null, message: "Claude Code Hooks 已在外部修改（与打开时读到的内容不同），没有写入；请刷新后重新操作" }],
    });
    await openHooksTab(routesMap);
    await userEvent.click(screen.getByRole("button", { name: "编辑 Claude Code Hooks" }));
    await userEvent.click(await screen.findByTestId("hooks-save"));
    const alert = await screen.findByTestId("hooks-save-error");
    expect(alert).toHaveTextContent("已在外部修改");
    expect(alert).toHaveTextContent("页面已刷新");
    expect(screen.getByRole("dialog", { name: "编辑 Claude Code Hooks" })).toBeInTheDocument();
  });

  it("清理建议：确认后才 hooks:apply --cleanup，只针对这一条", async () => {
    const { calls } = await openHooksTab(nativeHooksRoutes());
    await userEvent.click(screen.getByRole("button", { name: `清理 ${OLD_BLOCK}` }));
    expect(calls.some((call) => call.command === "hooks:apply")).toBe(false);
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "hooks:apply")).toBe(true));
    expect(calls.find((call) => call.command === "hooks:apply")!.args).toEqual([
      "--only", "hooks:claude", "--base", CLAUDE_HOOKS_BASE, "--cleanup", "cleanup-0123456789abcdef",
    ]);
  });

  it("旧版未生效建议：加入走 hooks:apply --legacy；丢弃只调 hooks:discard-legacy", async () => {
    const { calls } = await openHooksTab(nativeHooksRoutes());
    await userEvent.click(screen.getByRole("button", { name: "加入实际配置 echo saved-old" }));
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "hooks:apply")).toBe(true));
    expect(calls.find((call) => call.command === "hooks:apply")!.args).toEqual([
      "--only", "hooks:claude", "--base", CLAUDE_HOOKS_BASE, "--legacy", "legacy-0123456789abcdef",
    ]);

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await userEvent.click(screen.getByRole("button", { name: "丢弃 echo saved-old" }));
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "hooks:discard-legacy")).toBe(true));
    expect(calls.find((call) => call.command === "hooks:discard-legacy")!.args).toEqual(["--legacy", "legacy-0123456789abcdef"]);
    expect(calls.filter((call) => call.command === "hooks:apply")).toHaveLength(1);
  });
});
