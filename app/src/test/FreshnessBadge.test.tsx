import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { FreshnessBadge } from "../components/FreshnessBadge";
import { makeFreshness } from "./fixtures";

describe("FreshnessBadge 五态", () => {
  it("current：渲染绿色「已是最新」", () => {
    const { container } = render(
      <FreshnessBadge
        freshness={makeFreshness({ state: "current", current: "2.1.3", upstream: "2.1.3", checked_at: "2026-09-05T10:00:00Z", probe: "npm-view", cache_age_sec: 120 })}
      />,
    );
    const badge = screen.getByText("已是最新");
    expect(badge).toBeInTheDocument();
    expect(badge.className).toContain("success");
    expect(container).toMatchSnapshot();
  });

  it("behind：显示版本箭头 current → upstream", () => {
    render(
      <FreshnessBadge
        freshness={makeFreshness({ state: "behind", current: "1.2.0", upstream: "1.8.0", checked_at: "2026-09-05T10:00:00Z", probe: "npm-view", cache_age_sec: 1820 })}
      />,
    );
    const badge = screen.getByText(/可更新/);
    expect(badge.textContent).toContain("1.2.0");
    expect(badge.textContent).toContain("→");
    expect(badge.textContent).toContain("1.8.0");
    expect(badge.className).toContain("warning");
  });

  it("ahead：蓝色「比官方版还新」", () => {
    render(
      <FreshnessBadge freshness={makeFreshness({ state: "ahead", current: "e4f5g6h", upstream: "a1b2c3d" })} />,
    );
    const badge = screen.getByText(/比官方版还新/);
    expect(badge.textContent).toContain("e4f5g6h");
    expect(badge.className).toContain("info");
  });

  it("unknown：灰显「未检查」", () => {
    render(<FreshnessBadge freshness={makeFreshness({ state: "unknown", checked_at: "2026-09-05T08:00:00Z" })} />);
    const badge = screen.getByText("未检查");
    expect(badge.className).toContain("muted");
  });

  it("unknown 且从未探测过：仍标注「未检查」", () => {
    render(<FreshnessBadge freshness={makeFreshness({ state: "unknown" })} />);
    expect(screen.getByText("未检查")).toBeInTheDocument();
  });

  it("not-applicable：不渲染徽章", () => {
    const { container } = render(<FreshnessBadge freshness={makeFreshness({ state: "not-applicable" })} />);
    expect(container.firstChild).toBeNull();
  });

  it("cache_age_sec 人类化：1820 秒显示「30 分钟前检查」", () => {
    render(<FreshnessBadge freshness={makeFreshness({ state: "current", cache_age_sec: 1820 })} />);
    expect(screen.getByTestId("freshness-cache-age")).toHaveTextContent("30 分钟前检查");
  });

  it("超过 24h TTL 的缓存标记「检查结果较旧」", () => {
    render(<FreshnessBadge freshness={makeFreshness({ state: "current", cache_age_sec: 90000 })} />);
    const age = screen.getByTestId("freshness-cache-age");
    expect(age).toHaveTextContent("检查结果较旧");
    expect(age.className).toContain("warning");
  });
});
