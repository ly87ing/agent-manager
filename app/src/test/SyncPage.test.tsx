import { describe, expect, it } from "vitest";
import SyncPage from "../pages/SyncPage";
import { makeAuditData, makeEnvelope, makeMode, makeProfileData } from "./fixtures";
import { renderWithEngine, screen } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

function routes(overrides: Record<string, Envelope> = {}) {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit" }),
    "profile:status": makeEnvelope(
      makeProfileData({
        conflicts: [
          {
            key: "mcp.grok-search.enabled",
            base: { type: "boolean", digest: "sha256:base00aa11bb22cc33dd44ee55" },
            local: { type: "boolean", digest: "sha256:local1aa11bb22cc33dd44ee55" },
            remote: { type: "boolean", digest: "sha256:remote2aa11bb22cc33dd44ee" },
          },
        ],
      }),
      { command: "profile.status" },
    ),
    ...overrides,
  };
}

describe("配置同步页", () => {
  it("状态卡渲染（脱敏地址/状态/ahead/behind），冲突 UI 只读：展示 digest 摘要与保留本地语义，无「以远端为准」动作按钮", async () => {
    renderWithEngine(<SyncPage />, routes());
    const card = await screen.findByTestId("profile-status-card");
    expect(card).toHaveTextContent("git.example.com");
    expect(screen.getByTestId("profile-state")).toHaveTextContent("已同步");

    const conflicts = screen.getByTestId("profile-conflicts");
    expect(conflicts).toHaveTextContent("mcp.grok-search.enabled");
    expect(conflicts).toHaveTextContent("sha256:…");
    // digest 截断形式（shortDigest：前 7 字符含 "sha256:" 前缀 + 后 6 位）
    expect(conflicts).toHaveTextContent(/sha256:…44ee55/);
    expect(conflicts).toHaveTextContent(/sha256:…dd44ee/);
    // 只读语义：保留本地说明 + 策略编辑器引导，无任何解决按钮
    expect(conflicts).toHaveTextContent(/保留这台电脑的值/);
    expect(conflicts).toHaveTextContent(/「个性化配置」页/);
    expect(conflicts.querySelector("button")).toBeNull();
  });

  it("offline：pull/push 禁用并标注", async () => {
    const offlineRoutes = routes();
    offlineRoutes["system:audit"] = makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline: true }) });
    offlineRoutes["profile:status"] = makeEnvelope(makeProfileData({ state: "local-only", ahead: null, behind: null }), { command: "profile.status" });
    renderWithEngine(<SyncPage />, offlineRoutes);

    expect(await screen.findByTestId("profile-pull-button")).toBeDisabled();
    expect(screen.getByTestId("profile-push-button")).toBeDisabled();
    expect(screen.getByText(/离线模式：需要联网的操作已禁用/)).toBeInTheDocument();
    expect(screen.getByTestId("profile-state")).toHaveTextContent("仅本机（离线）");
  });

  it("后端未配置：展示配置引导卡", async () => {
    const unconfigured = routes({
      "profile:status": makeEnvelope({}, {
        command: "profile.status",
        ok: false,
        errors: [{ code: "PROFILE_BACKEND_UNCONFIGURED", element: null, message: "未配置同步后端地址" }],
      }),
    });
    renderWithEngine(<SyncPage />, unconfigured);
    expect(await screen.findByText("还没有配置同步后端")).toBeInTheDocument();
    expect(screen.getByText(/PROFILE_SYNC_REPO_URL/)).toBeInTheDocument();
    expect(screen.queryByTestId("profile-pull-button")).toBeNull();
  });
});
