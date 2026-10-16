use super::Target;
use crate::{Context, Request};
use anyhow::Result;
use serde_json::json;
use serde_json::Value;
use std::net::IpAddr;
use std::sync::Mutex;
#[cfg(target_os = "macos")]
use std::{process::Stdio, time::Duration};
#[cfg(target_os = "macos")]
type MutationGuard = std::fs::File;
#[cfg(any(target_os = "linux", windows))]
type MutationGuard = super::mutation_lock::Guard;

static REPAIR: Mutex<()> = Mutex::new(());

pub(super) enum Adjustment {
    Applied,
    Cancelled,
    Failed,
    Stale,
    #[cfg(any(target_os = "linux", windows, test))]
    NotNeeded,
}

fn clock_state(report: &Value) -> Option<&str> {
    report["checks"]
        .as_array()?
        .iter()
        .find(|row| row["id"] == "ntp.clock_offset")?["state"]
        .as_str()
}

fn finish(mut report: Value, before: &Value, status: &str) -> Value {
    report["time_repair"] = json!({"status":status,
        "before_offset_ms":before["time_sample"]["offset_ms"],
        "before_uncertainty_ms":before["time_sample"]["uncertainty_ms"]});
    report
}

// The command runner is a separate boundary so tests can prove that no
// authorization or clock write occurs for an uncertain/in-tolerance sample.
fn run(
    mut measure: impl FnMut() -> Result<Value>,
    adjust: impl FnOnce(IpAddr) -> Adjustment,
    mut audit: impl FnMut(&Value) -> Result<()>,
) -> Result<Value> {
    let before = measure()?;
    match clock_state(&before) {
        Some("match") => return Ok(finish(before.clone(), &before, "not_needed")),
        Some("mismatch") => (),
        _ => return Ok(finish(before.clone(), &before, "unavailable")),
    }
    let peer: IpAddr = before["time_sample"]["peer_address"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No measured time-server address."))?
        .parse()?;
    audit(&finish(before.clone(), &before, "requested")).map_err(|_| {
        anyhow::anyhow!(
            "Could not record the time correction request; no authorization was requested."
        )
    })?;
    let outcome = adjust(peer);
    let after = measure()?;
    let status = match outcome {
        Adjustment::Cancelled => "cancelled",
        Adjustment::Failed => "failed",
        Adjustment::Stale => "unavailable",
        #[cfg(any(target_os = "linux", windows, test))]
        Adjustment::NotNeeded if clock_state(&after) == Some("match") => "not_needed",
        #[cfg(any(target_os = "linux", windows, test))]
        Adjustment::NotNeeded => "unverified",
        Adjustment::Applied if clock_state(&after) == Some("match") => "verified",
        Adjustment::Applied => "unverified",
    };
    let mut result = finish(after, &before, status);
    result["time_repair"]["audit_recorded"] = json!(audit(&result).is_ok());
    Ok(result)
}

fn record(ctx: &Context, req: &Request, report: &Value) -> Result<()> {
    let repair = &report["time_repair"];
    let number = |value: &Value| value.as_f64().filter(|value| value.is_finite());
    let detail = json!({
        "status":repair["status"],
        "before_offset_ms":number(&repair["before_offset_ms"]),
        "before_uncertainty_ms":number(&repair["before_uncertainty_ms"]),
        "offset_ms":number(&report["time_sample"]["offset_ms"]),
        "uncertainty_ms":number(&report["time_sample"]["uncertainty_ms"]),
    });
    let conn = crate::store::open_write(ctx)?;
    crate::store::record_operation(
        &conn,
        Some(&req.run_id),
        "environment:sync-time",
        Some("system-clock"),
        Some(&detail),
    )
}

#[cfg(target_os = "macos")]
const CLOCK_COMMAND: &str = r#"age=$(( $(/bin/date +%s) - $2 )); if [ $age -lt 0 ] || [ $age -gt 10 ]; then echo stale; exit 0; fi; /usr/bin/sntp -S -s -n 1 -g 4000 -t 5 "$1" >/dev/null 2>&1 && echo applied"#;

#[cfg(target_os = "macos")]
const SCRIPT: &str = r#"on run argv
try
    return do shell script "/usr/bin/env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin LC_ALL=C /bin/sh -c " & quoted form of (item 3 of argv) & " clock " & quoted form of (item 1 of argv) & " " & quoted form of (item 2 of argv) with administrator privileges
on error messageText number errorNumber
    if errorNumber is -128 then return "cancelled"
    return "failed"
end try
end run"#;

#[cfg(target_os = "macos")]
fn adjust(peer: IpAddr, measured_at: i64) -> Adjustment {
    super::time_probe::wait_for_interval();
    let mut command = crate::process::launch_command("/usr/bin/osascript");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LC_ALL", "C")
        .args([
            "-e",
            SCRIPT,
            &peer.to_string(),
            &measured_at.to_string(),
            CLOCK_COMMAND,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = crate::process::capture_group(&mut command, Duration::from_secs(150), 16 * 1024);
    // The privileged SNTP request is external to our UDP client. Start a new
    // cooldown after it returns before the independent verification query.
    super::time_probe::mark_query();
    match output {
        Ok(value) if value.status.success() && value.stdout == b"applied\n" => Adjustment::Applied,
        Ok(value) if value.status.success() && value.stdout == b"cancelled\n" => {
            Adjustment::Cancelled
        }
        Ok(value) if value.status.success() && value.stdout == b"stale\n" => Adjustment::Stale,
        _ => Adjustment::Failed,
    }
}

pub(super) fn handle(ctx: &Context, req: &Request, target: &Target) -> Result<Value> {
    target.validate_time_repair()?;
    #[cfg(target_os = "macos")]
    let guard = super::repair_macos::mutation_lock()?;
    #[cfg(any(target_os = "linux", windows))]
    let guard = super::mutation_lock::acquire()?;
    handle_locked(ctx, req, target, &guard)
}

pub(super) fn handle_locked(
    ctx: &Context,
    req: &Request,
    target: &Target,
    _guard: &MutationGuard,
) -> Result<Value> {
    target.validate_time_repair()?;
    let _guard = REPAIR
        .try_lock()
        .map_err(|_| anyhow::anyhow!("A time correction is already running."))?;
    #[cfg(target_os = "macos")]
    let measured_at = std::cell::Cell::new(0);
    run(
        || {
            super::time_probe::wait_for_interval();
            let report = super::report(target, true)?;
            #[cfg(target_os = "macos")]
            measured_at.set(chrono::Utc::now().timestamp());
            Ok(report)
        },
        |peer| {
            #[cfg(target_os = "macos")]
            {
                adjust(peer, measured_at.get())
            }
            #[cfg(any(target_os = "linux", windows))]
            {
                super::time_repair_native::adjust(peer, target.max_clock_offset_ms.unwrap())
            }
        },
        |report| record(ctx, req, report),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[cfg(target_os = "macos")]
    #[test]
    fn authorization_delay_or_backwards_clock_never_reaches_sntp() {
        for measured_at in [0, chrono::Utc::now().timestamp() + 60] {
            let output = std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    CLOCK_COMMAND,
                    "clock",
                    "192.0.2.1",
                    &measured_at.to_string(),
                ])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, b"stale\n");
        }
    }

    fn sample(state: &str) -> Value {
        json!({"checks":[{"id":"ntp.clock_offset","state":state}],"time_sample":{"peer_address":"192.0.2.1","offset_ms":3000.0,"uncertainty_ms":20.0}})
    }

    #[test]
    fn uncertain_or_matching_clocks_never_request_authorization() {
        for (state, status) in [("match", "not_needed"), ("unknown", "unavailable")] {
            let result = run(
                || Ok(sample(state)),
                |_| panic!("must not adjust"),
                |_| panic!("must not audit"),
            )
            .unwrap();
            assert_eq!(result["time_repair"]["status"], status);
        }
    }

    #[test]
    fn success_requires_a_new_matching_measurement_and_preserves_cancellation() {
        for (outcome, measured, status) in [
            (Adjustment::Applied, "match", "verified"),
            (Adjustment::Applied, "unknown", "unverified"),
            (Adjustment::Applied, "mismatch", "unverified"),
            (Adjustment::Cancelled, "mismatch", "cancelled"),
            (Adjustment::Failed, "match", "failed"),
            (Adjustment::Stale, "match", "unavailable"),
            (Adjustment::NotNeeded, "match", "not_needed"),
            (Adjustment::NotNeeded, "mismatch", "unverified"),
        ] {
            let calls = Cell::new(0);
            let result = run(
                || {
                    calls.set(calls.get() + 1);
                    Ok(sample(if calls.get() == 1 {
                        "mismatch"
                    } else {
                        measured
                    }))
                },
                |peer| {
                    assert_eq!(peer.to_string(), "192.0.2.1");
                    outcome
                },
                |_| Ok(()),
            )
            .unwrap();
            assert_eq!(calls.get(), 2);
            assert_eq!(result["time_repair"]["status"], status);
        }
    }

    #[test]
    fn correction_records_request_and_outcome_without_server_or_environment_data() {
        let root =
            std::env::temp_dir().join(format!("time-repair-audit-{}", crate::store::uuid_v4()));
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        let req = Request {
            command: "environment:sync-time".into(),
            args: vec![],
            run_id: "repair-audit".into(),
        };
        let calls = Cell::new(0);
        let result = run(
            || {
                calls.set(calls.get() + 1);
                Ok(sample(if calls.get() == 1 {
                    "mismatch"
                } else {
                    "match"
                }))
            },
            |_| Adjustment::Applied,
            |report| record(&ctx, &req, report),
        )
        .unwrap();
        assert_eq!(result["time_repair"]["audit_recorded"], true);
        let conn = crate::store::open_readonly(&ctx).unwrap().unwrap();
        let mut query = conn
            .prepare("SELECT run_id, target, detail_json FROM operation_logs WHERE operation_type='environment:sync-time' ORDER BY rowid")
            .unwrap();
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        for (index, (run_id, target, detail)) in rows.iter().enumerate() {
            assert_eq!(run_id, "repair-audit");
            assert_eq!(target, "system-clock");
            let detail: Value = serde_json::from_str(detail).unwrap();
            assert_eq!(
                detail["status"],
                if index == 0 { "requested" } else { "verified" }
            );
            assert_eq!(detail.as_object().unwrap().len(), 5);
            assert_eq!(detail["before_offset_ms"], 3000.0);
        }
        assert!(!ctx.home_dir.exists());
        drop(query);
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn audit_failure_prevents_authorization_or_preserves_the_completed_measurement() {
        assert!(run(
            || Ok(sample("mismatch")),
            |_| panic!("must not authorize"),
            |_| anyhow::bail!("audit failed")
        )
        .is_err());
        let count = Cell::new(0);
        let result = run(
            || Ok(sample("mismatch")),
            |_| Adjustment::Cancelled,
            |_| {
                count.set(count.get() + 1);
                if count.get() == 2 {
                    anyhow::bail!("audit failed")
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(result["time_repair"]["status"], "cancelled");
        assert_eq!(result["time_repair"]["audit_recorded"], false);
        assert_eq!(result["time_sample"]["offset_ms"], 3000.0);
    }
}
