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
  it("接入与独立环境用到的密钥：列出用在哪里、有没有取值，并说明改了之后要重新写入独立环境", async () => {
    const element = mcpEnvElement([], 0, "managed-ok");
    (element.observed as Record<string, unknown>).provider_keys = [
      {
        variable: "GLM_API_KEY",
        resolved: true,
        stored: true,
        used_by: [
          { kind: "connection", agent: "claude-code", name: "glm", label: "智谱" },
          { kind: "profile", id: "cc-glm", label: "Claude · GLM" },
        ],
      },
      { variable: "KIMI_API_KEY", resolved: false, stored: false, used_by: [{ kind: "profile", id: "cx-kimi", label: "Codex · Kimi" }] },
    ];
    renderSecrets({
      "secrets:status": makeEnvelope({ domain: "secrets", elements: [element] }, { command: "secrets.status" }),
    });
    const section = await screen.findByTestId("secrets-provider-keys");
    expect(section).toHaveTextContent("用到它的独立环境卡片会提示「密钥换过了」，点卡片上的「写入新密钥」才生效");
    expect(screen.getByTestId("provider-key-GLM_API_KEY")).toHaveTextContent("接入 智谱（claude-code）、独立环境 Claude · GLM（cc-glm）");
    expect(screen.getByTestId("provider-key-GLM_API_KEY")).toHaveTextContent("外部文件");
    expect(screen.getByTestId("provider-key-KIMI_API_KEY")).toHaveTextContent("未配置");
    await userEvent.click(screen.getByTestId("fill-provider-key-KIMI_API_KEY"));
    expect(await screen.findByRole("dialog")).toHaveTextContent("KIMI_API_KEY");
  });

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
                { server: "example-search", env_key: "SEARCH_API_KEY", placeholder: "SEARCH_API_KEY", resolved: true, stored: true },
                { server: "example-search", env_key: "CRAWL_API_KEY", placeholder: "CRAWL_API_KEY", resolved: false },
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
    expect(screen.getByText("SEARCH_API_KEY")).toBeInTheDocument();
    expect(screen.getByText("CRAWL_API_KEY")).toBeInTheDocument();
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
              [{ server: "example-search", env_key: "SEARCH_API_KEY", placeholder: "SEARCH_API_KEY", resolved: false, stored: false }],
              1,
              "missing",
            ),
          ],
        },
        { command: "secrets.status" },
      ),
      "secrets:set": makeEnvelope({ domain: "secrets", variable: "SEARCH_API_KEY", stored: true }, { command: "secrets.set" }),
    }, calls);

    await waitFor(() => expect(screen.getByTestId("secrets-env-matrix")).toBeInTheDocument());
    await userEvent.click(screen.getByTestId("fill-secret-SEARCH_API_KEY"));
    const input = await screen.findByTestId("secret-value-input");
    // 输入框是密码型：取值不以明文呈现
    expect(input).toHaveAttribute("type", "password");
    expect(screen.getByRole("dialog")).toHaveTextContent("只写入“设置 → 密钥”选择的外部文件");
    await userEvent.type(input, "value-under-test");
    await userEvent.click(screen.getByTestId("secret-value-save"));

    await waitFor(() => expect(calls.some((call) => call.command === "secrets:set")).toBe(true));
    const call = calls.find((item) => item.command === "secrets:set")!;
    expect(call.args).toEqual(["--var", "SEARCH_API_KEY", "--value", "value-under-test"]);
  });

  it("未选择外部文件时禁止填写，并说明先做什么", async () => {
    renderSecrets({
      "secrets:status": makeEnvelope(
        {
          domain: "secrets",
          elements: [
            mcpEnvElement(
              [{ server: "example-search", env_key: "SEARCH_API_KEY", placeholder: "SEARCH_API_KEY", resolved: false, stored: false }],
              1,
              "missing",
              { configured: false, path: null, ready: false, error: null },
            ),
          ],
        },
        { command: "secrets.status" },
      ),
    });
    const button = await screen.findByTestId("fill-secret-SEARCH_API_KEY");
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("title", "先在上方选择可用的外部密钥文件");
    expect(screen.getByTestId("secrets-source-status")).toHaveTextContent("尚未选择");
  });
});

describe("SecretsPage 新建密钥文件", () => {
  it("还没有密钥文件时可以就地新建：留空用默认位置，带 --create", async () => {
    const calls: { command: string; args: string[] }[] = [];
    const element = mcpEnvElement([], 0, "managed-ok", { configured: false, path: null, ready: false, error: null });
    renderSecrets(
      {
        "secrets:status": makeEnvelope({ domain: "secrets", elements: [element] }, { command: "secrets.status" }),
        "secrets:source": makeEnvelope({ domain: "secrets", configured: true, created: true }, { command: "secrets.source" }),
      },
      calls,
    );
    const create = await screen.findByTestId("secrets-source-create");
    expect(screen.getByTestId("secrets-source-card")).toHaveTextContent("也可选择你自己的私有 secrets 仓库中的 .env 文件");
    await userEvent.click(create);
    await waitFor(() => expect(calls.some((call) => call.command === "secrets:source")).toBe(true));
    expect(calls.find((call) => call.command === "secrets:source")!.args).toEqual([
      "--path",
      "~/.config/agent-manager/secrets.env",
      "--create",
    ]);
  });

  it("已经选了密钥文件就不再给新建按钮", async () => {
    renderSecrets({
      "secrets:status": makeEnvelope(
        { domain: "secrets", elements: [mcpEnvElement([], 0, "managed-ok")] },
        { command: "secrets.status" },
      ),
    });
    await screen.findByTestId("secrets-source-status");
    expect(screen.queryByTestId("secrets-source-create")).toBeNull();
  });
});
