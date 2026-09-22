import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import SecretsPage from "../pages/SecretsPage";
import { setEngineClient } from "../lib/engine";
import { ExecutionProvider } from "../lib/execution";
import { createQueryClient } from "../lib/queries";
import { createMswEngineClient, startEngineServer, type EngineRoute } from "./engine-test-client";
import { makeElement, makeEnvelope } from "./fixtures";
import type { EngineElement } from "../lib/contract";

const servers: Array<ReturnType<typeof startEngineServer>> = [];

afterEach(() => {
  while (servers.length > 0) {
    servers.pop()?.close();
  }
});

function renderSecrets(routes: Record<string, EngineRoute>, calls?: { command: string; args: string[] }[]) {
  servers.push(startEngineServer(routes));
  const base = createMswEngineClient();
  setEngineClient(
    calls
      ? {
          async run(command, args = [], options) {
            calls.push({ command, args: [...args] });
            return base.run(command, args, options);
          },
        }
      : base,
  );
  const queryClient = createQueryClient({ retry: false });
  return render(
    <QueryClientProvider client={queryClient}>
      {/* 页面会配置外部文件并写入取值，执行通道需要 ExecutionProvider */}
      <ExecutionProvider>
        <SecretsPage />
      </ExecutionProvider>
    </QueryClientProvider>,
  );
}

function mcpEnvElement(
  variables: unknown[],
  unresolved: number,
  state: EngineElement["state"],
  source: Record<string, unknown> = { configured: true, path: "~/private/secrets.env", ready: true, error: null },
  legacyDatabaseValues = 0,
) {
  return makeElement({
    id: "secrets:mcp-env",
    domain: "secrets",
    spec: { sources: ["process env"] },
    policy: { enabled: true },
    observed: { variables, unresolved, source, legacy_database_values: legacyDatabaseValues },
    state,
  });
}

describe("SecretsPage", () => {
  it("无变量时显示空态且全部为已配置", async () => {
    renderSecrets({
      "secrets:status": makeEnvelope(
        { domain: "secrets", elements: [mcpEnvElement([], 0, "managed-ok")] },
        { command: "secrets.status" },
      ),
    });
    await waitFor(() => expect(screen.getByTestId("secrets-env-matrix")).toBeInTheDocument());
    expect(screen.getByText("没有需要配置的变量")).toBeInTheDocument();
    expect(screen.getByTestId("env-unresolved-summary")).toHaveTextContent("全部 0 个变量已配置");
  });

  it("混合已配置/未配置时按行展示状态且不渲染值", async () => {
    const sentinel = "SENTINEL-SECRET-VALUE-9c31";
    renderSecrets({
      "secrets:status": makeEnvelope(
        {
          domain: "secrets",
          elements: [
            mcpEnvElement(
              [
                { server: "grok-search", env_key: "GROK_API_KEY", placeholder: "GROK_API_KEY", resolved: true, stored: true },
                { server: "grok-search", env_key: "TAVILY_API_KEY", placeholder: "TAVILY_API_KEY", resolved: false },
                // 哨兵：即使后端误带 value 字段，页面也绝不渲染
                { server: "other", env_key: "OTHER_KEY", placeholder: "OTHER_KEY", resolved: false, value: sentinel },
              ],
              2,
              "missing",
            ),
          ],
        },
        { command: "secrets.status" },
      ),
    });
    await waitFor(() => expect(screen.getByTestId("secrets-env-matrix")).toBeInTheDocument());
    expect(screen.getByTestId("env-unresolved-summary")).toHaveTextContent("2 个变量未配置");
    expect(screen.getByText("GROK_API_KEY")).toBeInTheDocument();
    expect(screen.getByText("TAVILY_API_KEY")).toBeInTheDocument();
    // 区分「在这里填过」与「来自进程环境」：用户要知道改哪儿才有用
    expect(screen.getAllByText("外部文件")).toHaveLength(1);
    expect(screen.getAllByText("未配置")).toHaveLength(2);
    expect(screen.queryByText(sentinel)).not.toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain(sentinel);
  });

  it("后端错误时展示错误面板", async () => {
    renderSecrets({
      "secrets:status": makeEnvelope(
        { domain: "secrets", elements: [] },
        {
          command: "secrets.status",
          ok: false,
          errors: [{ code: "SECRET_ERROR", element: null, message: "MCP 环境变量定义存在非法内容（已脱敏），拒绝输出状态" }],
        },
      ),
    });
    await waitFor(() => expect(screen.getByText(/密钥状态读取失败/)).toBeInTheDocument());
    expect(screen.getByText(/已脱敏/)).toBeInTheDocument();
  });

  it("明确展示外部密钥文件及连接状态", async () => {
    renderSecrets({
      "secrets:status": makeEnvelope(
        { domain: "secrets", elements: [mcpEnvElement([], 0, "managed-ok")] },
        { command: "secrets.status" },
      ),
    });
    await waitFor(() => expect(screen.getByTestId("secrets-source-card")).toBeInTheDocument());
    expect(screen.getByTestId("secrets-source-path")).toHaveValue("~/private/secrets.env");
    expect(screen.getByTestId("secrets-source-status")).toHaveTextContent("已连接");
    expect(screen.getByText(/只保存路径/)).toBeInTheDocument();
  });

  it("验证新路径后只向后端提交用户选择的外部文件", async () => {
    const calls: { command: string; args: string[] }[] = [];
    renderSecrets({
      "secrets:status": makeEnvelope(
        { domain: "secrets", elements: [mcpEnvElement([], 0, "managed-ok")] },
        { command: "secrets.status" },
      ),
      "secrets:source": makeEnvelope(
        { domain: "secrets", source: { configured: true, path: "~/private/next.env", ready: true } },
        { command: "secrets.source" },
      ),
    }, calls);

    const input = await screen.findByTestId("secrets-source-path");
    await userEvent.clear(input);
    await userEvent.type(input, "~/private/next.env");
    await userEvent.click(screen.getByTestId("secrets-source-save"));

    await waitFor(() => expect(calls.some((call) => call.command === "secrets:source")).toBe(true));
    expect(calls.find((call) => call.command === "secrets:source")?.args).toEqual([
      "--path",
      "~/private/next.env",
    ]);
  });

  it("旧数据库密钥必须输入确认词后才执行迁移", async () => {
    const calls: { command: string; args: string[] }[] = [];
    renderSecrets({
      "secrets:status": makeEnvelope(
        { domain: "secrets", elements: [mcpEnvElement([], 0, "managed-ok", undefined, 2)] },
        { command: "secrets.status" },
      ),
      "secrets:migrate-legacy": makeEnvelope(
        { domain: "secrets", migrated: 2 },
        { command: "secrets.migrate-legacy" },
      ),
    }, calls);

    await userEvent.click(await screen.findByTestId("migrate-legacy-secrets"));
    const dialog = await screen.findByRole("dialog", { name: "迁移旧数据库密钥" });
    expect(dialog).toHaveTextContent("全部成功后才删除数据库里的旧密钥表");
    expect(screen.getByTestId("confirm-modal-confirm")).toBeDisabled();
    expect(calls.some((call) => call.command === "secrets:migrate-legacy")).toBe(false);

    await userEvent.type(screen.getByTestId("confirm-modal-input"), "迁移");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(calls.some((call) => call.command === "secrets:migrate-legacy")).toBe(true));
    expect(calls.find((call) => call.command === "secrets:migrate-legacy")?.args).toEqual([]);
  });

  it("填取值：明确写入外部文件且不回显，保存走 secrets:set", async () => {
    const calls: { command: string; args: string[] }[] = [];
    renderSecrets({
      "secrets:status": makeEnvelope(
        {
          domain: "secrets",
          elements: [
            mcpEnvElement(
              [{ server: "grok-search", env_key: "GROK_API_KEY", placeholder: "GROK_API_KEY", resolved: false, stored: false }],
              1,
              "missing",
            ),
          ],
        },
        { command: "secrets.status" },
      ),
      "secrets:set": makeEnvelope({ domain: "secrets", variable: "GROK_API_KEY", stored: true }, { command: "secrets.set" }),
    }, calls);

    await waitFor(() => expect(screen.getByTestId("secrets-env-matrix")).toBeInTheDocument());
    await userEvent.click(screen.getByTestId("fill-secret-GROK_API_KEY"));
    const input = await screen.findByTestId("secret-value-input");
    // 输入框是密码型：取值不以明文呈现
    expect(input).toHaveAttribute("type", "password");
    expect(screen.getByRole("dialog")).toHaveTextContent("只写入“密钥”页选择的外部文件");
    await userEvent.type(input, "value-under-test");
    await userEvent.click(screen.getByTestId("secret-value-save"));

    await waitFor(() => expect(calls.some((call) => call.command === "secrets:set")).toBe(true));
    const call = calls.find((item) => item.command === "secrets:set")!;
    expect(call.args).toEqual(["--var", "GROK_API_KEY", "--value", "value-under-test"]);
  });

  it("未选择外部文件时禁止填写，并说明先做什么", async () => {
    renderSecrets({
      "secrets:status": makeEnvelope(
        {
          domain: "secrets",
          elements: [
            mcpEnvElement(
              [{ server: "grok-search", env_key: "GROK_API_KEY", placeholder: "GROK_API_KEY", resolved: false, stored: false }],
              1,
              "missing",
              { configured: false, path: null, ready: false, error: null },
            ),
          ],
        },
        { command: "secrets.status" },
      ),
    });
    const button = await screen.findByTestId("fill-secret-GROK_API_KEY");
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("title", "先在上方选择可用的外部密钥文件");
    expect(screen.getByTestId("secrets-source-status")).toHaveTextContent("尚未选择");
  });
});
