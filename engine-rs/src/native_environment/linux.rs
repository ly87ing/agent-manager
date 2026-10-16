//! Linux observations use machine-readable systemd properties, with file fallbacks.
#[cfg(target_os = "linux")]
use super::{command::read, Observation};
use std::collections::BTreeMap;

pub(super) fn properties(text: &str) -> BTreeMap<&str, &str> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .collect()
}

fn boolean(value: Option<&&str>) -> Option<bool> {
    match value.copied() {
        Some("yes" | "true") => Some(true),
        Some("no" | "false") => Some(false),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn bus_property(
    service: &str,
    path: &str,
    interface: &str,
    key: &str,
    user: bool,
) -> Option<serde_json::Value> {
    let text = read(
        "/usr/bin/busctl",
        &[
            if user { "--user" } else { "--system" },
            "--json=short",
            "get-property",
            service,
            path,
            interface,
            key,
        ],
    )?;
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()?
        .get("data")
        .cloned()
}

#[cfg(target_os = "linux")]
pub(super) fn collect() -> Observation {
    let time = read("/usr/bin/timedatectl", &["show"]).unwrap_or_default();
    let values = properties(&time);
    let locale = bus_property(
        "org.freedesktop.locale1",
        "/org/freedesktop/locale1",
        "org.freedesktop.locale1",
        "Locale",
        false,
    )
    .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok());
    let language = locale.as_ref().and_then(|list| {
        list.iter()
            .find_map(|value| value.strip_prefix("LANG=").map(str::to_string))
    });
    let timesync = read("/usr/bin/timedatectl", &["show-timesync"]).unwrap_or_default();
    let ntp = properties(&timesync);
    let servers: Vec<_> = ["RuntimeNTPServers", "SystemNTPServers", "LinkNTPServers"]
        .iter()
        .filter_map(|key| ntp.get(key))
        .flat_map(|value| value.split_whitespace())
        .collect();
    let mut observed = Observation {
        platform: Some("linux"),
        timezone: values
            .get("Timezone")
            .filter(|value| !value.is_empty())
            .map(|value| (*value).into())
            .or_else(|| {
                std::fs::read_link("/etc/localtime")
                    .ok()?
                    .to_str()?
                    .split("/zoneinfo/")
                    .nth(1)
                    .map(str::to_string)
            }),
        // localed describes system defaults, not this user's desktop override.
        apple_locale: language.clone(),
        languages: language
            .as_ref()
            .map(|value| vec![value.split('.').next().unwrap_or(value).replace('_', "-")]),
        format_overrides: locale.map(|list| serde_json::json!(list)),
        automatic_time: boolean(values.get("NTP")),
        time_synchronized: boolean(values.get("NTPSynchronized")),
        ntp_server: if servers.len() == 1 && super::time_probe::valid_host(servers[0]) {
            Some(servers[0].trim_end_matches('.').to_ascii_lowercase())
        } else {
            None
        },
        local_time: read("/bin/date", &["+%Y-%m-%d %H:%M:%S %Z %z"]),
        os_version: std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|text| {
                properties(&text)
                    .get("PRETTY_NAME")
                    .map(|value| value.trim_matches('"').to_string())
            }),
        env: super::command::process_environment(),
        ..Observation::default()
    };
    observed.sources.extend([
        ("ntp.actual_source", "time service peer unavailable"),
        ("os.version", "/etc/os-release"),
        (
            "time.timezone",
            "timedatectl Timezone; /etc/localtime fallback",
        ),
        (
            "time.automatic_timezone",
            "desktop-specific; no systemd automatic-zone switch",
        ),
        (
            "language.preferred",
            "systemd-localed LANG (system default language)",
        ),
        (
            "language.region",
            "systemd-localed LANG (system default region)",
        ),
        (
            "language.locale",
            "systemd-localed LANG (system default locale)",
        ),
        (
            "language.formats",
            "systemd-localed Locale (system defaults)",
        ),
        (
            "ntp.server",
            "timedatectl show-timesync configured server lists",
        ),
        ("ntp.automatic_time", "timedatectl NTP"),
        ("ntp.service", "time daemon service unavailable"),
        (
            "ntp.synchronized",
            "timedatectl NTPSynchronized (kernel state)",
        ),
        ("ntp.kernel_max_error", "kernel error unavailable"),
        ("ntp.kernel_estimated_error", "kernel error unavailable"),
    ]);
    observed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_properties_are_unknown_and_values_do_not_require_localized_labels() {
        let values = properties("Timezone=Asia/Singapore\nNTP=yes\nNTPSynchronized=no\nUnknown=\n");
        assert_eq!(values["Timezone"], "Asia/Singapore");
        assert_eq!(boolean(values.get("NTP")), Some(true));
        assert_eq!(boolean(values.get("NTPSynchronized")), Some(false));
        assert_eq!(boolean(values.get("missing")), None);
        assert_eq!(boolean(values.get("Unknown")), None);
    }
}
