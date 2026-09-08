import { describe, expect, it } from "vitest";
import { fireEvent } from "@testing-library/react";
import SkillsPage from "../pages/SkillsPage";
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
                "claude-code": { state: "linked", link_path: "/home/.claude/skills/alpha", target_dir_exists: true },
                codex: { state: "dead-link", link_path: "/home/.codex/skills/alpha", target_dir_exists: true },
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
                "claude-code": { state: "missing-link", link_path: "/home/.claude/skills/beta", target_dir_exists: true },
                codex: { state: "disabled", link_path: "/home/.codex/skills/beta", target_dir_exists: true },
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
          baseline: { skills: { "agent-skills": { url: "https://github.com/example/agent-skills.git" } } },
          synced: null,
          local: null,
        },
      }),
      { command: "policy.get" },
    ),
    "policy:set": makeEnvelope(
      makePolicyData({ layers: null, effective: null, attribution: null, layer: "local", key: "skills.definitions.agent-skills", value: null, path: "/repo/config/manager.local.json" }),
      { command: "policy.set" },
    ),
    "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
  };
}

describe("SkillsPage 三级视图", () => {
  it("编辑集合可清空发现范围，并只预览当前集合", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("edit-collection-agent-skills"));
    expect(screen.getByTestId("collection-name-input")).toBeDisabled();
    expect(screen.getByTestId("collection-url-input")).toHaveValue("https://github.com/example/agent-skills.git");
    fireEvent.change(screen.getByTestId("collection-paths-input"), { target: { value: "" } });
    await userEvent.click(screen.getByTestId("collection-form-submit"));
    await screen.findByRole("dialog", { name: /将要执行的操作/ });
    const write = calls.find((call) => call.command === "policy:set")!;
    expect(JSON.parse(write.args[write.args.indexOf("--value") + 1])).toMatchObject({ skill_paths: null, _delete: false });
    expect(calls.find((call) => call.command === "skills:plan")?.args).toEqual(["--only", "skill:agent-skills"]);
  });

  it("更新集合预览限定在所选集合", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    await userEvent.click(await screen.findByTestId("update-collection-agent-skills"));
    await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(calls.find((call) => call.command === "skills:plan")?.args).toEqual(["--only", "skill:agent-skills"]);
  });
  it("集合卡片：模式/默认姿态/缓存/收件箱计数徽标", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    const card = await screen.findByTestId("collection-card-agent-skills");
    expect(within(card).getByText("按助手逐个授权")).toBeInTheDocument();
    expect(within(card).getByText(/新技能默认:不启用/)).toBeInTheDocument();
    expect(within(card).getByText("已下载到本地")).toBeInTheDocument();
    expect(within(card).getByText("自研")).toBeInTheDocument();
    expect(within(card).getByTestId("inbox-count-agent-skills")).toHaveTextContent("新发现 2");
  });

  it("新发现的技能区：列出 disabled+discovery_inbox 的 skill 与说明", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    const inbox = await screen.findByTestId("discovery-inbox");
    expect(within(inbox).getByTestId("inbox-total")).toHaveTextContent("2");
    expect(within(inbox).getByText("inbox-one")).toBeInTheDocument();
    expect(within(inbox).getByText("inbox-two")).toBeInTheDocument();
    expect(within(inbox).getAllByText(/启用后才会分发到助手/)).toHaveLength(2);
  });

  it("启用矩阵：单元格态 已链接/死链/未链接/已停用，收件箱 skill 不进矩阵", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    expect(await screen.findByTitle("claude-code: linked")).toHaveTextContent("已启用");
    expect(screen.getByTitle("codex: dead-link")).toHaveTextContent("失效链接");
    expect(screen.getByTitle("claude-code: missing-link")).toHaveTextContent("未生效");
    const disabled = screen.getByTitle("codex: disabled");
    expect(disabled).toHaveTextContent("已停用");
    expect(disabled.className).toContain("zinc");
    // 收件箱元素不出现在矩阵行中（只在收件箱区）
    expect(screen.queryByText("inbox-one", { selector: "button" })).toBeNull();
  });

  it("移除集合：确认后执行 skills:plan 和 skills:apply 并携带 --only skill:<name>", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, skillsRoutes());
    const removeBtn = await screen.findByTestId("remove-collection-agent-skills");
    await userEvent.click(removeBtn);

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("移除 agent-skills");
    fireEvent.change(screen.getByTestId("confirm-modal-input"), { target: { value: "agent-skills" } });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await screen.findByRole("dialog", { name: /将要执行的操作/ });
    const planCall = calls.find((c) => c.command === "skills:plan");
    expect(planCall).toBeDefined();
    expect(planCall!.args).toEqual(["--only", "skill:agent-skills"]);

    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    const applyCall = calls.find((c) => c.command === "skills:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.args).toEqual(["--only", "skill:agent-skills"]);
  });
});
