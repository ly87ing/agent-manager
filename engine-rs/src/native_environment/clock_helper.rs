//! Restricted elevated clock helper: fixed peer, fresh measurement and one clock-only write.
use std::net::IpAddr;
use std::time::Duration;
#[cfg(any(target_os = "linux", windows))]
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const FLAG: &str = "--environment-clock-helper";
const LEASE_MS: u64 = 120_000;

fn arguments(args: &[String]) -> Option<(IpAddr, u32, u64)> {
    if args.len() != 5 || args[0] != FLAG || args[4] != "--confirm" {
        return None;
    }
    let peer = args[1].parse().ok()?;
    let limit = args[2]
        .parse()
        .ok()
        .filter(|limit| (1..=60_000).contains(limit))?;
    Some((peer, limit, args[3].parse().ok()?))
}

#[cfg(target_os = "linux")]
pub(super) fn boot_ms() -> Option<u64> {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    (unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut value) } == 0)
        .then(|| (value.tv_sec as u64).saturating_mul(1000) + (value.tv_nsec as u64) / 1_000_000)
}
#[cfg(windows)]
pub(super) fn boot_ms() -> Option<u64> {
    Some(unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() })
}

pub(super) fn valid_window(now: u64, deadline: u64) -> bool {
    deadline
        .checked_sub(now)
        .is_some_and(|remaining| remaining > 0 && remaining <= LEASE_MS)
}
fn stable_clock(age: Duration, wall_elapsed: f64) -> bool {
    age <= Duration::from_secs(10)
        && wall_elapsed.is_finite()
        && (wall_elapsed - age.as_secs_f64()).abs() <= 0.001
}

fn correct(
    limit: u32,
    offset_ms: f64,
    uncertainty_ms: f64,
    fresh: bool,
    write: impl FnOnce(f64) -> bool,
) -> i32 {
    if !fresh || !offset_ms.is_finite() || !uncertainty_ms.is_finite() || uncertainty_ms < 0.0 {
        return 74;
    }
    match super::clock_state(offset_ms, uncertainty_ms, limit) {
        "match" => 72,
        "mismatch" => {
            if write(offset_ms) {
                0
            } else {
                1
            }
        }
        _ => 74,
    }
}

#[cfg(target_os = "linux")]
fn prepare() -> bool {
    unsafe { libc::geteuid() == 0 }
}
#[cfg(windows)]
fn prepare() -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, SetLastError, LUID},
        Security::{
            AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES,
            SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let mut token = std::ptr::null_mut();
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
    } == 0
    {
        return false;
    }
    let name: Vec<_> = "SeSystemtimePrivilege\0".encode_utf16().collect();
    let mut luid = LUID {
        LowPart: 0,
        HighPart: 0,
    };
    let result = unsafe {
        if LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut luid) == 0 {
            false
        } else {
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            SetLastError(0);
            AdjustTokenPrivileges(
                token,
                0,
                &privileges,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) != 0
                && GetLastError() == 0
        }
    };
    unsafe {
        CloseHandle(token);
    }
    result
}

#[cfg(target_os = "linux")]
fn clock_delta(offset_ms: f64) -> Option<libc::timex> {
    let micros = offset_ms * 1000.0;
    if !micros.is_finite() || micros.abs() > i64::MAX as f64 {
        return None;
    }
    let micros = micros.round() as i64;
    let mut change: libc::timex = unsafe { std::mem::zeroed() };
    // The kernel adds this offset to the clock at the syscall, avoiding user-space
    // read/compute/set delays. Other time-service/frequency/status fields stay unset.
    // ADJ_NANO would persistently change the kernel's NTP resolution mode.
    change.modes = libc::ADJ_SETOFFSET;
    change.time.tv_sec = micros.div_euclid(1_000_000) as _;
    change.time.tv_usec = micros.rem_euclid(1_000_000) as _;
    Some(change)
}
#[cfg(target_os = "linux")]
fn write_clock(offset_ms: f64) -> bool {
    let Some(mut change) = clock_delta(offset_ms) else {
        return false;
    };
    unsafe { libc::clock_adjtime(libc::CLOCK_REALTIME, &mut change) >= 0 }
}
#[cfg(windows)]
fn write_clock(offset_ms: f64) -> bool {
    use chrono::{Datelike, Timelike};
    use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::SetSystemTime};
    let nanos = offset_ms * 1_000_000.0;
    if !nanos.is_finite() || nanos.abs() > i64::MAX as f64 {
        return false;
    }
    let Some(time) = chrono::DateTime::<chrono::Utc>::from(SystemTime::now())
        .checked_add_signed(chrono::Duration::nanoseconds(nanos.round() as i64))
    else {
        return false;
    };
    let time = SYSTEMTIME {
        wYear: time.year() as u16,
        wMonth: time.month() as u16,
        wDayOfWeek: 0,
        wDay: time.day() as u16,
        wHour: time.hour() as u16,
        wMinute: time.minute() as u16,
        wSecond: time.second() as u16,
        wMilliseconds: time.timestamp_subsec_millis() as u16,
    };
    unsafe { SetSystemTime(&time) != 0 }
}

#[cfg(any(target_os = "linux", windows))]
pub(super) fn run(args: &[String]) -> i32 {
    let Some((peer, limit, deadline)) = arguments(args) else {
        return 2;
    };
    if !boot_ms().is_some_and(|now| valid_window(now, deadline)) {
        return 74;
    }
    if !prepare() {
        return 1;
    }
    let Ok(sample) = super::time_probe::query(&peer.to_string()) else {
        return 74;
    };
    let fresh = || {
        let elapsed = sample.captured.elapsed();
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|value| value.as_secs_f64() - sample.captured_wall);
        boot_ms().is_some_and(|now| valid_window(now, deadline))
            && wall.is_some_and(|wall| stable_clock(elapsed, wall))
    };
    correct(
        limit,
        sample.offset_ms,
        sample.uncertainty_ms,
        fresh(),
        |offset| fresh() && write_clock(offset),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[cfg(target_os = "linux")]
    #[test]
    fn kernel_delta_normalizes_negative_offsets_without_changing_ntp_mode() {
        for (offset, seconds, micros) in [
            (1250.0, 1, 250_000),
            (-1250.0, -2, 750_000),
            (-0.0005, -1, 999_999),
        ] {
            let value = clock_delta(offset).unwrap();
            assert_eq!(value.modes, libc::ADJ_SETOFFSET);
            assert_eq!((value.time.tv_sec, value.time.tv_usec), (seconds, micros));
            assert_eq!((value.offset, value.freq, value.status), (0, 0, 0));
        }
        assert!(clock_delta(f64::NAN).is_none());
        assert!(clock_delta(f64::MAX).is_none());
    }
    #[test]
    fn helper_accepts_only_a_fixed_peer_bounded_tolerance_and_confirmation() {
        let valid: Vec<String> = [FLAG, "192.0.2.1", "2000", "100", "--confirm"]
            .map(String::from)
            .into();
        assert_eq!(
            arguments(&valid),
            Some(("192.0.2.1".parse().unwrap(), 2000, 100))
        );
        for (index, value) in [
            (1, "example.invalid"),
            (1, "127.0.0.1:123"),
            (2, "0"),
            (2, "60001"),
            (3, "-1"),
            (4, "yes"),
        ] {
            let mut invalid = valid.clone();
            invalid[index] = value.into();
            assert!(arguments(&invalid).is_none());
        }
        let mut extra = valid;
        extra.push("--extra".into());
        assert!(arguments(&extra).is_none());
    }
    #[test]
    fn uncertain_expired_and_already_corrected_samples_never_write() {
        for (offset, error, fresh, code) in [
            (5000.0, 1.0, false, 74),
            (5.0, 10.0, true, 74),
            (2.0, 1.0, true, 72),
        ] {
            assert_eq!(
                correct(10, offset, error, fresh, |_| panic!(
                    "No clock write is allowed"
                )),
                code
            );
        }
        let written = Cell::new(0.0);
        assert_eq!(
            correct(10, 1250.0, 1.0, true, |offset| {
                written.set(offset);
                true
            }),
            0
        );
        assert_eq!(written.get(), 1250.0);
    }
    #[test]
    fn final_gate_rejects_deadline_expiry_and_clock_steps_in_both_directions() {
        assert!(valid_window(1000, 121000));
        for (now, end) in [(1000, 1000), (1001, 1000), (1000, 121001)] {
            assert!(!valid_window(now, end));
        }
        assert!(stable_clock(Duration::from_secs(2), 2.0));
        for wall in [7.0, -3.0, f64::NAN] {
            assert!(!stable_clock(Duration::from_secs(2), wall));
        }
        assert!(!stable_clock(Duration::from_secs(11), 11.0));
    }
}
