//! Bind an existing native timesyncd instance without loading or starting units.
use super::repair_linux_ntp_config as config;
use serde::Serialize;
use std::{
    collections::HashMap,
    io::Read,
    os::fd::{FromRawFd, OwnedFd},
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::Path,
    time::Duration,
};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SERVICE: &str = "org.freedesktop.timesync1";
const PATH: &str = "/org/freedesktop/timesync1";
const INTERFACE: &str = "org.freedesktop.timesync1.Manager";
const SYSTEMD: &str = "org.freedesktop.systemd1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct BusPath {
    directory: (u64, u64, u32),
    socket: (u64, u64, u32),
}
fn bus_path() -> Option<BusPath> {
    let mut directory = None;
    for path in ["/", "/run", "/run/dbus"] {
        let meta = std::fs::symlink_metadata(path).ok()?;
        if !meta.is_dir() || meta.uid() != 0 || meta.mode() & 0o022 != 0 {
            return None;
        }
        directory = Some((meta.dev(), meta.ino(), meta.mode()));
    }
    let socket = std::fs::symlink_metadata("/run/dbus/system_bus_socket").ok()?;
    if !socket.file_type().is_socket() || socket.uid() != 0 {
        return None;
    }
    Some(BusPath {
        directory: directory?,
        socket: (socket.dev(), socket.ino(), socket.mode()),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Process {
    pid: u32,
    birth: u64,
    uid: u32,
}

// Only the elevated helper can inspect another UID's /proc/PID/exe. Pin this
// additional proof through READY and each mutation; it is not preview evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Executable {
    process: Process,
    executable: String,
    dev: u64,
    ino: u64,
    metadata: (u32, u32, u32, u64, u64, (i64, i64), (i64, i64)),
    digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Instance {
    bus: String,
    bus_path: BusPath,
    owner: String,
    manager: String,
    invocation: Vec<u8>,
    process: Process,
    unit_file_state: String,
    runtime: Vec<String>,
    system: Vec<String>,
    link: Vec<String>,
    fallback: Vec<String>,
    root_distance: u64,
    poll_min: u64,
    poll_max: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct State {
    instance: Instance,
    pub configuration: config::State,
}

impl State {
    pub(super) fn digest(&self) -> Option<String> {
        Some(crate::native_mcp::model::sha256_hex(
            &serde_json::to_string(self).ok()?,
        ))
    }

    pub(super) fn runtime_before(&self) -> serde_json::Value {
        match self.instance.runtime.as_slice() {
            [] => serde_json::json!(""),
            [server] => serde_json::json!(server.trim_end_matches('.').to_ascii_lowercase()),
            servers => serde_json::json!(servers),
        }
    }

    pub(super) fn runtime_matches(&self, server: &str) -> bool {
        self.instance.runtime.len() == 1
            && self.instance.runtime[0]
                .trim_end_matches('.')
                .eq_ignore_ascii_case(server.trim_end_matches('.'))
    }

    pub(super) fn available(&self, server: &str) -> bool {
        super::time_probe::valid_host(server)
            && (self.instance.runtime.is_empty() || self.runtime_matches(server))
    }

    pub(super) fn with_runtime(&self, server: &str) -> Option<Self> {
        if !super::time_probe::valid_host(server) {
            return None;
        }
        let mut after = self.clone();
        if !after.runtime_matches(server) {
            after.instance.runtime = vec![server.trim_end_matches('.').to_ascii_lowercase()];
        }
        Some(after)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Dispatch {
    Written,
    Unchanged,
    Refused,
    Stale,
    Unknown,
}

pub(super) struct Session {
    connection: zbus::Connection,
}

fn proc_text(pid: u32, name: &str) -> Option<String> {
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

fn process_uid(pid: u32) -> Option<u32> {
    let status = proc_text(pid, "status")?;
    let ids = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_ascii_whitespace()
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (ids.len() == 4 && ids[0] != 0 && ids.iter().all(|value| *value == ids[0])).then(|| ids[0])
}

fn same_executable(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.is_file()
        && b.is_file()
        && a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.uid() == b.uid()
        && a.gid() == b.gid()
        && a.mode() == b.mode()
        && a.nlink() == b.nlink()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}

fn process(pid: u32, uid: u32) -> Option<Process> {
    if uid == 0 || pid <= 1 {
        return None;
    }
    let stat = proc_text(pid, "stat")?;
    let birth = super::repair_linux_time::start_time(&stat)?;
    // Native privilege fast-path uses the daemon's real UID, not just its bus UID.
    if process_uid(pid)? != uid {
        return None;
    }
    Some(Process { pid, birth, uid })
}

fn executable(expected: &Process) -> Option<Executable> {
    let Process { pid, birth, uid } = *expected;
    if process(pid, uid).as_ref() != Some(expected) {
        return None;
    }
    for directory in ["/usr", "/usr/lib", "/usr/lib/systemd"] {
        let meta = std::fs::symlink_metadata(directory).ok()?;
        if !meta.is_dir() || meta.uid() != 0 || meta.mode() & 0o022 != 0 {
            return None;
        }
    }
    let path = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let installed = std::fs::canonicalize("/usr/lib/systemd/systemd-timesyncd").ok()?;
    if path != installed || installed != Path::new("/usr/lib/systemd/systemd-timesyncd") {
        return None;
    }
    let meta = std::fs::symlink_metadata(&installed).ok()?;
    if !meta.is_file()
        || meta.uid() != 0
        || meta.mode() & 0o022 != 0
        || meta.len() > 16 * 1024 * 1024
    {
        return None;
    }
    let mut file = std::fs::File::open(format!("/proc/{pid}/exe")).ok()?;
    let opened = file.metadata().ok()?;
    if !same_executable(&opened, &meta) {
        return None;
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(meta.len().checked_add(1)?)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 != meta.len()
        || std::fs::read_link(format!("/proc/{pid}/exe")).ok()? != installed
        || super::repair_linux_time::start_time(&proc_text(pid, "stat")?)? != birth
    {
        return None;
    }
    let after = file.metadata().ok()?;
    let current = std::fs::symlink_metadata(&installed).ok()?;
    if !same_executable(&after, &meta)
        || !same_executable(&current, &meta)
        || process_uid(pid)? != uid
    {
        return None;
    }
    Some(Executable {
        process: expected.clone(),
        executable: installed.to_str()?.into(),
        dev: meta.dev(),
        ino: meta.ino(),
        metadata: (
            meta.uid(),
            meta.gid(),
            meta.mode(),
            meta.nlink(),
            meta.len(),
            (meta.mtime(), meta.mtime_nsec()),
            (meta.ctime(), meta.ctime_nsec()),
        ),
        digest: crate::native_mcp::model::sha256_hex_bytes(&bytes),
    })
}

fn pidfd(pid: u32) -> Option<OwnedFd> {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

fn servers(values: &HashMap<String, OwnedValue>, key: &str) -> Option<Vec<String>> {
    let values = Vec::<String>::try_from(values.get(key)?.try_clone().ok()?).ok()?;
    if values.len() > 64
        || values.iter().any(|value| {
            value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
        })
    {
        return None;
    }
    Some(values)
}

fn runtime_api(xml: &str) -> bool {
    if xml.len() > 64 * 1024 {
        return false;
    }
    let Ok(node) = zbus_xml::Node::from_reader(xml.as_bytes()) else {
        return false;
    };
    let interfaces: Vec<_> = node
        .interfaces()
        .iter()
        .filter(|interface| interface.name().as_str() == INTERFACE)
        .collect();
    if interfaces.len() != 1 {
        return false;
    }
    let methods: Vec<_> = interfaces[0]
        .methods()
        .iter()
        .filter(|method| method.name().as_str() == "SetRuntimeNTPServers")
        .collect();
    methods.len() == 1
        && methods[0].args().len() == 1
        && methods[0].args()[0].ty().to_string() == "as"
        && matches!(
            methods[0].args()[0].direction(),
            None | Some(zbus_xml::ArgDirection::In)
        )
}

impl Session {
    pub(super) async fn open() -> Option<Self> {
        // A caller-provided bus address must not redirect privileged settings.
        bus_path()?;
        let connection =
            zbus::connection::Builder::address("unix:path=/run/dbus/system_bus_socket")
                .ok()?
                .method_timeout(Duration::from_secs(8))
                .build()
                .await
                .ok()?;
        Some(Self { connection })
    }

    async fn instance(&self) -> Option<Instance> {
        let socket = bus_path()?;
        let bus = zbus::Proxy::new(
            &self.connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .await
        .ok()?;
        let id: String = bus.call("GetId", &()).await.ok()?;
        let owner: String = bus.call("GetNameOwner", &(SERVICE,)).await.ok()?;
        let manager: String = bus.call("GetNameOwner", &(SYSTEMD,)).await.ok()?;
        zbus::names::UniqueName::try_from(owner.as_str()).ok()?;
        zbus::names::UniqueName::try_from(manager.as_str()).ok()?;
        let manager_uid: u32 = bus.call("GetConnectionUnixUser", &(&manager,)).await.ok()?;
        let manager_pid: u32 = bus
            .call("GetConnectionUnixProcessID", &(&manager,))
            .await
            .ok()?;
        if manager_uid != 0 || manager_pid != 1 {
            return None;
        }
        let pid: u32 = bus
            .call("GetConnectionUnixProcessID", &(&owner,))
            .await
            .ok()?;
        let uid: u32 = bus.call("GetConnectionUnixUser", &(&owner,)).await.ok()?;
        let fd = pidfd(pid)?;
        let process = blocking::unblock(move || process(pid, uid)).await?;
        let systemd = zbus::Proxy::new(
            &self.connection,
            manager.as_str(),
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
        )
        .await
        .ok()?;
        // GetUnitByPIDFD is lookup-only and never falls back to loading a unit.
        let (_, unit, invocation): (OwnedObjectPath, String, Vec<u8>) = systemd
            .call_with_flags(
                "GetUnitByPIDFD",
                zbus::proxy::MethodFlags::NoAutoStart.into(),
                &(zbus::zvariant::Fd::from(&fd),),
            )
            .await
            .ok()??;
        if unit != "systemd-timesyncd.service"
            || invocation.len() != 16
            || invocation.iter().all(|byte| *byte == 0)
        {
            return None;
        }
        let unit_file_state: String = systemd
            .call_with_flags(
                "GetUnitFileState",
                zbus::proxy::MethodFlags::NoAutoStart.into(),
                &(&unit,),
            )
            .await
            .ok()??;
        let introspection = zbus::Proxy::new(
            &self.connection,
            owner.as_str(),
            PATH,
            "org.freedesktop.DBus.Introspectable",
        )
        .await
        .ok()?;
        let xml: String = introspection
            .call_with_flags(
                "Introspect",
                zbus::proxy::MethodFlags::NoAutoStart.into(),
                &(),
            )
            .await
            .ok()??;
        if !runtime_api(&xml) {
            return None;
        }
        let properties = zbus::Proxy::new(
            &self.connection,
            owner.as_str(),
            PATH,
            "org.freedesktop.DBus.Properties",
        )
        .await
        .ok()?;
        let values: HashMap<String, OwnedValue> = properties
            .call_with_flags(
                "GetAll",
                zbus::proxy::MethodFlags::NoAutoStart.into(),
                &(INTERFACE,),
            )
            .await
            .ok()??;
        let instance = Instance {
            bus: id.clone(),
            bus_path: socket.clone(),
            owner: owner.clone(),
            manager: manager.clone(),
            invocation,
            process,
            unit_file_state,
            runtime: servers(&values, "RuntimeNTPServers")?,
            system: servers(&values, "SystemNTPServers")?,
            link: servers(&values, "LinkNTPServers")?,
            fallback: servers(&values, "FallbackNTPServers")?,
            root_distance: u64::try_from(values.get("RootDistanceMaxUSec")?).ok()?,
            poll_min: u64::try_from(values.get("PollIntervalMinUSec")?).ok()?,
            poll_max: u64::try_from(values.get("PollIntervalMaxUSec")?).ok()?,
        };
        let after_id: String = bus.call("GetId", &()).await.ok()?;
        let after_owner: String = bus.call("GetNameOwner", &(SERVICE,)).await.ok()?;
        let after_manager: String = bus.call("GetNameOwner", &(SYSTEMD,)).await.ok()?;
        if id != after_id
            || owner != after_owner
            || manager != after_manager
            || bus_path()? != socket
        {
            return None;
        }
        Some(instance)
    }

    pub(super) async fn snapshot(&self) -> Option<State> {
        let instance = self.instance().await?;
        let configuration = blocking::unblock(config::read).await?;
        if self.instance().await? != instance {
            return None;
        }
        Some(State {
            instance,
            configuration,
        })
    }

    pub(super) async fn root_sender(&self) -> Option<()> {
        if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
            return None;
        }
        let name = self.connection.unique_name()?;
        let bus = zbus::Proxy::new(
            &self.connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .await
        .ok()?;
        let uid: u32 = bus.call("GetConnectionUnixUser", &(name,)).await.ok()?;
        (uid == 0).then_some(())
    }

    pub(super) fn executable(&self, before: &State) -> Option<Executable> {
        executable(&before.instance.process)
    }

    pub(super) async fn set_runtime(
        &self,
        before: &State,
        server: &str,
        ready_to_write: impl Fn() -> bool,
    ) -> Dispatch {
        if !before.available(server) || self.root_sender().await.is_none() {
            return Dispatch::Refused;
        }
        if self.snapshot().await.as_ref() != Some(before) {
            return Dispatch::Stale;
        }
        if before.runtime_matches(server) {
            return Dispatch::Unchanged;
        }
        let Ok(service) = zbus::Proxy::new(
            &self.connection,
            before.instance.owner.as_str(),
            PATH,
            INTERFACE,
        )
        .await
        else {
            return Dispatch::Unknown;
        };
        let server = server.trim_end_matches('.').to_ascii_lowercase();
        if !ready_to_write() {
            return Dispatch::Stale;
        }
        let result: zbus::Result<Option<()>> = service
            .call_with_flags(
                "SetRuntimeNTPServers",
                zbus::proxy::MethodFlags::NoAutoStart.into(),
                &(vec![server],),
            )
            .await;
        match result {
            Ok(Some(())) => Dispatch::Written,
            Err(zbus::Error::MethodError(name, ..))
                if matches!(
                    name.as_str(),
                    "org.freedesktop.DBus.Error.AccessDenied"
                        | "org.freedesktop.DBus.Error.UnknownMethod"
                        | "org.freedesktop.DBus.Error.InvalidArgs"
                ) =>
            {
                Dispatch::Refused
            }
            // Allocation errors can follow flushing the runtime list. A timeout
            // or lost response cannot revoke an already-dispatched request.
            _ => Dispatch::Unknown,
        }
    }
}

pub(super) fn read() -> Option<State> {
    zbus::block_on(futures_lite::future::race(
        async { Session::open().await?.snapshot().await },
        async {
            async_io::Timer::after(Duration::from_secs(8)).await;
            None
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repair_requires_the_typed_native_runtime_setter_without_calling_it() {
        let xml = |args: &str| {
            format!("<node><interface name=\"{INTERFACE}\"><method name=\"SetRuntimeNTPServers\">{args}</method></interface></node>")
        };
        assert!(runtime_api(&xml("<arg type=\"as\" direction=\"in\"/>")));
        assert!(runtime_api(&xml("<arg type=\"as\"/>")));
        for invalid in [
            xml(""),
            xml("<arg type=\"s\"/>"),
            xml("<arg type=\"as\" direction=\"out\"/>"),
            xml("<arg type=\"as\"/><arg type=\"b\"/>"),
            "<broken>".into(),
            "x".repeat(64 * 1024 + 1),
        ] {
            assert!(!runtime_api(&invalid));
        }
    }
}
