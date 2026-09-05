import { http, HttpResponse } from "msw";
import { setupServer, type SetupServer } from "msw/node";
import { EngineBridgeError, type EngineClient } from "../lib/engine";
import type { Envelope } from "../lib/contract";

/**
 * 测试用引擎通道：EngineClient 注入 + msw 拦截传输层。
 * fake client 把命令路由到 http://engine.test/api/invoke，由 msw 按 command 分发表路由，
 * 借此同时覆盖「client 注入」与「react-query 异步流」两层。
 */

const ENDPOINT = "http://engine.test/api/invoke";

export type EngineRoute = Envelope | { status: number; body: unknown };

export function createMswEngineClient(): EngineClient {
  return {
    async run(command, args = []) {
      const response = await fetch(ENDPOINT, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ command, args }),
      });
      const body: unknown = await response.json();
      if (!response.ok) {
        if (typeof body === "object" && body !== null && "message" in body) {
          throw new EngineBridgeError(body as { kind?: string; message: string; exit_code?: number | null; stderr?: string | null });
        }
        throw new EngineBridgeError({ kind: "http_error", message: `engine transport HTTP ${response.status}` });
      }
      return body as Envelope;
    },
  };
}

export function startEngineServer(routes: Record<string, EngineRoute>): SetupServer {
  const server = setupServer(
    http.post(ENDPOINT, async ({ request }) => {
      const { command } = (await request.json()) as { command: string };
      const route = routes[command];
      if (!route) {
        return HttpResponse.json(
          { kind: "unknown_command", message: `测试未注册命令路由: ${command}` },
          { status: 500 },
        );
      }
      if ("status" in route) {
        return HttpResponse.json(route.body as never, { status: route.status });
      }
      return HttpResponse.json(route as never);
    }),
  );
  server.listen({ onUnhandledRequest: "error" });
  return server;
}

/** 直注 fake client（不经 msw）：route 为 envelope 或异步函数（可挂起模拟进行中态）。 */
export function createFakeEngineClient(
  routes: Record<string, Envelope | ((args: string[]) => Promise<Envelope>)>,
): EngineClient {
  return {
    async run(command, args = []) {
      const route = routes[command];
      if (!route) {
        throw new EngineBridgeError({ kind: "unknown_command", message: `测试未注册命令路由: ${command}` });
      }
      return typeof route === "function" ? route(args) : structuredClone(route);
    },
  };
}
