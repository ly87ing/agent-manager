import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ErrorPanel } from "../components/ErrorPanel";

describe("ErrorPanel", () => {
  it("渲染 envelope errors 的 code / element / message", () => {
    render(
      <ErrorPanel
        errors={[
          { code: "AUDIT_ERRORS", element: null, message: "doctor 报告 6 个错误" },
          { code: "MCP_ENV_UNRESOLVED", element: "mcp:grok-search", message: "2 个 env 占位符未解析" },
        ]}
      />,
    );
    expect(screen.getByText("AUDIT_ERRORS")).toBeInTheDocument();
    expect(screen.getByText("doctor 报告 6 个错误")).toBeInTheDocument();
    expect(screen.getByText("MCP_ENV_UNRESOLVED")).toBeInTheDocument();
    expect(screen.getByText("mcp:grok-search")).toBeInTheDocument();
  });

  it("空 errors 不渲染", () => {
    const { container } = render(<ErrorPanel errors={[]} />);
    expect(container.firstChild).toBeNull();
  });
});
