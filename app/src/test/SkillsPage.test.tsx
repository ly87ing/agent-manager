import { describe, expect, it } from "vitest";
import { fireEvent } from "@testing-library/react";
import SkillsPage from "../pages/SkillsPage";
import type { Envelope } from "../lib/contract";
import { makeApplyResult, makeElement, makeEnvelope, makePlanAction, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

function skillsRoutes() {
  return {
    "skills:status": makeEnvelope(
      {
        domain: "skills",
        elements: [
          makeElement({
            id: "skill:agent-skills",
            domain: "skills",
            spec: {
              url: "https://github.com/example/agent-skills.git",
              skill_paths: ["alpha", "beta"],
              agents: ["claude-code", "codex"],
              matrix_mode: true,
              new_skill_default: "disabled",
            },
            observed: {
              repo_cache: "/repo/agent-skill/repositories/agent-skills",
              repo_cached: true,
              owner_marked: true,
              discovered: ["alpha", "beta", "inbox-one", "inbox-two"],
              discovery_inbox: ["inbox-one", "inbox-two"],
              extra_links: [],
              actions: ["update", "enable", "disable", "remove"],
              blocked_reason: null,
            },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:agent-skills/alpha",
            domain: "skills",
            spec: { collection: "agent-skills", name: "alpha", source_path: "/repo/.../alpha" },
            policy: { enabled: true, agents: { "claude-code": true, codex: true } },
            observed: {
              agents: {
                "claude-code": { state: "linked", link_path: "/home/.claude/skills/alpha", target_dir_exists: true, assigned: true, enabled: true },
                codex: { state: "dead-link", link_path: "/home/.codex/skills/alpha", target_dir_exists: true, assigned: true, enabled: true },
                "kimi-cli": { state: "unsupported", assigned: false, enabled: false, actions: [] },
              },
              discovery_inbox: false,
            },
            state: "drifted",
          }),
          makeElement({
            id: "skill:agent-skills/beta",
            domain: "skills",
            spec: { collection: "agent-skills", name: "beta", source_path: "/repo/.../beta" },
            policy: { enabled: true, agents: { "claude-code": true, codex: false } },
            observed: {
              agents: {
                "claude-code": { state: "missing-link", link_path: "/home/.claude/skills/beta", target_dir_exists: true, assigned: true, enabled: true },
                codex: { state: "disabled", link_path: "/home/.codex/skills/beta", target_dir_exists: true, assigned: true, enabled: false },
              },
              discovery_inbox: false,
            },
            state: "missing",
          }),
          makeElement({
            id: "skill:agent-skills/inbox-one",
            domain: "skills",
            spec: { collection: "agent-skills", name: "inbox-one", source_path: "/repo/.../inbox-one" },
            policy: { enabled: false, agents: {} },
            observed: { agents: {}, discovery_inbox: true },
            state: "disabled",
          }),
          makeElement({
            id: "skill:agent-skills/inbox-two",
            domain: "skills",
            spec: { collection: "agent-skills", name: "inbox-two", source_path: "/repo/.../inbox-two" },
            policy: { enabled: false, agents: {} },
            observed: { agents: {}, discovery_inbox: true },
            state: "disabled",
          }),
        ],
      },
      { command: "skills.status" },
    ),
    "skills:plan": makeEnvelope(
      {
        domain: "skills",
        actions: [makePlanAction({ op: "unlink", element: "skill:agent-skills", risk: "write-local", note: "清理链接" })],
        summary: { read: 0, "write-local": 1, "exec-local": 0, network: 0, "install-remove": 0 },
      },
      { command: "skills.plan" },
    ),
    "policy:get": makeEnvelope(
      makePolicyData({
        layers: {
          // sqlite 有效定义是完整定义：含表单未展示的自定义字段 branch
          sqlite: {
            skills: {
              repositories: {
                "agent-skills": {
                  url: "https://github.com/example/agent-skills.git",
                  skill_paths: ["alpha", "beta"],
                  agents: ["claude-code", "codex"],
                  branch: "release",
                },
              },
            },
          },
        },
      }),
      { command: "policy.get" },
    ),
    "policy:set": makeEnvelope(
      makePolicyData({ layers: null, effective: null, attribution: null, layer: "sqlite", key: "skills.definitions.agent-skills", value: null, path: "/native-manager.db" }),
      { command: "policy.set" },
    ),
    "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
  };
}

describe("SkillsPage 三级视图", () => {
  it("编辑集合：以 sqlite 完整定义整条替换保存，清空发现范围不复活，未展示字段不丢，不预览/同步", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    expect(screen.getByTestId("collection-name-input")).toBeDisabled();
    expect(screen.getByTestId("collection-url-input")).toHaveValue("https://github.com/example/agent-skills.git");
    fireEvent.change(screen.getByTestId("collection-paths-input"), { target: { value: "" } });
    await userEvent.click(screen.getByTestId("collection-form-submit"));

    // 唯一一次 policy:set，完整定义整条替换；无需迁移（sqlite 定义已在）
    const setCalls = calls.filter((call) => call.command === "policy:set");
    expect(setCalls).toHaveLength(1);
    expect(calls.some((call) => call.command === "db:migrate")).toBe(false);
    const write = setCalls[0];
    expect(write.args[write.args.indexOf("--key") + 1]).toBe("skills.repositories.agent-skills");
    const definition = JSON.parse(write.args[write.args.indexOf("--value") + 1]);
    expect(definition).toMatchObject({
      url: "https://github.com/example/agent-skills.git",
      skill_paths: null,
      agents: ["claude-code", "codex"],
      _delete: false,
      _replace: true,
    });
    // 反例断言：完整原定义里的自定义字段 branch 必须保留，不能只验证新 payload
    expect(definition.branch).toBe("release");
    // 不写启用键、不 plan/apply
    expect(calls.some((call) => call.command === "policy:set" && call.args[3] === "skills.agent-skills.enabled")).toBe(false);
    expect(calls.some((call) => call.command === "skills:plan")).toBe(false);
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
    expect(screen.queryByRole("dialog", { name: /将要执行的操作/ })).toBeNull();
  });

  it("编辑集合（无 DB 旧存储）：迁移后仍取不到完整定义则停止保存，零写入", async () => {
    const legacyRoutes: Record<string, Envelope> = {
      ...skillsRoutes(),
      "db:migrate": makeEnvelope({ migrated: true }, { command: "db.migrate" }),
    };
    // 旧存储：policy:get 始终返回无 sqlite 的 JSON 层，迁移后仍取不到完整定义
    legacyRoutes["policy:get"] = makeEnvelope(
      makePolicyData({
        layers: {
          sqlite: null,
        },
      }),
      { command: "policy.get" },
    );
    const { calls } = renderWithEngine(<SkillsPage />, legacyRoutes);
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    fireEvent.change(screen.getByTestId("collection-paths-input"), { target: { value: "" } });
    await userEvent.click(screen.getByTestId("collection-form-submit"));

    await screen.findByText(/已停止保存/);
    expect(calls.filter((call) => call.command === "db:migrate")).toHaveLength(1);
    expect(calls.filter((call) => call.command === "policy:set")).toHaveLength(0);
  });

  it("新增集合表单点取消：零写入且绝不迁移", async () => {
    const cancelRoutes = { ...skillsRoutes(), "db:migrate": makeEnvelope({ migrated: true }, { command: "db.migrate" }) };
    const { calls } = renderWithEngine(<SkillsPage />, cancelRoutes);
    await userEvent.click(await screen.findByTestId("add-collection-button"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("collection-name-input"), { target: { value: "my-skills" } });
    fireEvent.change(within(dialog).getByTestId("collection-url-input"), { target: { value: "https://github.com/example/my-skills.git" } });
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls.filter((call) => call.command === "policy:set")).toHaveLength(0);
    expect(calls.filter((call) => call.command === "db:migrate")).toHaveLength(0);
  });

  it("更新集合预览限定在所选集合", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("update-collection-agent-skills"));
    await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(calls.find((call) => call.command === "skills:plan")?.args).toEqual(["--only", "skill:agent-skills", "--action", "update"]);
  });

  it("集合卡片：缓存/收件箱计数徽标与新技能提示，不展示来源猜测徽标", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    const card = await screen.findByTestId("collection-card-agent-skills");
    expect(within(card).getByText("已下载到本地")).toBeInTheDocument();
    expect(within(card).getByTestId("inbox-count-agent-skills")).toHaveTextContent("新发现 2");
    expect(within(card).getByText(/新技能需单独添加到助手/)).toBeInTheDocument();
    // owner_marked 只表示本工具拥有缓存，不能判断来源：不展示 自研/第三方/整包分发 等旧徽标
    expect(within(card).queryByText("自研")).toBeNull();
    expect(within(card).queryByText("第三方")).toBeNull();
    expect(within(card).queryByText(/整包分发|按助手逐个授权|新技能默认/)).toBeNull();
  });

  it("新发现的技能区：列出 disabled+discovery_inbox 的 skill 与说明", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    const inbox = await screen.findByTestId("discovery-inbox");
    expect(within(inbox).getByTestId("inbox-total")).toHaveTextContent("2");
    expect(within(inbox).getByText("inbox-one")).toBeInTheDocument();
    expect(within(inbox).getByText("inbox-two")).toBeInTheDocument();
    expect(within(inbox).getAllByText(/启用后才会分发到助手/)).toHaveLength(2);
  });

  it("启用矩阵：单元格态 已启用/失效链接/未生效/已停用，新发现技能也进列表但无 cell 时明确未配置", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));
    expect(await screen.findByTitle("claude-code: linked")).toHaveTextContent("已启用");
    expect(screen.getByTitle("codex: dead-link")).toHaveTextContent("失效链接");
    expect(screen.getByTitle("claude-code: missing-link")).toHaveTextContent("未生效");
    const disabled = screen.getByTitle("codex: disabled");
    expect(disabled).toHaveTextContent("已停用");
    expect(disabled.className).toContain("zinc");
    // 新发现技能进入全部技能列表，可按助手添加；引擎未报告 cell 时不伪造可执行操作
    expect(screen.getAllByText("inbox-one").length).toBeGreaterThan(0);
    expect(screen.getAllByTitle("claude-code: 未配置").length).toBeGreaterThan(0);
    expect(screen.queryByTestId("action-add-skill:agent-skills/inbox-one-claude-code")).toBeNull();
  });

  it("移除集合：单个预览内确认，说明受影响分配与源目录保留，执行 skills:plan/apply 携带 --only skill:<name> --action remove", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    const removeBtn = await screen.findByTestId("remove-collection-agent-skills");
    await userEvent.click(removeBtn);

    // 无前置确认弹窗：直接进入唯一的计划预览
    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(screen.queryByTestId("confirm-modal-input")).toBeNull();
    // 预览说明显示受影响分配数（仅 assigned=true：alpha×2 + beta×2 = 4 条；未分配的 kimi-cli cell 不计）与源目录保留
    expect(dialog).toHaveTextContent("清理 4 条受影响的助手分配");
    expect(dialog).toHaveTextContent(/源目录保留/);

    const planCall = calls.find((c) => c.command === "skills:plan");
    expect(planCall).toBeDefined();
    expect(planCall!.args).toEqual(["--only", "skill:agent-skills", "--action", "remove"]);

    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    const applyCall = calls.find((c) => c.command === "skills:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.args).toEqual(["--only", "skill:agent-skills", "--action", "remove"]);
  });

  it("视图切换与键盘导航：支持键盘切换标签，点击集合卡片名称进入全部技能并筛选对应集合", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    const tabCollections = await screen.findByRole("tab", { name: "集合列表" });
    const tabSkills = screen.getByRole("tab", { name: "全部技能" });

    // 默认集合列表
    expect(tabCollections).toHaveAttribute("aria-selected", "true");
    expect(tabCollections).toHaveAttribute("tabIndex", "0");
    expect(tabSkills).toHaveAttribute("aria-selected", "false");
    expect(tabSkills).toHaveAttribute("tabIndex", "-1");

    // 键盘 ArrowRight 切换到全部技能
    fireEvent.keyDown(tabCollections, { key: "ArrowRight" });
    expect(tabSkills).toHaveAttribute("aria-selected", "true");
    expect(screen.getByTestId("filter-collection-select")).toBeInTheDocument();

    // 键盘 ArrowLeft 切回集合列表
    fireEvent.keyDown(tabSkills, { key: "ArrowLeft" });
    expect(tabCollections).toHaveAttribute("aria-selected", "true");

    // 点击集合名称下钻
    const collectionLink = screen.getByRole("button", { name: "agent-skills" });
    await userEvent.click(collectionLink);
    expect(tabSkills).toHaveAttribute("aria-selected", "true");
    const colSelect = screen.getByTestId("filter-collection-select") as HTMLSelectElement;
    expect(colSelect.value).toBe("agent-skills");
  });

  it("全部技能列表：集合筛选与助手筛选，可清除筛选", async () => {
    renderWithEngine(<SkillsPage initialAgent="claude-code" />, skillsRoutes());
    // initialAgent 预设时默认进入全部技能且预选助手
    const tabSkills = await screen.findByRole("tab", { name: "全部技能" });
    expect(tabSkills).toHaveAttribute("aria-selected", "true");

    const agentSelect = screen.getByTestId("filter-agent-select") as HTMLSelectElement;
    expect(agentSelect.value).toBe("claude-code");

    // 仅展示 claude-code 的行
    expect(screen.queryByTitle("codex: dead-link")).toBeNull();
    expect(screen.getByTitle("claude-code: linked")).toBeInTheDocument();

    // 清除筛选按钮
    const clearBtn = screen.getByRole("button", { name: /清除筛选/ });
    await userEvent.click(clearBtn);
    expect(agentSelect.value).toBe("all");
    expect(screen.getByTitle("codex: dead-link")).toBeInTheDocument();
  });

  it("基于 observed.agents[agent].actions 渲染明确操作按钮，无 actions 时显示占位提示，展示明确阻断原因", async () => {
    const customRoutes = skillsRoutes();
    customRoutes["skills:status"] = makeEnvelope(
      {
        domain: "skills",
        elements: [
          makeElement({
            id: "skill:agent-skills",
            domain: "skills",
            spec: { url: "https://github.com/example/agent-skills.git", skill_paths: ["alpha"], agents: ["claude-code", "codex"] },
            observed: { repo_cache: "/repo/agent-skills", repo_cached: true, owner_marked: true, discovered: ["alpha"], discovery_inbox: [] },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:agent-skills/alpha",
            domain: "skills",
            spec: { collection: "agent-skills", name: "alpha", source_path: "/repo/.../alpha" },
            policy: { enabled: true, agents: { "claude-code": true, codex: false } },
            observed: {
              agents: {
                "claude-code": {
                  state: "linked",
                  link_path: "/home/.claude/skills/alpha",
                  target_dir_exists: true,
                  actions: ["update", "disable", "remove"],
                  blocked_reason: null,
                },
                codex: {
                  state: "disabled",
                  link_path: null,
                  target_dir_exists: false,
                  actions: [], // 无 actions
                  blocked_reason: "Rust 原生引擎暂未提供此操作",
                },
              },
              discovery_inbox: false,
            },
            state: "managed-ok",
          }),
        ],
      },
      { command: "skills.status" },
    );

    const { calls } = renderWithEngine(<SkillsPage />, customRoutes);
    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));

    // claude-code: 具有 actions 按钮
    expect(screen.getByTestId("action-update-skill:agent-skills/alpha-claude-code")).toBeInTheDocument();
    expect(screen.getByTestId("action-disable-skill:agent-skills/alpha-claude-code")).toBeInTheDocument();
    expect(screen.getByTestId("action-remove-skill:agent-skills/alpha-claude-code")).toBeInTheDocument();
    // 没有 assign / enable 按钮
    expect(screen.queryByTestId("action-assign-skill:agent-skills/alpha-claude-code")).toBeNull();
    expect(screen.queryByTestId("action-enable-skill:agent-skills/alpha-claude-code")).toBeNull();

    // 点击禁用按钮触发 skills:plan 携带对应 action 与 agent
    await userEvent.click(screen.getByTestId("action-disable-skill:agent-skills/alpha-claude-code"));
    await screen.findByRole("dialog", { name: /将要执行的操作/ });
    const planCall = calls.find((c) => c.command === "skills:plan");
    expect(planCall).toBeDefined();
    expect(planCall!.args).toEqual(["--only", "skill:agent-skills/alpha", "--action", "disable", "--agent", "claude-code"]);

    // codex: actions 为空，不推测动作，展示「当前引擎未提供操作」
    expect(screen.getByText("当前引擎未提供操作")).toBeInTheDocument();
    // codex: 展示明确阻断原因
    expect(screen.getByText("Rust 原生引擎暂未提供此操作")).toBeInTheDocument();
  });
});

describe("SkillsPage 空态引导与发现失败呈现", () => {
  it("存在集合但全部 repo_cached=false 时，全部技能空态引导去「集合列表」更新资源库", async () => {
    const routes = {
      "skills:status": makeEnvelope(
        {
          domain: "skills",
          elements: [
            makeElement({
              id: "skill:agent-skills",
              domain: "skills",
              spec: { url: "https://github.com/example/agent-skills.git", skill_paths: null, agents: ["codex"] },
              observed: {
                repo_cache: "/repo/agent-skills",
                repo_cached: false,
                owner_marked: false,
                discovered: [],
                discovery_error: null,
                discovery_inbox: [],
                extra_links: [],
                actions: ["update", "disable", "remove"],
                blocked_reason: null,
              },
              state: "not-materialized",
            }),
          ],
        },
        { command: "skills.status" },
      ),
    };
    renderWithEngine(<SkillsPage />, routes);
    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));
    const empty = await screen.findByTestId("skills-empty-state");
    expect(empty).toHaveTextContent("集合资源库尚未拉取：请到「集合列表」点击「更新资源库」，拉取成功后技能会出现在这里");
    expect(empty).not.toHaveTextContent("无符合筛选条件的技能分配");
  });

  it("集合已拉取但无技能行时保持现有空态文案；发现失败在集合卡片以警告样式呈现", async () => {
    const routes = {
      "skills:status": makeEnvelope(
        {
          domain: "skills",
          elements: [
            makeElement({
              id: "skill:broken-skills",
              domain: "skills",
              spec: { url: "https://github.com/example/broken-skills.git", skill_paths: ["alpha", "ghost"], agents: ["codex"] },
              observed: {
                repo_cache: "/repo/broken-skills",
                repo_cached: true,
                owner_marked: true,
                discovered: [],
                discovery_error: {
                  category: "missing-dir",
                  skill_paths_item: "ghost",
                  message: "技能发现失败[missing-dir]: skill_paths 指向的目录不存在: ghost",
                },
                discovery_inbox: [],
                extra_links: [],
                actions: ["update", "disable", "remove"],
                blocked_reason: "技能发现失败（missing-dir），集合暂按空发现处理；修复 skill_paths 后自动恢复",
              },
              state: "managed-ok",
            }),
          ],
        },
        { command: "skills.status" },
      ),
    };
    renderWithEngine(<SkillsPage />, routes);
    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));
    const empty = await screen.findByTestId("skills-empty-state");
    expect(empty).toHaveTextContent("无符合筛选条件的技能分配");
    expect(empty).not.toHaveTextContent("集合资源库尚未拉取");

    await userEvent.click(screen.getByRole("tab", { name: "集合列表" }));
    const card = await screen.findByTestId("collection-card-broken-skills");
    expect(within(card).getByTestId("discovery-error-broken-skills")).toHaveTextContent(
      "技能发现失败[missing-dir]: skill_paths 指向的目录不存在: ghost",
    );
    expect(within(card).getByText(/技能发现失败（missing-dir）/)).toBeInTheDocument();
  });
});
