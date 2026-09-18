import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { toast } from "sonner";
import RulesHooksPage from "../pages/RulesHooksPage";
import RulesPage from "../pages/RulesPage";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode } from "./fixtures";
import type { Envelope } from "../lib/contract";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

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
    expect(await screen.findByRole("button", { name: "立即同步" })).toBeInTheDocument();
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
    expect(await screen.findByTestId("apply-hooks-button")).toBeInTheDocument();

    // 切换回规则 Tab
    const rulesTab = screen.getByRole("tab", { name: "规则" });
    await userEvent.click(rulesTab);
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("button", { name: "立即同步" })).toBeInTheDocument();
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
    expect(await screen.findByTestId("apply-hooks-button")).toBeInTheDocument();

    // 按下左方向键 -> 切回 规则 Tab
    await userEvent.keyboard("{ArrowLeft}");
    expect(rulesTab).toHaveFocus();
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("button", { name: "立即同步" })).toBeInTheDocument();

    // 按下 End 键 -> 跳至最后一个 Tab (Hooks)
    await userEvent.keyboard("{End}");
    expect(hooksTab).toHaveFocus();
    expect(hooksTab).toHaveAttribute("aria-selected", "true");

    // 按下 Home 键 -> 跳至第一个 Tab (规则)
    await userEvent.keyboard("{Home}");
    expect(rulesTab).toHaveFocus();
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
  });

  it("规则详情展开为纯状态展示（drift 面板），不请求任何 plan", async () => {
    const detailRoutes: Record<string, Envelope> = routes();
    detailRoutes["rules:status"] = makeEnvelope({
      domain: "rules",
      elements: [
        makeElement({
          id: "rules:claude_global",
          domain: "rules",
          spec: { name: "Claude Code 全局规则", source_keys: ["rules.shared", "rules.claude"], target: "~/.claude/CLAUDE.md" },
          observed: { target: "/tmp/.claude/CLAUDE.md", target_exists: true },
          state: "drifted",
          freshness: makeFreshness(),
          actions: [{ id: "sync", risk: "write-local", needs_lock: true, offline_ok: true }],
        }),
      ],
    }, { command: "rules.status" });
    detailRoutes["sources:get"] = makeEnvelope({ sources: [] }, { command: "sources.get" });
    const { calls } = renderWithEngine(<RulesPage />, detailRoutes);

    await userEvent.click(await screen.findByRole("button", { name: "Claude Code 全局规则" }));
    const panel = await screen.findByTestId("drift-panel");
    expect(panel).toBeInTheDocument();
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
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

function nativeRulesRoutes(): Record<string, Envelope> {
  return {
    "rules:status": makeEnvelope(
      {
        domain: "rules",
        elements: [
          makeElement({
            id: "rules:codex_global",
            domain: "rules",
            spec: { name: "Codex 全局规则", group: "global-rules", source_keys: ["rules.shared", "rules.codex"], target: "~/.codex/AGENTS.md" },
            policy: { enabled: true, agent_id: "codex" },
            observed: { target: "/tmp/home/.codex/AGENTS.md", target_exists: true, expected_digest: "aaaa", applied_digest: "aaaa", shared_revision: "r1", adapter_revision: "r2", blocked_reason: null, registration_needed: false },
            state: "managed-ok",
            freshness: makeFreshness(),
            actions: [],
          }),
          makeElement({
            id: "rules:claude_global",
            domain: "rules",
            spec: { name: "Claude Code 全局规则", group: "global-rules", source_keys: ["rules.shared", "rules.claude"], target: "~/.claude/CLAUDE.md" },
            policy: { enabled: true, agent_id: "claude-code" },
            observed: { target: "/tmp/home/.claude/CLAUDE.md", target_exists: true, expected_digest: "bbbb", applied_digest: "aaaa", shared_revision: "r1", adapter_revision: "r3", blocked_reason: null, registration_needed: false },
            state: "drifted",
            freshness: makeFreshness(),
            actions: [{ id: "sync", risk: "write-local", needs_lock: true, offline_ok: true }],
          }),
        ],
      },
      { command: "rules.status" },
    ),
    "rules:apply": makeEnvelope(
      { domain: "rules", steps: [{ name: "rules:sync rules:claude_global", ok: true, exit_code: 0 }], skipped: [] },
      { command: "rules.apply" },
    ),
    "sources:get": makeEnvelope({ sources: [] }, { command: "sources.get" }),
  };
}

describe("RulesPage 原生规则目标", () => {
  it("显示两个真实目标与来源定义，完全不出现 dist 卡与生成/备份旧文案", async () => {
    renderWithEngine(<RulesPage />, nativeRulesRoutes());

    expect(await screen.findByRole("button", { name: "Codex 全局规则" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Claude Code 全局规则" })).toBeInTheDocument();
    expect(screen.getByText("rules.shared + rules.codex")).toBeInTheDocument();
    expect(screen.getByText("rules.shared + rules.claude")).toBeInTheDocument();
    expect(screen.getByText("/tmp/home/.codex/AGENTS.md")).toBeInTheDocument();
    expect(screen.getByText("/tmp/home/.claude/CLAUDE.md")).toBeInTheDocument();

    expect(screen.queryByTestId("rules-dist-card")).not.toBeInTheDocument();
    const bodyText = document.body.textContent ?? "";
    expect(bodyText).not.toContain("rules:dist");
    expect(bodyText).not.toContain("生成");
    expect(bodyText).not.toContain("备份");
  });

  it("仅带 sync 动作的行显示「同步此目标」，点击直接 rules:apply 精确 --only（带 run_id），不取 plan、零 policy:set", async () => {
    const { calls } = renderWithEngine(<RulesPage />, nativeRulesRoutes());

    const syncButton = await screen.findByTestId("action-sync-rules:claude_global");
    expect(screen.queryByTestId("action-sync-rules:codex_global")).not.toBeInTheDocument();

    await userEvent.click(syncButton);
    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));

    const applyCalls = calls.filter((call) => call.command === "rules:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "rules:claude_global"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("全局「立即同步」：点击直接 rules:apply 且不带 --only", async () => {
    const { calls } = renderWithEngine(<RulesPage />, nativeRulesRoutes());

    await userEvent.click(await screen.findByRole("button", { name: "立即同步" }));
    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));

    const applyCalls = calls.filter((call) => call.command === "rules:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual([]);
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

function nativeHooksRoutes(): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "rules:status": makeEnvelope({ domain: "rules", elements: [] }, { command: "rules.status" }),
    "hooks:status": makeEnvelope(
      {
        domain: "hooks",
        elements: [
          makeElement({
            id: "hooks:claude",
            domain: "hooks",
            spec: { name: "Claude Hooks", merge: "signature-set", source_key: "hooks.claude", target: "~/.claude/settings.json" },
            policy: { enabled: true, agent_id: "claude-code" },
            observed: { target: "/tmp/home/.claude/settings.json", target_exists: true, expected_digest: "bbbb", applied_digest: "aaaa", actual_digest: "aaaa", source_revision: "r1", blocked_reason: null, registration_needed: false, managed_hook_count: 1, missing_signatures: 1, removed_signatures_present: 0 },
            state: "drifted",
            freshness: makeFreshness(),
            actions: [{ id: "sync", risk: "write-local", needs_lock: true, offline_ok: true }],
          }),
          makeElement({
            id: "hooks:codex",
            domain: "hooks",
            spec: { name: "Codex Hooks", merge: "whole-file", source_key: "hooks.codex", target: "~/.codex/hooks.json" },
            policy: { enabled: true, agent_id: "codex" },
            observed: { target: "/tmp/home/.codex/hooks.json", target_exists: true, expected_digest: "cccc", applied_digest: "cccc", actual_digest: "cccc", source_revision: "r2", blocked_reason: null, registration_needed: false, managed_hook_count: 0, missing_signatures: 0, removed_signatures_present: 0 },
            state: "managed-ok",
            freshness: makeFreshness(),
            actions: [],
          }),
        ],
      },
      { command: "hooks.status" },
    ),
    "hooks:apply": makeEnvelope(
      { domain: "hooks", steps: [{ name: "hooks:sync hooks:claude", ok: true, exit_code: 0 }], skipped: [] },
      { command: "hooks.apply" },
    ),
    "sources:get": makeEnvelope(
      { sources: [
        { key: "hooks.claude", label: "Claude Hooks", value: { hooks: {}, removed_signatures: [] }, revision: "rev-c", format: "json" },
        { key: "hooks.codex", label: "Codex Hooks", value: { hooks: {} }, revision: "rev-x", format: "json" },
      ] },
      { command: "sources.get" },
    ),
    "policy:get": makeEnvelope({ layers: { sqlite: null } }, { command: "policy.get" }),
  };
}

describe("Hooks 页原生目标", () => {
  it("列出两个真实目标，页面与弹窗无备份/生成文案", async () => {
    renderWithEngine(<RulesHooksPage />, nativeHooksRoutes());
    await userEvent.click(screen.getByRole("tab", { name: "Hooks" }));

    expect(await screen.findByText("hooks:claude")).toBeInTheDocument();
    expect(screen.getByText("hooks:codex")).toBeInTheDocument();
    // 默认是按项填写的结构化编辑；原文编辑收在按需展开的入口里
    expect(await screen.findByTestId("hooks-editor-hooks.claude")).toBeInTheDocument();
    expect(screen.getByTestId("hooks-editor-hooks.codex")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "编辑 Claude Hooks" })).toBeNull();
    await userEvent.click(screen.getByTestId("toggle-hooks-raw-panel"));
    expect(await screen.findByRole("button", { name: "编辑 Claude Hooks" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "编辑 Codex Hooks" })).toBeInTheDocument();
    const bodyText = document.body.textContent ?? "";
    expect(bodyText).not.toContain("备份");
    expect(bodyText).not.toContain("生成");
  });

  it("全局「立即同步」：点击直接 hooks:apply 且不带 --only，页面无备份/生成承诺", async () => {
    const { calls } = renderWithEngine(<RulesHooksPage />, nativeHooksRoutes());
    await userEvent.click(screen.getByRole("tab", { name: "Hooks" }));

    await userEvent.click(await screen.findByTestId("apply-hooks-button"));
    await waitFor(() => expect(calls.some((call) => call.command === "hooks:apply")).toBe(true));

    const applyCalls = calls.filter((call) => call.command === "hooks:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual([]);
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("Hooks 阻塞目标的显式接管入口", () => {
  function blockedHooksRoutes(): Record<string, Envelope> {
    const routes = nativeHooksRoutes();
    routes["hooks:status"] = makeEnvelope(
      {
        domain: "hooks",
        elements: [
          makeElement({
            id: "hooks:claude",
            domain: "hooks",
            spec: { name: "Claude Hooks", merge: "signature-set", source_key: "hooks.claude", target: "~/.claude/settings.json" },
            policy: { enabled: true, agent_id: "claude-code" },
            observed: { target: "/tmp/home/.claude/settings.json", target_exists: true, expected_digest: "bbbb", applied_digest: "bbbb", actual_digest: "bbbb", source_revision: "r1", blocked_reason: null, registration_needed: false, managed_hook_count: 1, missing_signatures: 0, removed_signatures_present: 0 },
            state: "managed-ok",
            freshness: makeFreshness(),
            actions: [],
          }),
          makeElement({
            id: "hooks:codex",
            domain: "hooks",
            spec: { name: "Codex Hooks", merge: "whole-file", source_key: "hooks.codex", target: "~/.codex/hooks.json" },
            policy: { enabled: true, agent_id: "codex" },
            observed: { target: "/tmp/home/.codex/hooks.json", target_exists: true, expected_digest: "eeee", applied_digest: null, actual_digest: "ffff", source_revision: "r2", blocked_reason: "这个文件里已经有本工具以外写的内容，不会覆盖它", registration_needed: false, managed_hook_count: 1, missing_signatures: 0, removed_signatures_present: 0 },
            state: "unmanaged-detected",
            freshness: makeFreshness(),
            actions: [],
          }),
        ],
      },
      { command: "hooks.status" },
    );
    routes["hooks:apply"] = makeEnvelope(
      { domain: "hooks", steps: [{ name: "hooks:sync hooks:codex", ok: true, exit_code: 0 }], skipped: [] },
      { command: "hooks.apply" },
    );
    return routes;
  }

  it("阻塞的 codex 目标显示「接管原内容」与整文件替换提示；正常目标不显示", async () => {
    renderWithEngine(<RulesHooksPage />, blockedHooksRoutes());
    await userEvent.click(screen.getByRole("tab", { name: "Hooks" }));

    expect(await screen.findByTestId("takeover-hooks:codex")).toBeInTheDocument();
    expect(screen.getByTestId("takeover-hint-hooks:codex")).toHaveTextContent(/整文件替换/);
    expect(screen.queryByTestId("takeover-hooks:claude")).not.toBeInTheDocument();
  });

  it("接管原内容：一键直接 apply 带 --only 与 --allow-takeover，不取 plan、不开预览，成功走既有执行 toast", async () => {
    const toastSuccessSpy = vi.spyOn(toast, "success");
    const { calls } = renderWithEngine(<RulesHooksPage />, blockedHooksRoutes());
    await userEvent.click(screen.getByRole("tab", { name: "Hooks" }));

    // 整文件替换警示仍在按钮旁展示
    expect(await screen.findByTestId("takeover-hint-hooks:codex")).toHaveTextContent(/整文件替换/);
    await userEvent.click(screen.getByTestId("takeover-hooks:codex"));

    await waitFor(() => expect(toastSuccessSpy).toHaveBeenCalled());
    const applyCalls = calls.filter((call) => call.command === "hooks:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "hooks:codex", "--allow-takeover"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command === "hooks:plan")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(toastSuccessSpy.mock.calls[toastSuccessSpy.mock.calls.length - 1][0]).toContain("接管原内容 hooks:codex");
  });

  it("blocked/unsafe 且 actual_digest:null 的目标不显示接管按钮", async () => {
    const unsafeRoutes = blockedHooksRoutes();
    unsafeRoutes["hooks:status"] = makeEnvelope(
      {
        domain: "hooks",
        elements: [
          makeElement({
            id: "hooks:codex",
            domain: "hooks",
            spec: { name: "Codex Hooks", merge: "whole-file", source_key: "hooks.codex", target: "~/.codex/hooks.json" },
            policy: { enabled: true, agent_id: "codex" },
            observed: { target: "/tmp/home/.codex/hooks.json", target_exists: true, expected_digest: "eeee", applied_digest: null, actual_digest: null, source_revision: "r2", blocked_reason: "这个文件里已经有本工具以外写的内容，不会覆盖它", registration_needed: false, managed_hook_count: 1, missing_signatures: 0, removed_signatures_present: 0 },
            state: "unmanaged-detected",
            freshness: makeFreshness(),
            actions: [],
          }),
        ],
      },
      { command: "hooks.status" },
    );

    const { calls } = renderWithEngine(<RulesHooksPage />, unsafeRoutes);
    await userEvent.click(screen.getByRole("tab", { name: "Hooks" }));

    expect(await screen.findByText("hooks:codex")).toBeInTheDocument();
    expect(screen.queryByTestId("takeover-hooks:codex")).not.toBeInTheDocument();
    expect(screen.queryByTestId("takeover-hint-hooks:codex")).not.toBeInTheDocument();
    expect(calls.some((call) => call.command === "hooks:apply")).toBe(false);
  });

  it("安全 unmanaged 且 actual_digest 为 string 的目标仍显示接管按钮，点击直接 hooks:apply --allow-takeover", async () => {
    const safeRoutes = blockedHooksRoutes();
    safeRoutes["hooks:status"] = makeEnvelope(
      {
        domain: "hooks",
        elements: [
          makeElement({
            id: "hooks:codex",
            domain: "hooks",
            spec: { name: "Codex Hooks", merge: "whole-file", source_key: "hooks.codex", target: "~/.codex/hooks.json" },
            policy: { enabled: true, agent_id: "codex" },
            observed: { target: "/tmp/home/.codex/hooks.json", target_exists: true, expected_digest: "eeee", applied_digest: null, actual_digest: "ffff", source_revision: "r2", blocked_reason: null, registration_needed: false, managed_hook_count: 1, missing_signatures: 0, removed_signatures_present: 0 },
            state: "unmanaged-detected",
            freshness: makeFreshness(),
            actions: [],
          }),
        ],
      },
      { command: "hooks.status" },
    );

    const { calls } = renderWithEngine(<RulesHooksPage />, safeRoutes);
    await userEvent.click(screen.getByRole("tab", { name: "Hooks" }));

    await userEvent.click(await screen.findByTestId("takeover-hooks:codex"));
    await waitFor(() => expect(calls.some((call) => call.command === "hooks:apply")).toBe(true));

    const applyCalls = calls.filter((call) => call.command === "hooks:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "hooks:codex", "--allow-takeover"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });
});

describe("规则目标被外部改过时的出路", () => {
  it("给出「接管原内容」，点了带 --allow-takeover 精确到该目标执行", async () => {
    const rulesRoutes: Record<string, Envelope> = { ...routes() };
    rulesRoutes["rules:status"] = makeEnvelope(
      {
        domain: "rules",
        elements: [
          makeElement({
            id: "rules:claude_global",
            domain: "rules",
            spec: { name: "Claude Code 全局规则", target: "~/.claude/CLAUDE.md", source_keys: ["rules.shared", "rules.claude"] },
            observed: {
              target: "~/.claude/CLAUDE.md",
              // 内容读得到 → 可以接管
              actual_digest: "a".repeat(64),
              blocked_reason: "目标文件存在外部修改，拒绝覆盖",
            },
            state: "unmanaged-detected",
            actions: [],
          }),
        ],
      },
      { command: "rules.status" },
    );
    const { calls } = renderWithEngine(<RulesHooksPage />, rulesRoutes);

    const button = await screen.findByTestId("takeover-rules:claude_global");
    await userEvent.click(button);

    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));
    expect(calls.find((call) => call.command === "rules:apply")!.args).toEqual([
      "--only",
      "rules:claude_global",
      "--allow-takeover",
    ]);
  });

  it("内容读不到（actual_digest 为 null）时不提供接管", async () => {
    const rulesRoutes: Record<string, Envelope> = { ...routes() };
    rulesRoutes["rules:status"] = makeEnvelope(
      {
        domain: "rules",
        elements: [
          makeElement({
            id: "rules:claude_global",
            domain: "rules",
            spec: { name: "Claude Code 全局规则", target: "~/.claude/CLAUDE.md", source_keys: [] },
            observed: { target: "~/.claude/CLAUDE.md", actual_digest: null, blocked_reason: "目标路径不安全" },
            state: "unmanaged-detected",
            actions: [],
          }),
        ],
      },
      { command: "rules.status" },
    );
    renderWithEngine(<RulesHooksPage />, rulesRoutes);
    await screen.findByText("Claude Code 全局规则");
    expect(screen.queryByTestId("takeover-rules:claude_global")).toBeNull();
  });
});
