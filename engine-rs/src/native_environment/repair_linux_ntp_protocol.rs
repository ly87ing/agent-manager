//! Bounded fixed-purpose pipe messages; never paths, environments or raw configuration.
use super::repair_linux_ntp_file::Publication;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    io::Write,
    os::fd::AsRawFd,
    time::{Duration, Instant},
};

pub(super) const VERSION: u8 = 1;
const LIMIT: usize = 8192;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "message", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    Commit { version: u8, base: String },
    Ack { version: u8, after: String },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "message", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Response {
    Ready {
        version: u8,
        base: String,
    },
    Done {
        version: u8,
        publication: Publication,
        runtime: String,
        after: Option<String>,
    },
}

pub(super) fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn send(output: &mut impl Write, value: &impl Serialize) -> bool {
    let Ok(mut bytes) = serde_json::to_vec(value) else {
        return false;
    };
    if bytes.len() > LIMIT {
        return false;
    }
    bytes.push(b'\n');
    output
        .write_all(&bytes)
        .and_then(|_| output.flush())
        .is_ok()
}

// Poll and read the same kernel descriptor. Stdin's Read implementation buffers
// ahead, which can hide the rest of a frame from poll while its writer stays open.
// The caller owns the only reader; the deadline covers idle and partial frames.
pub(super) fn receive<T: DeserializeOwned>(
    input: &mut impl AsRawFd,
    deadline: Instant,
    live: impl Fn() -> bool,
) -> Option<T> {
    let mut bytes = Vec::new();
    loop {
        if Instant::now() >= deadline || !live() {
            return None;
        }
        let mut descriptor = libc::pollfd {
            fd: input.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let polled = unsafe { libc::poll(&mut descriptor, 1, 50) };
        if polled < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return None;
        }
        if polled == 0 {
            continue;
        }
        let mut byte = [0u8];
        if unsafe { libc::read(input.as_raw_fd(), byte.as_mut_ptr().cast(), 1) } != 1 {
            return None;
        }
        if byte[0] == b'\n' {
            return serde_json::from_slice(&bytes).ok();
        }
        if bytes.len() == LIMIT {
            return None;
        }
        bytes.push(byte[0]);
    }
}

pub(super) fn wait_window() -> Instant {
    Instant::now() + Duration::from_secs(120)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn standard_input_frames_do_not_hide_in_a_userspace_buffer() {
        const MARKER: &str = "AM_NTP_PROTOCOL_STDIN_FIXTURE";
        if std::env::var(MARKER).as_deref() == Ok("1") {
            let mut input = std::io::stdin();
            for expected in ["a".repeat(64), "b".repeat(64)] {
                let request: Option<Request> =
                    receive(&mut input, Instant::now() + Duration::from_secs(1), || true);
                assert!(matches!(request, Some(Request::Commit { base, .. }) if base == expected));
            }
            return;
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "native_environment::repair_linux_ntp_protocol::tests::standard_input_frames_do_not_hide_in_a_userspace_buffer", "--nocapture"])
            .env(MARKER, "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn().unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut frames = Vec::new();
        for base in ["a".repeat(64), "b".repeat(64)] {
            assert!(send(
                &mut frames,
                &Request::Commit {
                    version: VERSION,
                    base
                }
            ));
        }
        input.write_all(&frames).unwrap();
        input.flush().unwrap();
        // Keep the writer open: EOF must not make buffered data appear ready.
        let deadline = Instant::now() + Duration::from_secs(4);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("Stdin frame fixture exceeded its deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        drop(input);
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn pipe_frames_reject_extra_fields_eof_oversize_and_idle_input() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        let base = "a".repeat(64);
        assert!(send(
            &mut writer,
            &Request::Commit {
                version: VERSION,
                base: base.clone()
            }
        ));
        assert!(
            matches!(receive(&mut reader, wait_window(), || true), Some(Request::Commit { version: VERSION, base: observed }) if observed == base)
        );
        writer
            .write_all(
                b"{\"message\":\"commit\",\"version\":1,\"base\":\"x\",\"path\":\"unexpected\"}\n",
            )
            .unwrap();
        assert!(receive::<Request>(&mut reader, wait_window(), || true).is_none());
        let sender = std::thread::spawn(move || {
            writer.write_all(&vec![b'x'; LIMIT + 1]).unwrap();
        });
        assert!(receive::<Request>(&mut reader, wait_window(), || true).is_none());
        sender.join().unwrap();
        assert!(receive::<Request>(&mut reader, wait_window(), || true).is_none());
        let (mut reader, _writer) = UnixStream::pair().unwrap();
        let start = Instant::now();
        assert!(
            receive::<Request>(&mut reader, start + Duration::from_millis(20), || true).is_none()
        );
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(receive::<Request>(&mut reader, wait_window(), || false).is_none());
    }

    #[test]
    fn digests_are_exact_and_messages_do_not_encode_arbitrary_paths() {
        assert!(digest(&"ab01".repeat(16)));
        for invalid in [
            "a".repeat(63),
            "a".repeat(65),
            "A".repeat(64),
            "g".repeat(64),
        ] {
            assert!(!digest(&invalid));
        }
        let mut output = Vec::new();
        assert!(!send(
            &mut output,
            &Request::Commit {
                version: VERSION,
                base: "x".repeat(LIMIT)
            }
        ));
        assert!(output.is_empty());
    }
}
