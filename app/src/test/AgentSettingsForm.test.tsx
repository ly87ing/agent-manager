import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/**
 * 助手运行参数：保存即写进助手自己的配置文件。
 *
 * 两处必须先问过用户，引擎侧强制、界面负责问：降安全等级（YOLO）、目标被外部改过。
 */

function routes(overrides: Record<string, Envelope> = {}): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "policy:get": makeEnvelope(makePolicyData(), { command: "policy.get" }),
    "policy:set": makeEnvelope(makePolicyData(), { command: "policy.set" }),
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:claude-code",
            domain: "agents",
            spec: {
              display_name: "Claude Code",
              bin: "claude",
              preferred_provider: "native",
              providers: { native: { managed: true, install: "i.sh", upgrade: "u.sh" } },
            },
            policy: { enabled: true },
            observed: { install_source: "native", installed: true },
            state: "managed-ok",
            freshness: makeFreshness(),
            actions: [],
          }),
        ],
      },
      { command: "agents.status" },
    ),
    "agents:apply": makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }),
    "agents:configure": makeEnvelope(
      { domain: "agents", action: "configure", element: "agents:claude-code" },
      { command: "agents.configure" },
    ),
    "sources:get": makeEnvelope(
      {
        sources: [
          {
            key: "aliases.shell",
            label: "Shell Aliases",
            format: "json",
            revision: "rev-1",
            value: { timezone: "America/Los_Angeles", lang: "en_US.UTF-8", aliases: {} },
          },
        ],
      },
      { command: "sources.get" },
    ),
    "sources:set": makeEnvelope({ key: "aliases.shell", revision: "rev-2" }, { command: "sources.set" }),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
    ...overrides,
  };
}

async function openEditor() {
  await userEvent.click(await screen.findByRole("button", { name: "编辑命令" }));
  return screen.findByRole("dialog");
}

function configureCalls(calls: { command: string; args?: string[] }[]) {
  return calls.filter((call) => call.command === "agents:configure");
}

describe("助手运行参数", () => {
  it("填了接口地址保存：先存定义，再写进助手配置文件", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await userEvent.type(screen.getByTestId("agent-base-url-input"), "https://api.example.com");
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    // 定义先落库，参数才写文件
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.base_url).toBe("https://api.example.com");
    expect(configureCalls(calls)[0].args).toEqual(["--only", "agents:claude-code"]);
  });

  it("一项都没填：只存定义，不碰助手的配置文件", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    expect(configureCalls(calls)).toHaveLength(0);
  });

  it("令牌只填变量名：定义里存的是名字，不是取值", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await userEvent.type(screen.getByTestId("agent-token-var-input"), "MY_GATEWAY_TOKEN");
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    const saved = calls.find((call) => call.command === "policy:set")!;
    const definition = JSON.parse(saved.args![saved.args!.indexOf("--value") + 1]);
    expect(definition.settings.token_var).toBe("MY_GATEWAY_TOKEN");
    expect(JSON.stringify(definition)).not.toContain("sk-");
  });

  it("勾了跳过权限确认：先弹确认，取消就不写文件", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await userEvent.click(screen.getByTestId("agent-yolo-checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await screen.findByRole("dialog", { name: "跳过权限确认" });
    expect(configureCalls(calls)).toHaveLength(0);

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(configureCalls(calls)).toHaveLength(0);
  });

  it("勾了跳过权限确认并确认：带 --allow-yolo 写入", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());
    await openEditor();
    await userEvent.click(screen.getByTestId("agent-yolo-checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    await screen.findByRole("dialog", { name: "跳过权限确认" });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(configureCalls(calls)).toHaveLength(1));
    expect(configureCalls(calls)[0].args).toEqual([
      "--only",
      "agents:claude-code",
      "--allow-yolo",
    ]);
  });

  it("配置文件被外部改过：先问再覆盖，确认后带 --allow-takeover 重试", async () => {
    let allowed = false;
    const base = routes();
    const withRefusal: Record<string, Envelope | (() => Promise<Envelope>)> = {
      ...base,
      "agents:configure": async () => {
        if (allowed) {
          return makeEnvelope(
            { domain: "agents", action: "configure", element: "agents:claude-code" },
            { command: "agents.configure" },
          );
        }
        allowed = true;
        return makeEnvelope(
          {},
          {
            command: "agents.configure",
            ok: false,
            errors: [
              {
                code: "AGENT_ERROR",
                element: null,
                message: ".claude/settings.json 里这几项被本工具以外改过，拒绝覆盖；确认要用这里的内容替换请选择接管",
              },
            ],
          },
        );
      },
    };
    const { calls } = renderWithEngine(<AgentsPage />, withRefusal);
    await openEditor();
    await userEvent.type(screen.getByTestId("agent-base-url-input"), "https://api.example.com");
    await userEvent.click(screen.getByRole("button", { name: "保存定义" }));

    // 第一次被拒 → 弹接管确认
    await screen.findByRole("dialog", { name: "覆盖被改过的配置" });
    expect(configureCalls(calls)).toHaveLength(1);

    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(configureCalls(calls)).toHaveLength(2));
    expect(configureCalls(calls)[1].args).toContain("--allow-takeover");
  });

  it("不是 Claude 形态的助手：不显示运行参数这一段", async () => {
    const codexRoutes = routes({
      "agents:status": makeEnvelope(
        {
          domain: "agents",
          elements: [
            makeElement({
              id: "agents:codex",
              domain: "agents",
              spec: {
                display_name: "Codex",
                bin: "codex",
                preferred_provider: "native",
                providers: { native: { managed: true, install: "i.sh", upgrade: "u.sh" } },
              },
              policy: { enabled: true },
              observed: { install_source: "native", installed: true },
              state: "managed-ok",
              freshness: makeFreshness(),
              actions: [],
            }),
          ],
        },
        { command: "agents.status" },
      ),
    });
    renderWithEngine(<AgentsPage />, codexRoutes);
    await openEditor();
    expect(screen.queryByTestId("agent-base-url-input")).toBeNull();
    expect(screen.queryByTestId("agent-yolo-checkbox")).toBeNull();
  });
});
