import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import SkillsPage from "../pages/SkillsPage";
import type { Envelope } from "../lib/contract";
import { makeApplyResult, makeElement, makeEnvelope, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

/** 等集合出现，把收起的集合逐个展开（没有待处理的集合默认收起）。 */
async function expandAll() {
  await screen.findAllByTestId(/^collection-card-/);
  for (const toggle of screen.getAllByTestId(/^collection-toggle-/)) {
    if (toggle.getAttribute("aria-expanded") === "false") await userEvent.click(toggle);
  }
  await screen.findAllByTestId(/^skill-row-/);
}

/** 点某个动作：主动作是行上的开关；次要动作在某一行的「⋯」里，逐个打开找到它。 */
async function clickAction(testId: string) {
  const direct = screen.queryByTestId(testId);
  if (direct) return userEvent.click(direct);
  for (const menu of screen.getAllByRole("button", { name: /的更多操作$/ })) {
    await userEvent.click(menu);
    const item = screen.queryByTestId(testId);
    if (item) return userEvent.click(item);
    await userEvent.click(menu);
  }
  throw new Error(`找不到动作 ${testId}`);
}

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
              available: ["alpha", "beta", "inbox-one", "inbox-two"].map((name) => ({ name, path: `skills/${name}` })),
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
                "claude-code": { state: "linked", link_path: "/home/.claude/skills/alpha", target_dir_exists: true, assigned: true, enabled: true, actions: ["disable", "remove"] },
                codex: { state: "dead-link", link_path: "/home/.codex/skills/alpha", target_dir_exists: true, assigned: true, enabled: true, actions: ["update", "disable", "remove"] },
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
                "claude-code": { state: "missing-link", link_path: "/home/.claude/skills/beta", target_dir_exists: true, assigned: true, enabled: true, actions: ["update", "disable", "remove"] },
                codex: { state: "disabled", link_path: "/home/.codex/skills/beta", target_dir_exists: true, assigned: true, enabled: false, actions: ["enable", "remove"] },
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
  it("编辑集合：以 sqlite 完整定义整条替换保存，全选并自动包含写成技能所在目录，未展示字段不丢，不预览/同步", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    // 名称不能改：只在标题里显示
    expect(screen.queryByTestId("collection-name-input")).toBeNull();
    expect(screen.getByRole("heading", { name: "编辑来源 agent-skills" })).toBeInTheDocument();
    expect(screen.getByTestId("collection-url-input")).toHaveValue("https://github.com/example/agent-skills.git");
    await userEvent.click(screen.getByTestId("collection-paths-select-all"));
    await userEvent.click(screen.getByTestId("collection-auto-include"));
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
      skill_paths: ["skills"],
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

  it("本机位置可以直接选某个技能的 SKILL.md：存成它所在的技能文件夹；规则位置可改指", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("add-collection-button"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("collection-name-input"), { target: { value: "review" } });
    await userEvent.click(within(dialog).getByTestId("collection-source-local"));
    fireEvent.change(within(dialog).getByTestId("collection-source-path-input"), { target: { value: "~/my-skills/review/SKILL.md" } });
    fireEvent.change(within(dialog).getByTestId("collection-rules-path-input"), { target: { value: "docs/CLAUDE.md" } });
    await userEvent.click(within(dialog).getByTestId("collection-form-submit"));
    const write = calls.find((call) => call.command === "policy:set")!;
    expect(JSON.parse(write.args[write.args.indexOf("--value") + 1])).toMatchObject({
      source_path: "~/my-skills/review",
      rules_path: "docs/CLAUDE.md",
      url: null,
    });
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
    // 新建时来源还没扫过、列不出候选，直接填写
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

  it("更新集合：有新版本时行上常驻「更新」，点击直接 skills:apply 限定在所选集合（带 run_id），不取 plan、不开弹窗", async () => {
    const routes = skillsRoutes();
    const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>;
    status.data.elements[0].freshness = { ...status.data.elements[0].freshness, state: "behind" };
    const { calls } = renderWithEngine(<SkillsPage />, routes);
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

  it("集合标题行：只提示异常与新发现数量，不展示来源猜测徽标", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    const card = await screen.findByTestId("collection-card-agent-skills");
    // 已下载是正常状态，不占位；只有没下载才提示
    expect(within(card).queryByText("尚未下载")).toBeNull();
    expect(within(card).getByTestId("inbox-count-agent-skills")).toHaveTextContent("新发现 2");
    // owner_marked 只表示本工具拥有缓存，不能判断来源：不展示 自研/第三方/整包分发 等旧徽标
    expect(within(card).queryByText("自研")).toBeNull();
    expect(within(card).queryByText("第三方")).toBeNull();
    expect(within(card).queryByText(/整包分发|按助手逐个授权|新技能默认/)).toBeNull();
  });

  it("新发现的技能单独一行标「新发现」；没有可加的账号时不给「添加」", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    expect(await screen.findByTestId("inbox-count-agent-skills")).toHaveTextContent("新发现 2");
    expect(within(screen.getByTestId("skill-row-skill:agent-skills/inbox-one")).getByText("新发现")).toBeInTheDocument();
    expect(screen.queryByTestId("skill-add-all-skill:agent-skills/inbox-one")).toBeNull();
  });

  it("新发现技能点「添加」：一次加到全部可加的账号，逐个 add", async () => {
    const routes = skillsRoutes();
    const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>;
    const inbox = status.data.elements.find((element) => element.id === "skill:agent-skills/inbox-one")!;
    inbox.observed = {
      agents: {
        "claude-code": { state: "not-assigned", assigned: false, enabled: false, actions: ["add"] },
        codex: { state: "not-assigned", assigned: false, enabled: false, actions: ["add"] },
      },
      discovery_inbox: true,
    };
    const { calls } = renderWithEngine(<SkillsPage />, routes);
    await userEvent.click(await screen.findByTestId("skill-add-all-skill:agent-skills/inbox-one"));
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(2));
    expect(calls.filter((call) => call.command === "skills:apply").map((call) => call.args)).toEqual([
      ["--only", "skill:agent-skills/inbox-one", "--action", "add", "--agent", "claude-code"],
      ["--only", "skill:agent-skills/inbox-one", "--action", "add", "--agent", "codex"],
    ]);
  });

  it("补齐：表头按账号给出「补齐」，只补这个账号缺的在用技能，账号上停用的不动", async () => {
    const routes = skillsRoutes();
    const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>;
    const alpha = status.data.elements.find((element) => element.id === "skill:agent-skills/alpha")!;
    (alpha.observed as { agents: Record<string, unknown> }).agents.work = { state: "not-assigned", assigned: false, enabled: false, actions: ["add"] };
    const { calls } = renderWithEngine(<SkillsPage />, routes);
    // beta 在 codex 上是停用（例外），不算缺：codex 列显示「齐了」
    await userEvent.click(await screen.findByTestId("skills-fill-work"));
    expect(screen.queryByTestId("skills-fill-codex")).toBeNull();
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(1));
    expect(calls.find((call) => call.command === "skills:apply")!.args).toEqual(["--only", "skill:agent-skills/alpha", "--action", "add", "--agent", "work"]);
  });

  it("全部更新：先逐个取回集合，再把在用、已有新版本的技能换到新版本；新发现的不加，停用的不动", async () => {
    const routes: Record<string, Envelope> = {
      ...skillsRoutes(),
      "system:refresh-upstream": makeEnvelope({}, { command: "system.refresh-upstream" }),
    };
    const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>;
    const alpha = status.data.elements.find((element) => element.id === "skill:agent-skills/alpha")!;
    Object.assign((alpha.observed as { agents: Record<string, Record<string, unknown>> }).agents["claude-code"], { version: "v1", available_version: "v2", actions: ["update", "disable", "remove"] });
    const { calls } = renderWithEngine(<SkillsPage />, routes);
    await userEvent.click(await screen.findByTestId("skills-update-all"));
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(2));
    const applies = calls.filter((call) => call.command === "skills:apply").map((call) => call.args);
    expect(applies[0]).toEqual(["--only", "skill:agent-skills", "--action", "update"]);
    expect(applies[1]).toEqual(["--only", "skill:agent-skills/alpha", "--action", "update", "--agent", "claude-code"]);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("启用 / 停用后提示条上可以撤销：停用的撤销是重新启用，同一账号", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await expandAll();
    await userEvent.click(screen.getByTestId("action-disable-skill:agent-skills/alpha-claude-code"));
    await userEvent.click(await screen.findByRole("button", { name: "撤销" }));
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(2));
    const applies = calls.filter((call) => call.command === "skills:apply").map((call) => call.args);
    expect(applies[0]).toEqual(["--only", "skill:agent-skills/alpha", "--action", "disable", "--agent", "claude-code"]);
    expect(applies[1]).toEqual(["--only", "skill:agent-skills/alpha", "--action", "enable", "--agent", "claude-code"]);
  });

  it("账号开关：位置反映账号里有没有这个技能的链接，损坏的链接显示为需要处理但仍算开", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    await expandAll();
    const linked = screen.getByTestId("action-disable-skill:agent-skills/alpha-claude-code");
    expect(linked).toHaveAttribute("role", "switch");
    expect(linked).toHaveAttribute("aria-checked", "true");
    expect(linked).toHaveAttribute("data-tone", "on");
    const broken = screen.getByTestId("action-update-skill:agent-skills/alpha-codex");
    expect(broken).toHaveAttribute("aria-checked", "true");
    expect(broken).toHaveAttribute("data-tone", "warn");
    const disabled = screen.getByTestId("action-enable-skill:agent-skills/beta-codex");
    expect(disabled).toHaveAttribute("aria-checked", "false");
  });

  it("启用矩阵：正常状态由启停动作表达，链接异常保留状态，新发现技能无 cell 时明确未配置", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    await expandAll();
    // 开关直接在行上：点一下就是它的主动作，说明在悬停提示里
    expect(await screen.findByTestId("action-disable-skill:agent-skills/alpha-claude-code")).toHaveAttribute("title", "claude-code：已启用，点击停用");
    expect(screen.getByTestId("action-update-skill:agent-skills/alpha-codex")).toHaveAttribute("title", "codex：链接损坏，点击修复");
    expect(screen.getByTestId("action-update-skill:agent-skills/beta-claude-code")).toHaveAttribute("data-tone", "warn");
    expect(screen.getByTestId("action-enable-skill:agent-skills/beta-codex")).toHaveAttribute("title", "codex：已停用，点击启用");
    // 原来有一列叫「当前分配版本」，里面放的却是「已链接/已分配」——那是状态不是版本，
    // 且与状态列语义重叠。矩阵里状态只由格子表达，版本等细节进悬停，不再单开一列。
    // 引擎未报告 cell 时不伪造可执行操作
    expect(screen.getAllByText("inbox-one").length).toBeGreaterThan(0);
    expect(screen.queryByTestId("action-add-skill:agent-skills/inbox-one-claude-code")).toBeNull();
  });

  it("移除集合：点击先打开 ConfirmModal，取消时零 apply/零 policy:set", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    const removeBtn = await screen.findByTestId("remove-collection-agent-skills");
    await userEvent.click(removeBtn);

    const dialog = await screen.findByRole("dialog", { name: "移除来源 agent-skills" });
    // 说清会发生什么：技能链接移除、自己的目录不动、规则文件保留、启动命令不动
    expect(dialog).toHaveTextContent("你自己放进账号的同名技能目录不动");
    expect(dialog).toHaveTextContent("规则文件内容保留");
    expect(screen.getByTestId("confirm-modal-input")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("移除集合：输入集合名确认后 skills:apply 携带 --only skill:<name> --action remove，git 集合默认一并清理（--purge），无 plan、零 policy:set", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    const removeBtn = await screen.findByTestId("remove-collection-agent-skills");
    await userEvent.click(removeBtn);

    await screen.findByRole("dialog", { name: "移除来源 agent-skills" });
    expect(screen.getByTestId("remove-collection-purge")).toBeChecked();
    await userEvent.type(screen.getByTestId("confirm-modal-input"), "agent-skills");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    const applyCalls = calls.filter((call) => call.command === "skills:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "skill:agent-skills", "--action", "remove", "--purge"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("移除集合时取消勾选清理：只移除，下载的仓库留着；推荐来源之后不再推荐", async () => {
    const routes = recommendationRoutes();
    const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[]; recommendations: { name: string }[] }>;
    status.data.recommendations[0].name = "agent-skills";
    const { calls } = renderWithEngine(<SkillsPage />, routes);
    await userEvent.click(await screen.findByTestId("remove-collection-agent-skills"));
    await userEvent.click(screen.getByTestId("remove-collection-purge"));
    await userEvent.type(screen.getByTestId("confirm-modal-input"), "agent-skills");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")!.args).toEqual(["--only", "skill:agent-skills", "--action", "remove"]);
    expect(JSON.parse(localStorage.getItem("agent-manager:dismissed-recommendations") ?? "[]")).toContain("agent-skills");
  });

  it("已分配技能：软链接模式只显示可逆的停用，不暴露与其重复的删除分配", async () => {
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
                  actions: ["disable", "remove"],
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
    await expandAll();
    expect(await screen.findByTestId("action-disable-skill:agent-skills/alpha-claude-code")).toHaveAttribute("title", "claude-code：已启用，点击停用");
    expect(screen.queryByTestId("action-remove-skill:agent-skills/alpha-claude-code")).toBeNull();
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
  });

  it("集合筛选：只显示所选集合", async () => {
    const routes = skillsRoutes();
    const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>;
    status.data.elements.push(makeElement({ id: "skill:team", domain: "skills", spec: { url: "https://example.com/team.git" }, observed: { repo_cached: true, actions: ["update"] } }));
    renderWithEngine(<SkillsPage />, routes);
    expect(await screen.findByTestId("collection-card-team")).toBeInTheDocument();
    await userEvent.selectOptions(screen.getByTestId("filter-collection-select"), "agent-skills");
    expect(screen.queryByTestId("collection-card-team")).toBeNull();
    expect(screen.getByTestId("collection-card-agent-skills")).toBeInTheDocument();
  });

  it("从账号页进来只看一个账号：每行直接给这个账号的操作，切回全部账号看汇总", async () => {
    renderWithEngine(<SkillsPage initialAgent="claude-code" />, skillsRoutes());
    const agentSelect = (await screen.findByTestId("filter-agent-select")) as HTMLSelectElement;
    expect(agentSelect.value).toBe("claude-code");
    expect(await screen.findByTestId("action-disable-skill:agent-skills/alpha-claude-code")).toBeInTheDocument();
    expect(screen.queryByTestId("action-update-skill:agent-skills/alpha-codex")).toBeNull();

    await userEvent.selectOptions(agentSelect, "all");
    expect(agentSelect.value).toBe("all");
    // 链接损坏的行默认展开，codex 的状态可见
    expect(screen.getByTestId("action-update-skill:agent-skills/alpha-codex")).toBeInTheDocument();
    expect(screen.getByTestId("action-enable-skill:agent-skills/beta-codex")).toHaveAttribute("data-tone", "off");
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
                  version: "old-version",
                  available_version: "new-version",
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
    await expandAll();

    // 真有新版本才显示更新；删除分配与软链接停用重复，不进入日常界面。
    // 有新版本：「更新到新版本」在行尾「⋯」里，开关仍是停用
    await userEvent.click(within(screen.getByTestId("skill-row-skill:agent-skills/alpha")).getByRole("button", { name: /的更多操作$/ }));
    expect(screen.getByTestId("action-update-skill:agent-skills/alpha-claude-code")).toHaveTextContent("更新到新版本");
    expect(screen.getByTestId("action-disable-skill:agent-skills/alpha-claude-code")).toBeInTheDocument();
    expect(screen.queryByTestId("action-remove-skill:agent-skills/alpha-claude-code")).toBeNull();
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
    const blocked = screen.getByTestId("chip-skill:agent-skills/alpha-codex");
    expect(blocked).toHaveAttribute("title", "codex：Rust 原生引擎暂未提供此操作");
    expect(blocked).toBeDisabled();
  });
});

/** 在 skillsRoutes（agent-skills 有损坏链接与新发现）之外加一个一切正常的集合 healthy。 */
function collapseRoutes() {
  const routes = skillsRoutes();
  const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>;
  const cell = (state: string) => ({ state, assigned: true, enabled: state === "linked", actions: [state === "linked" ? "disable" : "enable"] });
  status.data.elements.push(
    makeElement({ id: "skill:healthy", domain: "skills", spec: { url: "https://example.com/healthy.git" }, observed: { repo_cached: true, actions: ["update"] } }),
    makeElement({ id: "skill:healthy/one", domain: "skills", spec: { collection: "healthy", name: "one" }, observed: { agents: { "claude-code": cell("linked"), codex: cell("linked") }, discovery_inbox: false } }),
    makeElement({ id: "skill:healthy/two", domain: "skills", spec: { collection: "healthy", name: "two" }, observed: { agents: { "claude-code": cell("linked"), codex: cell("disabled") }, discovery_inbox: false } }),
  );
  return routes;
}

describe("SkillsPage 集合展开与收起", () => {
  it("没有待处理的集合默认收起，标题行给出技能数与各账号启用数；有待处理的默认展开", async () => {
    renderWithEngine(<SkillsPage />, collapseRoutes());
    const healthy = await screen.findByTestId("collection-card-healthy");
    expect(screen.getByTestId("collection-toggle-healthy")).toHaveAttribute("aria-expanded", "false");
    expect(within(healthy).queryByTestId(/^skill-row-/)).toBeNull();
    expect(within(healthy).getByText("2 个技能")).toBeInTheDocument();
    // 各账号启用数在与表头对齐的列里
    expect(screen.getByTestId("collection-count-healthy-claude-code")).toHaveTextContent("2/2");
    expect(screen.getByTestId("collection-count-healthy-codex")).toHaveTextContent("1/2");

    expect(screen.getByTestId("collection-toggle-agent-skills")).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByTestId("collection-attention-agent-skills")).toHaveTextContent("4 个待处理");
    expect(screen.getByTestId("skill-row-skill:agent-skills/alpha")).toBeInTheDocument();
  });

  it("手动展开会记住，重新打开页面仍是展开的；只在本机存储，不调用引擎写入", async () => {
    const first = renderWithEngine(<SkillsPage />, collapseRoutes());
    await userEvent.click(await screen.findByTestId("collection-toggle-healthy"));
    expect(screen.getByTestId("skill-row-skill:healthy/one")).toBeInTheDocument();
    expect(first.calls.filter((call) => /:(apply|configure|set|seed|shared-apply)$/.test(call.command))).toEqual([]);
    first.unmount();

    renderWithEngine(<SkillsPage />, collapseRoutes());
    expect(await screen.findByTestId("skill-row-skill:healthy/one")).toBeInTheDocument();
  });

  it("全部收起 / 全部展开；搜索时命中的集合自动展开", async () => {
    renderWithEngine(<SkillsPage />, collapseRoutes());
    await userEvent.click(await screen.findByTestId("skills-toggle-all"));
    expect(screen.getByTestId("skills-toggle-all")).toHaveTextContent("全部收起");
    expect(screen.getByTestId("skill-row-skill:healthy/two")).toBeInTheDocument();
    await userEvent.click(screen.getByTestId("skills-toggle-all"));
    expect(screen.queryByTestId(/^skill-row-/)).toBeNull();

    await userEvent.type(screen.getByTestId("skills-search"), "two");
    expect(screen.getByTestId("skill-row-skill:healthy/two")).toBeInTheDocument();
  });
});

describe("改成我的版本", () => {
  it("从行尾菜单打开，确认前不写；确认后先复制登记，再把用着它的账号逐个换过去", async () => {
    const routes: Record<string, Envelope> = {
      ...skillsRoutes(),
      "skills:fork": makeEnvelope({ domain: "skills", collection: "my-skills", skill: "alpha", path: "/home/me/my-skills/alpha" }, { command: "skills.fork" }),
    };
    const { calls } = renderWithEngine(<SkillsPage />, routes);
    await expandAll();
    await clickAction("fork-skill:agent-skills/alpha");
    const dialog = await screen.findByRole("dialog", { name: "把 alpha 改成我的版本" });
    expect(within(dialog).getByTestId("fork-dest")).toHaveValue("~/my-skills");
    expect(within(dialog).getByTestId("fork-switch-claude-code")).toBeChecked();
    // codex 那条链接损坏也算在用
    expect(within(dialog).getByTestId("fork-switch-codex")).toBeChecked();
    await userEvent.click(within(dialog).getByTestId("fork-switch-codex"));
    expect(calls.some((call) => call.command === "skills:fork" || call.command === "skills:apply")).toBe(false);
    await userEvent.click(within(dialog).getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(2));
    const writes = calls.filter((call) => call.command === "skills:fork" || call.command === "skills:apply").map((call) => [call.command, ...call.args]);
    expect(writes).toEqual([
      ["skills:fork", "--only", "skill:agent-skills/alpha", "--dest", "~/my-skills", "--collection", "my-skills"],
      ["skills:apply", "--only", "skill:agent-skills/alpha", "--action", "remove", "--agent", "claude-code"],
      ["skills:apply", "--only", "skill:my-skills/alpha", "--action", "add", "--agent", "claude-code"],
    ]);
  });

  it("用户的版本标出来历；上游出了新版只提示", async () => {
    const routes = skillsRoutes();
    const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>;
    status.data.elements.push(
      makeElement({ id: "skill:my-skills", domain: "skills", spec: { source_path: "~/my-skills" }, observed: { repo_cached: true, actions: ["update"] } }),
      makeElement({
        id: "skill:my-skills/alpha",
        domain: "skills",
        spec: { collection: "my-skills", name: "alpha" },
        observed: {
          agents: { "claude-code": { state: "linked", assigned: true, enabled: true, actions: ["disable"] } },
          discovery_inbox: false,
          forked_from: { collection: "agent-skills", version: "v1", upstream_version: "v2", upstream_changed: true },
        },
      }),
    );
    renderWithEngine(<SkillsPage />, routes);
    await expandAll();
    expect(await screen.findByTestId("forked-from-skill:my-skills/alpha")).toHaveTextContent("基于 agent-skills");
    expect(screen.getByTestId("upstream-changed-skill:my-skills/alpha")).toHaveTextContent("原版有更新");
  });
});

describe("技能集合的分享链接", () => {
  it("从分享链接导入：打开预填好的新增集合表单，名称可改，保存前不写", async () => {
    const { calls } = renderWithEngine(
      <SkillsPage importPayload={{ kind: "skills", name: "team-skills", definition: { url: "https://github.com/example/team.git", skill_paths: ["skills"], rules_path: null } }} />,
      skillsRoutes(),
    );
    const dialog = await screen.findByRole("dialog", { name: "添加来源" });
    expect(within(dialog).getByTestId("collection-name-input")).toHaveValue("team-skills");
    expect(within(dialog).getByTestId("collection-url-input")).toHaveValue("https://github.com/example/team.git");
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });
});

describe("SkillsPage 空态引导与发现失败呈现", () => {
  it("集合还没下载：标题行标「尚未下载」，就地给「更新」", async () => {
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
    const card = await screen.findByTestId("collection-card-agent-skills");
    expect(card).toHaveTextContent("尚未下载");
    expect(card).toHaveTextContent("点「更新」取回后技能会出现在这里");
    expect(within(card).getByTestId("update-collection-agent-skills")).toBeInTheDocument();
  });

  it("集合已取回但没有技能：说「没有找到技能」，发现失败以警告呈现", async () => {
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
    const card = await screen.findByTestId("collection-card-broken-skills");
    expect(card).toHaveTextContent("没有找到技能");
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
                  // 未分配且无链接：底层 add 在界面统一表达为首次启用
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

  it("extra-link：动作为 纳入管理/取消链接，行上标「有外部链接」", async () => {
    renderWithEngine(<SkillsPage />, extraLinkRoutes());
    await expandAll();

    const addBtn = await screen.findByTestId("action-add-skill:agent-skills/alpha-codex");
    expect(addBtn).toHaveAttribute("title", "codex：技能目录里有一条本工具以外的链接，点击纳入管理");
    expect(addBtn).toHaveAttribute("data-tone", "warn");
    await userEvent.click(within(screen.getByTestId("skill-row-skill:agent-skills/alpha")).getByRole("button", { name: /的更多操作$/ }));
    expect(screen.getByTestId("action-remove-skill:agent-skills/alpha-codex")).toHaveTextContent("取消这条外部链接 · codex");

    const skillRow = screen.getByTestId("skill-row-skill:agent-skills/alpha");
    expect(within(skillRow).getByText("有外部链接")).toBeInTheDocument();
  });

  it("非受管外部链接：纳入管理先弹确认，取消零 apply，确认后 apply 携带 --allow-takeover", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, extraLinkRoutes());
    await expandAll();
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
    await expandAll();

    await clickAction("action-remove-skill:agent-skills/alpha-codex");
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
    await expandAll();

    await clickAction("action-remove-skill:agent-skills/beta-codex");
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

  it("not-assigned 行：底层 add 对用户显示为「启用」，不重复显示未分配状态", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, extraLinkRoutes());
    await expandAll();

    const addBtn = await screen.findByTestId("action-add-skill:agent-skills/gamma-codex");
    expect(addBtn).toHaveAttribute("title", "codex：未启用，点击启用");
    // 普通状态由可执行动作表达，不再重复放「未分配」徽标。
    const skillRow = screen.getByTestId("skill-row-skill:agent-skills/gamma");
    expect(within(skillRow).queryByTitle(/not-assigned/)).toBeNull();
    expect(within(skillRow).queryByText("未分配")).toBeNull();
    expect(screen.queryByTestId("action-remove-skill:agent-skills/gamma-codex")).toBeNull();

    await userEvent.click(addBtn);
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(1));
    expect(screen.queryByRole("dialog")).toBeNull();
    const args = calls.filter((call) => call.command === "skills:apply")[0].args;
    expect(args).toEqual(["--only", "skill:agent-skills/gamma", "--action", "add", "--agent", "codex"]);
    expect(args).not.toContain("--allow-takeover");
  });
});

/** 编辑 agent-skills 时按给定 skill_paths 打开对话框。 */
function editWith(skillPaths: string[] | null) {
  const routes = skillsRoutes();
  const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>;
  (status.data.elements[0].spec as { skill_paths: string[] | null }).skill_paths = skillPaths;
  return routes;
}

function savedSkillPaths(calls: { command: string; args: string[] }[]) {
  const writes = calls.filter((call) => call.command === "policy:set");
  const write = writes[writes.length - 1];
  return JSON.parse(write.args[write.args.indexOf("--value") + 1]).skill_paths;
}

describe("集合的「用哪些技能」改成勾选", () => {
  it("勾选反映实际在用的技能：逐个列出的勾上，其余不勾", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    expect(screen.getByTestId("collection-path-alpha")).toBeChecked();
    expect(screen.getByTestId("collection-path-beta")).toBeChecked();
    expect(screen.getByTestId("collection-path-inbox-one")).not.toBeChecked();
    expect(screen.getByTestId("collection-paths-count")).toHaveTextContent("已选 2 / 4");
  });

  it("配置是装技能的目录（skills）：全部勾上并自动包含，不把目录当成找不到的技能；原样保存不改配置", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, editWith(["skills"]));
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    for (const name of ["alpha", "beta", "inbox-one", "inbox-two"]) {
      expect(screen.getByTestId(`collection-path-${name}`)).toBeChecked();
    }
    expect(screen.getByTestId("collection-auto-include")).toBeChecked();
    expect(screen.queryByTestId("collection-paths-missing")).toBeNull();
    await userEvent.click(screen.getByTestId("collection-form-submit"));
    expect(savedSkillPaths(calls)).toEqual(["skills"]);
  });

  it("从目录模式取消一个：改存剩下技能的实际路径", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, editWith(["skills"]));
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    await userEvent.click(screen.getByTestId("collection-path-beta"));
    expect(screen.queryByTestId("collection-auto-include")).toBeNull();
    await userEvent.click(screen.getByTestId("collection-form-submit"));
    expect(savedSkillPaths(calls)).toEqual(["skills/alpha", "skills/inbox-one", "skills/inbox-two"]);
  });

  it("加勾一个：原有写法保留，新勾的用实际路径", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    await userEvent.click(screen.getByTestId("collection-path-inbox-one"));
    await userEvent.click(screen.getByTestId("collection-form-submit"));
    expect(savedSkillPaths(calls)).toEqual(["alpha", "beta", "skills/inbox-one"]);
  });

  it("配置里有来源里找不到的项：单独提示，默认保留，取消勾选才去掉", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, editWith(["alpha", "gone"]));
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    expect(screen.getByTestId("collection-paths-missing")).toHaveTextContent("gone");
    await userEvent.click(screen.getByTestId("collection-path-missing-gone"));
    await userEvent.click(screen.getByTestId("collection-form-submit"));
    expect(savedSkillPaths(calls)).toEqual(["alpha"]);
  });

  it("一个都没勾：拦下来并说清怎么办，不写库", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    await userEvent.click(screen.getByTestId("collection-paths-clear"));
    await userEvent.click(screen.getByTestId("collection-form-submit"));

    expect(await screen.findByText(/至少勾一个技能/)).toBeInTheDocument();
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("仓库还没取下来时说清为什么列不出来，并把现有配置放在文本框里可改", async () => {
    const routes = skillsRoutes();
    const status = JSON.parse(JSON.stringify(routes["skills:status"]));
    status.data.elements[0].observed.repo_cached = false;
    status.data.elements[0].observed.discovered = [];
    status.data.elements[0].observed.available = [];
    routes["skills:status"] = status;
    const { calls } = renderWithEngine(<SkillsPage />, routes);
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    expect(screen.getByTestId("collection-paths-empty")).toHaveTextContent("仓库还没取到本地");
    expect(screen.getByTestId("collection-paths-input")).toHaveValue("alpha\nbeta");
    await userEvent.click(screen.getByTestId("collection-form-submit"));
    expect(savedSkillPaths(calls)).toEqual(["alpha", "beta"]);
  });
});

const RECOMMENDED_PATHS = ["skills"];

function recommendationRoutes(opts: { added?: boolean; collections?: boolean } = {}) {
  const base = skillsRoutes();
  const elements = opts.collections === false ? [] : (base["skills:status"].data as { elements: unknown[] }).elements;
  return {
    ...base,
    "skills:status": makeEnvelope(
      {
        domain: "skills",
        elements: elements as ReturnType<typeof makeElement>[],
        recommendations: [
          {
            name: "agent-skills-rec",
            description: "作者维护的通用工作技能",
            url: "https://github.com/ly87ing/agent-skills.git",
            skill_paths: RECOMMENDED_PATHS,
            added: opts.added ?? false,
            definition: { url: "https://github.com/ly87ing/agent-skills.git", skill_paths: RECOMMENDED_PATHS },
          },
        ],
        installed: [
          { agent: "claude-code", name: "alpha", link_path: "~/.claude/skills/alpha", target: "~/data/skill-versions/x/content", origin: "managed", collection: "agent-skills" },
          { agent: "claude-code", name: "vendor-lint", link_path: "~/.claude/skills/vendor-lint", target: "~/.vendor/skills/vendor-lint", origin: "external-link", collection: null },
          { agent: "claude-code", name: "synced", link_path: "~/.claude/skills/synced", target: null, origin: "native-dir", collection: null },
          { agent: "codex", name: "ghost", link_path: "~/.codex/skills/ghost", target: "~/gone/ghost", origin: "dead-link", collection: null },
        ],
      },
      { command: "skills.status" },
    ),
  };
}

describe("首次使用：推荐启用 agent-skills，可以不用", () => {
  it("没添加过就在顶部推荐；「不用了」之后不再出现，也不写任何东西", async () => {
    const first = renderWithEngine(<SkillsPage />, recommendationRoutes());
    const banner = await screen.findByTestId("recommend-banner-agent-skills-rec");
    expect(banner).toHaveTextContent("推荐启用 agent-skills-rec");
    await userEvent.click(within(banner).getByTestId("recommend-dismiss-agent-skills-rec"));
    expect(screen.queryByTestId("recommend-banner-agent-skills-rec")).toBeNull();
    expect(first.calls.some((call) => /:(apply|set|seed|configure)$/.test(call.command))).toBe(false);
    first.unmount();

    renderWithEngine(<SkillsPage />, recommendationRoutes());
    await screen.findByTestId("add-collection-button");
    expect(screen.queryByTestId("recommend-banner-agent-skills-rec")).toBeNull();
  });

  it("已添加过就不推荐", async () => {
    renderWithEngine(<SkillsPage />, recommendationRoutes({ added: true }));
    await screen.findByTestId("add-collection-button");
    expect(screen.queryByTestId("recommend-banner-agent-skills-rec")).toBeNull();
  });

  it("启用：先确认，确认后依次登记、下载、把技能链接到能加的账号；已有同名内容的账号跳过", async () => {
    const base = recommendationRoutes({ collections: false });
    let saved = false;
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = {
      ...base,
      "policy:set": async () => {
        saved = true;
        return (base["policy:set"] as Envelope);
      },
      "skills:status": async () => {
        const data = JSON.parse(JSON.stringify((base["skills:status"] as Envelope).data));
        if (saved) {
          data.recommendations[0].added = true;
          data.elements = [
            makeElement({ id: "skill:agent-skills-rec", domain: "skills", spec: { url: "https://github.com/ly87ing/agent-skills.git" }, observed: { repo_cached: true, actions: ["update"] } }),
            makeElement({
              id: "skill:agent-skills-rec/verification",
              domain: "skills",
              spec: { collection: "agent-skills-rec", name: "verification" },
              observed: {
                discovery_inbox: true,
                agents: {
                  "claude-code": { state: "not-assigned", assigned: false, enabled: false, actions: ["add"] },
                  // 账号里已有同名的外部内容：引擎不给 add，不覆盖
                  codex: { state: "extra-link", assigned: false, enabled: false, actions: [] },
                },
              },
            }),
          ];
        }
        return makeEnvelope(data, { command: "skills.status" });
      },
    };
    const { calls } = renderWithEngine(<SkillsPage />, routes);
    await userEvent.click(await screen.findByTestId("recommend-enable-agent-skills-rec"));
    const dialog = await screen.findByRole("dialog", { name: "启用 agent-skills-rec" });
    expect(within(dialog).getByTestId("recommend-enable-steps")).toHaveTextContent("不覆盖");
    expect(within(dialog).getByTestId("recommend-enable-steps")).toHaveTextContent("规则、MCP、启动命令都不动");
    expect(calls.some((call) => call.command === "policy:set" || call.command === "skills:apply")).toBe(false);
    await userEvent.click(within(dialog).getByRole("button", { name: "启用" }));

    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(2));
    const writes = calls.filter((call) => call.command === "policy:set" || call.command === "skills:apply");
    expect(writes[0].command).toBe("policy:set");
    expect(writes[1].args).toEqual(["--only", "skill:agent-skills-rec", "--action", "update"]);
    expect(writes[2].args).toEqual(["--only", "skill:agent-skills-rec/verification", "--action", "add", "--agent", "claude-code"]);
    expect(calls.some((call) => call.command.startsWith("rules:") || call.command.startsWith("mcp:apply") || call.command.startsWith("aliases:apply"))).toBe(false);
  });
});

describe("SkillsPage 推荐来源与当前安装", () => {
  it("当前安装：按助手列出实际条目并分类，只展示不给操作", async () => {
    renderWithEngine(<SkillsPage />, recommendationRoutes());
    const section = await screen.findByTestId("skills-installed");
    const claude = within(section).getByTestId("skills-installed-claude-code");
    expect(within(claude).getByTestId("skills-installed-claude-code-managed")).toHaveTextContent("本工具链接 1");
    expect(within(claude).getByTestId("skills-installed-claude-code-external")).toHaveTextContent("外部链接 1");
    expect(within(claude).getByTestId("skills-installed-claude-code-native")).toHaveTextContent("真实目录 1");
    expect(within(claude).getByTestId("skills-installed-entry-claude-code-alpha")).toHaveTextContent("本工具链接");
    expect(within(claude).getByTestId("skills-installed-entry-claude-code-alpha")).toHaveTextContent("来自集合 agent-skills");
    expect(within(claude).getByTestId("skills-installed-entry-claude-code-vendor-lint")).toHaveTextContent("→ ~/.vendor/skills/vendor-lint");
    expect(within(claude).getByTestId("skills-installed-entry-claude-code-synced")).toHaveTextContent("真实目录");
    const codex = within(section).getByTestId("skills-installed-codex");
    expect(within(codex).getByTestId("skills-installed-codex-dead")).toHaveTextContent("失效链接 1");
    // 不把外部条目当作待处理项：整个区块没有任何按钮
    expect(within(section).queryAllByRole("button")).toHaveLength(0);
  });

  it("还没有任何集合时给出空态，推荐来源仍可启用", async () => {
    renderWithEngine(<SkillsPage />, recommendationRoutes({ collections: false }));
    expect(await screen.findByTestId("skills-collections-empty")).toHaveTextContent("还没有技能来源");
    expect(screen.getByTestId("recommend-enable-agent-skills-rec")).toBeInTheDocument();
  });
});

/** 来源里已经删掉的技能：两个助手上都还有分配，一个链接已丢、一个还指着旧快照。 */
function droppedRoutes() {
  const routes = skillsRoutes();
  const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>;
  status.data.elements.push(
    makeElement({
      id: "skill:agent-skills/gone",
      domain: "skills",
      spec: { collection: "agent-skills", name: "gone", source_path: null },
      policy: { enabled: true, agents: { "claude-code": true, codex: true } },
      observed: {
        agents: {
          "claude-code": {
            state: "missing-link",
            link_path: "/home/.claude/skills/gone",
            target_dir_exists: true,
            assigned: true,
            enabled: true,
            actions: ["remove"],
            blocked_reason: "来源里已没有这个技能，可以移除",
            source_missing: true,
          },
          codex: {
            state: "linked",
            link_path: "/home/.codex/skills/gone",
            target_dir_exists: true,
            assigned: true,
            enabled: true,
            managed: true,
            actions: ["remove"],
            blocked_reason: "来源里已没有这个技能，可以移除",
            source_missing: true,
          },
        },
        discovery_inbox: false,
      },
      state: "missing",
    }),
  );
  return routes;
}

describe("SkillsPage 来源里已删除的技能", () => {
  it("从总览定位进来：直接在全部技能里搜到它，只给「移除」，确认后按助手移除", async () => {
    const { calls } = renderWithEngine(<SkillsPage initialSearch="gone" />, droppedRoutes());
    const search = await screen.findByTestId("skills-search");
    expect(search).toHaveValue("gone");
    await screen.findByTestId("action-remove-skill:agent-skills/gone-claude-code");
    // 搜索只留下这一行
    expect(screen.queryByTestId("action-disable-skill:agent-skills/beta-claude-code")).toBeNull();
    // 停用做不成，不再给
    expect(screen.queryByTestId("action-disable-skill:agent-skills/gone-claude-code")).toBeNull();
    expect(within(screen.getByTestId("skill-row-skill:agent-skills/gone")).getByText("来源里已删除")).toBeInTheDocument();
    expect(screen.getByTestId("action-remove-skill:agent-skills/gone-claude-code")).toHaveAttribute("title", "claude-code：来源里已经没有这个技能，点击移除这条分配");

    await userEvent.click(screen.getByTestId("action-remove-skill:agent-skills/gone-claude-code"));
    const dialog = await screen.findByRole("dialog", { name: "移除 gone @ claude-code" });
    expect(dialog).toHaveTextContent("入口已经不在了，不改动任何文件");
    expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(0);
    await userEvent.click(within(dialog).getByRole("button", { name: "确认移除" }));

    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(1));
    expect(calls.find((call) => call.command === "skills:apply")!.args).toEqual([
      "--only",
      "skill:agent-skills/gone",
      "--action",
      "remove",
      "--agent",
      "claude-code",
    ]);
  });

  it("链接还在的那个助手：确认里说明会删掉本工具建的链接，取消则不写", async () => {
    const { calls } = renderWithEngine(<SkillsPage initialSearch="gone" />, droppedRoutes());
    await userEvent.click(await screen.findByTestId("action-remove-skill:agent-skills/gone-codex"));
    const dialog = await screen.findByRole("dialog", { name: "移除 gone @ codex" });
    expect(dialog).toHaveTextContent("并删除本工具建的入口链接");
    expect(dialog).toHaveTextContent("不影响其他账号");
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(0);
  });
});

function otherSourceRoutes() {
  const base = skillsRoutes();
  const status = base["skills:status"].data as { elements: ReturnType<typeof makeElement>[] };
  return {
    ...base,
    "skills:status": makeEnvelope(
      {
        domain: "skills",
        elements: status.elements,
        unmanaged: {
          skills: [
            {
              name: "archify",
              locations: [
                { agent: "claude-code", shared: null, path: "~/.claude/skills/archify", target: null, origin: "native-dir" },
                { agent: null, shared: "codex-shared", path: "~/.agents/skills/archify", target: null, origin: "native-dir" },
              ],
              source: { repository: "https://github.com/tt-a1i/archify.git", path: "archify", version: "3.0.1", installer: "npx-skills", evidence: ["skill-lock", "skill-release"] },
              adoptable: true,
              blocked_reason: null,
            },
            {
              name: "synced",
              locations: [{ agent: "claude-code", shared: null, path: "~/.claude/skills/synced", target: null, origin: "native-dir" }],
              source: null,
              adoptable: false,
              blocked_reason: "claude-synced",
            },
          ],
          shared_dirs: [{ id: "codex-shared", kind: "codex", path: "~/.agents/skills", exists: true }],
          duplicates: [{ name: "xiaoyu-bug-loop", agent: "codex", paths: ["~/.codex/skills/xiaoyu-bug-loop", "~/.agents/skills/xiaoyu-bug-loop"] }],
        },
      },
      { command: "skills.status" },
    ),
  };
}

describe("SkillsPage 其他来源", () => {
  it("列出没管理的技能、识别出的来源、Codex 共用与重名，只显示不写", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, otherSourceRoutes());
    const block = await screen.findByTestId("skills-other-sources");
    expect(block).toHaveTextContent("其他来源 · 2");
    expect(within(block).getByTestId("other-source-archify")).toHaveTextContent("tt-a1i/archify · v3.0.1 · npx skills 安装");
    expect(within(block).getByTestId("other-shared-archify")).toHaveTextContent("Codex 共用");
    expect(within(block).getByTestId("other-present-archify-claude-code")).toBeInTheDocument();
    expect(within(block).queryByTestId("other-present-archify-codex")).toBeNull();
    expect(within(block).getByTestId("other-source-synced")).toHaveTextContent("来源不明");
    expect(within(block).getByTestId("other-blocked-synced")).toHaveTextContent("Claude 自己同步的");
    expect(within(block).getByTestId("duplicate-xiaoyu-bug-loop-codex")).toHaveTextContent("Codex 会加载两份");
    expect(within(block).queryAllByRole("button")).toHaveLength(0);
    expect(calls.every((call) => call.command.endsWith(":status") || call.command === "policy:get")).toBe(true);
  });

  it("搜索与账号筛选同样作用于其他来源", async () => {
    renderWithEngine(<SkillsPage />, otherSourceRoutes());
    await screen.findByTestId("skills-other-sources");
    await userEvent.type(screen.getByTestId("skills-search"), "archify");
    const block = screen.getByTestId("skills-other-sources");
    expect(within(block).getByTestId("other-skill-archify")).toBeInTheDocument();
    expect(within(block).queryByTestId("other-skill-synced")).toBeNull();
    expect(within(block).queryByTestId("duplicate-xiaoyu-bug-loop-codex")).toBeNull();
  });
});
