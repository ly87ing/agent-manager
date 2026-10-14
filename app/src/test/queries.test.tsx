import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { setEngineClient } from "../lib/engine";
import { createQueryClient, engineKeys, invalidateEngineQueries, useEngineQuery, useRefreshUpstream } from "../lib/queries";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, userEvent } from "./render-with-engine";

describe("useEngineQuery 请求缓存键", () => {
  it("reports unknown update checks as incomplete rather than success", async () => {
    function Refresh() {
      const refresh = useRefreshUpstream();
      return <button onClick={() => refresh.mutate(undefined)}>Check updates</button>;
    }
    const { calls } = renderWithEngine(<Refresh />, { "system:refresh-upstream": makeEnvelope({ summary: { current: 1, behind: 0, ahead: 0, unknown: 1 }, elements: [] }) });
    await userEvent.click(screen.getByRole("button", { name: "Check updates" }));
    await waitFor(() => expect(document.querySelector('[data-sonner-toast][data-type="warning"]')).toHaveTextContent("未检查 1"));
    expect(document.querySelector('[data-sonner-toast][data-type="success"]')).toBeNull();
    expect(calls.filter((call) => call.command === "system:refresh-upstream")).toHaveLength(1);
  });
  it("MCP 写入使程序依赖列表过期，同时保留不相关的规则缓存", async () => {
    const client = createQueryClient({ retry: false });
    const programs = engineKeys.request("programs:status");
    const rules = engineKeys.request("rules:status");
    client.setQueryData(programs, makeEnvelope({ elements: [] }));
    client.setQueryData(rules, makeEnvelope({ elements: [] }));
    await invalidateEngineQueries(client, "mcp:apply");
    expect(client.getQueryState(programs)?.isInvalidated).toBe(true);
    expect(client.getQueryState(rules)?.isInvalidated).toBe(false);
    client.clear();
  });
  it("同一 QueryClient 下同命令不同 args 各自真实调用并保留各自结果", async () => {
    const calls: { command: string; args: string[] }[] = [];
    setEngineClient({
      async run(command, args = []) {
        calls.push({ command, args: [...args] });
        return makeEnvelope({ value: args.join(" ") }, { command: command.replace(":", ".") });
      },
    });

    function Probe() {
      const first = useEngineQuery<{ value: string }>("mcp:status", ["--only", "first"]);
      const second = useEngineQuery<{ value: string }>("mcp:status", ["--only", "second"]);
      return (
        <>
          <output data-testid="first-result">{first.data?.data.value ?? "pending"}</output>
          <output data-testid="second-result">{second.data?.data.value ?? "pending"}</output>
        </>
      );
    }

    render(
      <QueryClientProvider client={createQueryClient({ retry: false })}>
        <Probe />
      </QueryClientProvider>,
    );

    await waitFor(() => expect(calls).toHaveLength(2));
    expect(calls).toEqual([
      { command: "mcp:status", args: ["--only", "first"] },
      { command: "mcp:status", args: ["--only", "second"] },
    ]);
    expect(await screen.findByTestId("first-result")).toHaveTextContent("--only first");
    expect(await screen.findByTestId("second-result")).toHaveTextContent("--only second");
  });
});
