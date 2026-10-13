import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import { expect, it } from "vitest";
import { setEngineClient } from "../lib/engine";
import { boundedExecutions, executionSummary, ExecutionProvider, useExecutions, type ExecutionRecord } from "../lib/execution";
import { affectedDomains, createQueryClient, engineKeys, invalidateEngineQueries } from "../lib/queries";
import { makeEnvelope } from "./fixtures";

it("keeps running batches and only the newest fifty completed summaries", () => {
  const client = createQueryClient({ retry: false });
  const { result } = renderHook(() => useExecutions(), { wrapper: ({ children }) => <QueryClientProvider client={client}><ExecutionProvider>{children}</ExecutionProvider></QueryClientProvider> });
  act(() => {
    result.current.recordBatch({ id: "running", command: "skills:apply", label: "running", running: true, rows: [] });
    for (let i = 0; i < 80; i++) result.current.recordBatch({ id: String(i), command: "skills:apply", label: String(i), running: false, rows: [] });
  });
  expect(result.current.batches.map(batch => batch.id)).toEqual([...Array.from({ length: 50 }, (_, i) => String(79 - i)), "running"]);
  client.clear();
});

it("keeps running operations when older completed history is discarded", () => {
  const row = (runId: string, status: ExecutionRecord["status"]): ExecutionRecord => ({ runId, status, command: "mcp:apply", label: runId, startedAt: 0, finishedAt: null, envelope: null, bridgeError: null });
  const kept = boundedExecutions([row("first", "running"), ...Array.from({ length: 80 }, (_, i) => row(String(i), "ok")), row("last", "running")]);
  expect(kept.map(row => row.runId)).toEqual(["first", ...Array.from({ length: 50 }, (_, i) => String(i)), "last"]);
});

it("returns full results to callers while storing bounded list summaries", async () => {
  const envelope = makeEnvelope({ payload: "x".repeat(1_000_000), steps: Array.from({ length: 100 }, () => ({ name: "s".repeat(10_000), ok: true, exit_code: 0 })) }, { errors: [{ code: "FAILED", element: null, message: "e".repeat(10_000) }] });
  setEngineClient({ async run() { return envelope; } });
  const client = createQueryClient({ retry: false });
  const { result } = renderHook(() => useExecutions(), { wrapper: ({ children }) => <QueryClientProvider client={client}><ExecutionProvider>{children}</ExecutionProvider></QueryClientProvider> });
  let returned;
  await act(async () => { returned = await result.current.execute({ command: "mcp:apply", label: "large result", silent: true }); });
  expect(returned).toBe(envelope);
  const saved = result.current.executions[0].envelope!;
  expect(saved.data).not.toHaveProperty("payload");
  expect(JSON.stringify(saved).length).toBeLessThan(120_000);
  client.clear();
});

it("accepts program operation steps without treating them as resource apply steps", () => {
  const result = makeEnvelope({ steps: [{ label: "Install package", command: "fixture" }] });
  expect(executionSummary(result).data).toEqual({ steps: [] });
  expect(result.data.steps).toEqual([{ label: "Install package", command: "fixture" }]);
});

it("refreshes rules and audit without invalidating unrelated skill content", async () => {
  const client = createQueryClient({ retry: false });
  for (const command of ["rules:status", "runs:list", "skills:status"]) client.setQueryData(engineKeys.command(command), {});
  await invalidateEngineQueries(client, "rules:apply");
  expect(client.getQueryState(engineKeys.command("rules:status"))?.isInvalidated).toBe(true);
  expect(client.getQueryState(engineKeys.command("runs:list"))?.isInvalidated).toBe(true);
  expect(client.getQueryState(engineKeys.command("skills:status"))?.isInvalidated).toBe(false);
  expect(affectedDomains("skills.apply")).toEqual(expect.arrayContaining(["skills", "rules", "programs"]));
  expect(affectedDomains("skills.apply")).not.toContain("aliases");
  await invalidateEngineQueries(client, "policy:set");
  expect(client.getQueryState(engineKeys.command("skills:status"))?.isInvalidated).toBe(true);
  client.clear();
});
