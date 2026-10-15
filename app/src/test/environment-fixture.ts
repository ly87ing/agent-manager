import type { EnvironmentBuiltin, EnvironmentReport, EnvironmentTarget } from "../lib/environment";

export function environmentBuiltinsFixture(): EnvironmentBuiltin[] {
  return [
    { id: "builtin:los-angeles", name: "Los Angeles (US)", timezone: "America/Los_Angeles", language: "en-US", region: "US", locale: "en_US.UTF-8", server: "time.nist.gov", time_service: "NIST", source_url: "https://tf.nist.gov/tf-cgi/servers.cgi" },
    { id: "builtin:singapore", name: "Singapore (SG)", timezone: "Asia/Singapore", language: "en-SG", region: "SG", locale: "en_US.UTF-8", server: "nets.org.sg", time_service: "A*STAR NMC", source_url: "https://www.a-star.edu.sg/nmc/Research-and-Development/Labs/FL" },
  ].map(({ id, name, timezone, language, region, locale, server, time_service, source_url }) => ({ id, name, time_service, source_url,
    target: environmentFixture({ timezone, languages: [language], region, locale, strict_locale: true, ntp_server: server, automatic_time: true, time_synchronized: true, automatic_timezone: false, max_clock_offset_ms: 2000 }).target,
  }));
}

export function environmentFixture(target: EnvironmentTarget = {}, probe = false, synchronized: boolean | null = true): EnvironmentReport {
  const actual = { timezone: "Europe/London", languages: ["en-GB"], region: "GB", locale: "en_GB.UTF-8", ntp_server: "pool.ntp.org", automatic_time: null, automatic_timezone: false };
  const row = (id: string, group: EnvironmentReport["checks"][number]["group"], observed: string | null, expected: string | null = null, note = "inventory") => ({
    id, group, observed, expected, source: "Development fixture", note,
    state: observed === null ? "unknown" as const : expected === null ? "observed" as const : observed === expected ? "match" as const : "mismatch" as const,
  });
  const checks = [
    row("ntp.actual_source", "ntp", null, null, "actual_source_unavailable"),
    row("os.version", "system", "15.2"), row("os.arch", "system", "aarch64"),
    row("time.local", "time", "2026-10-13 18:00:00 BST +0100", null, "local_time"),
    row("time.timezone", "time", actual.timezone, target.timezone ?? null, "system_timezone"),
    row("time.automatic_timezone", "time", String(actual.automatic_timezone), target.automatic_timezone === undefined ? null : String(target.automatic_timezone), "automatic_timezone"),
    row("language.preferred", "language", actual.languages.join(", "), target.languages?.length ? target.languages.join(", ") : null, "language_order"),
    row("language.region", "language", actual.region, target.region ?? null, "user_region"),
    row("language.locale", "language", "en_GB", null, "user_region"),
    row("language.formats", "language", '{"AppleICUForce24HourTime":true}', null, "format_overrides"),
    row("process.timezone", "process", actual.timezone, target.timezone ?? null, "timezone_inherited"),
    row("process.variables", "process", '{"LANG":"en_GB.UTF-8"}', null, "runtime_scope"),
    ...["LC_CTYPE", "LC_COLLATE", "LC_MESSAGES", "LC_MONETARY", "LC_NUMERIC", "LC_TIME"].map((id) => row(id, "process", actual.locale, target.locale ?? null, "locale_precedence")),
    row("ntp.server", "ntp", actual.ntp_server, target.ntp_server ?? null, "configured_only"),
    row("ntp.automatic_time", "ntp", null, target.automatic_time === undefined ? null : String(target.automatic_time), "permission_required"),
    row("ntp.service", "ntp", "true", null, "service_only"),
    row("ntp.synchronized", "ntp", synchronized === null ? null : synchronized ? "synchronized" : "unsynchronized", target.time_synchronized === undefined ? null : target.time_synchronized ? "synchronized" : "unsynchronized", "kernel_sync"),
    row("ntp.kernel_max_error", "ntp", synchronized === null ? null : "250.0 ms", null, "kernel_error"),
    row("ntp.kernel_estimated_error", "ntp", synchronized === null ? null : "125.0 ms", null, "kernel_error"),
    row("ntp.clock_offset", "ntp", probe ? "+20.0 ms ± 5.0 ms" : null, target.max_clock_offset_ms ? `≤ ${target.max_clock_offset_ms} ms` : null, probe ? "time_sample" : "not_queried"),
  ];
  const clock = checks.find((item) => item.id === "ntp.clock_offset")!;
  if (target.strict_locale && target.locale) checks.push(row("process.raw_locale", "process", actual.locale, target.locale, "raw_locale_consistency"));
  if (probe && clock.expected) clock.state = target.max_clock_offset_ms! >= 25 ? "match" : target.max_clock_offset_ms! < 15 ? "mismatch" : "unknown";
  const matched = checks.filter((item) => item.expected !== null && item.state === "match").length;
  const mismatched = checks.filter((item) => item.expected !== null && item.state === "mismatch").length;
  const unknown = checks.filter((item) => item.expected !== null && item.state === "unknown").length;
  const compared = matched + mismatched + unknown;
  return { platform: "macos", checked_at: "2026-10-13T17:00:00Z", process_scope: "caller", probe_server: target.ntp_server ?? actual.ntp_server, probe_error: null,
    time_repair: null,
    target: { timezone: null, languages: [], region: null, locale: null, strict_locale: false, ntp_server: null, automatic_time: null, time_synchronized: null, automatic_timezone: null, max_clock_offset_ms: null, ...target },
    summary: { status: mismatched ? "mismatch" : unknown || !compared ? "incomplete" : "match", matched, mismatched, unknown, compared }, checks,
    time_sync: synchronized === null ? null : { synchronized, state_code: synchronized ? 0 : 5, max_error_ms: 250, estimated_error_ms: 125 },
    time_sample: probe ? { server: target.ntp_server ?? actual.ntp_server, peer_address: "198.18.0.1", special_use_address: true, stratum: 2, offset_ms: 20, round_trip_ms: 8, uncertainty_ms: 5 } : null };
}
