import { describe, expect, it } from "vitest";
import { fireEvent } from "@testing-library/react";
import PolicyPage from "../pages/PolicyPage";
import { makeEnvelope, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

function routes(): Record<string, Envelope> {
  return {
    "policy:get": makeEnvelope(makePolicyData(), { command: "policy.get" }),
    "policy:set": makeEnvelope(
      makePolicyData({ layers: null, effective: null, attribution: null, layer: "synced", key: "mcp.grok-search.enabled", value: false, path: "/repo/config/manager.synced.json" }),
      { command: "policy.set" },
    ),
  };
}

describe("策略编辑器", () => {
  it("三层视图：层归属徽标（基线灰/共享蓝/仅本机橙）与三层原文卡", async () => {
    renderWithEngine(<PolicyPage />, routes());
    // 顶层键归属：browser_us_sync 来自 local 层
    const row = (await screen.findByTestId("edit-browser_us_sync.enabled")).closest("tr")!;
    expect(row).toHaveTextContent("仅这台电脑");
    const agentsRow = screen.getByTestId("edit-agents.claude-code.enabled").closest("tr")!;
    expect(agentsRow).toHaveTextContent("仓库默认值");
    // 三层原文卡 + baseline 只读
    const baselineCard = screen.getByTestId("layer-card-baseline");
    expect(baselineCard).toHaveTextContent("只读");
    expect(screen.getByTestId("layer-card-synced")).toHaveTextContent("（文件不存在）");
    expect(screen.getByTestId("layer-card-local")).toHaveTextContent("browser_us_sync");
  });

  it("编辑任意键：落点选择共享层 → policy:set 参数正确", async () => {
    const { calls } = renderWithEngine(<PolicyPage />, routes());
    await userEvent.click(await screen.findByTestId("edit-mcp.grok-search.enabled"));
    // 值预填为当前值
    expect(screen.getByTestId("policy-value-input")).toHaveValue("true");
    // 选择共享层落点
    await userEvent.click(screen.getByTestId("policy-layer-synced"));
    // 改成 false
    const valueInput = screen.getByTestId("policy-value-input");
    await userEvent.clear(valueInput);
    await userEvent.type(valueInput, "false");
    await userEvent.click(screen.getByTestId("policy-set-submit"));

    const setCall = calls.find((call) => call.command === "policy:set");
    expect(setCall!.args).toEqual(["--layer", "synced", "--key", "mcp.grok-search.enabled", "--value", "false"]);
    // 成功后 dialog 关闭
    expect(await screen.findByTestId("edit-mcp.grok-search.enabled")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("客户端 JSON 校验：非法值不发起 policy:set", async () => {
    const { calls } = renderWithEngine(<PolicyPage />, routes());
    await userEvent.click(await screen.findByTestId("edit-mcp.grok-search.enabled"));
    const valueInput = screen.getByTestId("policy-value-input");
    fireEvent.change(valueInput, { target: { value: "{not json" } });
    await userEvent.click(screen.getByTestId("policy-set-submit"));
    expect(await screen.findByText(/值不是合法 JSON/)).toBeInTheDocument();
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("引擎门禁拒绝（疑似明文）时展示引擎 error 且弹窗保留", async () => {
    const gated = routes();
    gated["policy:set"] = makeEnvelope({}, {
      command: "policy.set",
      ok: false,
      errors: [{ code: "POLICY_SECRET_GATE", element: null, message: "疑似明文敏感信息（openai_like_key），拒绝写入策略层" }],
    });
    renderWithEngine(<PolicyPage />, gated);
    await userEvent.click(await screen.findByTestId("edit-mcp.grok-search.enabled"));
    await userEvent.click(screen.getByTestId("policy-set-submit"));
    expect(await screen.findByText("POLICY_SECRET_GATE")).toBeInTheDocument();
    expect(screen.getByText(/拒绝写入策略层/)).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});
