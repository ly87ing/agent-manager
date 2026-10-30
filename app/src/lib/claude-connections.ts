import { z } from "zod";
import { runEngine } from "./engine";

export const connectionKeys = ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "no_proxy", "all_proxy", "CLAUDE_CONFIG_DIR", "PROXY_MANAGED", "apiKeyHelper"] as const;
const keySchema = z.string().max(128).refine(key => /^[A-Za-z0-9_]+$/.test(key) && ((connectionKeys as readonly string[]).includes(key) || key.startsWith("ANTHROPIC_")));
export const connectionInspectionSchema = z.object({
  scope: z.literal("claude_connections"),
  root: z.string(),
  sources: z.array(z.object({ id: z.string(), path: z.string().nullable(), status: z.enum(["inspected", "not_found", "unavailable"]), keys: z.array(keySchema), items: z.array(z.object({ id: z.string(), key: keySchema, line: z.number().int().positive().nullable(), editable: z.boolean() }).strict()).max(1024), kind: z.enum(["settings", "references", "environment"]) }).strict()),
  cc_switch: z.enum(["running", "not_running", "unknown"]), custom_config_present: z.boolean(), clear_supported: z.boolean(),
}).strict();
export type ConnectionInspection = z.infer<typeof connectionInspectionSchema>;

export async function inspectClaudeConnections(root?: string): Promise<ConnectionInspection> {
  const result = await runEngine("claude-data:inspect-connections", root ? ["--connection-root", root] : []);
  if (!result.ok) throw new Error(result.errors.map(error => error.message).join("\n"));
  return connectionInspectionSchema.parse(result.data);
}

export const connectionPreviewSchema = z.object({ scope: z.literal("claude_connections"), base: z.string().regex(/^[a-f0-9]{64}$/), item: z.string(), path: z.string(), key: keySchema, kind: z.enum(["settings", "settings_root", "startup", "user_environment"]), line: z.number().int().positive().nullable(), root: z.string().nullable(), shared: z.boolean(), can_clear: z.boolean(), blockers: z.array(z.string()) }).strict()
  .refine(value => value.shared === (value.kind === "startup" || value.kind === "user_environment"), "Shared scope must be disclosed")
  .refine(value => !value.can_clear || value.blockers.length === 0, "Blocked previews cannot clear");
export type ConnectionPreview = z.infer<typeof connectionPreviewSchema>;
export const connectionResultSchema = z.object({ scope: z.literal("claude_connections"), status: z.enum(["complete", "partial"]), item: z.string(), removed: z.boolean(), retained_verified: z.boolean(), message: z.string() }).strict()
  .refine(value => value.status !== "complete" || (value.removed && value.retained_verified), "Complete requires removal and preservation readback");
export type ConnectionResult = z.infer<typeof connectionResultSchema>;

export async function previewConnection(item: string, root?: string): Promise<ConnectionPreview> {
  const result = await runEngine("claude-data:preview", ["--connection-items", JSON.stringify([item]), ...(root ? ["--connection-root", root] : [])]);
  if (!result.ok) throw new Error(result.errors.map(error => error.message).join("\n"));
  const preview = connectionPreviewSchema.parse(result.data);
  if (preview.item !== item || item.slice(item.lastIndexOf(":") + 1) !== preview.key || preview.root !== (root ?? null)) throw new Error("Selected source differs from the preview");
  return preview;
}

export async function clearConnection(preview: ConnectionPreview): Promise<ConnectionResult> {
  const result = await runEngine("claude-data:clear", ["--connection-items", JSON.stringify([preview.item]), ...(preview.root ? ["--connection-root", preview.root] : []), "--base", preview.base, "--confirm"], { runId: crypto.randomUUID() });
  const parsed = connectionResultSchema.safeParse(result.data);
  if (parsed.success && parsed.data.item === preview.item) return parsed.data;
  throw new Error(result.errors.map(error => error.message).join("\n") || "Invalid targeted cleanup result");
}
