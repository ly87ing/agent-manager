import { describe, expect, it } from "vitest";
import { fireEvent } from "@testing-library/react";
import SyncPage from "../pages/SyncPage";
import { makeAuditData, makeEnvelope, makeMode, makeProfileData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import type { Envelope, ProfileData } from "../lib/contract";

/** 多设备同步：差异对比表 + 四个方向按钮 + 键级以远端为准 + offline 禁用。 */

const DIFF: NonNullable<ProfileData["diff"]> = [
  { key: "agents", change: "none", local_type: "object", remote_type: "object", local_digest: "aa11bb22cc33", remote_digest: "aa11bb22cc33" },
  { key: "mcp", change: "both", local_type: "object", remote_type: "object", local_digest: "dd44ee55ff66", remote_digest: "001122334455" },
  { key: "skills", change: "local", local_type: "object", remote_type: "absent", local_digest: "66778899aabb", remote_digest: null },
  { key: "browser_us_sync", change: "remote", local_type: "absent", remote_type: "object", local_digest: null, remote_digest: "cc33dd44ee55" },
];

function routes(profileOverrides: Partial<ProfileData> = {}, offline = false): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline }) }),
    "profile:status": makeEnvelope(makeProfileData({ state: "diverged", ahead: 1, behind: 1, diff: DIFF, ...profileOverrides }), { command: "profile.status" }),
    "profile:pull": makeEnvelope(makeProfileData({ verb: "pull", state: "clean" }), { command: "profile.pull" }),
    "profile:push": makeEnvelope(makeProfileData({ verb: "push", state: "clean", published: true, gate: { scanned: true, findings: 0 } }), { command: "profile.push" }),
  };
}

describe("多设备同步·差异与方向控制", () => {
  it("差异表渲染：四种状态徽章 + 两侧类型与指纹截断，值不出现", async () => {
    renderWithEngine(<SyncPage />, routes());
    const table = await screen.findByTestId("profile-diff-table");
    expect(withinRow("agents").getByText("一致")).toBeInTheDocument();
    expect(withinRow("mcp").getByText("两边都改")).toBeInTheDocument();
    expect(withinRow("skills").getByText("仅本机改")).toBeInTheDocument();
    expect(withinRow("browser_us_sync").getByText("仅远端改")).toBeInTheDocument();
    // diff 指纹本来就是引擎截好的 12 位，原样展示；值永不出现
    expect(table).toHaveTextContent("dd44ee55ff66");
    // 缺失侧 absent 无指纹
    expect(rowOf("skills")).toHaveTextContent("absent");

    function withinRow(key: string) {
      return within(screen.getByTestId(`diff-${key}`));
    }
    function rowOf(key: string) {
      return screen.getByTestId(`diff-${key}`);
    }
  });

  it("四个方向按钮的确认形态：覆盖/强制推送二次确认；强制推送仅「两边都有改动」时可用", async () => {
    const { calls } = renderWithEngine(<SyncPage />, routes());

    // 拉取（合并）：单次确认
    await userEvent.click(await screen.findByTestId("profile-pull-button"));
    let dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("拉取远端改动");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    expect(calls.filter((c) => c.command === "profile:pull").pop()!.args).toEqual([]);

    // 以远端覆盖本地：须输入「覆盖」
    await userEvent.click(screen.getByTestId("profile-accept-remote-button"));
    dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("这台电脑上未推送的改动会被覆盖掉");
    expect(screen.getByTestId("confirm-modal-confirm")).toBeDisabled();
    fireEvent.change(screen.getByTestId("confirm-modal-input"), { target: { value: "覆盖" } });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    expect(calls.filter((c) => c.command === "profile:pull").pop()!.args).toEqual(["--accept-remote"]);

    // 分叉状态下强制推送可用，须输入「强制推送」
    expect(screen.getByTestId("profile-force-push-button")).toBeEnabled();
    await userEvent.click(screen.getByTestId("profile-force-push-button"));
    dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("远端未拉取的改动会丢失");
    fireEvent.change(screen.getByTestId("confirm-modal-input"), { target: { value: "强制推送" } });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    expect(calls.filter((c) => c.command === "profile:push").pop()!.args).toEqual(["--force"]);
  });

  it("「两边都改」键行内「以远端为准」→ pull --accept-remote --only <key> → 状态刷新", async () => {
    const { calls } = renderWithEngine(<SyncPage />, routes());
    await userEvent.click(await screen.findByTestId("accept-remote-mcp"));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("只把「mcp」这一项改成远端的值");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    const pullCall = calls.filter((c) => c.command === "profile:pull").pop();
    expect(pullCall!.args).toEqual(["--accept-remote", "--only", "mcp"]);
  });

  it("clean（无分叉）时强制推送禁用", async () => {
    renderWithEngine(<SyncPage />, routes({ state: "clean", ahead: 0, behind: 0, diff: DIFF.map((d) => ({ ...d, change: "none" as const })) }));
    expect(await screen.findByTestId("profile-force-push-button")).toBeDisabled();
    expect(screen.getByTestId("profile-pull-button")).toBeEnabled();
  });

  it("offline：四个方向按钮与键级按钮全部禁用并标注", async () => {
    renderWithEngine(<SyncPage />, routes({ state: "local-only", ahead: null, behind: null, diff: undefined }, true));
    expect(await screen.findByTestId("profile-pull-button")).toBeDisabled();
    expect(screen.getByTestId("profile-accept-remote-button")).toBeDisabled();
    expect(screen.getByTestId("profile-push-button")).toBeDisabled();
    expect(screen.getByTestId("profile-force-push-button")).toBeDisabled();
    expect(screen.getByText(/离线模式：需要联网的操作已禁用/)).toBeInTheDocument();
  });
});
