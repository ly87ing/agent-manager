import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { AppErrorBoundary } from "../components/AppErrorBoundary";

const bombState = { shouldThrow: false };

/** 渲染时读取可变开关：边界重试重新渲染时拿到最新值。 */
function Bomb() {
  if (bombState.shouldThrow) throw new Error("boom: 渲染失败");
  return <p>子树正常</p>;
}

describe("AppErrorBoundary", () => {
  it("无错误时正常渲染子组件", () => {
    bombState.shouldThrow = false;
    render(
      <AppErrorBoundary>
        <Bomb />
      </AppErrorBoundary>,
    );
    expect(screen.getByText("子树正常")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("子组件抛错时显示中文恢复界面（说明 + 重试按钮 + 重启提示 + 错误摘要）", () => {
    bombState.shouldThrow = true;
    render(
      <AppErrorBoundary>
        <Bomb />
      </AppErrorBoundary>,
    );
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("界面出现异常");
    expect(alert).toHaveTextContent("boom: 渲染失败");
    expect(alert).toHaveTextContent("重启应用");
    expect(screen.getByRole("button", { name: "重试" })).toBeInTheDocument();
    expect(screen.queryByText("子树正常")).toBeNull();
  });

  it("点击重试后重置错误状态并重新渲染子组件", async () => {
    bombState.shouldThrow = true;
    render(
      <AppErrorBoundary>
        <Bomb />
      </AppErrorBoundary>,
    );
    expect(screen.getByRole("alert")).toBeInTheDocument();

    // 修复抛错原因后重试：边界重置，子树恢复
    bombState.shouldThrow = false;
    await userEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByText("子树正常")).toBeInTheDocument();
  });

  it("重试后子组件仍抛错时保持恢复界面", async () => {
    bombState.shouldThrow = true;
    render(
      <AppErrorBoundary>
        <Bomb />
      </AppErrorBoundary>,
    );
    await userEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(screen.getByRole("alert")).toHaveTextContent("界面出现异常");
  });
});
