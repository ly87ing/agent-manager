import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import SourcesPage from "../pages/SourcesPage";
import type { Envelope } from "../lib/contract";
import { makeApplyResult, makeElement, makeEnvelope, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within, chooseOption } from "./render-with-engine";

/** 来源行的次要操作在「⋯」里：先打开这一行的菜单再取菜单项。 */
async function menuItem(testId: string) {
  const suggest = testId.startsWith("source-suggest-");
  const name = testId.replace(/^source-(versions|share|edit|toggle|remove|suggest-add|suggest-dismiss)-/, "");
  const row = await screen.findByTestId(suggest ? `source-suggest-${name}` : `source-row-${name}`);
  await userEvent.click(within(row).getByRole("button", { name: `${name} 的更多操作` }));
  return screen.getByTestId(testId);
}

function skillsRoutes() {
  return {
    "rules:status": makeEnvelope({ domain: "rules", elements: [], source_rules: [] }, { command: "rules.status" }),
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

describe("来源页：添加、编辑、更新、移除", () => {

  it("编辑集合：以 sqlite 完整定义整条替换保存，全选并自动包含写成技能所在目录，未展示字段不丢，不预览/同步", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, skillsRoutes());
    await userEvent.click(await menuItem("source-edit-agent-skills"));
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
    const { calls } = renderWithEngine(<SourcesPage />, legacyRoutes);
    await userEvent.click(await menuItem("source-edit-agent-skills"));
    await userEvent.click(screen.getByTestId("collection-form-submit"));

    await screen.findByText(/已停止保存/);
    expect(calls.filter((call) => call.command === "db:migrate")).toHaveLength(1);
    expect(calls.filter((call) => call.command === "policy:set")).toHaveLength(0);
  });

  it.each([
    "https://example.com/YOUR_REPO.git", "http://example.com/YOUR_REPO.git",
    "ssh://YOUR_USER@example.com:2222/YOUR_REPO.git", "git://example.com/YOUR_REPO.git",
    "YOUR_USER@example.com:YOUR_REPO.git", "YOUR_HOST:YOUR_REPO.git", "file:///YOUR_REPO",
  ])("添加来源保留用户选择的协议：%s", async (url) => {
    const { calls } = renderWithEngine(<SourcesPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("source-add-collection"));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByTestId("collection-url-input")).toHaveAttribute("placeholder", "https://github.com/example/skills.git");
    fireEvent.change(within(dialog).getByTestId("collection-name-input"), { target: { value: "my-skills" } });
    fireEvent.change(within(dialog).getByTestId("collection-url-input"), { target: { value: url } });
    await userEvent.click(within(dialog).getByTestId("collection-form-submit"));
    const write = calls.find((call) => call.command === "policy:set")!;
    expect(JSON.parse(write.args[write.args.indexOf("--value") + 1])).toMatchObject({ url });
  });

  it("新增集合表单点取消：零写入且绝不迁移", async () => {
    const cancelRoutes = { ...skillsRoutes(), "db:migrate": makeEnvelope({ migrated: true }, { command: "db.migrate" }) };
    const { calls } = renderWithEngine(<SourcesPage />, cancelRoutes);
    await userEvent.click(await screen.findByTestId("source-add-collection"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("collection-name-input"), { target: { value: "my-skills" } });
    fireEvent.change(within(dialog).getByTestId("collection-url-input"), { target: { value: "https://github.com/example/my-skills.git" } });
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls.filter((call) => call.command === "policy:set")).toHaveLength(0);
    expect(calls.filter((call) => call.command === "db:migrate")).toHaveLength(0);
  });

  it("本机位置可以直接选某个技能的 SKILL.md：存成它所在的技能文件夹；规则位置可改指", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("source-add-collection"));
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
    const { calls } = renderWithEngine(<SourcesPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("source-add-collection"));
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
    const { calls } = renderWithEngine(<SourcesPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("source-add-collection"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("collection-name-input"), { target: { value: "vendor-tool" } });
    await userEvent.click(within(dialog).getByTestId("collection-source-local"));
    fireEvent.change(within(dialog).getByTestId("collection-source-path-input"), { target: { value: "relative/skills" } });
    await userEvent.click(within(dialog).getByTestId("collection-form-submit"));

    expect(await within(dialog).findByText(/绝对路径或以 ~\/ 开头/)).toBeInTheDocument();
    expect(calls.filter((call) => call.command === "policy:set")).toHaveLength(0);
  });

  it("更新集合：有新版本时行上常驻「更新」，先预览要换的内容，确认后才 skills:apply（限定所选集合、带 run_id、不取 plan）", async () => {
    const routes = {
      ...skillsRoutes(),
      "skills:preview": makeEnvelope({
        from: { commit: "1".repeat(40), kind: "branch", name: "main", label: "main" },
        to: { commit: "2".repeat(40), kind: "branch", name: "main", label: "main" },
        downloaded: true, same: false, skills: [
          { name: "alpha", path: "skills/alpha", files: [{ status: "modified", file: "SKILL.md" }], accounts: ["claude-code"] },
          { name: "gamma", path: "skills/gamma", files: [{ status: "added", file: "SKILL.md" }, { status: "added", file: "scripts/run.sh" }], accounts: [] },
          { name: "omega", path: "skills/omega", files: [{ status: "deleted", file: "SKILL.md" }], accounts: [] },
        ], rules: [], other: [],
      }),
      "skills:versions": makeEnvelope({ collection: "agent-skills", track: { mode: "auto" }, auto_resolved: "default", latest: { commit: "2".repeat(40), kind: "branch", name: "main", label: "main" }, current: { commit: "1".repeat(40), kind: "branch", name: "main", label: "main" }, default_branch: "main", tags: [], tags_total: 0, branches: [{ name: "main", commit: "2".repeat(40) }], branches_total: 1 }),
    };
    const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>;
    status.data.elements[0].freshness = { ...status.data.elements[0].freshness, state: "behind" };
    const { calls } = renderWithEngine(<SourcesPage />, routes);
    await userEvent.click(await screen.findByTestId("source-update-agent-skills"));
    const dialog = await screen.findByRole("dialog", { name: "agent-skills 的版本" });
    await within(dialog).findByTestId("preview-skill-alpha");
    // 整个新加或被删掉的技能单独标出来
    expect(within(dialog).queryByTestId("preview-skill-new-alpha")).toBeNull();
    expect(within(dialog).getByTestId("preview-skill-new-gamma")).toHaveTextContent("新技能");
    expect(within(dialog).getByTestId("preview-skill-gamma")).toHaveTextContent("由你决定给哪些助手");
    expect(within(dialog).getByTestId("preview-skill-gone-omega")).toBeInTheDocument();
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
    await userEvent.click(within(dialog).getByTestId("versions-preview-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));

    const applyCalls = calls.filter((call) => call.command === "skills:apply");
    expect(applyCalls[0].args).toEqual(["--only", "skill:agent-skills", "--action", "update"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("全部更新：先逐个取回集合，再把在用、已有新版本的技能换到新版本；新发现的不加，停用的不动", async () => {
    const routes: Record<string, Envelope> = {
      ...skillsRoutes(),
      "system:refresh-upstream": makeEnvelope({}, { command: "system.refresh-upstream" }),
    };
    const status = routes["skills:status"] as Envelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>;
    const alpha = status.data.elements.find((element) => element.id === "skill:agent-skills/alpha")!;
    Object.assign((alpha.observed as { agents: Record<string, Record<string, unknown>> }).agents["claude-code"], { version: "v1", available_version: "v2", actions: ["update", "disable", "remove"] });
    const { calls } = renderWithEngine(<SourcesPage />, routes);
    await userEvent.click(await screen.findByTestId("sources-update-all"));
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(2));
    const applies = calls.filter((call) => call.command === "skills:apply").map((call) => call.args);
    expect(applies[0]).toEqual(["--only", "skill:agent-skills", "--action", "update"]);
    expect(applies[1]).toEqual(["--only", "skill:agent-skills/alpha", "--action", "update", "--agent", "claude-code"]);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it.each(["envelope", "bridge"])("全部更新：更新后状态读取失败（%s）明确报错，不报告技能已更新", async (failure) => {
    const r = skillsRoutes();
    const { calls } = renderWithEngine(<SourcesPage />, {
      ...r,
      "skills:status": async (args) => {
        if (args.includes("--collections-only")) return r["skills:status"];
        if (failure === "bridge") throw new Error("无法读取新状态");
        return makeEnvelope({}, { ok: false, errors: [{ code: "STATUS_FAILED", element: null, message: "无法读取新状态" }] });
      },
      "system:refresh-upstream": makeEnvelope({}),
    });
    await userEvent.click(await screen.findByTestId("sources-update-all"));
    expect(await screen.findByText(/无法读取新状态/)).toBeVisible();
    expect(screen.queryByText(/各账号的技能都已是最新|处技能换到新版本/)).toBeNull();
    expect(calls.filter(call => call.command === "skills:apply")).toHaveLength(1);
    await waitFor(() => expect(screen.getByTestId("sources-update-all")).toBeEnabled());
  });

  it("全部更新：部署完成之前不报告成功，部分失败显示实际结果并保留失败记录", async () => {
    const r = skillsRoutes();
    const status = r["skills:status"] as Envelope<{ elements: ReturnType<typeof makeElement>[] }>;
    const alpha = status.data.elements.find(element => element.id === "skill:agent-skills/alpha")!;
    Object.assign((alpha.observed as { agents: Record<string, Record<string, unknown>> }).agents["claude-code"], { version: "v1", available_version: "v2", actions: ["update", "disable", "remove"] });
    let release!: (value: Envelope) => void;
    const pending = new Promise<Envelope>(resolve => { release = resolve; });
    const { calls } = renderWithEngine(<SourcesPage />, {
      ...r,
      "system:refresh-upstream": makeEnvelope({}),
      "skills:apply": async args => args.includes("--agent") ? pending : makeEnvelope(makeApplyResult({ domain: "skills" })),
    });
    await userEvent.click(await screen.findByTestId("sources-update-all"));
    await waitFor(() => expect(calls.filter(call => call.command === "skills:apply")).toHaveLength(2));
    expect(screen.queryByText(/技能部署：|处技能换到新版本|各账号的技能都已是最新/)).toBeNull();
    expect(screen.getByTestId("sources-update-all")).toBeDisabled();
    release(makeEnvelope({}, { ok: false, errors: [{ code: "DEPLOY_FAILED", element: null, message: "部署被拒绝" }] }));
    expect(await screen.findByText("来源更新：成功 1，失败 0；技能部署：成功 0，失败 1（详见任务记录）")).toBeVisible();
    expect(screen.getByText(/部署被拒绝/)).toBeVisible();
  });

  it("移除集合：点击先打开 ConfirmModal，取消时零 apply/零 policy:set", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, skillsRoutes());
    const removeBtn = await menuItem("source-remove-agent-skills");
    await userEvent.click(removeBtn);

    const dialog = await screen.findByRole("dialog", { name: "移除来源 agent-skills" });
    // 说清会发生什么：技能链接移除、自己的目录不动、规则文件保留、启动命令不动
    expect(dialog).toHaveTextContent("你自己放进助手的同名技能目录不动");
    expect(dialog).toHaveTextContent("规则文件内容保留");
    expect(screen.getByTestId("confirm-modal-input")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("移除集合：输入集合名确认后 skills:apply 携带 --only skill:<name> --action remove，git 集合默认一并清理（--purge），无 plan、零 policy:set", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, skillsRoutes());
    const removeBtn = await menuItem("source-remove-agent-skills");
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
    const { calls } = renderWithEngine(<SourcesPage />, routes);
    await userEvent.click(await menuItem("source-remove-agent-skills"));
    await userEvent.click(screen.getByTestId("remove-collection-purge"));
    await userEvent.type(screen.getByTestId("confirm-modal-input"), "agent-skills");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")!.args).toEqual(["--only", "skill:agent-skills", "--action", "remove"]);
    expect(JSON.parse(localStorage.getItem("agent-manager:dismissed-recommendations") ?? "[]")).toContain("agent-skills");
  });
});

describe("来源的分享链接", () => {
  it("从分享链接导入：打开预填好的新增集合表单，名称可改，保存前不写", async () => {
    const { calls } = renderWithEngine(
      <SourcesPage importPayload={{ kind: "skills", name: "team-skills", definition: { url: "https://github.com/example/team.git", skill_paths: ["skills"], rules_path: null } }} />,
      skillsRoutes(),
    );
    const dialog = await screen.findByRole("dialog", { name: "添加来源" });
    expect(within(dialog).getByTestId("collection-name-input")).toHaveValue("team-skills");
    expect(within(dialog).getByTestId("collection-url-input")).toHaveValue("https://github.com/example/team.git");
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
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
    renderWithEngine(<SourcesPage />, skillsRoutes());
    await userEvent.click(await menuItem("source-edit-agent-skills"));
    expect(screen.getByTestId("collection-path-alpha")).toBeChecked();
    expect(screen.getByTestId("collection-path-beta")).toBeChecked();
    expect(screen.getByTestId("collection-path-inbox-one")).not.toBeChecked();
    expect(screen.getByTestId("collection-paths-count")).toHaveTextContent("已选 2 / 4");
  });

  it("配置是装技能的目录（skills）：全部勾上并自动包含，不把目录当成找不到的技能；原样保存不改配置", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, editWith(["skills"]));
    await userEvent.click(await menuItem("source-edit-agent-skills"));
    for (const name of ["alpha", "beta", "inbox-one", "inbox-two"]) {
      expect(screen.getByTestId(`collection-path-${name}`)).toBeChecked();
    }
    expect(screen.getByTestId("collection-auto-include")).toBeChecked();
    expect(screen.queryByTestId("collection-paths-missing")).toBeNull();
    await userEvent.click(screen.getByTestId("collection-form-submit"));
    expect(savedSkillPaths(calls)).toEqual(["skills"]);
  });

  it("从目录模式取消一个：改存剩下技能的实际路径", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, editWith(["skills"]));
    await userEvent.click(await menuItem("source-edit-agent-skills"));
    await userEvent.click(screen.getByTestId("collection-path-beta"));
    expect(screen.queryByTestId("collection-auto-include")).toBeNull();
    await userEvent.click(screen.getByTestId("collection-form-submit"));
    expect(savedSkillPaths(calls)).toEqual(["skills/alpha", "skills/inbox-one", "skills/inbox-two"]);
  });

  it("加勾一个：原有写法保留，新勾的用实际路径", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, skillsRoutes());
    await userEvent.click(await menuItem("source-edit-agent-skills"));
    await userEvent.click(screen.getByTestId("collection-path-inbox-one"));
    await userEvent.click(screen.getByTestId("collection-form-submit"));
    expect(savedSkillPaths(calls)).toEqual(["alpha", "beta", "skills/inbox-one"]);
  });

  it("配置里有来源里找不到的项：单独提示，默认保留，取消勾选才去掉", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, editWith(["alpha", "gone"]));
    await userEvent.click(await menuItem("source-edit-agent-skills"));
    expect(screen.getByTestId("collection-paths-missing")).toHaveTextContent("gone");
    await userEvent.click(screen.getByTestId("collection-path-missing-gone"));
    await userEvent.click(screen.getByTestId("collection-form-submit"));
    expect(savedSkillPaths(calls)).toEqual(["alpha"]);
  });

  it("一个都没勾：拦下来并说清怎么办，不写库", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, skillsRoutes());
    await userEvent.click(await menuItem("source-edit-agent-skills"));
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
    const { calls } = renderWithEngine(<SourcesPage />, routes);
    await userEvent.click(await menuItem("source-edit-agent-skills"));
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

describe("optional recommended sources", () => {

  it("dismissal hides the recommendation without engine writes", async () => {
    const first = renderWithEngine(<SourcesPage />, recommendationRoutes());
    const banner = await screen.findByTestId("source-suggest-agent-skills-rec");
    expect(banner).toHaveTextContent("agent-skills-rec推荐");
    await userEvent.click(await menuItem("source-suggest-dismiss-agent-skills-rec"));
    expect(screen.queryByTestId("source-suggest-agent-skills-rec")).toBeNull();
    expect(first.calls.some((call) => /:(apply|set|seed|configure)$/.test(call.command))).toBe(false);
    first.unmount();

    renderWithEngine(<SourcesPage />, recommendationRoutes());
    await screen.findByTestId("source-add-collection");
    expect(screen.queryByTestId("source-suggest-agent-skills-rec")).toBeNull();
  });

  it("已添加过就不推荐", async () => {
    renderWithEngine(<SourcesPage />, recommendationRoutes({ added: true }));
    await screen.findByTestId("source-add-collection");
    expect(screen.queryByTestId("source-suggest-agent-skills-rec")).toBeNull();
  });

  it("启用：先确认，确认后依次登记、下载、把技能链接到能加的助手；已有同名内容的助手跳过", async () => {
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
                  // 助手里已有同名的外部内容：引擎不给 add，不覆盖
                  codex: { state: "extra-link", assigned: false, enabled: false, actions: [] },
                },
              },
            }),
          ];
        }
        return makeEnvelope(data, { command: "skills.status" });
      },
    };
    const { calls } = renderWithEngine(<SourcesPage />, routes);
    await userEvent.click(await screen.findByTestId("source-suggest-enable-agent-skills-rec"));
    const dialog = await screen.findByRole("dialog", { name: "添加并启用 agent-skills-rec" });
    expect(within(dialog).getByTestId("recommend-enable-steps")).toHaveTextContent("不覆盖");
    expect(within(dialog).getByTestId("recommend-enable-steps")).toHaveTextContent("规则、MCP 和启动命令保持不变");
    expect(calls.some((call) => call.command === "policy:set" || call.command === "skills:apply")).toBe(false);
    await userEvent.click(within(dialog).getByRole("button", { name: "添加并启用" }));

    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(2));
    const writes = calls.filter((call) => call.command === "policy:set" || call.command === "skills:apply");
    expect(writes[0].command).toBe("policy:set");
    expect(writes[1].args).toEqual(["--only", "skill:agent-skills-rec", "--action", "update"]);
    expect(writes[2].args).toEqual(["--only", "skill:agent-skills-rec/verification", "--action", "add", "--agent", "claude-code"]);
    expect(calls.some((call) => /^(rules:(?!status)|mcp:apply|aliases:apply)/.test(call.command))).toBe(false);
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

describe("来源页：未纳管", () => {

  it("列出没管理的技能、识别出的来源、Codex 共用与重名，只显示不写", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, otherSourceRoutes());
    const block = await screen.findByTestId("skills-other-sources");
    expect(block).toHaveTextContent("未纳管 · 2");
    expect(within(block).getByTestId("other-source-archify")).toHaveTextContent("tt-a1i/archify · v3.0.1 · npx skills 安装");
    expect(within(block).getByTestId("other-shared-archify")).toHaveTextContent("Codex 共用");
    expect(within(block).getByTestId("other-present-archify-claude-code")).toBeInTheDocument();
    expect(within(block).queryByTestId("other-present-archify-codex")).toBeNull();
    expect(within(block).getByTestId("other-source-synced")).toHaveTextContent("来源不明");
    expect(within(block).getByTestId("other-blocked-synced")).toHaveTextContent("Claude 自己同步的");
    expect(within(block).getByTestId("duplicate-xiaoyu-bug-loop-codex")).toHaveTextContent("Codex 会加载两份");
    expect(within(block).getByTestId("adopt-archify")).toHaveTextContent("纳入管理");
    expect(within(block).queryByTestId("adopt-synced")).toBeNull();
    expect(within(block).getByTestId("dedupe-xiaoyu-bug-loop-codex")).toHaveTextContent("清理多余");
    expect(calls.every((call) => call.command.endsWith(":status") || ["policy:get", "resources:targets"].includes(call.command) || call.command === "system:discover")).toBe(true);
  });
});

const V300 = "3".repeat(40);
const V301 = "4".repeat(40);

function versionRoutes() {
  const collection = (name: string, extra: Record<string, unknown> = {}) =>
    makeElement({
      id: `skill:${name}`,
      domain: "skills",
      spec: { url: `https://github.com/example/${name}.git`, skill_paths: [name], track: null },
      observed: {
        repo_cached: true,
        discovered: [name],
        available: [{ name, path: name }],
        actions: ["update", "disable", "remove"],
        version: { current: { commit: V300, kind: "tag", name: "v3.0.0", label: "v3.0.0" }, track_error: null },
        ...extra,
      },
      freshness: { state: "behind", current: "v3.0.0", upstream: "v3.0.1", checked_at: "2026-09-29T00:00:00Z", probe: "git-ls-remote", cache_age_sec: 1 },
    });
  const skill = (coll: string) =>
    makeElement({
      id: `skill:${coll}/${coll}`,
      domain: "skills",
      spec: { collection: coll, name: coll },
      policy: { enabled: true },
      observed: {
        agents: { "claude-code": { state: "linked", assigned: true, enabled: true, version: "old", available_version: "new", actions: ["update", "disable", "remove"] } },
        discovery_inbox: false,
      },
    });
  const status = makeEnvelope(
    { domain: "skills", elements: [collection("archify"), skill("archify"), collection("other"), skill("other")] },
    { command: "skills.status" },
  );
  return {
    ...skillsRoutes(),
    "skills:status": status,
    "skills:versions": makeEnvelope({
      collection: "archify",
      track: { mode: "auto" },
      auto_resolved: "tag",
      latest: { commit: V301, kind: "tag", name: "v3.0.1", label: "v3.0.1" },
      current: { commit: V300, kind: "tag", name: "v3.0.0", label: "v3.0.0" },
      default_branch: "main",
      tags: [
        { name: "v3.1.0-rc.1", commit: "5".repeat(40), version: true, prerelease: true },
        { name: "v3.0.1", commit: V301, version: true, prerelease: false },
        { name: "v3.0.0", commit: V300, version: true, prerelease: false },
      ],
      tags_total: 3,
      branches: [{ name: "main", commit: "6".repeat(40) }],
      branches_total: 1,
    }),
    "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" })),
    "system:refresh-upstream": makeEnvelope({}),
    "skills:preview": makeEnvelope({
      collection: "archify",
      from: { commit: V300, kind: "tag", name: "v3.0.0", label: "v3.0.0" },
      to: { commit: V301, kind: "tag", name: "v3.0.1", label: "v3.0.1" },
      downloaded: true,
      same: false,
      skills: [{ name: "archify", path: "archify", files: [{ status: "modified", file: "SKILL.md" }, { status: "added", file: "references/new.md" }], accounts: ["claude-code"] }],
      rules: [{ status: "modified", file: "rules/team.md" }],
      other: [],
    }),
    "policy:get": makeEnvelope(
      makePolicyData({
        layers: { sqlite: { skills: { repositories: { archify: { url: "https://github.com/example/archify.git", skill_paths: ["archify"] } } } } },
      }),
      { command: "policy.get" },
    ),
  };
}

describe("来源页：版本", () => {

  it("来源已更新但技能应用失败时保留逐项目标结果，不报告全部完成", async () => {
    const routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = versionRoutes();
    routes["skills:apply"] = async (args: string[]) => makeEnvelope({}, {
      ok: !args.includes("skill:archify/archify"),
      errors: args.includes("skill:archify/archify") ? [{ code: "FIXTURE", element: null, message: "Fixture skill update failed" }] : [],
    });
    const { calls } = renderWithEngine(<SourcesPage />, routes);
    await userEvent.click(await screen.findByTestId("source-update-archify"));
    const dialog = await screen.findByRole("dialog", { name: "archify 的版本" });
    await within(dialog).findByTestId("preview-skill-archify");
    await userEvent.click(within(dialog).getByTestId("versions-preview-confirm"));
    await screen.findByText("成功 0 · 失败 1");
    expect(screen.getByText(/Fixture skill update failed/)).toBeInTheDocument();
    expect(await screen.findByText(/来源已更新，但技能应用未全部完成/)).toBeInTheDocument();
    expect(calls.filter(call => call.command === "skills:apply")).toHaveLength(2);
  });

  it("来源行显示当前版本与可更新到的版本", async () => {
    renderWithEngine(<SourcesPage />, versionRoutes());
    expect(await screen.findByTestId("source-version-archify")).toHaveTextContent("v3.0.0");
    const row = screen.getByTestId("source-row-archify");
    expect(row).toHaveTextContent("可更新");
    expect(within(row).getByTitle("可更新到 v3.0.1")).toBeInTheDocument();
  });

  it("版本对话框：列出版本，切换到旧版本后只把这个来源在用的技能换过去", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, versionRoutes());
    await userEvent.click(await menuItem("source-versions-archify"));
    const dialog = await screen.findByRole("dialog", { name: "archify 的版本" });
    expect(await within(dialog).findByTestId("versions-summary")).toHaveTextContent("跟踪：自动（按版本） · 当前 v3.0.0 · 最新 v3.0.1");
    expect(within(dialog).getByTestId("version-v3.1.0-rc.1")).toHaveTextContent("预发布");
    expect(within(dialog).getByTestId("version-v3.0.0")).toHaveTextContent("当前");
    expect(within(dialog).queryByTestId("version-switch-v3.0.0")).toBeNull();
    expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(0);

    await userEvent.click(within(dialog).getByTestId("version-switch-v3.1.0-rc.1"));
    // 先预览：列出会变的文件与跟着换的助手，确认前不写
    const preview = await within(dialog).findByTestId("preview-skill-archify");
    expect(preview).toHaveTextContent("跟着换：");
    expect(preview).toHaveTextContent("修改 SKILL.md");
    expect(preview).toHaveTextContent("新增 references/new.md");
    expect(within(dialog).getByTestId("preview-rules")).toHaveTextContent("规则变化 1 处");
    expect(calls.find((call) => call.command === "skills:preview")?.args).toEqual(["--only", "skill:archify", "--ref", "v3.1.0-rc.1"]);
    expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(0);
    await userEvent.click(within(dialog).getByTestId("versions-preview-confirm"));
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(2));
    const applies = calls.filter((call) => call.command === "skills:apply");
    expect(applies[0].args).toEqual(["--only", "skill:archify", "--action", "update", "--ref", "v3.1.0-rc.1"]);
    expect(applies[1].args).toEqual(["--only", "skill:archify/archify", "--action", "update", "--agent", "claude-code"]);
    expect(applies.some((call) => call.args.includes("skill:other/other"))).toBe(false);
  });

  it("更新到最新与来源行的更新按钮都按跟踪方式走、不带 --ref", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, versionRoutes());
    await userEvent.click(await screen.findByTestId("source-update-archify"));
    const dialog = await screen.findByRole("dialog", { name: "archify 的版本" });
    await within(dialog).findByTestId("preview-skill-archify");
    expect(calls.find((call) => call.command === "skills:preview")?.args).toEqual(["--only", "skill:archify"]);
    expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(0);
    await userEvent.click(within(dialog).getByTestId("versions-preview-confirm"));
    await waitFor(() => expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(2));
    expect(calls.filter((call) => call.command === "skills:apply")[0].args).toEqual(["--only", "skill:archify", "--action", "update"]);
  });

  it("编辑来源：跟踪方式存进定义，自动存为 null", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, versionRoutes());
    await userEvent.click(await menuItem("source-edit-archify"));
    const dialog = await screen.findByRole("dialog", { name: "编辑来源" });
    await chooseOption(within(dialog).getByTestId("collection-track-mode"), "tag");
    await userEvent.type(within(dialog).getByTestId("collection-track-value"), "v*");
    await userEvent.click(within(dialog).getByTestId("collection-track-prerelease"));
    await userEvent.click(within(dialog).getByTestId("collection-form-submit"));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const saved = JSON.parse(calls.find((call) => call.command === "policy:set")!.args.slice(-1)[0] ?? "{}");
    expect(saved.track).toEqual({ mode: "tag", pattern: "v*", prerelease: true });
  });

  it("跟踪方式写错时来源行给出提示；分支名为空不能保存", async () => {
    const routes = versionRoutes();
    const data = routes["skills:status"].data as { elements: ReturnType<typeof makeElement>[] };
    (data.elements[0].observed as { version: { track_error: string } }).version.track_error = "[archify] 不认识的跟踪方式：nightly";
    renderWithEngine(<SourcesPage />, routes);
    expect(await screen.findByTestId("source-row-archify")).toHaveTextContent("跟踪方式有误");
    expect(screen.getByTestId("source-problem-archify")).toHaveTextContent("不认识的跟踪方式：nightly");
    await userEvent.click(await menuItem("source-edit-archify"));
    const dialog = await screen.findByRole("dialog", { name: "编辑来源" });
    await chooseOption(within(dialog).getByTestId("collection-track-mode"), "branch");
    await userEvent.click(within(dialog).getByTestId("collection-form-submit"));
    expect(await within(dialog).findByText("填分支名，例如 stable")).toBeInTheDocument();
  });
});

function adoptRoutes(preview: Record<string, unknown>) {
  const base = otherSourceRoutes();
  return {
    ...base,
    "skills:adopt-preview": makeEnvelope(preview),
    "skills:adopt": makeEnvelope({ domain: "skills", action: "adopt", name: "archify", backup: "20260929T101010000-archify", removed: ["~/.claude/skills/archify"], linked: ["claude-code", "codex"] }),
  };
}

const ADOPT_BASE = {
  name: "archify",
  kind: "adopt",
  locations: [
    { agent: "claude-code", shared: null, path: "~/.claude/skills/archify", origin: "native-dir" },
    { agent: null, shared: "codex-shared", path: "~/.agents/skills/archify", origin: "native-dir" },
  ],
  removes: ["~/.claude/skills/archify", "~/.agents/skills/archify"],
  source: { repository: "https://github.com/tt-a1i/archify.git", path: "archify", version: "3.0.0", installer: "npx-skills" },
  collection: { name: "archify", exists: false, url: "https://github.com/tt-a1i/archify.git", path: "archify", linked_as: "archify" },
  local: { root: "~/skills/my-skills", dest: "~/skills/my-skills/archify", dest_exists: false, collection: "my-skills", digest: "l".repeat(64), accounts: ["claude-code", "codex"], from: "~/.claude/skills/archify", variants: 1 },
};
const DIGEST = "d".repeat(64);

describe("来源页：纳入管理", () => {

  it("内容一致：预览写明版本与会做的事，确认后按预览给的参数纳入，提示里可撤销", async () => {
    const { calls } = renderWithEngine(
      <SourcesPage />,
      adoptRoutes({
        ...ADOPT_BASE,
        source_check: {
          same: true, path: "archify",
          adopt_at: { name: "v3.0.0", label: "v3.0.0", commit: "3".repeat(40) },
          latest: { name: "v3.0.1", label: "v3.0.1", commit: "4".repeat(40) },
          ref: "v3.0.0", locations: [{ same: true, files: [] }, { same: true, files: [] }], accounts: ["claude-code", "codex"], digest: DIGEST,
        },
      }),
    );
    await userEvent.click(await screen.findByTestId("adopt-archify"));
    const dialog = await screen.findByRole("dialog", { name: "纳入管理 archify" });
    expect(await within(dialog).findByTestId("adopt-version")).toHaveTextContent("纳入到来源 archify（新建），版本 v3.0.0，之后可更新到 v3.0.1");
    expect(within(dialog).getByTestId("adopt-compare")).toHaveTextContent("完全一致");
    expect(within(dialog).getByTestId("adopt-plan")).toHaveTextContent("备份并移走 ~/.agents/skills/archify");
    expect(within(dialog).queryByTestId("adopt-choice-source")).toBeNull();
    expect(calls.some((call) => call.command === "skills:adopt")).toBe(false);
    await userEvent.click(within(dialog).getByTestId("adopt-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:adopt")).toBe(true));
    expect(calls.find((call) => call.command === "skills:adopt")!.args).toEqual(["--name", "archify", "--digest", DIGEST, "--path", "archify", "--ref", "v3.0.0", "--agents", "claude-code,codex"]);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "纳入管理 archify" })).toBeNull());
  });

  it("本地改过：列出差异；选用来源版本带 --accept-source，选保留走 --keep-local", async () => {
    const differs = {
      ...ADOPT_BASE,
      source_check: {
        same: false, path: "archify",
        adopt_at: { name: "v3.0.1", label: "v3.0.1", commit: "4".repeat(40) },
        latest: { name: "v3.0.1", label: "v3.0.1", commit: "4".repeat(40) },
        ref: null, locations: [{ same: false, files: [{ status: "modified", file: "SKILL.md" }] }, { same: true, files: [] }], accounts: ["claude-code", "codex"], digest: DIGEST,
      },
    };
    const { calls } = renderWithEngine(<SourcesPage />, adoptRoutes(differs));
    await userEvent.click(await screen.findByTestId("adopt-archify"));
    const dialog = await screen.findByRole("dialog", { name: "纳入管理 archify" });
    expect(await within(dialog).findByTestId("adopt-compare")).toHaveTextContent("改过 SKILL.md");
    await userEvent.click(within(dialog).getByTestId("adopt-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:adopt")).toBe(true));
    expect(calls.find((call) => call.command === "skills:adopt")!.args).toEqual(["--name", "archify", "--digest", DIGEST, "--path", "archify", "--accept-source", "--agents", "claude-code,codex"]);
  });

  it("认不出来源：只能保留本地内容；Codex 重名只做清理", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, adoptRoutes({ ...ADOPT_BASE, source: null, collection: undefined, source_check: { error: "认不出来源仓库，只能作为本机来源纳入" } }));
    await userEvent.click(await screen.findByTestId("adopt-archify"));
    const dialog = await screen.findByRole("dialog", { name: "纳入管理 archify" });
    expect(await within(dialog).findByTestId("adopt-source-error")).toHaveTextContent("只能作为本机来源纳入");
    expect(within(dialog).getByTestId("adopt-choice-source")).toBeDisabled();
    expect(within(dialog).getByTestId("adopt-plan")).toHaveTextContent("复制到 ~/skills/my-skills/archify");
    await userEvent.click(within(dialog).getByTestId("adopt-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:adopt")).toBe(true));
    expect(calls.find((call) => call.command === "skills:adopt")!.args).toEqual(["--name", "archify", "--digest", "l".repeat(64), "--keep-local", "--dest", "~/skills/my-skills", "--agents", "claude-code,codex"]);
  });

  it("重名清理：写明已由哪个来源管理，只移走共用目录那份", async () => {
    const { calls } = renderWithEngine(
      <SourcesPage />,
      adoptRoutes({ name: "xiaoyu-bug-loop", kind: "dedupe", locations: [], removes: ["~/.agents/skills/xiaoyu-bug-loop"], source: null, collection: { name: "xiaoyu-dev", exists: true }, digest: DIGEST }),
    );
    await userEvent.click(await screen.findByTestId("dedupe-xiaoyu-bug-loop-codex"));
    const dialog = await screen.findByRole("dialog", { name: "纳入管理 xiaoyu-bug-loop" });
    expect(await within(dialog).findByTestId("adopt-dedupe")).toHaveTextContent("已由来源 xiaoyu-dev 管理");
    await userEvent.click(within(dialog).getByTestId("adopt-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:adopt")).toBe(true));
    expect(calls.find((call) => call.command === "skills:adopt")!.args).toEqual(["--name", "xiaoyu-bug-loop", "--digest", DIGEST]);
  });

  it("修改个人集合目录后必须重新预览，确认只使用新目录和新指纹", async () => {
    const customRoot = "/custom skill library/my-skills";
    const customDigest = "c".repeat(64);
    const { calls } = renderWithEngine(<SourcesPage />, {
      ...adoptRoutes(ADOPT_BASE),
      "skills:adopt-preview": async (args) => {
        const custom = args.includes("--dest");
        return makeEnvelope({ ...ADOPT_BASE, source: null, source_check: { error: "认不出来源仓库" }, local: { ...ADOPT_BASE.local, ...(custom ? { root: customRoot, dest: `${customRoot}/archify`, digest: customDigest } : {}) } });
      },
    });
    await userEvent.click(await screen.findByTestId("adopt-archify"));
    const dialog = await screen.findByRole("dialog", { name: "纳入管理 archify" });
    const dest = await within(dialog).findByTestId("adopt-dest");
    await userEvent.click(within(dialog).getByTestId("adopt-account-codex"));
    fireEvent.change(dest, { target: { value: customRoot } });
    expect(within(dialog).getByTestId("adopt-confirm")).toBeDisabled();
    expect(calls.filter((call) => call.command === "skills:adopt-preview")).toHaveLength(1);
    expect(calls.some((call) => call.command === "skills:adopt")).toBe(false);
    await userEvent.click(within(dialog).getByTestId("adopt-refresh-preview"));
    await waitFor(() => expect(within(dialog).getByTestId("adopt-plan")).toHaveTextContent(`${customRoot}/archify`));
    expect(within(dialog).getByTestId("adopt-account-codex")).not.toBeChecked();
    expect(calls.filter((call) => call.command === "skills:adopt-preview")[1].args).toEqual(["--name", "archify", "--dest", customRoot]);
    await userEvent.click(within(dialog).getByTestId("adopt-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:adopt")).toBe(true));
    expect(calls.find((call) => call.command === "skills:adopt")!.args).toEqual(["--name", "archify", "--digest", customDigest, "--keep-local", "--dest", customRoot, "--agents", "claude-code"]);
  });
});

describe("来源页：纳入管理的助手与执行中", () => {
  const same = {
    ...ADOPT_BASE,
    current_accounts: ["codex"],
    source_check: {
      same: true, path: "archify",
      adopt_at: { name: "v3.0.1", label: "v3.0.1", commit: "4".repeat(40) },
      latest: { name: "v3.0.1", label: "v3.0.1", commit: "4".repeat(40) },
      ref: null, locations: [{ same: true, files: [] }, { same: true, files: [] }], accounts: ["claude-code", "codex"], digest: DIGEST,
    },
  };

  it("可选助手：默认全选并标出原来就有的；取消勾选后只链接选中的；一个不选不能确认", async () => {
    const { calls } = renderWithEngine(<SourcesPage />, adoptRoutes(same));
    await userEvent.click(await screen.findByTestId("adopt-archify"));
    const dialog = await screen.findByRole("dialog", { name: "纳入管理 archify" });
    const accounts = await within(dialog).findByTestId("adopt-accounts");
    expect(within(accounts).getByTestId("adopt-account-claude-code")).toBeChecked();
    expect(within(accounts).getByTestId("adopt-account-codex")).toBeChecked();
    expect(within(accounts).getAllByText("原来就有")).toHaveLength(1);
    await userEvent.click(within(accounts).getByTestId("adopt-account-claude-code"));
    await userEvent.click(within(accounts).getByTestId("adopt-account-codex"));
    expect(within(dialog).getByTestId("adopt-confirm")).toBeDisabled();
    expect(within(accounts).getByText("至少选一个助手")).toBeInTheDocument();
    await userEvent.click(within(accounts).getByTestId("adopt-account-codex"));
    await userEvent.click(within(dialog).getByTestId("adopt-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "skills:adopt")).toBe(true));
    expect(calls.find((call) => call.command === "skills:adopt")!.args.slice(-2)).toEqual(["--agents", "codex"]);
  });

  it("执行中：显示进度步骤，不给取消、Esc 关不掉，可以在后台继续", async () => {
    let finish: (value: unknown) => void = () => undefined;
    const pending = new Promise((resolve) => {
      finish = resolve;
    });
    const routes = {
      ...adoptRoutes(same),
      "skills:adopt": async () => {
        await pending;
        return makeEnvelope({ domain: "skills", action: "adopt", name: "archify", backup: "b1", removed: [], linked: ["claude-code"] });
      },
      "runs:progress": makeEnvelope({
        progress: {
          started_at: 0,
          steps: [
            { key: "check", label: "核对预览后有没有变化", state: "done", detail: null },
            { key: "backup", label: "备份并移走原来的目录", state: "active", detail: "1/2：~/.claude/skills/archify" },
            { key: "link", label: "链接到助手", state: "pending", detail: null },
          ],
        },
      }),
    };
    renderWithEngine(<SourcesPage />, routes);
    await userEvent.click(await screen.findByTestId("adopt-archify"));
    const dialog = await screen.findByRole("dialog", { name: "纳入管理 archify" });
    await userEvent.click(await within(dialog).findByTestId("adopt-confirm"));
    const steps = await within(dialog).findByTestId("adopt-progress");
    await waitFor(() => expect(within(steps).getByTestId("adopt-progress-backup")).toHaveAttribute("data-state", "active"));
    expect(within(steps).getByTestId("adopt-progress-backup")).toHaveTextContent("1/2：~/.claude/skills/archify");
    expect(within(dialog).queryByTestId("adopt-cancel")).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "关闭" })).toBeNull();
    expect(within(dialog).getByTestId("adopt-running-note")).toHaveTextContent("执行中不能取消");
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("dialog", { name: "纳入管理 archify" })).toBeInTheDocument();
    await userEvent.click(within(dialog).getByTestId("adopt-background"));
    expect(screen.queryByRole("dialog", { name: "纳入管理 archify" })).toBeNull();
    finish(null);
  });
});
