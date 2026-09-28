import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { makeAuditData, makeEnvelope, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function Harness({
  domain,
  isNew = true,
  complete,
  definition,
  expectedDefinition,
}: {
  domain: "agents" | "skills";
  isNew?: boolean;
  complete?: boolean;
  definition?: Record<string, unknown>;
  expectedDefinition?: Record<string, unknown> | null;
}) {
  const ops = useDefinitionOps({ section: domain, domain });
  return (
    <div>
      <button
        type="button"
        onClick={() =>
          void ops.saveDefinition({
            name: "demo",
            definition: definition ?? { bin: "demo", install: "install.sh", upgrade: "upgrade.sh" },
            isNew,
            complete,
            expectedDefinition,
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
  it("Agent 定义保存把打开表单时的定义交给写入命令核对", async () => {
    const expectedDefinition = { bin: "demo", settings: { profiles: { other: { label: "旧接入" } } } };
    const { calls } = renderWithEngine(<Harness domain="agents" expectedDefinition={expectedDefinition} />, {
      "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline: false }) }),
      "policy:set": policySetRoute,
    });
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    const write = calls.find((call) => call.command === "policy:set");
    expect(write?.args.slice(-2)).toEqual(["--expect-definition", JSON.stringify(expectedDefinition)]);
  });

  it("agents 域：保存定义只写定义本身，不写 enabled、不 apply/preflight", async () => {
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
    expect(policySets[0].args[3]).toBe("agents.definitions.demo");
    // 不写启用键
    expect(policySets.some((c) => c.args[3] === "agents.demo.enabled")).toBe(false);
    // 不触发安装/升级/计划/应用
    expect(calls.some((c) => c.command === "agents:plan")).toBe(false);
    expect(calls.some((c) => c.command === "agents:apply")).toBe(false);
  });

  it("skills 域：保存定义只写一条整条替换的定义，不写 enabled、不 apply/preflight", async () => {
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

  // 原先用 mcp 域举例；MCP 不再走定义覆盖层（条目直接由 mcp:apply 编辑），改用 skills 验证同一语义
  it("skills 域：db:migrate 失败则停止保存，零写入", async () => {
    const { calls } = renderWithEngine(<Harness domain="skills" isNew={false} complete={false} />, {
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
});
