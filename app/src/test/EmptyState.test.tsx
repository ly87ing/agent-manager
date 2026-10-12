import { render, screen } from "@testing-library/react";
import { SearchX } from "lucide-react";
import { describe, expect, it } from "vitest";
import { EmptyState } from "../components/EmptyState";

describe("EmptyState", () => {
  it("renders an inline guide with an icon, title and description", () => {
    const { container } = render(
      <EmptyState icon={SearchX} title="没有结果" description="换个筛选条件试试" />,
    );
    expect(screen.getByText("没有结果")).toBeInTheDocument();
    expect(screen.getByText("换个筛选条件试试")).toBeInTheDocument();
    const iconWrap = container.querySelector(".rounded-md.bg-muted");
    expect(iconWrap).not.toBeNull();
    expect(container.firstChild).toHaveClass("border", "text-left", "flex-wrap");
    expect(container.firstChild).not.toHaveClass("border-dashed");
  });

  it("omits the border inside an existing panel", () => {
    const { container } = render(<EmptyState compact title="空" />);
    expect(container.firstChild).not.toHaveClass("border");
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
