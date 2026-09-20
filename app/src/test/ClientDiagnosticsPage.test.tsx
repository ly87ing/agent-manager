import { describe, expect, it } from "vitest";
import ClientDiagnosticsPage from "../pages/ClientDiagnosticsPage";
import { makeElement, makeEnvelope, makeFreshness } from "./fixtures";
import { renderWithEngine, screen } from "./render-with-engine";

describe("客户端诊断", () => {
  it("只读显示安装状态、版本与来源，不提供生命周期动作", async () => {
    const { calls } = renderWithEngine(<ClientDiagnosticsPage />, {
      "agents:status": makeEnvelope(
        {
          domain: "agents",
          elements: [makeElement({
            id: "agents:claude-code",
            domain: "agents",
            spec: {
              display_name: "Claude Code",
              bin: "claude",
              providers: { native: { install: "hidden-install.sh", upgrade: "hidden-upgrade.sh" } },
            },
            policy: { enabled: true },
            observed: { installed: true, version: "2.0.0", install_source: "native" },
            state: "managed-ok",
            freshness: makeFreshness({ state: "not-applicable", current: null }),
            actions: [
              { id: "upgrade", risk: "network", needs_lock: true, offline_ok: false },
              { id: "uninstall", risk: "install-remove", needs_lock: true, offline_ok: false },
            ],
          })],
        },
        { command: "agents.status" },
      ),
    });

    expect(await screen.findByRole("heading", { name: "客户端诊断" })).toBeInTheDocument();
    expect(screen.getByText("2.0.0")).toBeInTheDocument();
    expect(screen.getByText("官方安装器")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /安装|升级|卸载/ })).toBeNull();
    expect(screen.queryByText("hidden-install.sh")).toBeNull();
    expect(calls.map((call) => call.command)).toEqual(["agents:status"]);
  });
});
