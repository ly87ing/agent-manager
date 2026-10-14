import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PageLoading } from "../components/PageState";
import { Skeleton } from "../components/Skeleton";

describe("Skeleton", () => {
  it("渲染 pulse 占位块（muted token 色、对辅助技术隐藏）", () => {
    const { container } = render(<Skeleton className="h-4 w-1/3" />);
    const block = container.firstChild as HTMLElement;
    expect(block).toHaveClass("animate-pulse", "rounded-md", "bg-muted", "h-4", "w-1/3");
    expect(block).toHaveAttribute("aria-hidden");
  });
});

describe("PageLoading", () => {
  it("renders a visible loading message and page skeleton", () => {
    const { container } = render(<PageLoading text="正在读取应用设置…" />);
    expect(screen.getByText("正在读取应用设置…")).toBeVisible();
    expect(screen.getByText("正在读取应用设置…")).not.toHaveClass("sr-only");
    expect(screen.getByRole("status")).toBeInTheDocument();
    // 3 张卡片轮廓，配方对齐真实卡片 rounded-lg border bg-card p-5
    const cardOutlines = container.querySelectorAll(".rounded-lg.border.bg-card.p-5");
    expect(cardOutlines.length).toBe(3);
    // 骨架块使用 pulse 动画
    expect(container.querySelectorAll(".animate-pulse.bg-muted").length).toBeGreaterThan(3);
  });
});
