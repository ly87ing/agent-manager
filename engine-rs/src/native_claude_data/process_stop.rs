//! Native process-incarnation bindings. Never signal an unbound numeric PID.
use anyhow::{bail, Result};
use std::time::{Duration, Instant};

pub(super) struct Target {
    pub pid: u32,
    pub identity: Vec<u8>,
    native: platform::Process,
}

#[cfg(windows)]
pub(super) fn current_user(pid: u32) -> Result<bool> {
    platform::Process::current_user(pid)
}

pub(super) fn capture() -> Result<Vec<Target>> {
    let pids = super::processes::active()?;
    if pids.len() > 1024 {
        bail!("Too many Claude processes to stop safely.");
    }
    let targets: Vec<_> = pids
        .iter()
        .map(|pid| {
            let native = platform::Process::open(*pid)?;
            Ok(Target {
                pid: *pid,
                identity: native.identity()?,
                native,
            })
        })
        .collect::<Result<_>>()?;
    // Classification must still describe the bound incarnations, not an earlier PID occupant.
    if super::processes::active()? != pids {
        bail!("Claude processes changed during inspection; request a new preview.");
    }
    for target in &targets {
        if target.native.identity()? != target.identity {
            bail!("A Claude process changed during inspection; request a new preview.");
        }
    }
    Ok(targets)
}

pub(super) fn stop(targets: &[Target], stopped: &mut u64, forced: &mut u64) -> Result<()> {
    // Unit-test cleanup may inspect real processes, but must never terminate them.
    #[cfg(test)]
    if !targets.is_empty() {
        bail!("Unit tests cannot stop current-user Claude processes.");
    }
    let mut forced_targets = Vec::new();
    let result = (|| {
        for target in targets {
            target.native.signal(false)?;
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while targets
            .iter()
            .any(|target| !target.native.exited().unwrap_or(false))
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        for (index, target) in targets.iter().enumerate() {
            if !target.native.exited()? && target.native.signal(true)? {
                forced_targets.push(index);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            *stopped = 0;
            for target in targets {
                if target.native.exited()? {
                    *stopped += 1;
                }
            }
            if *stopped == targets.len() as u64 {
                break;
            }
            if Instant::now() >= deadline {
                bail!("Claude processes did not exit; no data was deleted. Request a new preview.");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !super::processes::active()?.is_empty() {
            bail!("A new Claude process appeared; no data was deleted. Request a new preview.");
        }
        Ok(())
    })();
    // Preserve independently observed progress even when a later stop fails.
    *stopped = targets
        .iter()
        .filter(|target| target.native.exited().unwrap_or(false))
        .count() as u64;
    *forced = forced_targets
        .iter()
        .filter(|index| targets[**index].native.exited().unwrap_or(false))
        .count() as u64;
    result
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    unsafe extern "C" {
        fn task_name_for_pid(task: u32, pid: i32, port: *mut u32) -> i32;
        fn mach_port_deallocate(task: u32, port: u32) -> i32;
        fn proc_signal_with_audittoken(token: *const [u32; 8], signal: i32) -> i32;
    }
    pub struct Process {
        pid: u32,
        token: [u32; 8],
    }
    fn token(pid: u32) -> Result<[u32; 8]> {
        let mut port = 0;
        let mut value = [0u32; 8];
        let mut count = 8;
        #[allow(deprecated)]
        let task = unsafe { libc::mach_task_self() };
        if unsafe { task_name_for_pid(task, pid as i32, &mut port) } != 0 {
            bail!("A Claude process identity could not be bound; cleanup is unavailable.");
        }
        let result = unsafe { libc::task_info(port, 15, value.as_mut_ptr().cast(), &mut count) };
        unsafe {
            mach_port_deallocate(task, port);
        }
        if result != 0
            || count != 8
            || value[5] != pid
            || value[1] != unsafe { libc::geteuid() }
            || value[3] != unsafe { libc::getuid() }
        {
            bail!("A Claude process owner or identity could not be verified.");
        }
        Ok(value)
    }
    impl Process {
        pub fn open(pid: u32) -> Result<Self> {
            Ok(Self {
                pid,
                token: token(pid)?,
            })
        }
        pub fn identity(&self) -> Result<Vec<u8>> {
            Ok(token(self.pid)?
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect())
        }
        pub fn exited(&self) -> Result<bool> {
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of_val(&info) as i32;
            let read = unsafe {
                libc::proc_pidinfo(
                    self.pid as i32,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    (&mut info as *mut libc::proc_bsdinfo).cast(),
                    size,
                )
            };
            if read == size {
                // task_name_for_pid can disappear before BSD reports a zombie.
                // Unavailable task metadata is still treated as running until a later poll.
                return Ok(info.pbi_status == 5
                    || token(self.pid).is_ok_and(|current| current != self.token));
            }
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                return Ok(true);
            }
            bail!("A Claude process exit could not be verified.")
        }
        pub fn signal(&self, force: bool) -> Result<bool> {
            if self.exited()? {
                return Ok(false);
            }
            // The kernel checks pidversion, including when the PID has been reused.
            let code = unsafe {
                proc_signal_with_audittoken(
                    &self.token,
                    if force { libc::SIGKILL } else { libc::SIGTERM },
                )
            };
            if code != 0 && code != libc::ESRCH && !self.exited()? {
                bail!("A Claude process could not be stopped; cleanup stopped.");
            }
            Ok(code == 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_process_helper() {
        let Some(marker) = std::env::var_os("AM_CLAUDE_STOP_HELPER") else {
            return;
        };
        std::fs::write(marker, b"ready").unwrap();
        std::thread::sleep(Duration::from_secs(20));
    }

    #[test]
    fn native_stop_binds_only_the_owned_child_and_verifies_exit() {
        struct Owned(std::process::Child);
        impl Drop for Owned {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let work = std::env::temp_dir().join(format!("am-claude-stop-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&work).unwrap();
        let start = |name: &str| {
            let marker = work.join(name);
            let child = Owned(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "native_claude_data::process_stop::tests::owned_process_helper",
                    ])
                    .env("AM_CLAUDE_STOP_HELPER", &marker)
                    .env("HOME", &work)
                    .env("USERPROFILE", &work)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            while !marker.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(marker.exists(), "Owned fixture did not become ready");
            child
        };
        let mut selected = start("selected");
        let mut unrelated = start("unrelated");
        let process = platform::Process::open(selected.0.id()).unwrap();
        let identity = process.identity().unwrap();
        assert!(!identity.is_empty());
        assert_eq!(identity, process.identity().unwrap());
        process.signal(false).unwrap();
        process.signal(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !process.exited().unwrap() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(process.exited().unwrap());
        assert!(
            !process.signal(true).unwrap(),
            "An exited process received no forced signal"
        );
        assert!(selected.0.wait().is_ok());
        assert!(unrelated.0.try_wait().unwrap().is_none());
        drop(process);
        drop(selected);
        drop(unrelated);
        std::fs::remove_dir_all(work).unwrap();
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::MetadataExt,
    };
    pub struct Process {
        pid: u32,
        fd: OwnedFd,
    }
    impl Process {
        pub fn open(pid: u32) -> Result<Self> {
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
            if fd < 0 {
                bail!("A Claude process could not be bound with pidfd; cleanup is unavailable.");
            }
            let value = Self {
                pid,
                fd: unsafe { OwnedFd::from_raw_fd(fd as i32) },
            };
            value.identity()?;
            Ok(value)
        }
        pub fn identity(&self) -> Result<Vec<u8>> {
            let path = format!("/proc/{}", self.pid);
            if std::fs::metadata(&path)?.uid() != unsafe { libc::geteuid() } || self.exited()? {
                bail!("A Claude process owner or identity changed.");
            }
            let stat = std::fs::read_to_string(format!("{path}/stat"))?;
            let start = stat
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().nth(19))
                .ok_or_else(|| {
                    anyhow::anyhow!("A Claude process start identity is unavailable.")
                })?;
            Ok(start.as_bytes().to_vec())
        }
        pub fn exited(&self) -> Result<bool> {
            let mut poll = libc::pollfd {
                fd: self.fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            if unsafe { libc::poll(&mut poll, 1, 0) } < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(poll.revents & libc::POLLIN != 0)
        }
        pub fn signal(&self, force: bool) -> Result<bool> {
            if self.exited()? {
                return Ok(false);
            }
            let result = unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    self.fd.as_raw_fd(),
                    if force { libc::SIGKILL } else { libc::SIGTERM },
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            };
            if result < 0 && !self.exited()? {
                bail!("A Claude process could not be stopped; cleanup stopped.");
            }
            Ok(result == 0)
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::{Foundation::*, Security::*, System::Threading::*};
    pub struct Process {
        handle: HANDLE,
    }
    impl Drop for Process {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
    fn user(process: HANDLE) -> Result<Vec<u8>> {
        unsafe {
            let mut token = std::ptr::null_mut();
            if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
                bail!("A Claude process owner could not be read.");
            }
            let token = Process { handle: token };
            let mut length = 0;
            GetTokenInformation(
                token.handle,
                TokenUser,
                std::ptr::null_mut(),
                0,
                &mut length,
            );
            if length == 0 || length > 65536 {
                bail!("A Claude process owner is unavailable.");
            }
            let mut bytes = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
            if GetTokenInformation(
                token.handle,
                TokenUser,
                bytes.as_mut_ptr().cast(),
                length,
                &mut length,
            ) == 0
            {
                bail!("A Claude process owner could not be read.");
            }
            let sid = (*(bytes.as_ptr().cast::<TOKEN_USER>())).User.Sid;
            if IsValidSid(sid) == 0 {
                bail!("A Claude process owner is invalid.");
            }
            Ok(std::slice::from_raw_parts(sid.cast(), GetLengthSid(sid) as usize).to_vec())
        }
    }
    impl Process {
        fn query(pid: u32) -> Result<Self> {
            let handle = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    pid,
                )
            };
            if handle.is_null() {
                bail!("A possible Claude process owner could not be verified; cleanup is unavailable.");
            }
            Ok(Self { handle })
        }
        pub fn current_user(pid: u32) -> Result<bool> {
            let process = Self::query(pid)?;
            Ok(user(process.handle)? == user(unsafe { GetCurrentProcess() })?)
        }
        pub fn open(pid: u32) -> Result<Self> {
            // QUERY only in previews. Termination permission is requested after confirmation.
            let value = Self::query(pid)?;
            if user(value.handle)? != user(unsafe { GetCurrentProcess() })? {
                bail!("A Claude process belongs to another user.");
            }
            Ok(value)
        }
        pub fn identity(&self) -> Result<Vec<u8>> {
            let mut birth: FILETIME = unsafe { std::mem::zeroed() };
            let mut exit = birth;
            let mut kernel = birth;
            let mut time = birth;
            if unsafe {
                GetProcessTimes(self.handle, &mut birth, &mut exit, &mut kernel, &mut time)
            } == 0
                || self.exited()?
            {
                bail!("A Claude process start identity is unavailable.");
            }
            Ok([
                birth.dwLowDateTime.to_le_bytes(),
                birth.dwHighDateTime.to_le_bytes(),
            ]
            .concat())
        }
        pub fn exited(&self) -> Result<bool> {
            match unsafe { WaitForSingleObject(self.handle, 0) } {
                WAIT_OBJECT_0 => Ok(true),
                WAIT_TIMEOUT => Ok(false),
                _ => bail!("A Claude process exit could not be verified."),
            }
        }
        pub fn signal(&self, force: bool) -> Result<bool> {
            if !force || self.exited()? {
                return Ok(false);
            }
            let pid = unsafe { GetProcessId(self.handle) };
            let handle = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                    0,
                    pid,
                )
            };
            if handle.is_null() {
                bail!("A Claude process could not be stopped.");
            }
            let writable = Self { handle };
            if writable.identity()? != self.identity()?
                || user(handle)? != user(unsafe { GetCurrentProcess() })?
            {
                bail!("A Claude process identity changed; cleanup stopped.");
            }
            let signalled = unsafe { TerminateProcess(handle, 1) } != 0;
            if !signalled && !self.exited()? {
                bail!("A Claude process could not be stopped.");
            }
            Ok(signalled)
        }
    }
}
