//! Bounded fixed-purpose pipe messages; never paths, environments or raw configuration.
use super::repair_linux_ntp_file::Publication;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    io::{Read, Write},
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

// Reading a byte after poll avoids buffering a later frame or blocking on a
// partial line. The deadline covers idle pipes and oversized unterminated data.
pub(super) fn receive<T: DeserializeOwned>(
    input: &mut (impl Read + AsRawFd),
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
        if input.read(&mut byte).ok()? != 1 {
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
