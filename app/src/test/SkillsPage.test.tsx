import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import SkillsPage from "../pages/SkillsPage";
import type { Envelope } from "../lib/contract";
import { makeApplyResult, makeElement, makeEnvelope, makePolicyData } from "./fixtures";
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
    // 「不限定范围」现在是一个显式选项，不再靠把文本框清空来表达
    await userEvent.click(screen.getByTestId("collection-scope-all"));
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
    // 不写启用键、不 apply/preflight
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
    await userEvent.click(screen.getByTestId("collection-scope-all"));
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

  it("新增集合可选本机目录来源：只写 source_path 并清空 url", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("add-collection-button"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("collection-name-input"), { target: { value: "vendor-tool" } });
    await userEvent.click(within(dialog).getByTestId("collection-source-local"));
    // 切到本机目录后不再要求 git 地址
    expect(within(dialog).queryByTestId("collection-url-input")).toBeNull();
    fireEvent.change(within(dialog).getByTestId("collection-source-path-input"), { target: { value: "~/.vendor-tool/skills" } });
    // 新建时来源还没扫过、列不出候选，用手动填写补上
    await userEvent.click(within(dialog).getByTestId("collection-scope-limited"));
    await userEvent.click(within(dialog).getByTestId("collection-paths-manual-toggle"));
    fireEvent.change(within(dialog).getByTestId("collection-paths-input"), { target: { value: "ones" } });
    await userEvent.click(within(dialog).getByTestId("collection-form-submit"));

    const setCalls = calls.filter((call) => call.command === "policy:set");
    expect(setCalls).toHaveLength(1);
    const write = setCalls[0];
    expect(write.args[write.args.indexOf("--key") + 1]).toBe("skills.repositories.vendor-tool");
    expect(JSON.parse(write.args[write.args.indexOf("--value") + 1])).toMatchObject({
      source_path: "~/.vendor-tool/skills",
      url: null,
      skill_paths: ["ones"],
      _replace: true,
    });
    // 保存定义不触发任何执行
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
  });

  it("本机目录来源填写非法路径时停在表单，不写入", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("add-collection-button"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("collection-name-input"), { target: { value: "vendor-tool" } });
    await userEvent.click(within(dialog).getByTestId("collection-source-local"));
    fireEvent.change(within(dialog).getByTestId("collection-source-path-input"), { target: { value: "relative/skills" } });
    await userEvent.click(within(dialog).getByTestId("collection-form-submit"));

    expect(await within(dialog).findByText(/绝对路径或以 ~\/ 开头/)).toBeInTheDocument();
    expect(calls.filter((call) => call.command === "policy:set")).toHaveLength(0);
  });

  it("更新集合：点击直接 skills:apply 限定在所选集合（带 run_id），不取 plan、不开弹窗", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("update-collection-agent-skills"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));

    const applyCalls = calls.filter((call) => call.command === "skills:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "skill:agent-skills", "--action", "update"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
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

  it("集合卡片仅提示新发现数量，不再重复列出技能与本机路径", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    expect(await screen.findByTestId("inbox-count-agent-skills")).toHaveTextContent("新发现 2");
    expect(screen.queryByTestId("discovery-inbox")).toBeNull();
    expect(screen.queryByText("inbox-one")).toBeNull();
    expect(screen.queryByText("inbox-two")).toBeNull();
  });

  it("启用矩阵：单元格态 已启用/失效链接/未生效/已停用，新发现技能也进列表但无 cell 时明确未配置", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));
    expect(await screen.findByTitle("claude-code: linked")).toHaveTextContent("已启用");
    expect(screen.getByTitle("codex: dead-link")).toHaveTextContent("失效链接");
    expect(screen.getByTitle("claude-code: missing-link")).toHaveTextContent("未生效");
    const disabled = screen.getByTitle("codex: disabled");
    expect(disabled).toHaveTextContent("已停用");
    expect(disabled.className).toContain("muted");
    // 原来有一列叫「当前分配版本」，里面放的却是「已链接/已分配」——那是状态不是版本，
    // 且与状态列语义重叠。矩阵里状态只由格子表达，版本等细节进悬停，不再单开一列。
    // 新发现技能进入全部技能列表，可按助手添加；引擎未报告 cell 时不伪造可执行操作
    expect(screen.getAllByText("inbox-one").length).toBeGreaterThan(0);
    expect(screen.getAllByTitle("claude-code: 未配置").length).toBeGreaterThan(0);
    expect(screen.queryByTestId("action-add-skill:agent-skills/inbox-one-claude-code")).toBeNull();
  });

  it("移除集合：点击先打开 ConfirmModal，取消时零 apply/零 policy:set", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    const removeBtn = await screen.findByTestId("remove-collection-agent-skills");
    await userEvent.click(removeBtn);

    const dialog = await screen.findByRole("dialog", { name: "删除集合 agent-skills" });
    expect(dialog).toHaveTextContent(/源目录保留/);
    expect(screen.getByTestId("confirm-modal-input")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("移除集合：输入集合名确认后 skills:apply 携带 --only skill:<name> --action remove（带 run_id），无 plan、零 policy:set", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    const removeBtn = await screen.findByTestId("remove-collection-agent-skills");
    await userEvent.click(removeBtn);

    await screen.findByRole("dialog", { name: "删除集合 agent-skills" });
    await userEvent.type(screen.getByTestId("confirm-modal-input"), "agent-skills");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    const applyCalls = calls.filter((call) => call.command === "skills:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "skill:agent-skills", "--action", "remove"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("技能 × 助手 remove：点击先确认，确认后 skills:apply 精确 --only/--action/--agent，取消零 apply", async () => {
    const customRoutes = skillsRoutes();
    customRoutes["skills:status"] = makeEnvelope(
      {
        domain: "skills",
        elements: [
          makeElement({
            id: "skill:agent-skills",
            domain: "skills",
            spec: { url: "https://github.com/example/agent-skills.git", skill_paths: ["alpha"], agents: ["claude-code"] },
            observed: { repo_cache: "/repo/agent-skills", repo_cached: true, owner_marked: true, discovered: ["alpha"], discovery_inbox: [] },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:agent-skills/alpha",
            domain: "skills",
            spec: { collection: "agent-skills", name: "alpha", source_path: "/repo/.../alpha" },
            policy: { enabled: true, agents: { "claude-code": true } },
            observed: {
              agents: {
                "claude-code": {
                  state: "linked",
                  link_path: "/home/.claude/skills/alpha",
                  target_dir_exists: true,
                  assigned: true,
                  enabled: true,
                  actions: ["remove"],
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
    const removeBtn = await screen.findByTestId("action-remove-skill:agent-skills/alpha-claude-code");

    await userEvent.click(removeBtn);
    const dialog = await screen.findByRole("dialog", { name: "删除分配 alpha @ claude-code" });
    expect(dialog).toHaveTextContent(/只清理该助手的入口/);

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);

    await userEvent.click(removeBtn);
    await screen.findByRole("dialog", { name: "删除分配 alpha @ claude-code" });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(1));
    expect(calls.filter((call) => call.command === "skills:apply")[0].args).toEqual([
      "--only", "skill:agent-skills/alpha", "--action", "remove", "--agent", "claude-code",
    ]);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
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

    // 点击禁用按钮直接 skills:apply 携带对应 action 与 agent（带 run_id），不取 plan、不开弹窗
    await userEvent.click(screen.getByTestId("action-disable-skill:agent-skills/alpha-claude-code"));
    await waitFor(() => expect(calls.some((c) => c.command === "skills:apply")).toBe(true));
    const applyCalls = calls.filter((c) => c.command === "skills:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "skill:agent-skills/alpha", "--action", "disable", "--agent", "claude-code"]);
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((c) => c.command.endsWith(":plan"))).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();

    // codex: actions 为空，不推测动作。引擎给了阻断原因就显示原因（下一句断言），
    // 没给才退到「这里没有可做的」——都不报告引擎的内部情况。
    // codex: 展示明确阻断原因
    expect(screen.getAllByText("Rust 原生引擎暂未提供此操作").length).toBeGreaterThan(0);
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
    expect(empty).toHaveTextContent("集合资源库尚未拉取");
    expect(empty).toHaveTextContent("请到「集合列表」点击「更新资源库」，拉取成功后技能会出现在这里");
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

describe("SkillsPage 计划外链接（extra-link）接管与取消链接", () => {
  function extraLinkRoutes(): Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> {
    return {
      "skills:status": makeEnvelope(
        {
          domain: "skills",
          elements: [
            makeElement({
              id: "skill:agent-skills",
              domain: "skills",
              spec: { url: "https://github.com/example/agent-skills.git", skill_paths: ["alpha", "beta", "gamma"], agents: ["codex"] },
              observed: { repo_cache: "/repo/agent-skills", repo_cached: true, owner_marked: true, discovered: ["alpha", "beta", "gamma"], discovery_inbox: [], extra_links: [], actions: ["update", "disable", "remove"], blocked_reason: null },
              state: "managed-ok",
            }),
            makeElement({
              id: "skill:agent-skills/alpha",
              domain: "skills",
              spec: { collection: "agent-skills", name: "alpha", source_path: "/repo/.../alpha" },
              policy: { enabled: true, agents: { codex: false } },
              observed: {
                agents: {
                  // 旧体系遗留的外部链接：未分配 + 非受管
                  codex: { state: "extra-link", link_path: "/home/.codex/skills/alpha", target_dir_exists: true, assigned: false, enabled: false, managed: false, version: null, actions: ["add", "remove"] },
                },
                discovery_inbox: false,
              },
              state: "managed-ok",
            }),
            makeElement({
              id: "skill:agent-skills/beta",
              domain: "skills",
              spec: { collection: "agent-skills", name: "beta", source_path: "/repo/.../beta" },
              policy: { enabled: true, agents: { codex: false } },
              observed: {
                agents: {
                  // 受管残留链接：未分配但链接指向受管基目录内
                  codex: { state: "extra-link", link_path: "/home/.codex/skills/beta", target_dir_exists: true, assigned: false, enabled: false, managed: true, version: null, actions: ["add", "remove"] },
                },
                discovery_inbox: false,
              },
              state: "managed-ok",
            }),
            makeElement({
              id: "skill:agent-skills/gamma",
              domain: "skills",
              spec: { collection: "agent-skills", name: "gamma", source_path: "/repo/.../gamma" },
              policy: { enabled: true, agents: { codex: false } },
              observed: {
                agents: {
                  // 未分配且无链接：维持既有「添加」语义
                  codex: { state: "not-assigned", link_path: "/home/.codex/skills/gamma", target_dir_exists: true, assigned: false, enabled: false, managed: false, version: null, actions: ["add"] },
                },
                discovery_inbox: false,
              },
              state: "managed-ok",
            }),
          ],
        },
        { command: "skills.status" },
      ),
      "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
    };
  }

  it("extra-link：动作为 纳入管理/取消链接，状态由上面矩阵那一格表达", async () => {
    renderWithEngine(<SkillsPage />, extraLinkRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));

    const addBtn = await screen.findByTestId("action-add-skill:agent-skills/alpha-codex");
    expect(addBtn).toHaveTextContent("纳入管理");
    expect(addBtn).toHaveAttribute("title", "保存当前技能内容并接管既有链接");
    const removeBtn = screen.getByTestId("action-remove-skill:agent-skills/alpha-codex");
    expect(removeBtn).toHaveTextContent("取消链接");
    expect(removeBtn).toHaveAttribute("title", "移除磁盘上的链接入口，不删除技能本身");

    // 状态和操作在同一个助手单元格内，不再有重复的展开行。
    const matrixRow = screen.getByTestId("matrix-row-skill:agent-skills/alpha");
    expect(within(matrixRow).getByTitle(/extra-link/)).toHaveTextContent("计划外链接");
    expect(screen.queryByTestId("expand-skill:agent-skills/alpha")).toBeNull();
  });

  it("非受管外部链接：纳入管理先弹确认，取消零 apply，确认后 apply 携带 --allow-takeover", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, extraLinkRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));
    const addBtn = await screen.findByTestId("action-add-skill:agent-skills/alpha-codex");

    await userEvent.click(addBtn);
    const dialog = await screen.findByRole("dialog", { name: "纳入管理 alpha @ codex" });
    expect(dialog).toHaveTextContent(/目标位置存在外部链接内容，确认后将被替换（此操作会修改你的文件系统）/);

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);

    await userEvent.click(addBtn);
    await screen.findByRole("dialog", { name: "纳入管理 alpha @ codex" });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(1));
    const applyCalls = calls.filter((call) => call.command === "skills:apply");
    expect(applyCalls[0].args).toEqual([
      "--only", "skill:agent-skills/alpha", "--action", "add", "--agent", "codex", "--allow-takeover",
    ]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("非受管外部链接：取消链接先弹确认，确认后 apply 以 remove + --allow-takeover 执行", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, extraLinkRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));

    await userEvent.click(await screen.findByTestId("action-remove-skill:agent-skills/alpha-codex"));
    const dialog = await screen.findByRole("dialog", { name: "取消链接 alpha @ codex" });
    expect(dialog).toHaveTextContent(/目标位置存在外部链接内容，确认后将被移除（此操作会修改你的文件系统）/);

    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(1));
    expect(calls.filter((call) => call.command === "skills:apply")[0].args).toEqual([
      "--only", "skill:agent-skills/alpha", "--action", "remove", "--agent", "codex", "--allow-takeover",
    ]);
  });

  it("受管残留链接（managed=true）：纳入管理与取消链接均直接执行，不弹确认、不带 --allow-takeover", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, extraLinkRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));

    await userEvent.click(await screen.findByTestId("action-remove-skill:agent-skills/beta-codex"));
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(1));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls.filter((call) => call.command === "skills:apply")[0].args).toEqual([
      "--only", "skill:agent-skills/beta", "--action", "remove", "--agent", "codex",
    ]);

    await userEvent.click(screen.getByTestId("action-add-skill:agent-skills/beta-codex"));
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(2));
    expect(screen.queryByRole("dialog")).toBeNull();
    const addArgs = calls.filter((call) => call.command === "skills:apply")[1].args;
    expect(addArgs).toEqual(["--only", "skill:agent-skills/beta", "--action", "add", "--agent", "codex"]);
    expect(addArgs).not.toContain("--allow-takeover");
  });

  it("not-assigned 行：add 维持「添加」标签，点击直接 apply，不弹确认、不带 --allow-takeover", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, extraLinkRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));

    const addBtn = await screen.findByTestId("action-add-skill:agent-skills/gamma-codex");
    expect(addBtn).toHaveTextContent("添加");
    // 未分配且无链接的不得误显「已链接」，由同一单元格直接表达。
    const matrixRow = screen.getByTestId("matrix-row-skill:agent-skills/gamma");
    expect(within(matrixRow).getByTitle(/not-assigned/)).toHaveTextContent("未分配");
    expect(within(matrixRow).queryByText("已链接")).toBeNull();
    expect(screen.queryByTestId("action-remove-skill:agent-skills/gamma-codex")).toBeNull();

    await userEvent.click(addBtn);
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(1));
    expect(screen.queryByRole("dialog")).toBeNull();
    const args = calls.filter((call) => call.command === "skills:apply")[0].args;
    expect(args).toEqual(["--only", "skill:agent-skills/gamma", "--action", "add", "--agent", "codex"]);
    expect(args).not.toContain("--allow-takeover");
  });
});

describe("集合的「用哪些技能」改成勾选", () => {
  it("来源里发现的技能列成复选项，勾选后保存成清单", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    await userEvent.click(screen.getByTestId("collection-scope-limited"));

    // 引擎报告的 discovered 直接成为候选，不用自己敲目录名
    for (const name of ["alpha", "beta", "inbox-one", "inbox-two"]) {
      expect(screen.getByTestId(`collection-path-${name}`)).toBeInTheDocument();
    }
    await userEvent.click(screen.getByTestId("collection-paths-clear"));
    await userEvent.click(screen.getByTestId("collection-path-alpha"));
    await userEvent.click(screen.getByTestId("collection-path-beta"));
    await userEvent.click(screen.getByTestId("collection-form-submit"));

    const setCalls = calls.filter((call) => call.command === "policy:set");
    const write = setCalls[setCalls.length - 1];
    expect(JSON.parse(write.args[write.args.indexOf("--value") + 1])).toMatchObject({
      skill_paths: ["alpha", "beta"],
    });
  });

  it("全选一键勾上全部候选", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    await userEvent.click(screen.getByTestId("collection-scope-limited"));
    await userEvent.click(screen.getByTestId("collection-paths-clear"));
    await userEvent.click(screen.getByTestId("collection-paths-select-all"));
    await userEvent.click(screen.getByTestId("collection-form-submit"));

    const setCalls = calls.filter((call) => call.command === "policy:set");
    const write = setCalls[setCalls.length - 1];
    const saved = JSON.parse(write.args[write.args.indexOf("--value") + 1]) as { skill_paths: string[] };
    expect([...saved.skill_paths].sort()).toEqual(["alpha", "beta", "inbox-one", "inbox-two"]);
  });

  it("选了「只用其中这些」却一个没勾：拦下来并说清怎么办，不写库", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    await userEvent.click(screen.getByTestId("collection-scope-limited"));
    await userEvent.click(screen.getByTestId("collection-paths-clear"));
    await userEvent.click(screen.getByTestId("collection-form-submit"));

    expect(await screen.findByText(/想全用就切回/)).toBeInTheDocument();
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("仓库还没取下来时说清为什么列不出来，而不是给个空白框", async () => {
    const routes = skillsRoutes();
    const status = JSON.parse(JSON.stringify(routes["skills:status"]));
    status.data.elements[0].observed.repo_cached = false;
    status.data.elements[0].observed.discovered = [];
    routes["skills:status"] = status;
    renderWithEngine(<SkillsPage />, routes);
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    await userEvent.click(screen.getByTestId("collection-scope-limited"));
    expect(screen.getByTestId("collection-paths-empty")).toHaveTextContent("仓库还没取到本地");
  });
});
