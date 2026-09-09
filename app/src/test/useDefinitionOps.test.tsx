import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { toast } from "sonner";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { makeAuditData, makeEnvelope, makeMode, makePlanAction, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function Harness({
  domain,
  isNew = true,
  complete,
  definition,
}: {
  domain: "agents" | "tools" | "mcp" | "skills";
  isNew?: boolean;
  complete?: boolean;
  definition?: Record<string, unknown>;
}) {
  const ops = useDefinitionOps({ section: domain, domain });
  return (
    <div>
      <button
        type="button"
        onClick={() =>
          void ops.saveAndApply({
            name: "demo",
            definition: definition ?? { bin: "demo", install: "install.sh", upgrade: "upgrade.sh" },
            isNew,
            elementId: `${domain}:demo`,
            complete,
          })
        }
      >
        保存
      </button>
      {ops.modals}
    </div>
  );
}

const policySetRoute = makeEnvelope({ applied: true }, { command: "policy.set" });

describe("useDefinitionOps 保存语义", () => {
  it("agents 域：保存定义只写定义本身，不写 enabled、不 plan/apply", async () => {
    const { calls } = renderWithEngine(<Harness domain="agents" />, {
      "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline: false }) }),
      "policy:set": policySetRoute,
    });

    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(calls.some((c) => c.command === "policy:set")).toBe(true);
    });

    const policySets = calls.filter((c) => c.command === "policy:set");
    expect(policySets).toHaveLength(1);
    expect(policySets[0].args[1]).toBe("local");
    expect(policySets[0].args[3]).toBe("agent_definitions.demo");
    // 不写启用键
    expect(policySets.some((c) => c.args[3] === "agents.demo.enabled")).toBe(false);
    // 不触发安装/升级/计划/应用
    expect(calls.some((c) => c.command === "agents:plan")).toBe(false);
    expect(calls.some((c) => c.command === "agents:apply")).toBe(false);
  });

  it("mcp 域：保存定义只写一条整条替换的定义，不写 enabled、不 plan/apply", async () => {
    const { calls } = renderWithEngine(<Harness domain="mcp" />, {
      "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline: false }) }),
      "policy:set": policySetRoute,
    });

    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(calls.some((c) => c.command === "policy:set")).toBe(true);
    });

    const policySets = calls.filter((c) => c.command === "policy:set");
    expect(policySets).toHaveLength(1);
    expect(policySets[0].args[1]).toBe("local");
    expect(policySets[0].args[3]).toBe("mcp.definitions.demo");
    const definition = JSON.parse(policySets[0].args[5]);
    expect(definition._replace).toBe(true);
    expect(definition._delete).toBe(false);
    // 不写启用键
    expect(policySets.some((c) => c.args[3] === "mcp.demo.enabled")).toBe(false);
    // 不触发同步/计划/应用
    expect(calls.some((c) => c.command === "mcp:plan")).toBe(false);
    expect(calls.some((c) => c.command === "mcp:apply")).toBe(false);
  });

  it("skills 域：保存定义只写一条整条替换的定义，不写 enabled、不 plan/apply", async () => {
    const { calls } = renderWithEngine(<Harness domain="skills" />, {
      "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline: false }) }),
      "policy:set": policySetRoute,
    });

    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(calls.some((c) => c.command === "policy:set")).toBe(true);
    });

    const policySets = calls.filter((c) => c.command === "policy:set");
    expect(policySets).toHaveLength(1);
    expect(policySets[0].args[1]).toBe("local");
    expect(policySets[0].args[3]).toBe("skills.repositories.demo");
    const definition = JSON.parse(policySets[0].args[5]);
    expect(definition._replace).toBe(true);
    expect(definition._delete).toBe(false);
    expect(policySets.some((c) => c.args[3] === "skills.demo.enabled")).toBe(false);
    expect(calls.some((c) => c.command === "skills:plan")).toBe(false);
    expect(calls.some((c) => c.command === "skills:apply")).toBe(false);
  });

  it("skills 域：编辑且缺完整原定义时先 db:migrate，再以 sqlite 完整定义合并整条替换（未知字段不丢）", async () => {
    const { calls } = renderWithEngine(<Harness domain="skills" isNew={false} complete={false} />, {
      "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline: false }) }),
      "policy:set": policySetRoute,
      "db:migrate": makeEnvelope({ migrated: true }, { command: "db.migrate" }),
      "policy:get": makeEnvelope(
        makePolicyData({
          layers: {
            sqlite: {
              skills: {
                repositories: {
                  demo: { url: "https://github.com/example/demo.git", skill_paths: ["old-path"], branch: "release" },
                },
              },
            },
          },
        }),
        { command: "policy.get" },
      ),
    });

    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(calls.some((c) => c.command === "policy:set")).toBe(true);
    });

    // db:migrate 与只读 policy:get 发生在唯一一次 policy:set 之前
    const migrateIndex = calls.findIndex((c) => c.command === "db:migrate");
    const getIndex = calls.findIndex((c) => c.command === "policy:get");
    const policySets = calls.filter((c) => c.command === "policy:set");
    expect(migrateIndex).toBeGreaterThanOrEqual(0);
    expect(getIndex).toBeGreaterThan(migrateIndex);
    expect(policySets).toHaveLength(1);
    expect(calls.indexOf(policySets[0])).toBeGreaterThan(getIndex);

    // 完整定义合并表单字段后整条替换：表单未展示的 branch 保留，表单字段覆盖
    const definition = JSON.parse(policySets[0].args[5]);
    expect(definition._replace).toBe(true);
    expect(definition._delete).toBe(false);
    expect(definition.branch).toBe("release");
    expect(definition.url).toBe("https://github.com/example/demo.git");
    expect(definition.bin).toBe("demo");
    expect(calls.some((c) => c.command === "skills:plan")).toBe(false);
    expect(calls.some((c) => c.command === "skills:apply")).toBe(false);
  });

  it("mcp/skills 域：db:migrate 失败则停止保存，零写入", async () => {
    const { calls } = renderWithEngine(<Harness domain="mcp" isNew={false} complete={false} />, {
      "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline: false }) }),
      "db:migrate": makeEnvelope(null, { command: "db.migrate", ok: false, errors: [{ code: "db", element: null, message: "数据库损坏" }] }),
      "policy:set": policySetRoute,
    });

    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(calls.some((c) => c.command === "db:migrate")).toBe(true);
    });
    // 给潜在的后续写入一个节拍，确认没有发生
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(calls.some((c) => c.command === "policy:set")).toBe(false);
    expect(calls.some((c) => c.command === "policy:get")).toBe(false);
  });

  it("mcp 域：编辑重建 env 时若 <literal> 在完整原定义找不到对应 string，toast 失败且停止保存（零写入）", async () => {
    const toastErrorSpy = vi.spyOn(toast, "error");
    const { calls } = renderWithEngine(
      <Harness
        domain="mcp"
        isNew={false}
        complete={false}
        definition={{
          command: "npx",
          args: ["-y", "demo"],
          env: {
            API_KEY: "<literal>",
          },
        }}
      />,
      {
        "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline: false }) }),
        "policy:set": policySetRoute,
        "db:migrate": makeEnvelope({ migrated: true }, { command: "db.migrate" }),
        "policy:get": makeEnvelope(
          makePolicyData({
            layers: {
              sqlite: {
                mcp: {
                  definitions: {
                    demo: {
                      command: "npx",
                      args: ["-y", "demo"],
                      env: {},
                    },
                  },
                },
              },
            },
          }),
          { command: "policy.get" },
        ),
      },
    );

    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(calls.some((c) => c.command === "db:migrate")).toBe(true);
      expect(calls.some((c) => c.command === "policy:get")).toBe(true);
    });

    await waitFor(() => {
      expect(toastErrorSpy).toHaveBeenCalledWith(expect.stringContaining("API_KEY"));
    });

    // 零写入 policy:set，绝对不能把脱敏占位符存回 DB
    expect(calls.some((c) => c.command === "policy:set")).toBe(false);
  });

  it("tools 域：新增定义也带 _replace 整条替换语义", async () => {
    const { calls } = renderWithEngine(<Harness domain="tools" />, {
      "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline: false }) }),
      "policy:set": policySetRoute,
      "tools:plan": makeEnvelope(
        {
          domain: "tools",
          actions: [makePlanAction({ op: "run-cmd", element: "tools:demo", target: null, risk: "network", cmd: "install.sh" })],
          summary: { read: 0, "write-local": 0, "exec-local": 0, network: 1, "install-remove": 0 },
        },
        { command: "tools.plan" },
      ),
    });

    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      const policySets = calls.filter((c) => c.command === "policy:set");
      expect(policySets.some((c) => c.args[3] === "tools.demo.enabled" && c.args[5] === "true")).toBe(true);
    });
    const definitionSet = calls.find((c) => c.command === "policy:set" && c.args[3] === "tool_definitions.demo");
    expect(definitionSet).toBeDefined();
    expect(JSON.parse(definitionSet!.args[5])._replace).toBe(true);
  });
});
