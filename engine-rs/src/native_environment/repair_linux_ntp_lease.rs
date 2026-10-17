//! Fixed helper arguments and a live original-caller lease; never app storage.
pub(super) const FLAG: &str = "--environment-ntp-helper";

pub(super) struct Arguments {
    pub server: String,
    pub pid: u32,
    pub birth: u64,
    pub uid: u32,
    pub deadline: u64,
}
pub(super) fn arguments(args: &[String]) -> Option<Arguments> {
    if args.len() != 7
        || args[0] != FLAG
        || args[6] != "--confirm"
        || !super::time_probe::valid_host(&args[1])
    {
        return None;
    }
    let pid = args[2].parse::<u32>().ok().filter(|pid| *pid > 1)?;
    let birth = args[3].parse::<u64>().ok().filter(|birth| *birth > 0)?;
    Some(Arguments {
        server: args[1].trim_end_matches('.').to_ascii_lowercase(),
        pid,
        birth,
        uid: args[4].parse().ok()?,
        deadline: args[5].parse().ok()?,
    })
}

#[cfg(target_os = "linux")]
mod native {
    use super::Arguments;
    use std::{
        io::Read,
        os::fd::{AsRawFd, FromRawFd, OwnedFd},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };

    fn text(pid: u32, name: &str) -> Option<String> {
        let mut bytes = Vec::new();
        std::fs::File::open(format!("/proc/{pid}/{name}"))
            .ok()?
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        (bytes.len() <= 16 * 1024)
            .then(|| String::from_utf8(bytes).ok())
            .flatten()
    }

    fn caller(args: &Arguments) -> Option<()> {
        if super::super::repair_linux_time::start_time(&text(args.pid, "stat")?)? != args.birth {
            return None;
        }
        let status = text(args.pid, "status")?;
        let ids = status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))?
            .split_ascii_whitespace()
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        (ids.len() == 4 && ids.iter().all(|uid| *uid == args.uid)).then_some(())
    }

    // trusted.* access uses capable(CAP_SYS_ADMIN) in the initial user namespace.
    // /proc/1 can itself belong to a container namespace. Prove the required
    // kernel gate on our anonymous memfd, never by changing a host file.
    fn inspection_privilege() -> Option<OwnedFd> {
        if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
            return None;
        }
        let fd =
            unsafe { libc::memfd_create(c"agent-manager-inspection".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let name = c"trusted.agent-manager-inspection";
        let value = [1u8];
        if unsafe {
            libc::fsetxattr(
                fd.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                1,
                libc::XATTR_CREATE,
            )
        } != 0
        {
            return None;
        }
        inspection_current(&fd).then_some(fd)
    }

    fn inspection_current(fd: &OwnedFd) -> bool {
        let name = c"trusted.agent-manager-inspection";
        let mut actual = [0u8];
        if unsafe { libc::fgetxattr(fd.as_raw_fd(), name.as_ptr(), actual.as_mut_ptr().cast(), 1) }
            != 1
            || actual != [1]
        {
            return false;
        }
        let mut names = [0u8; 1024];
        let length =
            unsafe { libc::flistxattr(fd.as_raw_fd(), names.as_mut_ptr().cast(), names.len()) };
        if length <= 0 || length as usize > names.len() {
            return false;
        }
        let names = &names[..length as usize];
        names.last() == Some(&0)
            && names
                .split(|byte| *byte == 0)
                .any(|entry| entry == name.to_bytes())
    }

    pub(in super::super) struct Lease {
        args: Arguments,
        pidfd: OwnedFd,
        pipe: Arc<AtomicBool>,
        inspection: OwnedFd,
    }
    impl Lease {
        pub(in super::super) fn open(args: Arguments, pipe: Arc<AtomicBool>) -> Option<Self> {
            let inspection = inspection_privilege()?;
            if unsafe { libc::getppid() } as u32 != args.pid {
                return None;
            }
            match std::env::var("PKEXEC_UID") {
                Ok(uid) if uid.parse::<u32>().ok()? == args.uid => {}
                Err(std::env::VarError::NotPresent) if args.uid == 0 => {}
                _ => return None,
            }
            caller(&args)?;
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, args.pid, 0) };
            if fd < 0 {
                return None;
            }
            let lease = Self {
                args,
                pidfd: unsafe { OwnedFd::from_raw_fd(fd as i32) },
                pipe,
                inspection,
            };
            lease.fresh().then_some(lease)
        }

        pub(in super::super) fn fresh(&self) -> bool {
            let mut fd = libc::pollfd {
                fd: self.pidfd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            self.pipe.load(Ordering::Acquire)
                && inspection_current(&self.inspection)
                && unsafe { libc::poll(&mut fd, 1, 0) } == 0
                && super::super::clock_helper::boot_ms().is_some_and(|now| {
                    super::super::clock_helper::valid_window(now, self.args.deadline)
                })
                && caller(&self.args).is_some()
        }

        pub(in super::super) fn server(&self) -> &str {
            &self.args.server
        }
    }
}
#[cfg(target_os = "linux")]
pub(super) use native::Lease;

#[cfg(test)]
mod tests {
    use super::*;
    fn valid() -> Vec<String> {
        [
            FLAG,
            "Selected.Example.",
            "100",
            "200",
            "1000",
            "300",
            "--confirm",
        ]
        .map(str::to_string)
        .to_vec()
    }
    #[test]
    fn fixed_arguments_allow_only_a_server_and_original_caller_tuple() {
        let args = arguments(&valid()).unwrap();
        assert_eq!(args.server, "selected.example");
        assert_eq!(
            (args.pid, args.birth, args.uid, args.deadline),
            (100, 200, 1000, 300)
        );
        for (index, value) in [
            (0, "--other"),
            (1, "https://time.example"),
            (1, "time.example:123"),
            (1, "$(unexpected)"),
            (2, "1"),
            (2, "-1"),
            (3, "0"),
            (4, "root"),
            (5, "-1"),
            (6, "--other"),
        ] {
            let mut args = valid();
            args[index] = value.into();
            assert!(arguments(&args).is_none());
        }
        let mut extra = valid();
        extra.push("/arbitrary/path".into());
        assert!(arguments(&extra).is_none());
        for len in 0..7 {
            assert!(arguments(&valid()[..len]).is_none());
        }
    }
}
