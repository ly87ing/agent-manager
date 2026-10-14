//! Read-only Darwin kernel clock status, independent of an external NTP probe.
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct KernelTime {
    pub synchronized: bool,
    pub state_code: i32,
    pub max_error_ms: f64,
    pub estimated_error_ms: f64,
}

pub(super) fn collect() -> Option<KernelTime> {
    // SAFETY: ntptimeval contains only integer fields. The initialized buffer
    // remains valid for this synchronous, read-only call.
    let mut value: libc::ntptimeval = unsafe { std::mem::zeroed() };
    value.time_state = -1;
    let result = unsafe { libc::ntp_gettime(&mut value) };
    let error = std::io::Error::last_os_error().raw_os_error();
    decode(result, error, &value)
}

fn decode(result: i32, error: Option<i32>, value: &libc::ntptimeval) -> Option<KernelTime> {
    let code = value.time_state;
    // Darwin copies out the clock state, then returns it as the syscall error.
    // TIME_ERROR therefore becomes -1/EIO with a populated time_state, whereas
    // an ordinary read failure leaves the sentinel untouched. See XNU's
    // bsd/kern/kern_ntptime.c: ntp_gettime and ntp_is_time_error.
    if !(libc::TIME_OK..=libc::TIME_ERROR).contains(&code)
        || !(result == code || (result == -1 && error == Some(code)))
        || value.maxerror < 0
        || value.esterror < 0
    {
        return None;
    }
    Some(KernelTime {
        synchronized: code != libc::TIME_ERROR,
        state_code: code,
        max_error_ms: value.maxerror as f64 / 1000.0,
        estimated_error_ms: value.esterror as f64 / 1000.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_kernel_state_without_treating_read_errors_as_synchronization() {
        let mut value: libc::ntptimeval = unsafe { std::mem::zeroed() };
        value.time_state = libc::TIME_OK;
        value.maxerror = 250_000;
        value.esterror = 125_000;
        let status = decode(0, None, &value).unwrap();
        assert!(status.synchronized);
        assert_eq!(status.max_error_ms, 250.0);
        assert_eq!(status.estimated_error_ms, 125.0);

        value.time_state = libc::TIME_ERROR;
        assert!(!decode(-1, Some(libc::EIO), &value).unwrap().synchronized);
        assert!(decode(-1, Some(libc::EPERM), &value).is_none());
        value.time_state = -1;
        assert!(decode(-1, Some(libc::EIO), &value).is_none());
        value.time_state = 99;
        assert!(decode(99, None, &value).is_none());
        value.time_state = libc::TIME_OK;
        value.maxerror = -1;
        assert!(decode(0, None, &value).is_none());
    }
}
