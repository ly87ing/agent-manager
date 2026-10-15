import { z } from "zod";
import { runEngine } from "./engine";
import type { LaunchRecipe } from "./launch";

export interface EnvironmentTarget {
  timezone?: string;
  languages?: string[];
  region?: string;
  locale?: string;
  strict_locale?: boolean;
  ntp_server?: string;
  automatic_time?: boolean;
  time_synchronized?: boolean;
  automatic_timezone?: boolean;
  max_clock_offset_ms?: number;
}

const checkSchema = z.object({
  id: z.string(), group: z.enum(["system", "time", "language", "process", "ntp"]),
  expected: z.string().nullable(), observed: z.string().nullable(),
  state: z.enum(["match", "mismatch", "unknown", "observed"]), source: z.string(), note: z.string(),
}).strict();
const targetSchema = z.object({ timezone: z.string().nullable(), languages: z.array(z.string()), region: z.string().nullable(), locale: z.string().nullable(), strict_locale: z.boolean(),
  ntp_server: z.string().nullable(), automatic_time: z.boolean().nullable(), time_synchronized: z.boolean().nullable(), automatic_timezone: z.boolean().nullable(), max_clock_offset_ms: z.number().nullable() }).strict();
const reportSchema = z.object({
  platform: z.literal("macos"), checked_at: z.string(),
  target: targetSchema,
  checks: z.array(checkSchema),
  summary: z.object({ status: z.enum(["match", "mismatch", "incomplete"]), matched: z.number().int().nonnegative(), mismatched: z.number().int().nonnegative(), unknown: z.number().int().nonnegative(), compared: z.number().int().nonnegative() }).strict(),
  process_scope: z.literal("caller"), probe_server: z.string().nullable(),
  probe_error: z.string().nullable(),
  time_repair: z.object({ status: z.enum(["not_needed", "unavailable", "verified", "unverified", "cancelled", "failed"]), before_offset_ms: z.number().nullable(), before_uncertainty_ms: z.number().nullable(), audit_recorded: z.boolean().optional() }).strict().nullable(),
  time_sync: z.object({ synchronized: z.boolean(), state_code: z.number().int().min(0).max(5), max_error_ms: z.number().finite().nonnegative(), estimated_error_ms: z.number().finite().nonnegative() }).strict().nullable(),
  time_sample: z.object({ server: z.string(), peer_address: z.string(), special_use_address: z.boolean(), stratum: z.number(), offset_ms: z.number(), round_trip_ms: z.number(), uncertainty_ms: z.number() }).strict().nullable(),
}).strict();

export type EnvironmentReport = z.infer<typeof reportSchema>;
export type EnvironmentCheck = z.infer<typeof checkSchema>;
const repairItemSchema = z.object({ id: z.string(), scope: z.enum(["system", "user", "session"]), before: z.union([z.string(), z.boolean(), z.array(z.string())]).nullable(),
  expected: z.union([z.string(), z.boolean(), z.array(z.string())]).nullable(), status: z.enum(["unchanged", "change", "unavailable", "verified", "verified_after_error", "outcome_unknown", "stale", "not_applied", "unverified", "failed"]) }).strict();
const repairPlanSchema = z.object({ base: z.string().regex(/^[0-9a-f]{64}$/), target: targetSchema, items: z.array(repairItemSchema), session_scope: z.string() }).strict();
const repairResultSchema = z.object({ status: z.enum(["not_needed", "applied", "partial", "cancelled", "audit_failed"]), items: z.array(repairItemSchema), report: reportSchema.nullable(), restart_required: z.boolean(), clock_error: z.string().nullable() }).strict();
export type EnvironmentRepairPlan = z.infer<typeof repairPlanSchema>;
export type EnvironmentRepairResult = z.infer<typeof repairResultSchema>;
const profileSchema = z.object({ id: z.string(), revision: z.string(), name: z.string(), target: targetSchema, auto_sync: z.boolean() }).strict();
export type EnvironmentProfile = z.infer<typeof profileSchema>;
const builtinSchema = z.object({ id: z.string(), name: z.string(), target: targetSchema, time_service: z.string(), source_url: z.string().url() }).strict();
export type EnvironmentBuiltin = z.infer<typeof builtinSchema>;
export interface EnvironmentProfileInput { name: string; target: EnvironmentTarget }

async function request(operation: string, args: string[] = []) {
  const result = await runEngine(`environment:${operation}`, args,
    ["save-profile", "delete-profile", "sync-time", "repair"].includes(operation) ? { runId: crypto.randomUUID() } : undefined);
  if (!result.ok) throw new Error(result.errors.map((error) => error.message).join("\n"));
  return result.data;
}

export function profileTarget(profile: Pick<EnvironmentProfile, "target">): EnvironmentTarget {
  return Object.fromEntries(Object.entries(profile.target).filter(([, value]) => value !== null)) as EnvironmentTarget;
}

export async function listEnvironmentProfiles() {
  return z.object({ items: z.array(profileSchema), builtins: z.array(builtinSchema) }).strict().parse(await request("profiles"));
}

export async function saveEnvironmentProfile(input: EnvironmentProfileInput, previous?: EnvironmentProfile): Promise<EnvironmentProfile> {
  return z.object({ item: profileSchema }).strict().parse(await request("save-profile", ["--spec", JSON.stringify({ ...input, auto_sync: false }), ...(previous ? ["--id", previous.id, "--base", previous.revision] : [])])).item;
}

export async function deleteEnvironmentProfile(profile: EnvironmentProfile): Promise<void> {
  z.object({ id: z.string() }).strict().parse(await request("delete-profile", ["--id", profile.id, "--base", profile.revision]));
}

export async function repairEnvironmentTime(target: EnvironmentTarget): Promise<EnvironmentReport> {
  return reportSchema.parse(await request("sync-time", ["--spec", JSON.stringify(target), "--confirm"]));
}

export async function previewEnvironmentRepair(target: EnvironmentTarget): Promise<EnvironmentRepairPlan> {
  return repairPlanSchema.parse(await request("repair-preview", ["--spec", JSON.stringify(target)]));
}

export async function repairEnvironment(target: EnvironmentTarget, base: string): Promise<EnvironmentRepairResult> {
  return repairResultSchema.parse(await request("repair", ["--spec", JSON.stringify(target), "--base", base, "--confirm"]));
}

export async function checkEnvironment(target: EnvironmentTarget = {}, probeTime = false): Promise<EnvironmentReport> {
  return reportSchema.parse(await request("check", ["--spec", JSON.stringify(target), ...(probeTime ? ["--probe-time"] : [])]));
}

export function environmentLaunchRecipe(target: EnvironmentTarget): LaunchRecipe {
  return { platform: "macos", shell: "zsh", kind: "app", target: "", args: [],
    env: target.locale ? ["LANG", "LC_ALL", "LC_CTYPE", "LC_COLLATE", "LC_MESSAGES", "LC_MONETARY", "LC_NUMERIC", "LC_TIME"].map((name) => ({ name, value: target.locale! })) : [],
    variants: [{ name: "environment", timezone: target.timezone ?? "" }] };
}

export async function openEnvironmentSettings(pane: "date-time" | "language-region"): Promise<void> {
  z.object({ pane: z.enum(["date-time", "language-region"]) }).strict().parse(await request("open-settings", ["--pane", pane]));
}

export async function checkTimeSettings(target: EnvironmentTarget, probeTime: boolean) {
  return z.discriminatedUnion("status", [
    z.object({ status: z.literal("read"), report: reportSchema }).strict(),
    z.object({ status: z.literal("cancelled"), report: z.null() }).strict(),
    z.object({ status: z.literal("failed"), report: z.null() }).strict(),
  ]).parse(await request("check-time-settings", ["--spec", JSON.stringify(target), "--confirm", ...(probeTime ? ["--probe-time"] : [])]));
}
