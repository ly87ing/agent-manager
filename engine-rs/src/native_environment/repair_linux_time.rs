//! Caller-owned Polkit authorization precedes fresh timedated/disk verification.
#[cfg(target_os = "linux")]
use super::repair::Item;
use serde_json::Value;

fn zone(value: &Value) -> Option<String> {
    if value["type"] != "a{sv}" || value["data"].as_array()?.len() != 1 {
        return None;
    }
    let values = value["data"][0].as_object()?;
    let timezone = values.get("Timezone")?;
    let rtc = values.get("LocalRTC")?;
    if timezone["type"] != "s" || rtc["type"] != "b" || rtc["data"] != false {
        return None;
    }
    let timezone = timezone["data"].as_str()?;
    let target = super::Target {
        timezone: Some(timezone.into()),
        ..Default::default()
    };
    target.validate().ok()?;
    Some(timezone.into())
}

pub(super) fn start_time(stat: &str) -> Option<u64> {
    // comm is parenthesized and may itself contain spaces and parentheses.
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct State {
    pub zone: String,
    link: String,
    dev: u64,
    ino: u64,
    adjtime: Option<String>,
}

#[cfg(target_os = "linux")]
fn trusted_file(path: &std::path::Path, limit: u64) -> Option<Vec<u8>> {
    use std::{io::Read, os::unix::fs::MetadataExt};
    let file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.uid() != 0 || meta.mode() & 0o022 != 0 || meta.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

#[cfg(target_os = "linux")]
pub(super) fn installed(zone: &str) -> bool {
    trusted_file(
        &std::path::Path::new("/usr/share/zoneinfo").join(zone),
        2 * 1024 * 1024,
    )
    .is_some()
}

#[cfg(target_os = "linux")]
pub(super) fn read() -> Option<State> {
    let raw = super::command::read(
        "/usr/bin/busctl",
        &[
            "--system",
            "--auto-start=no",
            "--json=short",
            "call",
            "org.freedesktop.timedate1",
            "/org/freedesktop/timedate1",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            "s",
            "org.freedesktop.timedate1",
        ],
    )?;
    let zone = zone(&serde_json::from_str(&raw).ok()?)?;
    disk(zone)
}

#[cfg(target_os = "linux")]
fn disk(zone: String) -> Option<State> {
    use std::os::unix::fs::MetadataExt;
    let current = std::path::Path::new("/etc/localtime");
    let meta = std::fs::symlink_metadata(current).ok()?;
    if !meta.file_type().is_symlink() || meta.uid() != 0 {
        return None;
    }
    let link = std::fs::read_link(current).ok()?.to_str()?.to_string();
    let expected = std::path::Path::new("/usr/share/zoneinfo").join(&zone);
    if std::fs::canonicalize(current).ok()? != std::fs::canonicalize(&expected).ok()?
        || trusted_file(&expected, 2 * 1024 * 1024)? != trusted_file(current, 2 * 1024 * 1024)?
    {
        return None;
    }
    let adjtime = match std::fs::symlink_metadata("/etc/adjtime") {
        Ok(meta) if meta.is_file() => {
            let bytes = trusted_file(std::path::Path::new("/etc/adjtime"), 4096)?;
            let text = std::str::from_utf8(&bytes).ok()?;
            if text.lines().count() != 3 || text.lines().nth(2)? != "UTC" {
                return None;
            }
            Some(crate::native_mcp::model::sha256_hex(text))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        _ => return None,
    };
    // Detect replacement while the independent disk evidence was being read.
    let after = std::fs::symlink_metadata(current).ok()?;
    if meta.dev() != after.dev()
        || meta.ino() != after.ino()
        || std::fs::read_link(current).ok()?.to_str()? != link
    {
        return None;
    }
    Some(State {
        zone,
        link,
        dev: meta.dev(),
        ino: meta.ino(),
        adjtime,
    })
}

#[cfg(target_os = "linux")]
fn authorization_code(action: &str) -> Option<i32> {
    use std::{process::Stdio, time::Duration};
    let start = start_time(&std::fs::read_to_string("/proc/self/stat").ok()?)?;
    let subject = format!("{},{},{}", std::process::id(), start, unsafe {
        libc::geteuid()
    });
    let mut command = crate::process::launch_command("/usr/bin/pkcheck");
    command
        .args([
            "--action-id",
            action,
            "--process",
            &subject,
            "--allow-user-interaction",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::process::capture_group(&mut command, Duration::from_secs(120), 4096)
        .ok()?
        .status
        .code()
}

#[cfg(target_os = "linux")]
pub(super) struct Authorization {
    code: Option<i32>,
    deadline: Option<u64>,
}

#[cfg(target_os = "linux")]
impl Authorization {
    pub(super) fn failure(&self) -> Option<&'static str> {
        match guarded(self.code, self.fresh(), true, || true) {
            "written" => None,
            other => Some(other),
        }
    }

    fn fresh(&self) -> bool {
        self.deadline
            .zip(super::clock_helper::boot_ms())
            .is_some_and(|(deadline, now)| super::clock_helper::valid_window(now, deadline))
    }
}

#[cfg(target_os = "linux")]
pub(super) fn authorize(action: &str) -> Authorization {
    let deadline = super::clock_helper::boot_ms().and_then(|now| now.checked_add(120_000));
    Authorization {
        code: authorization_code(action),
        deadline,
    }
}

// Authorization may take time or let an external settings change occur.
fn guarded(
    authorized: Option<i32>,
    fresh: bool,
    unchanged: bool,
    write: impl FnOnce() -> bool,
) -> &'static str {
    match authorized {
        Some(3) => "cancelled",
        Some(0) if !fresh || !unchanged => "stale",
        Some(0) if write() => "written",
        _ => "failed",
    }
}

#[cfg(target_os = "linux")]
#[derive(PartialEq, Eq)]
enum Dispatch {
    Written,
    Refused,
    Unknown,
    Stale,
}

fn definite_refusal(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.AccessDenied"
            | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired"
            | "org.freedesktop.DBus.Error.InvalidArgs"
    )
}

#[cfg(target_os = "linux")]
fn set_zone(
    target: &str,
    before: &State,
    deadline: Option<u64>,
    related_unchanged: impl FnOnce() -> bool + Send + 'static,
) -> Dispatch {
    use std::time::{Duration, Instant};
    let started = Instant::now();
    zbus::block_on(futures_lite::future::race(
        async {
            let Ok(connection) = zbus::Connection::system().await else {
                return Dispatch::Unknown;
            };
            let fresh = async {
                let reply = connection
                    .call_method(
                        Some("org.freedesktop.timedate1"),
                        "/org/freedesktop/timedate1",
                        Some("org.freedesktop.DBus.Properties"),
                        "GetAll",
                        &("org.freedesktop.timedate1",),
                    )
                    .await
                    .ok()?;
                let values: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
                    reply.body().deserialize().ok()?;
                if bool::try_from(values.get("LocalRTC")?).ok()? {
                    return None;
                }
                let zone = <&str>::try_from(values.get("Timezone")?).ok()?.to_string();
                super::Target {
                    timezone: Some(zone.clone()),
                    ..Default::default()
                }
                .validate()
                .ok()?;
                Some(zone)
            }
            .await;
            let Some(zone) = fresh else {
                return Dispatch::Stale;
            };
            let requested = target.to_string();
            let (fresh, available, related) =
                blocking::unblock(move || (disk(zone), installed(&requested), related_unchanged()))
                    .await;
            if started.elapsed() >= Duration::from_secs(8) {
                return Dispatch::Unknown;
            }
            if fresh.as_ref() != Some(before) || !available || !related {
                return Dispatch::Stale;
            }
            if !deadline
                .zip(super::clock_helper::boot_ms())
                .is_some_and(|(deadline, now)| super::clock_helper::valid_window(now, deadline))
            {
                return Dispatch::Stale;
            }
            // call_method uses an empty message flag set; the body also forbids
            // interaction. The sender PID is this same authorized process.
            match connection
                .call_method(
                    Some("org.freedesktop.timedate1"),
                    "/org/freedesktop/timedate1",
                    Some("org.freedesktop.timedate1"),
                    "SetTimezone",
                    &(target, false),
                )
                .await
            {
                Ok(_) => Dispatch::Written,
                Err(zbus::Error::MethodError(name, ..)) if definite_refusal(name.as_str()) => {
                    Dispatch::Refused
                }
                Err(_) => Dispatch::Unknown,
            }
        },
        async {
            async_io::Timer::after(Duration::from_secs(8)).await;
            Dispatch::Unknown
        },
    ))
}

#[cfg(target_os = "linux")]
pub(super) fn apply(
    item: &Item,
    before: &State,
    authorization: &Authorization,
    related_unchanged: impl FnOnce() -> bool + Send + 'static,
) -> (String, Item) {
    let mut result = item.clone();
    let Some(target) = item.expected.as_str().filter(|zone| installed(zone)) else {
        result.status = "unavailable".into();
        return ("partial".into(), result);
    };
    let current = read();
    let mut dispatch = Dispatch::Refused;
    let status = guarded(
        authorization.code,
        authorization.fresh(),
        current.as_ref() == Some(before),
        || {
            if !installed(target) {
                return false;
            }
            dispatch = set_zone(target, before, authorization.deadline, related_unchanged);
            dispatch == Dispatch::Written
        },
    );
    if status == "cancelled" {
        result.status = "not_applied".into();
        return ("cancelled".into(), result);
    }
    if status == "stale" || dispatch == Dispatch::Stale {
        result.status = "stale".into();
        return ("partial".into(), result);
    }
    result.status = match read() {
        Some(after) if after.zone == target && after.adjtime == before.adjtime => {
            if status == "written" {
                "verified"
            } else {
                "verified_after_error"
            }
        }
        None => "outcome_unknown",
        _ if dispatch == Dispatch::Unknown => "outcome_unknown",
        _ if status == "written" => "unverified",
        _ => "failed",
    }
    .into();
    let status = if result.status == "verified" {
        "applied"
    } else {
        "partial"
    };
    (status.into(), result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn broker_timeout_or_service_errors_do_not_prove_a_setting_was_refused() {
        for name in [
            "org.freedesktop.DBus.Error.NoReply",
            "org.freedesktop.DBus.Error.Timeout",
            "org.freedesktop.DBus.Error.Failed",
            "org.freedesktop.timedate1.Error.Failed",
        ] {
            assert!(!definite_refusal(name));
        }
        assert!(definite_refusal("org.freedesktop.DBus.Error.AccessDenied"));
        assert!(definite_refusal(
            "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired"
        ));
    }
    #[test]
    fn cached_zone_requires_typed_utc_rtc_and_valid_zone() {
        let mut raw = serde_json::json!({"type":"a{sv}","data":[{"Timezone":{"type":"s","data":"Europe/London"},"LocalRTC":{"type":"b","data":false}}]});
        assert_eq!(zone(&raw).as_deref(), Some("Europe/London"));
        for invalid in [
            Value::Null,
            serde_json::json!(true),
            serde_json::json!("false"),
        ] {
            raw["data"][0]["LocalRTC"]["data"] = invalid;
            assert!(zone(&raw).is_none());
        }
    }
    #[test]
    fn process_subject_handles_names_with_parentheses_and_spaces() {
        let stat = format!("123 (a ) b) S {} 456 0", vec!["0"; 18].join(" "));
        assert_eq!(start_time(&stat), Some(456));
        assert!(start_time("123 (name) S").is_none());
    }
    #[test]
    fn authorization_cancellation_expiry_and_changed_state_never_dispatch_a_setter() {
        for (auth, fresh, same, expected) in [
            (Some(3), true, true, "cancelled"),
            (Some(1), true, true, "failed"),
            (None, true, true, "failed"),
            (Some(0), false, true, "stale"),
            (Some(0), true, false, "stale"),
        ] {
            assert_eq!(
                guarded(auth, fresh, same, || panic!("Unauthorized or stale write")),
                expected
            );
        }
        assert_eq!(guarded(Some(0), true, true, || true), "written");
        assert_eq!(guarded(Some(0), true, true, || false), "failed");
    }
}
