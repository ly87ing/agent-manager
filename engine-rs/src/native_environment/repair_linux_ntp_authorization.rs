//! Caller-owned authorization holds a zero-write READY helper until COMMIT.
use super::{
    repair_linux_ntp as native, repair_linux_ntp_lease as lease,
    repair_linux_ntp_protocol as protocol,
    time_repair_native::{self, Prepared},
};
use std::{
    process::{Child, ChildStdin, ChildStdout, Stdio},
    time::{Duration, Instant},
};

pub(super) struct Authorization {
    _prepared: Prepared,
    child: Option<Child>,
    group: crate::process::Group,
    input: Option<ChildStdin>,
    output: ChildStdout,
    deadline: Instant,
    base: String,
}

pub(super) struct Outcome {
    pub complete: bool,
    pub saved: String,
    pub runtime: String,
    pub directory: String,
    pub expected: Option<native::State>,
}

impl Outcome {
    fn unknown() -> Self {
        Self {
            complete: false,
            saved: "outcome_unknown".into(),
            runtime: "outcome_unknown".into(),
            directory: "outcome_unknown".into(),
            expected: None,
        }
    }
}

impl Drop for Authorization {
    fn drop(&mut self) {
        // EOF and the helper's pidfd/deadline gate future writes. A non-root
        // signal is not proof an already elevated child or in-flight RPC stopped.
        self.input.take();
        if let Some(mut child) = self.child.take() {
            self.group.kill(&mut child);
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

pub(super) fn authorize(
    before: &native::State,
    server: &str,
) -> Result<Authorization, &'static str> {
    let base = before.digest().ok_or("unavailable")?;
    let prepared = time_repair_native::prepare().ok_or("unavailable")?;
    let birth = std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|text| super::repair_linux_time::start_time(&text))
        .ok_or("unavailable")?;
    let boot_deadline = super::clock_helper::boot_ms()
        .and_then(|now| now.checked_add(120_000))
        .ok_or("unavailable")?;
    let deadline = protocol::wait_window();
    let mut command = crate::process::launch_command("/usr/bin/pkexec");
    command
        .arg("--disable-internal-agent")
        .arg(&prepared.path)
        .args([
            lease::FLAG.to_string(),
            server.to_string(),
            std::process::id().to_string(),
            birth.to_string(),
            unsafe { libc::geteuid() }.to_string(),
            boot_deadline.to_string(),
            "--confirm".into(),
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let (mut child, group) = crate::process::spawn_group(&mut command).map_err(|_| "failed")?;
    let input = child.stdin.take();
    let output = child.stdout.take().expect("piped helper output");
    let mut authorization = Authorization {
        _prepared: prepared,
        child: Some(child),
        group,
        input,
        output,
        deadline,
        base,
    };
    match protocol::receive(&mut authorization.output, deadline, || true) {
        Some(protocol::Response::Ready {
            version: protocol::VERSION,
            base,
        }) if protocol::digest(&base) && base == authorization.base => Ok(authorization),
        _ => Err(authorization.failure()),
    }
}

impl Authorization {
    fn failure(&mut self) -> &'static str {
        // EOF can become visible just before waitpid reports process exit.
        // Briefly reap the actual result instead of misreporting dismissal.
        let end = (Instant::now() + Duration::from_millis(250)).min(self.deadline);
        loop {
            match self
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten())
            {
                Some(status) => {
                    return match status.code() {
                        Some(126) => "cancelled",
                        Some(69) => "unavailable",
                        Some(74) => "stale",
                        _ => "failed",
                    }
                }
                None if Instant::now() < end => std::thread::sleep(Duration::from_millis(10)),
                _ => return "stale",
            }
        }
    }
    pub(super) fn apply(
        mut self,
        before: &native::State,
        server: &str,
        verify: impl FnOnce(&native::State) -> bool,
    ) -> Outcome {
        if Instant::now() >= self.deadline || before.digest().as_deref() != Some(&self.base) {
            return Outcome::unknown();
        }
        let Some(input) = self.input.as_mut() else {
            return Outcome::unknown();
        };
        if !protocol::send(
            input,
            &protocol::Request::Commit {
                version: protocol::VERSION,
                base: self.base.clone(),
            },
        ) {
            return Outcome::unknown();
        }
        let Some(protocol::Response::Done {
            version: protocol::VERSION,
            publication,
            runtime,
            after,
        }) = protocol::receive(&mut self.output, self.deadline, || true)
        else {
            return Outcome::unknown();
        };
        if !matches!(
            runtime.as_str(),
            "written" | "unchanged" | "failed" | "stale" | "outcome_unknown" | "not_applied"
        ) {
            return Outcome::unknown();
        }
        let Some(configuration) = publication.project(&before.configuration, server) else {
            return Outcome::unknown();
        };
        let mut expected = before.clone();
        expected.configuration = configuration;
        if matches!(
            runtime.as_str(),
            "written" | "unchanged" | "outcome_unknown"
        ) {
            let Some(projected) = expected.with_runtime(server) else {
                return Outcome::unknown();
            };
            expected = projected;
        }
        let digest = expected.digest();
        let independently_verified = verify(&expected);
        let acknowledged = independently_verified
            && after.as_ref().is_some_and(|after| protocol::digest(after))
            && digest == after
            && protocol::send(
                input,
                &protocol::Request::Ack {
                    version: protocol::VERSION,
                    after: after.unwrap(),
                },
            );
        let exited = if acknowledged {
            let end = (Instant::now() + Duration::from_secs(8)).min(self.deadline);
            loop {
                match self
                    .child
                    .as_mut()
                    .and_then(|child| child.try_wait().ok().flatten())
                {
                    Some(status) => break status.success(),
                    None if Instant::now() < end => std::thread::sleep(Duration::from_millis(20)),
                    _ => break false,
                }
            }
        } else {
            false
        };
        let complete = independently_verified
            && acknowledged
            && exited
            && publication.durable
            && matches!(publication.status.as_str(), "saved" | "unchanged")
            && matches!(runtime.as_str(), "written" | "unchanged");
        let readback = |written: bool, unchanged: bool| {
            if unchanged && independently_verified {
                "unchanged"
            } else if written && independently_verified {
                if complete {
                    "verified"
                } else {
                    "verified_after_error"
                }
            } else {
                "outcome_unknown"
            }
        };
        Outcome {
            complete,
            saved: readback(
                publication.file.is_some(),
                publication.status == "unchanged",
            )
            .into(),
            runtime: readback(
                runtime == "written" || (runtime == "outcome_unknown" && independently_verified),
                runtime == "unchanged",
            )
            .into(),
            directory: readback(
                publication.directory.is_some(),
                !before.configuration.directory_missing(),
            )
            .into(),
            expected: independently_verified.then_some(expected),
        }
    }
}
