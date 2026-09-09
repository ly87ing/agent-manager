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

  it("警告面板渲染脱敏警告：保留 code 和 element，且不包含假 token", () => {
    const fakeToken = "ghp_1234567890abcdef1234567890abcdef1234";
    const fakeBearer = "Bearer secret-token-xyz-12345";
    render(
      <ErrorPanel
        title="测试警告"
        variant="warning"
        errors={[
          {
            code: "TOKEN_WARNING",
            element: "policy:local",
            message: "检测到凭据被脱敏：token=*** and Bearer ***",
          },
        ]}
      />,
    );
    expect(screen.getByText("TOKEN_WARNING")).toBeInTheDocument();
    expect(screen.getByText("policy:local")).toBeInTheDocument();
    expect(screen.getByText("检测到凭据被脱敏：token=*** and Bearer ***")).toBeInTheDocument();
    expect(screen.queryByText(new RegExp(fakeToken))).toBeNull();
    expect(screen.queryByText(new RegExp(fakeBearer))).toBeNull();
  });
});
