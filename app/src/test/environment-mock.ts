import type { EnvironmentProfile, EnvironmentProfileInput, EnvironmentReport } from "../lib/environment";
import { environmentFixture } from "./environment-fixture";
import { makeEnvelope } from "./fixtures";

/** Dev-only persistence lasts for this mock client's lifetime. */
export function createEnvironmentMock() {
  const items: EnvironmentProfile[] = [];
  const arg = (args: string[], key: string) => args[args.indexOf(key) + 1];
  return (command: string, args: string[]) => {
    if (command === "environment:profiles") return makeEnvelope({ items });
    if (command === "environment:save-profile") {
      const input = JSON.parse(arg(args, "--spec")) as EnvironmentProfileInput;
      const id = args.includes("--id") ? arg(args, "--id") : crypto.randomUUID();
      const item = { ...input, id, revision: crypto.randomUUID().replaceAll("-", "").repeat(2), target: environmentFixture(input.target).target };
      const index = items.findIndex((entry) => entry.id === id);
      if (index >= 0) items[index] = item; else items.push(item);
      return makeEnvelope({ item });
    }
    if (command === "environment:delete-profile") {
      const id = arg(args, "--id"); items.splice(items.findIndex((item) => item.id === id), 1);
      return makeEnvelope({ id });
    }
    const target = args.includes("--spec") ? JSON.parse(arg(args, "--spec")) : {};
    const probe = args.includes("--probe-time") || command === "environment:sync-time";
    const report = environmentFixture(target, probe);
    if (command === "environment:sync-time") {
      const clock = report.checks.find((row) => row.id === "ntp.clock_offset")!;
      const status: NonNullable<EnvironmentReport["time_repair"]>["status"] = clock.state === "match" ? "not_needed" : clock.state === "unknown" ? "unavailable" : "verified";
      report.time_repair = { status, before_offset_ms: 20, before_uncertainty_ms: 5 };
      if (status === "verified") {
        clock.observed = "+0.0 ms ± 1.0 ms"; clock.state = "match";
        report.time_sample!.offset_ms = 0; report.time_sample!.uncertainty_ms = 1;
        report.summary.matched++; report.summary.mismatched--;
        report.summary.status = report.summary.mismatched ? "mismatch" : report.summary.unknown ? "incomplete" : "match";
      }
    }
    return makeEnvelope(report);
  };
}
