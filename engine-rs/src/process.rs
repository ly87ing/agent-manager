//! External commands: desktop window flags, timeout process groups, and manual system proxies.
//! Explicit environment settings take precedence; discovery never writes system settings.

#[cfg(any(target_os = "linux", test))]
#[path = "process_proxy_linux.rs"]
mod linux_proxy;
#[cfg(target_os = "linux")]
#[path = "process_settings_linux.rs"]
mod settings;
#[cfg(target_os = "linux")]
pub(crate) use settings::deny_settings_file_writes;

use std::ffi::OsStr;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub fn command(program: impl AsRef<OsStr>) -> Command {
    command_with_env(program, std::iter::empty())
}

/// User-launched applications inherit the environment without manager proxy fallback.
/// Spawn directly: these applications must outlive temporary command-job cleanup.
pub fn launch_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    hide_window(&mut command);
    command
}

pub fn command_with_env<'a>(
    program: impl AsRef<OsStr>,
    env: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Command {
    let env: Vec<_> = env.into_iter().collect();
    let mut command = Command::new(program);
    hide_window(&mut command);
    for (key, value) in proxy_env_from(
        |name| {
            env.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| std::ffi::OsString::from(value))
                .or_else(|| std::env::var_os(name))
        },
        cached_system_proxy,
    ) {
        command.env(key, value);
    }
    command.envs(env);
    command
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(window_flags());
}

#[cfg(windows)]
fn window_flags() -> u32 {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if unsafe { windows_sys::Win32::System::Console::GetConsoleWindow() }.is_null() {
        CREATE_NO_WINDOW
    } else {
        0
    }
}

#[cfg(not(windows))]
fn hide_window(_: &mut Command) {}

pub struct Group {
    #[cfg(unix)]
    id: u32,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

static EXITING: std::sync::RwLock<bool> = std::sync::RwLock::new(false);

#[cfg(unix)]
static ACTIVE_GROUPS: Mutex<std::collections::BTreeSet<u32>> =
    Mutex::new(std::collections::BTreeSet::new());

/// Stop temporary commands before the desktop process exits.
pub fn shutdown_groups() {
    let mut exiting = EXITING.write().unwrap_or_else(|e| e.into_inner());
    *exiting = true;
    #[cfg(unix)]
    {
        let mut groups = ACTIVE_GROUPS.lock().unwrap_or_else(|e| e.into_inner());
        for id in std::mem::take(&mut *groups) {
            kill_unix_group(id);
        }
    }
    // Windows closes the process's kill-on-close Job Object handles on exit.
}

pub fn spawn_group(command: &mut Command) -> std::io::Result<(Child, Group)> {
    // Parallel spawns share the gate; exit waits for group/job setup and
    // prevents a child starting after the sweep in Tauri's event loop.
    let exiting = EXITING.read().unwrap_or_else(|e| e.into_inner());
    if *exiting {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "Application is shutting down",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        let child = command.spawn()?;
        let id = child.id();
        ACTIVE_GROUPS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id);
        Ok((child, Group { id }))
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;
        let group = windows_job()?;
        command.creation_flags(window_flags() | CREATE_SUSPENDED);
        let result = command.spawn();
        command.creation_flags(window_flags());
        let mut child = result?;
        if let Err(error) = assign_and_resume(&child, group.job) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok((child, group))
    }
}

/// Capture piped output under one deadline, including EOF after the parent exits.
/// Keep draining after the retention limit so a verbose child never blocks on a full pipe.
pub fn capture_group(
    command: &mut Command,
    timeout: Duration,
    limit: usize,
) -> std::io::Result<std::process::Output> {
    let start = Instant::now();
    let (mut child, group) = spawn_group(command)?;
    let stdout = capture_pipe(child.stdout.take(), limit);
    let stderr = capture_pipe(child.stderr.take(), limit);
    let mut status = None;
    let result = loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(error) => break Err(error),
            }
        }
        if let Some(status) = status {
            if stdout.is_finished() && stderr.is_finished() {
                break Ok(status);
            }
        }
        if start.elapsed() >= timeout {
            break Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Command or its output pipes timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(10).min(timeout.saturating_sub(start.elapsed())));
    };
    if result.is_err() {
        // The parent may already have exited; kill the group/job anyway to close inherited pipes.
        group.kill(&mut child);
        let cleanup = Instant::now();
        loop {
            let reaped = child.try_wait().map_or(true, |status| status.is_some());
            if (reaped && stdout.is_finished() && stderr.is_finished())
                || cleanup.elapsed() >= Duration::from_millis(250)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // Never join unfinished readers: even a descendant that escaped the group must not
        // make a timeout unbounded. Dropping their handles detaches them until their pipes close.
    }
    let status = result?;
    Ok(std::process::Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn capture_pipe<R: std::io::Read + Send + 'static>(
    pipe: Option<R>,
    limit: usize,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut output = Vec::new();
        if let Some(mut pipe) = pipe {
            let mut buffer = [0; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(len) => {
                        let retained = len.min(limit.saturating_sub(output.len()));
                        output.extend_from_slice(&buffer[..retained]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        }
        output
    })
}

#[cfg(windows)]
fn windows_job() -> std::io::Result<Group> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) != 0;
        if !configured {
            let error = std::io::Error::last_os_error();
            CloseHandle(job);
            return Err(error);
        }
        Ok(Group { job })
    }
}

/// The child starts suspended: it cannot spawn descendants before joining the job.
#[cfg(windows)]
fn assign_and_resume(
    child: &Child,
    job: windows_sys::Win32::Foundation::HANDLE,
) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    unsafe {
        if AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut found = Thread32First(snapshot, &mut entry) != 0;
        let mut result = Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Suspended child thread not found",
        ));
        while found {
            if entry.th32OwnerProcessID == child.id() {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                result = if thread.is_null() {
                    Err(std::io::Error::last_os_error())
                } else {
                    let resumed = ResumeThread(thread);
                    let error = std::io::Error::last_os_error();
                    CloseHandle(thread);
                    if resumed == u32::MAX {
                        Err(error)
                    } else {
                        Ok(())
                    }
                };
                break;
            }
            found = Thread32Next(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        result
    }
}

impl Group {
    pub fn kill(&self, child: &mut Child) {
        #[cfg(unix)]
        {
            let mut groups = ACTIVE_GROUPS.lock().unwrap_or_else(|e| e.into_inner());
            if groups.remove(&self.id) && !kill_unix_group(self.id) {
                let _ = child.kill();
            }
        }
        #[cfg(windows)]
        {
            unsafe {
                if windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1) == 0 {
                    let _ = child.kill();
                }
            }
        }
    }
}

#[cfg(unix)]
fn kill_unix_group(id: u32) -> bool {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const SIGKILL: i32 = 9;
    unsafe { kill(-(id as i32), SIGKILL) == 0 }
}

impl Drop for Group {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let mut groups = ACTIVE_GROUPS.lock().unwrap_or_else(|e| e.into_inner());
            if groups.remove(&self.id) {
                kill_unix_group(self.id);
            }
        }
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SystemProxy {
    pub http: Option<String>,
    pub https: Option<String>,
    pub socks: Option<String>,
    pub bypass: Option<String>,
}

const PROXY_VARS: &[&str] = &[
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
];

fn proxy_env_from(
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
    system: impl FnOnce() -> SystemProxy,
) -> Vec<(&'static str, String)> {
    // An explicitly empty variable also opts out of automatic proxy discovery.
    if PROXY_VARS.iter().any(|name| var(name).is_some()) {
        return Vec::new();
    }
    let mut env = env_for(&system());
    if var("NO_PROXY").is_some() || var("no_proxy").is_some() {
        env.retain(|(key, _)| !matches!(*key, "NO_PROXY" | "no_proxy"));
    }
    env
}

fn env_for(proxy: &SystemProxy) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut both = |upper: &'static str, lower: &'static str, value: &Option<String>| {
        if let Some(value) = value {
            out.push((upper, value.clone()));
            out.push((lower, value.clone()));
        }
    };
    both("HTTP_PROXY", "http_proxy", &proxy.http);
    both("HTTPS_PROXY", "https_proxy", &proxy.https);
    both("ALL_PROXY", "all_proxy", &proxy.socks);
    if proxy.http.is_some() || proxy.https.is_some() || proxy.socks.is_some() {
        both("NO_PROXY", "no_proxy", &proxy.bypass);
    }
    out
}

fn cached_system_proxy() -> SystemProxy {
    static CACHE: Mutex<Option<(Instant, SystemProxy)>> = Mutex::new(None);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, value)) = guard.as_ref() {
        if at.elapsed() < Duration::from_secs(60) {
            return value.clone();
        }
    }
    let value = read_system_proxy();
    *guard = Some((Instant::now(), value.clone()));
    value
}

#[cfg(target_os = "macos")]
fn read_system_proxy() -> SystemProxy {
    let Ok(output) = Command::new("/usr/sbin/scutil").arg("--proxy").output() else {
        return SystemProxy::default();
    };
    if !output.status.success() {
        return SystemProxy::default();
    }
    parse_scutil(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(windows)]
fn read_system_proxy() -> SystemProxy {
    use winreg::enums::HKEY_CURRENT_USER;
    let Ok(key) = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
    else {
        return SystemProxy::default();
    };
    let enabled = key.get_value::<u32, _>("ProxyEnable").unwrap_or(0) == 1;
    let server = key
        .get_value::<String, _>("ProxyServer")
        .unwrap_or_default();
    let bypass = key
        .get_value::<String, _>("ProxyOverride")
        .unwrap_or_default();
    parse_windows(enabled, &server, &bypass)
}

#[cfg(target_os = "linux")]
fn read_system_proxy() -> SystemProxy {
    linux_proxy::read()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn read_system_proxy() -> SystemProxy {
    SystemProxy::default()
}

#[cfg(any(target_os = "macos", test))]
fn parse_scutil(text: &str) -> SystemProxy {
    let mut values = std::collections::HashMap::new();
    let mut exceptions = Vec::new();
    let mut in_exceptions = false;
    for line in text.lines() {
        let line = line.trim();
        if in_exceptions {
            if line.starts_with('}') {
                in_exceptions = false;
            } else if let Some((_, host)) = line.split_once(" : ") {
                exceptions.push(host.trim().to_string());
            }
            continue;
        }
        if line.starts_with("ExceptionsList") && line.ends_with('{') {
            in_exceptions = true;
            continue;
        }
        if let Some((key, value)) = line.split_once(" : ") {
            values.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    // NO_PROXY cannot represent bypassing every single-label hostname or PAC.
    if [
        "ExcludeSimpleHostnames",
        "ProxyAutoConfigEnable",
        "ProxyAutoDiscoveryEnable",
    ]
    .iter()
    .any(|key| values.get(*key).map(String::as_str) == Some("1"))
    {
        return SystemProxy::default();
    }
    let Some(bypass) = bypass_hosts(exceptions.iter().map(String::as_str)) else {
        return SystemProxy::default();
    };
    let proxy = |kind: &str, scheme: &str| {
        (values.get(&format!("{kind}Enable")).map(String::as_str) == Some("1"))
            .then(|| {
                let host = values.get(&format!("{kind}Proxy"))?;
                let port = values.get(&format!("{kind}Port"))?;
                (!host.is_empty()).then(|| format!("{scheme}://{host}:{port}"))
            })
            .flatten()
    };
    SystemProxy {
        http: proxy("HTTP", "http"),
        https: proxy("HTTPS", "http"),
        socks: proxy("SOCKS", "socks5"),
        bypass,
    }
}

#[cfg(any(windows, test))]
fn parse_windows(enabled: bool, server: &str, bypass: &str) -> SystemProxy {
    if !enabled || server.trim().is_empty() {
        return SystemProxy::default();
    }
    let Some(bypass) = bypass_hosts(bypass.split(';')) else {
        return SystemProxy::default();
    };
    let with_scheme = |scheme: &str, address: &str| {
        let address = address.trim();
        if address.contains("://") {
            address.to_string()
        } else {
            format!("{scheme}://{address}")
        }
    };
    let mut proxy = SystemProxy::default();
    if server.contains('=') {
        for part in server.split(';') {
            let Some((kind, address)) = part.split_once('=') else {
                continue;
            };
            if address.trim().is_empty() {
                continue;
            }
            match kind.trim().to_ascii_lowercase().as_str() {
                "http" => proxy.http = Some(with_scheme("http", address)),
                "https" => proxy.https = Some(with_scheme("http", address)),
                "socks" => proxy.socks = Some(with_scheme("socks5", address)),
                _ => {}
            }
        }
    } else {
        proxy.http = Some(with_scheme("http", server));
        proxy.https = proxy.http.clone();
    }
    proxy.bypass = bypass;
    proxy
}

/// Unsupported bypass patterns must not silently send local requests to a proxy.
fn bypass_hosts<'a>(hosts: impl Iterator<Item = &'a str>) -> Option<Option<String>> {
    let mut out = Vec::new();
    for host in hosts.map(str::trim).filter(|host| !host.is_empty()) {
        let host = host
            .strip_prefix("*.")
            .map(|suffix| format!(".{suffix}"))
            .unwrap_or_else(|| host.to_string());
        if host.contains(['*', '?', '<', '>']) && host != "*" {
            return None;
        }
        out.push(host);
    }
    Some((!out.is_empty()).then(|| out.join(",")))
}

#[cfg(test)]
pub(crate) mod capture_test_support {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    pub fn root(label: &str) -> PathBuf {
        // Compile and complete cold startup before short capture deadlines.
        executable();
        fresh_root(label)
    }

    fn fresh_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "am-capture-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        let mut builder = builder;
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&root).unwrap();
        root
    }

    fn executable() -> &'static Path {
        static EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();
        EXECUTABLE.get_or_init(|| {
            let root = fresh_root("fixture");
            let executable = root.join("capture-fixture.exe");
            let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
            let mut compile = Command::new(compiler);
            compile
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/common/capture_fixture.rs"))
                .args([
                    "--crate-name",
                    "capture_fixture",
                    "--edition=2021",
                    "-C",
                    "debuginfo=0",
                    "-o",
                ])
                .arg(&executable);
            let mut warmup = Command::new(&executable);
            warmup
                .env("AM_CAPTURE_MODE", "output")
                .env("AM_CAPTURE_ROOT", &root)
                .stderr(Stdio::null());
            for (mut command, stage, exit_code) in
                [(compile, "compilation", 0), (warmup, "startup", 7)]
            {
                let mut child = command
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap_or_else(|error| panic!("capture fixture {stage}: {error}"));
                let start = Instant::now();
                loop {
                    if let Some(status) = child.try_wait().unwrap() {
                        assert_eq!(
                            status.code(),
                            Some(exit_code),
                            "capture fixture {stage} failed: {status}"
                        );
                        break;
                    }
                    if start.elapsed() >= Duration::from_secs(60) {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!("capture fixture {stage} timed out");
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            executable
        })
    }

    pub fn command(root: &Path, mode: &str) -> Command {
        let mut command = Command::new(executable());
        command
            .env("AM_CAPTURE_MODE", mode)
            .env("AM_CAPTURE_ROOT", root);
        command
    }

    pub fn assert_descendant_stopped(root: &Path) {
        assert!(root.join("ready").exists(), "Descendant never started");
        assert!(root.join("parent-exited").exists(), "Parent did not finish");
        // The descendant's five-second timer starts after its own startup,
        // not when the parent capture began. Observe the full window after ready.
        std::thread::sleep(Duration::from_secs(6));
        assert!(
            !root.join("late").exists(),
            "Descendant survived the timeout"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_test_dir(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "am-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        let mut builder = builder;
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&root).unwrap();
        root
    }

    const SCUTIL_ON: &str = "<dictionary> {
  ExceptionsList : <array> {
    0 : 127.0.0.1
    1 : localhost
    2 : *.local
  }
  ExcludeSimpleHostnames : 0
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7890
  HTTPSProxy : 127.0.0.1
  SOCKSEnable : 1
  SOCKSPort : 7891
  SOCKSProxy : 127.0.0.1
}";

    const SCUTIL_OFF: &str = "<dictionary> {
  FTPPassive : 1
  HTTPEnable : 0
  HTTPSEnable : 0
  ProxyAutoConfigEnable : 0
  SOCKSEnable : 0
}";

    #[test]
    fn macos_manual_proxy_becomes_proxy_variables() {
        let proxy = parse_scutil(SCUTIL_ON);
        assert_eq!(
            proxy,
            SystemProxy {
                http: Some("http://127.0.0.1:7890".into()),
                https: Some("http://127.0.0.1:7890".into()),
                socks: Some("socks5://127.0.0.1:7891".into()),
                bypass: Some("127.0.0.1,localhost,.local".into()),
            }
        );
        let env = env_for(&proxy);
        assert!(env.contains(&("https_proxy", "http://127.0.0.1:7890".to_string())));
        assert!(env.contains(&("ALL_PROXY", "socks5://127.0.0.1:7891".to_string())));
        assert!(env.contains(&("NO_PROXY", "127.0.0.1,localhost,.local".to_string())));
        assert_eq!(parse_scutil(SCUTIL_OFF), SystemProxy::default());
        assert!(env_for(&SystemProxy::default()).is_empty());
    }

    #[test]
    fn windows_manual_proxy_in_both_notations() {
        let shared = parse_windows(true, "127.0.0.1:7890", "localhost;127.0.0.1;*.corp.example");
        assert_eq!(shared.http.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(shared.https.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(
            shared.bypass.as_deref(),
            Some("localhost,127.0.0.1,.corp.example")
        );
        let split = parse_windows(
            true,
            "http=127.0.0.1:8080;https=127.0.0.1:8443;socks=127.0.0.1:1080",
            "",
        );
        assert_eq!(split.http.as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(split.https.as_deref(), Some("http://127.0.0.1:8443"));
        assert_eq!(split.socks.as_deref(), Some("socks5://127.0.0.1:1080"));
        assert_eq!(split.bypass, None);
        assert_eq!(
            parse_windows(false, "127.0.0.1:7890", ""),
            SystemProxy::default()
        );
    }

    #[test]
    fn explicit_proxy_environment_and_bypass_take_precedence() {
        for key in PROXY_VARS {
            let env = proxy_env_from(
                |name| (name == *key).then(|| "".into()),
                || panic!("Must not read system proxy"),
            );
            assert!(env.is_empty());
        }
        for key in ["NO_PROXY", "no_proxy"] {
            let env = proxy_env_from(
                |name| (name == key).then(|| "user.example".into()),
                || parse_windows(true, "127.0.0.1:7890", "system.example"),
            );
            assert!(env.iter().any(|(key, _)| *key == "http_proxy"));
            assert!(!env
                .iter()
                .any(|(key, _)| key.eq_ignore_ascii_case("no_proxy")));
        }
        let proxy = parse_scutil(&SCUTIL_ON.replace("HTTPSEnable : 1", "HTTPSEnable : 0"));
        assert!(!env_for(&proxy)
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("https_proxy")));
    }

    #[test]
    fn unrepresentable_bypass_and_pac_do_not_change_network_routing() {
        assert_eq!(
            parse_windows(true, "127.0.0.1:7890", "<local>"),
            SystemProxy::default()
        );
        assert_eq!(
            parse_windows(true, "127.0.0.1:7890", "host?.example"),
            SystemProxy::default()
        );
        assert_eq!(
            parse_scutil(
                &SCUTIL_ON.replace("ExcludeSimpleHostnames : 0", "ExcludeSimpleHostnames : 1")
            ),
            SystemProxy::default()
        );
        assert_eq!(
            parse_scutil(
                &SCUTIL_ON.replace("ExcludeSimpleHostnames : 0", "ProxyAutoConfigEnable : 1")
            ),
            SystemProxy::default()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_starts_and_terminates_descendants() {
        use std::process::Stdio;
        let root = fresh_test_dir("job");
        let mut cmd = command(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "process::tests::windows_job_helper",
            "--nocapture",
        ])
        .env("AM_JOB_HELPER", "parent")
        .env("AM_JOB_DIR", &root)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        let (mut child, group) = spawn_group(&mut cmd).unwrap();
        let start = Instant::now();
        while !root.join("ready").exists() {
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "Grandchild did not start"
            );
            assert!(
                child.try_wait().unwrap().is_none(),
                "Parent exited before grandchild started"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        group.kill(&mut child);
        assert!(!child.wait().unwrap().success());
        std::thread::sleep(Duration::from_secs(3));
        assert!(
            !root.join("late").exists(),
            "Grandchild survived termination"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_helper() {
        match std::env::var("AM_JOB_HELPER").as_deref() {
            Ok("exit") => {
                let root = std::path::PathBuf::from(std::env::var_os("AM_JOB_DIR").unwrap());
                let mut command = Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "process::tests::windows_job_helper",
                        "--nocapture",
                    ])
                    .env("AM_JOB_HELPER", "grandchild");
                let (_child, _group) = spawn_group(&mut command).unwrap();
                let start = Instant::now();
                while !root.join("ready").exists() {
                    assert!(start.elapsed() < Duration::from_secs(15));
                    std::thread::sleep(Duration::from_millis(10));
                }
                shutdown_groups();
                assert_eq!(
                    spawn_group(&mut command).err().unwrap().kind(),
                    std::io::ErrorKind::Interrupted
                );
                std::process::exit(0);
            }
            Ok("parent") => {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "process::tests::windows_job_helper",
                        "--nocapture",
                    ])
                    .env("AM_JOB_HELPER", "grandchild")
                    .status()
                    .unwrap();
                assert!(status.success());
            }
            Ok("grandchild") => {
                let root = std::path::PathBuf::from(std::env::var_os("AM_JOB_DIR").unwrap());
                std::fs::write(root.join("ready"), "ready").unwrap();
                std::thread::sleep(Duration::from_secs(2));
                std::fs::write(root.join("late"), "survived").unwrap();
            }
            _ => {}
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_exit_closes_jobs_without_running_guard_destructors() {
        let root = fresh_test_dir("job-exit");
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::tests::windows_job_helper",
                "--nocapture",
            ])
            .env("AM_JOB_HELPER", "exit")
            .env("AM_JOB_DIR", &root)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(root.join("ready").exists());
        std::thread::sleep(Duration::from_secs(3));
        assert!(
            !root.join("late").exists(),
            "Grandchild survived process exit"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn killing_a_group_ends_its_grandchildren() {
        let root = fresh_test_dir("group");
        let marker = root.join("late");
        let ready = root.join("ready");
        let mut cmd = command("/bin/sh");
        cmd.arg("-c").arg(format!(
            "(touch '{}'; sleep 2; touch '{}') & wait",
            ready.display(),
            marker.display()
        ));
        let (mut child, group) = spawn_group(&mut cmd).unwrap();
        let start = Instant::now();
        while !ready.exists() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "Grandchild did not start"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        group.kill(&mut child);
        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(2500));
        assert!(!marker.exists(), "Grandchild survived group termination");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn desktop_shutdown_cleans_groups_without_stopping_independent_commands() {
        let root = fresh_test_dir("shutdown");
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process::tests::shutdown_helper", "--nocapture"])
            .env("AM_SHUTDOWN_DIR", &root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Shutdown helper failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completed_capture_does_not_leave_a_detached_descendant() {
        use capture_test_support::{assert_descendant_stopped, command, root};
        let root = root("completed");
        let start = Instant::now();
        let output = capture_group(
            command(&root, "detached")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped()),
            Duration::from_secs(10),
            1024,
        )
        .unwrap();
        assert!(output.status.success());
        assert!(
            !root.join("late").exists(),
            "Fixture retained captured pipes until the descendant wrote its marker"
        );
        assert_descendant_stopped(&root);
    }

    // Shutdown is process-wide; exercise it in a separate test process.
    #[cfg(unix)]
    #[test]
    fn shutdown_helper() {
        use std::process::Stdio;
        let Some(root) = std::env::var_os("AM_SHUTDOWN_DIR") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let mut groups = Vec::new();
        for (name, wait) in [("running", "wait"), ("exited", "")] {
            let mut command = Command::new("/bin/sh");
            command
                .arg("-c")
                .arg(format!(
                    "(touch '{name}.ready'; sleep 2; touch '{name}.late') & \
                     while [ ! -e '{name}.ready' ]; do sleep 0.01; done; {wait}"
                ))
                .current_dir(&root)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let (mut child, group) = spawn_group(&mut command).unwrap();
            let start = Instant::now();
            while !root.join(format!("{name}.ready")).exists() {
                assert!(start.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(10));
            }
            if name == "exited" {
                assert!(child.wait().unwrap().success());
            }
            groups.push((child, group));
        }
        let mut independent = Command::new("/bin/sh")
            .args(["-c", "sleep 2; touch independent.late"])
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        shutdown_groups();
        shutdown_groups();
        for (child, _) in &mut groups {
            let _ = child.wait().unwrap();
        }
        assert!(independent.wait().unwrap().success());
        assert!(
            !root.join("running.late").exists(),
            "Running group survived"
        );
        assert!(
            !root.join("exited.late").exists(),
            "Exited parent's descendant survived"
        );
        assert!(root.join("independent.late").exists());
        let error = spawn_group(Command::new("/bin/sh").arg("-c").arg("true"))
            .err()
            .expect("A new command started during shutdown");
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    }

    #[test]
    fn production_code_builds_commands_through_this_module() {
        const EXCEPTIONS: &[&str] = &["process.rs", "native_programs/locate.rs"];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root.clone()];
        let mut offenders = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let rel = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if !rel.ends_with(".rs") || EXCEPTIONS.contains(&rel.as_str()) {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let production = text.split("#[cfg(test)]").next().unwrap_or_default();
                if production.contains("Command::new(") {
                    offenders.push(rel);
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "改用 crate::process::command：{offenders:?}"
        );
    }
}
