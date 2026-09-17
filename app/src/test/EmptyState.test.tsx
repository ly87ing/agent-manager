import { render, screen } from "@testing-library/react";
import { SearchX } from "lucide-react";
import { describe, expect, it } from "vitest";
import { EmptyState } from "../components/EmptyState";

describe("EmptyState", () => {
  it("渲染圆形图标底、标题与描述", () => {
    const { container } = render(
      <EmptyState icon={SearchX} title="没有结果" description="换个筛选条件试试" />,
    );
    expect(screen.getByText("没有结果")).toBeInTheDocument();
    expect(screen.getByText("换个筛选条件试试")).toBeInTheDocument();
    // 圆形 muted 图标底
    const iconWrap = container.querySelector(".rounded-full.bg-muted");
    expect(iconWrap).not.toBeNull();
    // 默认（非 compact）为虚线面板
    expect(container.firstChild).toHaveClass("border-dashed");
  });

  it("compact 变体不带虚线面板（用于已带边框的容器内）", () => {
    const { container } = render(<EmptyState compact title="空" />);
    expect(container.firstChild).not.toHaveClass("border-dashed");
  });

  it("渲染可选动作区与 testId", () => {
    render(
      <EmptyState testId="demo-empty" title="空" actions={<button type="button">立即创建</button>} />,
    );
    const wrapper = screen.getByTestId("demo-empty");
    expect(wrapper).toHaveTextContent("空");
    expect(screen.getByRole("button", { name: "立即创建" })).toBeInTheDocument();
  });

  it("无描述与动作时不渲染对应区块", () => {
    const { container } = render(<EmptyState title="只有标题" />);
    expect(screen.getByText("只有标题")).toBeInTheDocument();
    expect(container.querySelectorAll("p").length).toBe(1);
  });
});
