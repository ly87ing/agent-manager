import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import PlanPreviewModal from "../components/PlanPreviewModal";
import { makePlanAction } from "./fixtures";

function renderModal(actions: ReturnType<typeof makePlanAction>[], extra: Partial<Parameters<typeof PlanPreviewModal>[0]> = {}) {
  const onConfirm = vi.fn();
  const onClose = vi.fn();
  render(
    <PlanPreviewModal
      title="测试预览"
      plans={[{ domain: "mcp", actions }]}
      executing={false}
      onConfirm={onConfirm}
      onClose={onClose}
      {...extra}
    />,
  );
  return { onConfirm, onClose };
}

describe("PlanPreviewModal", () => {
  it("渲染动作清单与五级风险 summary 计数", () => {
    renderModal([
      makePlanAction({ op: "write-file", risk: "write-local", element: "mcp:a", target: "/tmp/a.json", note: "注入 mcp:a" }),
      makePlanAction({ op: "run-cmd", risk: "network", element: "agents:b", target: null, cmd: "npm i -g b", note: "升级 b" }),
      makePlanAction({ op: "symlink", risk: "exec-local", element: "skill:c/d", target: "/tmp/d" }),
    ]);
    expect(screen.getByTestId("plan-summary-write-local")).toHaveTextContent("修改配置文件 1");
    expect(screen.getByTestId("plan-summary-network")).toHaveTextContent("需要联网 1");
    expect(screen.getByTestId("plan-summary-exec-local")).toHaveTextContent("运行本地命令 1");
    expect(screen.getByTestId("plan-summary-install-remove")).toHaveTextContent("安装/卸载软件 0");
    expect(screen.getByText("注入 mcp:a")).toBeInTheDocument();
    expect(screen.getByText("npm i -g b")).toBeInTheDocument();
  });

  it("write-local 级：普通确认直接可点", async () => {
    const { onConfirm } = renderModal([makePlanAction({})]);
    const button = screen.getByTestId("plan-confirm-button");
    expect(button).toBeEnabled();
    await userEvent.click(button);
    expect(onConfirm).toHaveBeenCalledWith(false);
  });

  it("network 级：须勾选联网知情才可确认", async () => {
    const { onConfirm } = renderModal([makePlanAction({ op: "run-cmd", risk: "network", cmd: "npm view x", target: null })]);
    const button = screen.getByTestId("plan-confirm-button");
    expect(button).toBeDisabled();
    await userEvent.click(screen.getByTestId("network-ack-checkbox"));
    expect(button).toBeEnabled();
    await userEvent.click(button);
    expect(onConfirm).toHaveBeenCalledWith(false);
  });

  it("install-remove 级：二次确认须输入元素 id", async () => {
    const { onConfirm } = renderModal(
      [makePlanAction({ op: "uninstall", risk: "install-remove", element: "agents:qoder", target: null })],
      { requireConfirmText: "agents:qoder" },
    );
    const button = screen.getByTestId("plan-confirm-button");
    expect(button).toBeDisabled();
    const input = screen.getByTestId("install-remove-confirm-input");
    await userEvent.type(input, "agents:qode");
    expect(button).toBeDisabled();
    await userEvent.type(input, "r");
    expect(button).toBeEnabled();
    await userEvent.click(button);
    expect(onConfirm).toHaveBeenCalledWith(false);
  });

  it("演练开关：确认回调携带 dryRun=true", async () => {
    const { onConfirm } = renderModal([makePlanAction({})]);
    await userEvent.click(screen.getByLabelText(/演练模式/));
    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    expect(onConfirm).toHaveBeenCalledWith(true);
  });

  it("打开时采用演练默认值，仍可单次切换", async () => {
    const { onConfirm } = renderModal([makePlanAction({})], { defaultDryRun: true });
    expect(screen.getByLabelText(/演练模式/)).toBeChecked();
    await userEvent.click(screen.getByLabelText(/演练模式/));
    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    expect(onConfirm).toHaveBeenCalledWith(false);
  });

  it("没有操作时禁用确认", () => {
    renderModal([]);
    expect(screen.getByText(/没有需要执行的操作/)).toBeInTheDocument();
    expect(screen.getByTestId("plan-confirm-button")).toBeDisabled();
  });
});
