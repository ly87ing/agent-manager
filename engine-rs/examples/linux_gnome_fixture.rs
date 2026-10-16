//! CI preference fixture only; this does not implement geolocation or a desktop.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use std::{io::Write, time::Duration};
    anyhow::ensure!(
        std::env::var("CI").as_deref() == Ok("true")
            && std::env::var("AM_ISOLATED_LINUX_TIMEZONE").as_deref() == Ok("1")
            && unsafe { libc::geteuid() } != 0,
        "Only the disposable nonroot CI caller may run this fixture."
    );
    let connection = zbus::blocking::connection::Builder::session()?
        .method_timeout(Duration::from_secs(8))
        .build()?;
    let reply = connection.request_name_with_flags(
        "org.gnome.SettingsDaemon.Datetime",
        zbus::fdo::RequestNameFlags::DoNotQueue.into(),
    )?;
    anyhow::ensure!(
        reply == zbus::fdo::RequestNameReply::PrimaryOwner,
        "Never replace an existing desktop daemon."
    );
    println!("{{\"ready\":true}}");
    std::io::stdout().flush()?;
    // The parent terminates only this owned child. A deadline bounds failures.
    std::thread::sleep(Duration::from_secs(60));
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    std::process::exit(2);
}
