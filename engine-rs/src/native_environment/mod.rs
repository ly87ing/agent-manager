//! Read-only regional environment checks shared by the desktop and CLI.
mod actions;
#[cfg(target_os = "macos")]
mod macos;
mod profiles;
mod time_probe;
mod time_repair;
#[cfg(target_os = "macos")]
mod time_sync;

use crate::{diagnostic, Context, Envelope, Lock, Mode, Request};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(any(target_os = "macos", test))]
use std::collections::BTreeMap;

#[cfg(any(target_os = "macos", test))]
const LOCALE_CATEGORIES: &[&str] = &[
    "LC_CTYPE",
    "LC_COLLATE",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NUMERIC",
    "LC_TIME",
];

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Target {
    timezone: Option<String>,
    languages: Vec<String>,
    region: Option<String>,
    locale: Option<String>,
    strict_locale: bool,
    ntp_server: Option<String>,
    automatic_time: Option<bool>,
    time_synchronized: Option<bool>,
    automatic_timezone: Option<bool>,
    max_clock_offset_ms: Option<u32>,
}

impl Target {
    fn validate_time_repair(&self) -> Result<()> {
        if self.ntp_server.is_none() || self.max_clock_offset_ms.is_none() {
            bail!("Time correction requires an explicit server and clock tolerance.");
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if let Some(zone) = &self.timezone {
            if zone.is_empty()
                || zone.len() > 128
                || !zone
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"/_+-".contains(&c))
                || zone
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                bail!("Enter an IANA time zone such as Europe/London.");
            }
            #[cfg(target_os = "macos")]
            if !std::path::Path::new("/usr/share/zoneinfo")
                .join(zone)
                .is_file()
            {
                bail!("The time zone is not available on this Mac.");
            }
        }
        if self.languages.len() > 16
            || self.languages.iter().any(|value| {
                value.is_empty()
                    || value.len() > 64
                    || !value
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            })
            || self.languages.iter().enumerate().any(|(index, value)| {
                self.languages[..index]
                    .iter()
                    .any(|prior| prior.eq_ignore_ascii_case(value))
            })
        {
            bail!("Enter unique language tags in preference order, such as en-GB, fr-FR.");
        }
        if self
            .region
            .as_ref()
            .is_some_and(|value| value.len() != 2 || !value.bytes().all(|c| c.is_ascii_uppercase()))
        {
            bail!("Enter a two-letter uppercase region code, such as GB.");
        }
        if self.locale.as_ref().is_some_and(|value| {
            value.is_empty()
                || value.len() > 128
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_@.-".contains(&c))
        }) {
            bail!("Enter a POSIX locale such as en_GB.UTF-8.");
        }
        if let Some(server) = &self.ntp_server {
            if !time_probe::valid_host(server) {
                bail!("Enter a time-server hostname or IP address without a port or URL.");
            }
        }
        if self
            .max_clock_offset_ms
            .is_some_and(|value| value == 0 || value > 60_000)
        {
            bail!("The clock tolerance must be between 1 and 60000 milliseconds.");
        }
        Ok(())
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct Observation {
    timezone: Option<String>,
    languages: Option<Vec<String>>,
    apple_locale: Option<String>,
    format_overrides: Option<Value>,
    automatic_time: Option<bool>,
    automatic_timezone: Option<bool>,
    ntp_server: Option<String>,
    time_service: Option<bool>,
    time_synchronized: Option<bool>,
    kernel_max_error_ms: Option<f64>,
    kernel_estimated_error_ms: Option<f64>,
    local_time: Option<String>,
    os_version: Option<String>,
    env: BTreeMap<String, String>,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Serialize)]
struct Check {
    id: String,
    group: &'static str,
    expected: Option<String>,
    observed: Option<String>,
    state: &'static str,
    source: &'static str,
    note: &'static str,
}

#[cfg(any(target_os = "macos", test))]
fn check(
    id: &str,
    group: &'static str,
    observed: Option<String>,
    expected: Option<String>,
    source: &'static str,
    note: &'static str,
) -> Check {
    let state = match (&observed, &expected) {
        (None, _) => "unknown",
        (Some(_), None) => "observed",
        (Some(actual), Some(target)) if actual == target => "match",
        _ => "mismatch",
    };
    Check {
        id: id.into(),
        group,
        observed,
        expected,
        state,
        source,
        note,
    }
}

#[cfg(any(target_os = "macos", test))]
fn locale_key(value: &str) -> String {
    value.to_ascii_lowercase().replace("utf-8", "utf8")
}

#[cfg(any(target_os = "macos", test))]
fn locale_check(id: &str, observed: String, expected: Option<String>) -> Check {
    let matches = expected
        .as_ref()
        .map(|target| locale_key(target) == locale_key(&observed));
    let mut item = check(
        id,
        "process",
        Some(observed),
        expected,
        "process environment",
        "locale_precedence",
    );
    if matches == Some(true) {
        item.state = "match";
    }
    item
}

#[cfg(any(target_os = "macos", test))]
fn region(locale: &str) -> Option<String> {
    locale
        .split('@')
        .next()?
        .split(['_', '-'])
        .skip(1)
        .find(|part| part.len() == 2 && part.bytes().all(|c| c.is_ascii_uppercase()))
        .map(str::to_string)
}

#[cfg(any(target_os = "macos", test))]
fn evaluate(observed: &Observation, target: &Target) -> Vec<Check> {
    let mut rows = vec![
        check(
            "ntp.actual_source",
            "ntp",
            None,
            None,
            "macOS background time service",
            "actual_source_unavailable",
        ),
        check(
            "os.version",
            "system",
            observed.os_version.clone(),
            None,
            "sw_vers",
            "inventory",
        ),
        check(
            "os.arch",
            "system",
            Some(std::env::consts::ARCH.into()),
            None,
            "Rust platform",
            "inventory",
        ),
        check(
            "time.local",
            "time",
            observed.local_time.clone(),
            None,
            "date (without TZ override)",
            "local_time",
        ),
        check(
            "time.timezone",
            "time",
            observed.timezone.clone(),
            target.timezone.clone(),
            "/etc/localtime",
            "system_timezone",
        ),
        check(
            "time.automatic_timezone",
            "time",
            observed.automatic_timezone.map(|value| value.to_string()),
            target.automatic_timezone.map(|value| value.to_string()),
            "/Library/Preferences/com.apple.timezone.auto",
            "automatic_timezone",
        ),
        check(
            "language.preferred",
            "language",
            observed.languages.as_ref().map(|values| values.join(", ")),
            if target.languages.is_empty() {
                None
            } else {
                Some(target.languages.join(", "))
            },
            "current user AppleLanguages",
            "language_order",
        ),
        check(
            "language.region",
            "language",
            observed.apple_locale.as_deref().and_then(region),
            target.region.clone(),
            "current user AppleLocale",
            "user_region",
        ),
        check(
            "language.locale",
            "language",
            observed.apple_locale.clone(),
            None,
            "current user AppleLocale",
            "user_region",
        ),
        check(
            "language.formats",
            "language",
            observed.format_overrides.as_ref().map(Value::to_string),
            None,
            "current user global preferences",
            "format_overrides",
        ),
        check(
            "ntp.server",
            "ntp",
            observed.ntp_server.clone(),
            target
                .ntp_server
                .as_ref()
                .map(|s| s.trim_end_matches('.').to_ascii_lowercase()),
            "/etc/ntp.conf",
            "configured_only",
        ),
        check(
            "ntp.automatic_time",
            "ntp",
            observed.automatic_time.map(|value| value.to_string()),
            target.automatic_time.map(|value| value.to_string()),
            "systemsetup -getusingnetworktime",
            if observed.automatic_time.is_none() {
                "permission_required"
            } else {
                "automatic_time"
            },
        ),
        check(
            "ntp.service",
            "ntp",
            observed.time_service.map(|value| value.to_string()),
            None,
            "launchctl system/com.apple.timed",
            "service_only",
        ),
        check(
            "ntp.synchronized",
            "ntp",
            observed.time_synchronized.map(sync_value),
            target.time_synchronized.map(sync_value),
            "ntp_gettime (read-only kernel state)",
            "kernel_sync",
        ),
        check(
            "ntp.kernel_max_error",
            "ntp",
            observed
                .kernel_max_error_ms
                .map(|value| format!("{value:.1} ms")),
            None,
            "ntp_gettime maxerror",
            "kernel_error",
        ),
        check(
            "ntp.kernel_estimated_error",
            "ntp",
            observed
                .kernel_estimated_error_ms
                .map(|value| format!("{value:.1} ms")),
            None,
            "ntp_gettime esterror",
            "kernel_error",
        ),
    ];
    // Canonical language tag casing does not change an ordered language list.
    if let (Some(actual), false) = (&observed.languages, target.languages.is_empty()) {
        if actual
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .eq(target.languages.iter().map(|s| s.to_ascii_lowercase()))
        {
            rows.iter_mut()
                .find(|row| row.id == "language.preferred")
                .unwrap()
                .state = "match";
        }
    }
    let raw_tz = observed.env.get("TZ");
    let effective_tz = match raw_tz {
        None => observed.timezone.clone(),
        Some(value) if value.is_empty() => Some("UTC".into()),
        Some(value) => Some(value.trim_start_matches(':').into()),
    };
    rows.push(check(
        "process.timezone",
        "process",
        effective_tz,
        target.timezone.clone(),
        "process TZ, then system time zone",
        if raw_tz.is_none() {
            "timezone_inherited"
        } else {
            "timezone_override"
        },
    ));
    let raw: BTreeMap<_, _> = observed
        .env
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    rows.push(check(
        "process.variables",
        "process",
        Some(json!(raw).to_string()),
        None,
        "process environment allowlist",
        "runtime_scope",
    ));
    for category in LOCALE_CATEGORIES {
        let value = ["LC_ALL", category, "LANG"]
            .iter()
            .find_map(|key| observed.env.get(*key).filter(|value| !value.is_empty()))
            .map(String::as_str)
            .unwrap_or("C");
        rows.push(locale_check(category, value.into(), target.locale.clone()));
    }
    if let Some(expected) = target.locale.as_ref().filter(|_| target.strict_locale) {
        let conflicts: BTreeMap<_, _> = observed
            .env
            .iter()
            .filter(|(key, value)| {
                key.as_str() != "TZ"
                    && !value.is_empty()
                    && locale_key(value) != locale_key(expected)
            })
            .collect();
        rows.push(check(
            "process.raw_locale",
            "process",
            Some(if conflicts.is_empty() {
                expected.clone()
            } else {
                json!(conflicts).to_string()
            }),
            Some(expected.clone()),
            "explicit process locale variables",
            "raw_locale_consistency",
        ));
    }
    rows
}

#[cfg(any(target_os = "macos", test))]
fn sync_value(value: bool) -> String {
    if value {
        "synchronized"
    } else {
        "unsynchronized"
    }
    .into()
}

fn arguments(req: &Request) -> Result<(Target, bool)> {
    let command = req.command.replace('.', ":");
    let repair = command == "environment:sync-time";
    let authorized_read = command == "environment:check-time-settings";
    if !repair && !authorized_read && command != "environment:check" {
        bail!("Unsupported environment operation.");
    }
    let mut target = None;
    let mut probe = false;
    let mut confirm = false;
    let mut args = req.args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--spec" if target.is_none() => {
                let input = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("Missing environment target."))?;
                if input.len() > 8192 {
                    bail!("Environment target is too large.");
                }
                target = Some(
                    serde_json::from_str::<Target>(input)
                        .map_err(|_| anyhow::anyhow!("Invalid environment target fields."))?,
                );
            }
            "--probe-time" if !repair && !probe => probe = true,
            "--confirm" if (repair || authorized_read) && !confirm => confirm = true,
            _ => bail!("Unsupported or duplicate environment flag."),
        }
    }
    let target = target.unwrap_or_default();
    target.validate()?;
    if repair && !confirm {
        bail!("Time correction requires --confirm.");
    }
    if authorized_read && !confirm {
        bail!("Authorized time-settings inspection requires --confirm.");
    }
    Ok((target, probe))
}

#[cfg(any(target_os = "macos", test))]
fn clock_state(offset_ms: f64, uncertainty_ms: f64, limit_ms: u32) -> &'static str {
    if offset_ms.abs() + uncertainty_ms <= limit_ms as f64 {
        "match"
    } else if offset_ms.abs() - uncertainty_ms > limit_ms as f64 {
        "mismatch"
    } else {
        "unknown"
    }
}

fn report(target: &Target, probe: bool) -> Result<Value> {
    report_with_automatic_time(target, probe, None)
}

fn report_with_automatic_time(
    target: &Target,
    probe: bool,
    automatic_time: Option<bool>,
) -> Result<Value> {
    #[cfg(target_os = "macos")]
    let mut observed = macos::collect();
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (target, probe, automatic_time);
        bail!("Environment checks currently support macOS only.");
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(value) = automatic_time {
            observed.automatic_time = Some(value);
        }
        let time_sync = time_sync::collect();
        if let Some(value) = &time_sync {
            observed.time_synchronized = Some(value.synchronized);
            observed.kernel_max_error_ms = Some(value.max_error_ms);
            observed.kernel_estimated_error_ms = Some(value.estimated_error_ms);
        }
        let mut rows = evaluate(&observed, target);
        let server = target
            .ntp_server
            .clone()
            .or_else(|| observed.ntp_server.clone());
        let (sample, probe_note, probe_error) = if probe {
            match server.as_deref() {
                Some(host) => match time_probe::query(host) {
                    Ok(value) => (Some(value), "time_sample", None),
                    Err(error) => (None, "query_failed", Some(error.to_string())),
                },
                None => (None, "server_required", None),
            }
        } else {
            (None, "not_queried", None)
        };
        let mut clock = check(
            "ntp.clock_offset",
            "ntp",
            sample.as_ref().map(|value| {
                format!(
                    "{:+.1} ms ± {:.1} ms",
                    value.offset_ms, value.uncertainty_ms
                )
            }),
            target
                .max_clock_offset_ms
                .map(|value| format!("≤ {value} ms")),
            "one NTP request; does not adjust the clock",
            probe_note,
        );
        if let (Some(value), Some(limit)) = (&sample, target.max_clock_offset_ms) {
            clock.state = clock_state(value.offset_ms, value.uncertainty_ms, limit);
            if clock.state == "unknown" {
                clock.note = "clock_uncertain";
            }
        }
        rows.push(clock);
        let count = |state| {
            rows.iter()
                .filter(|row| row.expected.is_some() && row.state == state)
                .count()
        };
        let (matched, mismatched, unknown) = (count("match"), count("mismatch"), count("unknown"));
        let compared = matched + mismatched + unknown;
        let status = if mismatched > 0 {
            "mismatch"
        } else if unknown > 0 || compared == 0 {
            "incomplete"
        } else {
            "match"
        };
        Ok(
            json!({"platform":"macos", "checked_at":chrono::Utc::now().to_rfc3339(), "target":target, "checks":rows,
            "summary":{"status":status,"matched":matched,"mismatched":mismatched,"unknown":unknown,"compared":compared},
            "process_scope":"caller", "probe_server":server, "time_sync":time_sync, "time_sample":sample, "probe_error":probe_error, "time_repair":null}),
        )
    }
}

pub(crate) fn execute(ctx: &Context, req: &Request) -> Envelope {
    let command = req.command.replace('.', ":");
    if command == "environment:open-settings" {
        return envelope(req, true, actions::open_settings(req));
    }
    if matches!(
        command.as_str(),
        "environment:profiles" | "environment:save-profile" | "environment:delete-profile"
    ) {
        return envelope(req, true, profiles::handle(ctx, req));
    }
    let parsed = arguments(req);
    let repair = command == "environment:sync-time";
    let offline = !repair && parsed.as_ref().map(|(_, probe)| !probe).unwrap_or(true);
    let result = parsed.and_then(|(target, probe)| {
        if repair {
            time_repair::handle(&target)
        } else if command == "environment:check-time-settings" {
            actions::check_time_settings(&target, probe)
        } else {
            report(&target, probe)
        }
    });
    envelope(req, offline, result)
}

fn envelope(req: &Request, offline: bool, result: Result<Value>) -> Envelope {
    let (data, errors) = match result {
        Ok(value) => (value, vec![]),
        Err(error) => (
            json!({}),
            vec![diagnostic(
                "ENVIRONMENT_CHECK_ERROR",
                None,
                error.to_string(),
            )],
        ),
    };
    Envelope::new(
        req.command.replacen(':', ".", 1),
        Mode { offline },
        Lock {
            held: false,
            pid: None,
        },
        req.run_id.clone(),
        data,
        vec![],
        errors,
    )
}

#[cfg(test)]
mod tests;
