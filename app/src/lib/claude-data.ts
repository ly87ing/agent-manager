import { z } from "zod";
import { runEngine } from "./engine";

const count = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
export const claudeDataPreviewSchema = z.object({
  scope: z.literal("default_claude_code"), platform: z.string(), root: z.string(), config_path: z.string(),
  base: z.string().regex(/^[a-f0-9]{64}$/), can_clear: z.boolean(), has_changes: z.boolean(),
  groups: z.array(z.object({ id: z.string(), files: count, directories: count, bytes: count }).strict()),
  preserved: z.array(z.object({ file: z.string(), fields: count, changed: z.boolean() }).strict()),
  credential_items: count.nullable(), files_only: z.boolean(), blockers: z.array(z.string()),
}).strict().refine((value) => !value.can_clear || (value.has_changes && value.blockers.length === 0), "Blocked previews cannot clear");
export type ClaudeDataPreview = z.infer<typeof claudeDataPreviewSchema>;

export const claudeDataResultSchema = z.object({
  scope: z.literal("default_claude_code"),
  status: z.enum(["complete", "partial", "not_applied"]),
  deleted_files: count, deleted_directories: count, deleted_credentials: count,
  preserved_fields: count, files_only: z.boolean(),
  telemetry_verified: z.boolean(), message: z.string(),
}).strict().refine((value) => value.status !== "complete" || value.telemetry_verified, "Complete cleanup requires telemetry readback");
export type ClaudeDataResult = z.infer<typeof claudeDataResultSchema>;

export async function previewClaudeData(filesOnly: boolean): Promise<ClaudeDataPreview> {
  const response = await runEngine("claude-data:preview", filesOnly ? ["--files-only"] : []);
  if (!response.ok) throw new Error(response.errors.map((error) => error.message).join("\n"));
  const preview = claudeDataPreviewSchema.parse(response.data);
  if (preview.files_only !== filesOnly) throw new Error("Cleanup scope differs from the requested preview");
  return preview;
}

export async function clearClaudeData(preview: ClaudeDataPreview): Promise<ClaudeDataResult> {
  const response = await runEngine("claude-data:clear", ["--base", preview.base, "--confirm", ...(preview.files_only ? ["--files-only"] : [])], { runId: crypto.randomUUID() });
  const result = claudeDataResultSchema.safeParse(response.data);
  if (result.success && result.data.files_only === preview.files_only) return result.data;
  throw new Error(response.errors.map((error) => error.message).join("\n") || "Invalid cleanup result");
}
