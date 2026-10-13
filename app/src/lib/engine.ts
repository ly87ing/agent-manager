import { t } from "./i18n";
import { invoke } from "@tauri-apps/api/core";
import { z } from "zod";
import { CONTRACT_ID, type Envelope } from "./contract";

/**
 * 引擎客户端抽象：生产环境走 Tauri command `engine_invoke`；测试经
 * setEngineClient 注入 fake。
 */

const diagnosticSchema = z
  .object({
    code: z.string(),
    element: z.string().nullable(),
    message: z.string(),
  })
  .strict();

const envelopeSchema = z
  .object({
    contract: z.literal(CONTRACT_ID),
    run_id: z.string(),
    command: z.string(),
    mode: z.object({ offline: z.boolean() }).strict(),
    lock: z.object({ held: z.boolean(), pid: z.number().nullable() }).strict(),
    ok: z.boolean(),
    data: z.record(z.string(), z.unknown()),
    warnings: z.array(diagnosticSchema),
    errors: z.array(diagnosticSchema),
  })
  .strict();

/** Rust 桥返回的结构化错误（序列化后的 EngineError）。 */
export class EngineBridgeError extends Error {
  readonly kind: string;
  readonly exitCode: number | null;
  readonly stderr: string | null;

  constructor(init: { kind?: string; message: string; exit_code?: number | null; stderr?: string | null }) {
    super(init.message);
    this.name = "EngineBridgeError";
    this.kind = init.kind ?? "unknown";
    this.exitCode = init.exit_code ?? null;
    this.stderr = init.stderr ?? null;
  }
}

export interface EngineRunOptions {
  /** GUI 生成的 run id（写路径必传），经 AGENT_MANAGER_RUN_ID 透传到引擎操作记录。 */
  runId?: string;
}

export interface EngineClient {
  run(command: string, args?: string[], options?: EngineRunOptions): Promise<Envelope>;
}

function isBridgeErrorPayload(value: unknown): value is { kind?: string; message: string; exit_code?: number | null; stderr?: string | null } {
  return typeof value === "object" && value !== null && "message" in value;
}

const tauriEngineClient: EngineClient = {
  async run(command, args = [], options = {}) {
    let raw: unknown;
    try {
      raw = await invoke<unknown>("engine_invoke", { command, args, runId: options.runId ?? null });
    } catch (err) {
      if (isBridgeErrorPayload(err)) {
        throw new EngineBridgeError(err);
      }
      throw new EngineBridgeError({ kind: "invoke_failed", message: String(err) });
    }
    const parsed = envelopeSchema.safeParse(raw);
    if (!parsed.success) {
      const issues = parsed.error.issues
        .slice(0, 5)
        .map((issue) => `${issue.path.join(".") || "<root>"}: ${issue.message}`)
        .join("; ");
      throw new EngineBridgeError({
        kind: "contract_violation",
        message: t("引擎输出不符合 Contract v1 envelope：{0}", issues),
      });
    }
    return parsed.data as Envelope;
  },
};

let activeClient: EngineClient = tauriEngineClient;

export function getEngineClient(): EngineClient {
  return activeClient;
}

/** 测试注入 fake client；传 null 恢复为 Tauri 实现。 */
export function setEngineClient(client: EngineClient | null): void {
  activeClient = client ?? tauriEngineClient;
}

export function runEngine<T>(command: string, args?: string[], options?: EngineRunOptions): Promise<Envelope<T>> {
  return getEngineClient().run(command, args, options) as unknown as Promise<Envelope<T>>;
}

export function engineErrorMessage(err: unknown): string {
  if (err instanceof EngineBridgeError) {
    return err.stderr ? `${err.message}（${err.kind}${err.exitCode !== null ? t(", 退出码 {0}", err.exitCode) : ""}）` : err.message;
  }
  return err instanceof Error ? err.message : String(err);
}

/** 把桥错误映射成 envelope diagnostic 形态，供 ErrorPanel 统一渲染。 */
export function toDiagnostic(err: unknown): { code: string; element: null; message: string } {
  const kind = err instanceof EngineBridgeError ? err.kind : "unknown";
  return { code: kind.toUpperCase(), element: null, message: engineErrorMessage(err) };
}

/**
 * dev-only mock 开关（?mock=1 / ?mock=fresh）。
 * 生产构建中 import.meta.env.DEV 为 false，isMockRequested 恒返回 null。
 */
export function mockModeFromSearch(search: string, dev: boolean): "demo" | "fresh" | null {
  if (!dev) return null;
  const value = new URLSearchParams(search).get("mock");
  if (value === "1") return "demo";
  // fresh：本地配置层为空的全新机器，用来看首次使用引导
  if (value === "fresh") return "fresh";
  return null;
}

/**
 * 启动时按 URL 参数切换 EngineClient。仅 DEV 构建生效；
 * 生产构建里 `if (import.meta.env.DEV)` 是编译期常量分支，
 * dynamic import 的 mock 模块随死代码一起被 treeshake，不会打入产物。
 */
export async function initEngineClient(search?: string): Promise<void> {
  if (import.meta.env.DEV) {
    const mode = mockModeFromSearch(search ?? window.location.search, true);
    if (!mode) return;
    const { createMockEngineClient } = await import("../test/mock-client");
    setEngineClient(createMockEngineClient({ fresh: mode === "fresh" }));
  }
}
