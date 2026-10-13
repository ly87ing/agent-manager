import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import { DiscoverPanel } from "../components/DiscoverPanel";
import type { DiscoverData } from "../lib/contract";
import { makeApplyResult, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function discoverData(overrides: Partial<DiscoverData> = {}): DiscoverData {
  return {
    domain: "discover",
    skills: {
      items: [
        {
          agent: "claude-code",
          name: "ones",
          link_path: "~/.claude/skills/ones",
          target: "~/.vendor/skills/ones",
          origin: "external-link",
          manage: "symlink",
          collection: null,
          group: "~/.vendor/skills",
          suggested_collection: "vendor",
          has_skill_md: true,
        },
        {
          agent: "claude-code",
          name: "safe-merge-review",
          link_path: "~/.claude/skills/safe-merge-review",
          target: "~/elsewhere/safe-merge-review",
          origin: "collection-extra-link",
          manage: "collection",
          collection: "agent-skills",
          group: null,
          has_skill_md: true,
        },
        {
          agent: "claude-code",
          name: "synced",
          link_path: "~/.claude/skills/synced",
          target: null,
          origin: "native-dir",
          manage: "native",
          collection: null,
          group: null,
          has_skill_md: false,
        },
        {
          agent: "codex",
          name: "ghost",
          link_path: "~/.codex/skills/ghost",
          target: "~/gone/ghost",
          origin: "dead-link",
          manage: "cleanup",
          collection: null,
          group: null,
          has_skill_md: false,
        },
      ],
      groups: [
        {
          name: "vendor",
          source_path: "~/.vendor/skills",
          manage: "symlink",
          existing_collection: false,
          agents: ["claude-code"],
          // 源目录里还有一个没被链接过的，接管时一并收录
          skills: [
            { name: "ones", agents: ["claude-code"], has_skill_md: true, linked: true },
            { name: "unlinked-one", agents: [], has_skill_md: true, linked: false },
          ],
          suggested_definition: {
            source_path: "~/.vendor/skills",
            skill_paths: ["ones", "unlinked-one"],
            agents: ["claude-code"],
          },
        },
      ],
    },
    summary: { skills_symlink: 1, skills_collection: 1, skills_native: 1, skills_cleanup: 1 },
    ...overrides,
  };
}

function routes(data: DiscoverData = discoverData()) {
  return {
    "system:discover": makeEnvelope(data, { command: "system.discover" }),
    "policy:set": makeEnvelope({ layers: null, effective: {} }, { command: "policy.set" }),
    "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
  };
}

describe("扫描外部技能目录与残留链接", () => {
  it("按来源分类展示，原生安装项只标识不可勾选", async () => {
    renderWithEngine(
      <DiscoverPanel />,
      routes(),
    );
    await screen.findByTestId("takeover-skill-group-vendor");

    // 可接管项带勾选框
    expect(screen.queryByTestId("takeover-agents")).toBeNull();
    expect(screen.getByTestId("takeover-skill-group-vendor").querySelector("input[type=checkbox]")).toBeTruthy();
    // MCP 条目以各助手配置文件为准，直接在 MCP 页编辑，接管中心不再有 MCP 分区
    expect(screen.queryByTestId("takeover-mcp")).toBeNull();

    // 原生管理项只展示，不提供勾选（它归各自的安装方式管）
    expect(screen.getByTestId("takeover-native-synced-claude-code").querySelector("input[type=checkbox]")).toBeNull();
    expect(screen.getByTestId("takeover-native-synced-claude-code")).toHaveTextContent("原生管理");
    // 残留链接可勾选，但走的是清理而不是纳管
    expect(screen.getByTestId("takeover-dead-ghost-codex").querySelector("input[type=checkbox]")).toBeTruthy();
    // 助手登记只服务于识别和配置，软件生命周期一律交还原安装渠道。
    expect(screen.queryByTestId("takeover-agent-opencode")).toBeNull();
  });

  it("外部技能组接管：先登记本地来源集合定义，再逐助手 add（不带接管旗标）", async () => {
    const { calls } = renderWithEngine(<DiscoverPanel />, routes());
    await waitFor(() => expect(screen.getByTestId("takeover-skill-group-vendor")).toBeInTheDocument());

    await userEvent.click(screen.getByTestId("takeover-skill-group-vendor").querySelector("input[type=checkbox]")!);
    await userEvent.click(screen.getByTestId("takeover-run"));
    await userEvent.click(await screen.findByRole("button", { name: "开始接管" }));

    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    const policyCall = calls.find((call) => call.command === "policy:set");
    expect(policyCall?.args).toEqual([
      "--layer",
      "local",
      "--key",
      "skills.repositories.vendor",
      "--value",
      JSON.stringify({ source_path: "~/.vendor/skills", skill_paths: ["ones", "unlinked-one"], agents: ["claude-code"], _delete: false, _replace: true }),
    ]);
    const applyCall = calls.find((call) => call.command === "skills:apply");
    expect(applyCall?.args).toEqual(["--only", "vendor/ones", "--agent", "claude-code", "--action", "add"]);
    // 登记源根后链接已在受管基准内，无需显式接管旗标
    expect(applyCall?.args).not.toContain("--allow-takeover");
  });

  it("已定义集合的计划外链接走既有接管通道（带 --allow-takeover）", async () => {
    const { calls } = renderWithEngine(<DiscoverPanel />, routes());
    await waitFor(() => expect(screen.getByTestId("takeover-skill-extra-safe-merge-review-claude-code")).toBeInTheDocument());

    await userEvent.click(
      screen.getByTestId("takeover-skill-extra-safe-merge-review-claude-code").querySelector("input[type=checkbox]")!,
    );
    await userEvent.click(screen.getByTestId("takeover-run"));
    expect(await screen.findByText(/会替换助手里的同名外部条目/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "开始接管" }));

    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")?.args).toEqual([
      "--only",
      "agent-skills/safe-merge-review",
      "--agent",
      "claude-code",
      "--action",
      "add",
      "--allow-takeover",
    ]);
  });

  it("残留链接可在本行清理，确认后重新扫描并消失", async () => {
    let removed = false;
    const initial = discoverData();
    const afterRemoval = discoverData({
      skills: { items: initial.skills?.items.filter((item) => item.name !== "ghost") ?? [], groups: initial.skills?.groups ?? [] },
    });
    const { calls } = renderWithEngine(<DiscoverPanel />, {
      ...routes(),
      "system:discover": async () => makeEnvelope(removed ? afterRemoval : initial),
      "skills:apply": async () => {
        removed = true;
        return makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" });
      },
    });
    await waitFor(() => expect(screen.getByTestId("takeover-dead-ghost-codex")).toBeInTheDocument());

    await userEvent.click(screen.getByTestId("cleanup-link-ghost-codex"));
    expect(await screen.findByText(/将删除所选的 1 条链接本身/)).toBeInTheDocument();
    expect(screen.getByText("原目标：~/gone/ghost")).toBeInTheDocument();
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    await waitFor(() => expect(screen.queryByTestId("takeover-dead-ghost-codex")).toBeNull());
    expect(calls.find((call) => call.command === "skills:apply")?.args).toEqual([
      "--action",
      "remove-link",
      "--agent",
      "codex",
      "--link",
      "ghost",
    ]);
  });

  it("清理失败后显示原因并保留选择，接管按钮不包含清理项", async () => {
    renderWithEngine(<DiscoverPanel />, {
      ...routes(),
      "skills:apply": makeEnvelope({}, { ok: false, errors: [{ code: "LINK_CHANGED", element: null, message: "链接已被外部修改" }] }),
    });
    const row = await screen.findByTestId("takeover-dead-ghost-codex");
    await userEvent.click(row.querySelector("input[type=checkbox]")!);
    expect(screen.getByTestId("takeover-run")).toHaveTextContent("一键接管所选（0）");
    await userEvent.click(screen.getByTestId("cleanup-selected"));
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("链接已被外部修改"));
    expect(row.querySelector("input[type=checkbox]")).toBeChecked();
  });

  it("后续扫描仍见链接时不报成功，保留该项供重试", async () => {
    let attempts = 0;
    const initial = discoverData();
    const afterRemoval = discoverData({
      skills: { items: initial.skills?.items.filter((item) => item.name !== "ghost") ?? [], groups: initial.skills?.groups ?? [] },
    });
    renderWithEngine(<DiscoverPanel />, {
      ...routes(),
      "system:discover": async () => makeEnvelope(attempts > 1 ? afterRemoval : initial),
      "skills:apply": async () => {
        attempts += 1;
        return makeEnvelope(makeApplyResult({ domain: "skills" }));
      },
    });
    const row = await screen.findByTestId("takeover-dead-ghost-codex");
    await userEvent.click(screen.getByTestId("cleanup-link-ghost-codex"));
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(row).toHaveTextContent("重新扫描后链接仍在"));
    expect(row.querySelector("input[type=checkbox]")).toBeChecked();
    await userEvent.click(screen.getByTestId("cleanup-selected"));
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(screen.queryByTestId("takeover-dead-ghost-codex")).toBeNull());
    expect(attempts).toBe(2);
  });

  it("批量执行时显示进度：第几项、正在做哪一项", async () => {
    // 卡住第一条 policy:set，让进度停在第 1 项上可断言
    let release: (() => void) | null = null;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    renderWithEngine(<DiscoverPanel />, {
      ...routes(),
      "policy:set": async () => {
        await gate;
        return makeEnvelope({ layers: null, effective: {} }, { command: "policy.set" });
      },
    });
    await waitFor(() => expect(screen.getByTestId("takeover-skill-group-vendor")).toBeInTheDocument());

    // 勾选两类项目后，接管只执行技能集合，不混入残留链接清理。
    await userEvent.click(screen.getByTestId("takeover-skill-group-vendor").querySelector("input[type=checkbox]")!);
    await userEvent.click(screen.getByTestId("takeover-dead-ghost-codex").querySelector("input[type=checkbox]")!);
    await userEvent.click(screen.getByTestId("takeover-run"));
    await userEvent.click(await screen.findByRole("button", { name: "开始接管" }));

    const progress = await screen.findByTestId("takeover-progress");
    expect(progress).toHaveTextContent("正在处理第 1 / 1 项：技能来源 vendor");
    expect(screen.getByTestId("takeover-run")).toHaveTextContent("正在接管 1/1…");

    release!();
    await waitFor(() => expect(screen.queryByTestId("takeover-progress")).toBeNull());
    expect(screen.getByTestId("takeover-dead-ghost-codex").querySelector("input[type=checkbox]")).toBeChecked();
  });

  it("没有可处理的内容时两处都不显示，也不占地方", async () => {
    const empty: DiscoverData = {
      domain: "discover",
      skills: { items: [], groups: [] },
      summary: {},
    };
    const { calls } = renderWithEngine(
      <DiscoverPanel />,
      routes(empty),
    );
    await waitFor(() => expect(calls.some((call) => call.command === "system:discover")).toBe(true));
    expect(screen.queryByTestId("discover-skills")).toBeNull();
    expect(screen.queryByTestId("takeover-run")).toBeNull();
  });
});
