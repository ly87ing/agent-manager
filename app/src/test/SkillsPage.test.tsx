import { describe, expect, it } from "vitest";
import SkillsPage from "../pages/SkillsPage";
import { makeElement, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, within } from "./render-with-engine";

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
      { domain: "skills", actions: [], summary: { read: 0, "write-local": 0, "exec-local": 0, network: 0, "install-remove": 0 } },
      { command: "skills.plan" },
    ),
  };
}

describe("SkillsPage 三级视图", () => {
  it("集合卡片：模式/默认姿态/缓存/收件箱计数徽标", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    const card = await screen.findByTestId("collection-card-agent-skills");
    expect(within(card).getByText("矩阵模式")).toBeInTheDocument();
    expect(within(card).getByText(/new_skill_default: disabled/)).toBeInTheDocument();
    expect(within(card).getByText("仓库已缓存")).toBeInTheDocument();
    expect(within(card).getByText("自研")).toBeInTheDocument();
    expect(within(card).getByTestId("inbox-count-agent-skills")).toHaveTextContent("收件箱 2");
  });

  it("发现收件箱：列出 disabled+discovery_inbox 的 skill 与状态说明", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    const inbox = await screen.findByTestId("discovery-inbox");
    expect(within(inbox).getByTestId("inbox-total")).toHaveTextContent("2");
    expect(within(inbox).getByText("inbox-one")).toBeInTheDocument();
    expect(within(inbox).getByText("inbox-two")).toBeInTheDocument();
    expect(within(inbox).getAllByText(/显式启用才分发/)).toHaveLength(2);
  });

  it("启用矩阵：单元格态 已链接/死链/未链接/已停用，收件箱 skill 不进矩阵", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    expect(await screen.findByTitle("claude-code: linked")).toHaveTextContent("已链接");
    expect(screen.getByTitle("codex: dead-link")).toHaveTextContent("死链");
    expect(screen.getByTitle("claude-code: missing-link")).toHaveTextContent("未链接");
    const disabled = screen.getByTitle("codex: disabled");
    expect(disabled).toHaveTextContent("已停用");
    expect(disabled.className).toContain("zinc");
    // 收件箱元素不出现在矩阵行中（只在收件箱区）
    expect(screen.queryByText("inbox-one", { selector: "button" })).toBeNull();
  });
});
