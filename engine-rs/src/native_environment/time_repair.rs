use super::Target;
use anyhow::Result;
#[cfg(any(target_os = "macos", test))]
use serde_json::json;
use serde_json::Value;
#[cfg(any(target_os = "macos", test))]
use std::net::IpAddr;
#[cfg(target_os = "macos")]
use std::{process::Stdio, sync::Mutex, time::Duration};

#[cfg(target_os = "macos")]
static REPAIR: Mutex<()> = Mutex::new(());

#[cfg(any(target_os = "macos", test))]
enum Adjustment {
    Applied,
    Cancelled,
    Failed,
}

#[cfg(any(target_os = "macos", test))]
fn clock_state(report: &Value) -> Option<&str> {
    report["checks"]
        .as_array()?
        .iter()
        .find(|row| row["id"] == "ntp.clock_offset")?["state"]
        .as_str()
}

#[cfg(any(target_os = "macos", test))]
fn finish(mut report: Value, before: &Value, status: &str) -> Value {
    report["time_repair"] = json!({"status":status,
        "before_offset_ms":before["time_sample"]["offset_ms"],
        "before_uncertainty_ms":before["time_sample"]["uncertainty_ms"]});
    report
}

// The command runner is a separate boundary so tests can prove that no
// authorization or clock write occurs for an uncertain/in-tolerance sample.
#[cfg(any(target_os = "macos", test))]
fn run(
    mut measure: impl FnMut() -> Result<Value>,
    adjust: impl FnOnce(IpAddr) -> Adjustment,
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
    let outcome = adjust(peer);
    let after = measure()?;
    let status = match outcome {
        Adjustment::Cancelled => "cancelled",
        Adjustment::Failed => "failed",
        Adjustment::Applied if clock_state(&after) == Some("match") => "verified",
        Adjustment::Applied => "unverified",
    };
    Ok(finish(after, &before, status))
}

#[cfg(target_os = "macos")]
const SCRIPT: &str = r#"on run argv
try
    do shell script "/usr/bin/env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin LC_ALL=C /usr/bin/sntp -S -s -n 1 -g 4000 -t 5 " & quoted form of (item 1 of argv) with administrator privileges
    return "applied"
on error messageText number errorNumber
    if errorNumber is -128 then return "cancelled"
    return "failed"
end try
end run"#;

#[cfg(target_os = "macos")]
fn adjust(peer: IpAddr) -> Adjustment {
    super::time_probe::wait_for_interval();
    let mut command = crate::process::launch_command("/usr/bin/osascript");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LC_ALL", "C")
        .args(["-e", SCRIPT, &peer.to_string()])
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
        _ => Adjustment::Failed,
    }
}

pub(super) fn handle(target: &Target) -> Result<Value> {
    target.validate_time_repair()?;
    #[cfg(target_os = "macos")]
    {
        let _guard = REPAIR
            .try_lock()
            .map_err(|_| anyhow::anyhow!("A time correction is already running."))?;
        run(
            || {
                super::time_probe::wait_for_interval();
                super::report(target, true)
            },
            adjust,
        )
    }
    #[cfg(not(target_os = "macos"))]
    anyhow::bail!("Time correction currently supports macOS only.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn sample(state: &str) -> Value {
        json!({"checks":[{"id":"ntp.clock_offset","state":state}],"time_sample":{"peer_address":"192.0.2.1","offset_ms":3000.0,"uncertainty_ms":20.0}})
    }

    #[test]
    fn uncertain_or_matching_clocks_never_request_authorization() {
        for (state, status) in [("match", "not_needed"), ("unknown", "unavailable")] {
            let result = run(|| Ok(sample(state)), |_| panic!("must not adjust")).unwrap();
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
            )
            .unwrap();
            assert_eq!(calls.get(), 2);
            assert_eq!(result["time_repair"]["status"], status);
        }
    }
}
