import { z } from "zod";
import { runEngine } from "./engine";

const count = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const retainedItem = z.object({ path: z.string(), reason: z.enum(["project", "unclassified"]) }).strict();
export const claudeDataPreviewSchema = z.object({
  scope: z.literal("default_claude_code"), platform: z.string(), root: z.string(), config_path: z.string(),
  configuration_paths: z.array(z.string()).max(1024),
  base: z.string().regex(/^[a-f0-9]{64}$/), can_clear: z.boolean(), has_changes: z.boolean(),
  groups: z.array(z.object({ id: z.string(), files: count, directories: count, bytes: count }).strict()),
  telemetry_policy: z.literal("remove"),
  retained: z.array(retainedItem),
  credential_items: count.nullable(), files_only: z.boolean(), blockers: z.array(z.string()),
}).strict().refine((value) => !value.can_clear || (value.has_changes && value.blockers.length === 0), "Blocked previews cannot clear");
export type ClaudeDataPreview = z.infer<typeof claudeDataPreviewSchema>;

export const claudeDataResultSchema = z.object({
  scope: z.literal("default_claude_code"),
  status: z.enum(["complete", "partial", "not_applied"]),
  deleted_files: count, deleted_directories: count, deleted_credentials: count,
  telemetry_policy: z.literal("remove"), files_only: z.boolean(),
  scope_verified: z.boolean(), message: z.string(),
  retained: z.array(retainedItem),
  retained_verified: z.boolean(),
}).strict().refine((value) => value.status !== "complete" || (value.scope_verified && value.retained_verified && !value.retained.some((item) => item.reason === "unclassified")), "Complete cleanup requires independent scope and remaining-content readback");
export type ClaudeDataResult = z.infer<typeof claudeDataResultSchema>;

export const orcaSearchPreviewSchema = z.object({
  source: z.literal("orca_search"), path: z.string(), location_origin: z.enum(["default", "override"]),
  status: z.enum(["inspected", "not_found", "unavailable"]), schema_version: z.literal(6).nullable(),
  counts: z.object({
    claude_sessions: count, claude_messages: count, claude_file_cursors: count,
    non_claude_sessions: count, unattributed_messages: count,
    unattributed_file_cursors: count, unattributed_fts_rows: count,
  }).strict().nullable(),
  cleanup_supported: z.literal(false), scope_complete: z.literal(false), message: z.string(),
}).strict().refine((value) => value.status === "inspected"
  ? value.counts !== null && value.schema_version === 6
  : value.counts === null && value.schema_version === null, "Unchecked sources cannot report counts");
export type OrcaSearchPreview = z.infer<typeof orcaSearchPreviewSchema>;

export async function previewOrcaSearch(): Promise<OrcaSearchPreview> {
  const response = await runEngine("claude-data:orca-search-preview", []);
  if (!response.ok) throw new Error(response.errors.map((error) => error.message).join("\n"));
  return orcaSearchPreviewSchema.parse(response.data);
}

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
