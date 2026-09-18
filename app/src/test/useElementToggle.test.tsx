import { describe, expect, it, vi } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import { toast } from "sonner";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness } from "./fixtures";
import type { Envelope } from "../lib/contract";
import DomainApplyPage from "../pages/DomainApplyPage";

function routes(): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit" }),
    "aliases:status": makeEnvelope(
      {
        domain: "aliases",
        elements: [
          makeElement({
            id: "aliases:demo",
            domain: "aliases",
            spec: {},
            policy: { enabled: true },
            observed: {},
            state: "managed-ok",
            freshness: makeFreshness(),
          }),
        ],
      },
      { command: "aliases.status" },
    ),
    "policy:set": makeEnvelope(
      { layer: "sqlite", key: "aliases.demo.enabled", value: false, path: "/native-manager.db" },
      { command: "policy.set" },
    ),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
  };
}

describe("useElementToggle 启停开关", () => {
  it("policy:set 成功后立即自动 apply --only element，不再 toast 二次询问", async () => {
    const toastSuccessSpy = vi.spyOn(toast, "success");
    const { calls } = renderWithEngine(<DomainApplyPage domain="aliases" title="Aliases" description="desc" enableToggles />, routes());

    await userEvent.click(await screen.findByTestId("toggle-aliases:demo"));
    await waitFor(() => expect(calls.some((c) => c.command === "aliases:apply")).toBe(true));

    const policyCalls = calls.filter((c) => c.command === "policy:set");
    expect(policyCalls).toHaveLength(1);
    expect(policyCalls[0].args).toEqual(["--layer", "local", "--key", "aliases.demo.enabled", "--value", "false"]);

    const applyCalls = calls.filter((c) => c.command === "aliases:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "aliases:demo"]);

    expect(toastSuccessSpy.mock.calls.at(-1)?.[0]).toContain("同步 demo");
  });

  it("policy:set 被引擎拒绝时不执行 apply", async () => {
    const rejected = routes();
    rejected["policy:set"] = makeEnvelope(
      {},
      {
        command: "policy.set",
        ok: false,
        errors: [{ code: "POLICY_WRITE_DENIED", element: null, message: "拒绝写入" }],
      },
    );
    const { calls } = renderWithEngine(<DomainApplyPage domain="aliases" title="Aliases" description="desc" enableToggles />, rejected);

    await userEvent.click(await screen.findByTestId("toggle-aliases:demo"));
    await waitFor(() => expect(calls.some((c) => c.command === "policy:set")).toBe(true));

    expect(calls.some((c) => c.command === "aliases:apply")).toBe(false);
  });

  it("policy:set 桥层失败时不执行 apply", async () => {
    const broken: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    broken["policy:set"] = () => Promise.reject(new Error("bridge down"));
    const { calls } = renderWithEngine(<DomainApplyPage domain="aliases" title="Aliases" description="desc" enableToggles />, broken);

    await userEvent.click(await screen.findByTestId("toggle-aliases:demo"));
    await waitFor(() => expect(calls.some((c) => c.command === "policy:set")).toBe(true));

    expect(calls.some((c) => c.command === "aliases:apply")).toBe(false);
  });

  it("apply 在途期间禁用开关与全局同步按钮", async () => {
    let release: (() => void) | null = null;
    const gated: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    gated["aliases:apply"] = () =>
      new Promise<Envelope>((resolve) => {
        release = () => resolve(makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }));
      });

    renderWithEngine(<DomainApplyPage domain="aliases" title="Aliases" description="desc" enableToggles />, gated);

    await userEvent.click(await screen.findByTestId("toggle-aliases:demo"));
    expect(screen.getByTestId("toggle-aliases:demo")).toBeDisabled();
    expect(screen.getByTestId("apply-aliases-button")).toBeDisabled();

    release!();
    await waitFor(() => expect(screen.getByTestId("toggle-aliases:demo")).toBeEnabled());
    expect(screen.getByTestId("apply-aliases-button")).toBeEnabled();
  });

  it("同一 tick 连点只产生一次 policy:set 与一次 apply", async () => {
    const { calls } = renderWithEngine(<DomainApplyPage domain="aliases" title="Aliases" description="desc" enableToggles />, routes());
    const toggle = await screen.findByTestId("toggle-aliases:demo");

    fireEvent.click(toggle);
    fireEvent.click(toggle);
    fireEvent.click(toggle);

    await waitFor(() => expect(calls.filter((c) => c.command === "aliases:apply")).toHaveLength(1));
    expect(calls.filter((c) => c.command === "policy:set")).toHaveLength(1);
    expect(calls.filter((c) => c.command === "aliases:apply")[0].args).toEqual(["--only", "aliases:demo"]);
  });

  it("policy:set 失败后释放门闩，可再次点击重试", async () => {
    let attempt = 0;
    const brokenThenOk: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> = routes();
    brokenThenOk["policy:set"] = () => {
      attempt += 1;
      return attempt === 1 ? Promise.reject(new Error("bridge down")) : Promise.resolve(routes()["policy:set"]);
    };

    const { calls } = renderWithEngine(<DomainApplyPage domain="aliases" title="Aliases" description="desc" enableToggles />, brokenThenOk);
    const toggle = await screen.findByTestId("toggle-aliases:demo");

    fireEvent.click(toggle);
    await waitFor(() => expect(calls.filter((c) => c.command === "policy:set")).toHaveLength(1));

    await waitFor(() => expect(toggle).toBeEnabled());
    fireEvent.click(toggle);

    await waitFor(() => expect(calls.filter((c) => c.command === "aliases:apply")).toHaveLength(1));
    expect(calls.filter((c) => c.command === "policy:set")).toHaveLength(2);
  });
});
