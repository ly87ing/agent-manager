import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { makeAuditData, makeEnvelope, makeMode, makePlanAction } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function Harness({ domain }: { domain: "agents" | "tools" }) {
  const ops = useDefinitionOps({ section: domain, domain });
  return (
    <div>
      <button
        type="button"
        onClick={() =>
          void ops.saveAndApply({
            name: "demo",
            definition: { bin: "demo", install: "install.sh", upgrade: "upgrade.sh" },
            isNew: true,
            elementId: `${domain}:demo`,
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
    expect(policySets[0].args[3]).toBe("agent_definitions.demo");
    // 不写启用键
    expect(policySets.some((c) => c.args[3] === "agents.demo.enabled")).toBe(false);
    // 不触发安装/升级/计划/应用
    expect(calls.some((c) => c.command === "agents:plan")).toBe(false);
    expect(calls.some((c) => c.command === "agents:apply")).toBe(false);
  });

  it("tools 域：保留原流程，新增仍写 enabled 并打开预览", async () => {
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
  });
});
