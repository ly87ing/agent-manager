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

fn language_tag(locale: &str) -> Option<String> {
    let tag = locale.split(['.', '@']).next()?.replace('_', "-");
    let language = tag.split('-').next()?;
    (language.len() >= 2
        && language.len() <= 8
        && language.bytes().all(|c| c.is_ascii_alphabetic())
        && !matches!(language, "POSIX"))
    .then_some(tag)
}

#[cfg(target_os = "linux")]
pub(super) fn account_user_path() -> Option<String> {
    let uid = unsafe { libc::geteuid() }.to_string();
    let output = read(
        "/usr/bin/busctl",
        &[
            "--system",
            "--json=short",
            "call",
            "org.freedesktop.Accounts",
            "/org/freedesktop/Accounts",
            "org.freedesktop.Accounts",
            "FindUserById",
            "x",
            &uid,
        ],
    )?;
    let value: serde_json::Value = serde_json::from_str(&output).ok()?;
    value["data"][0]
        .as_str()
        .filter(|path| {
            path.starts_with("/org/freedesktop/Accounts/User")
                && path.chars().all(|c| c.is_ascii_alphanumeric() || c == '/')
        })
        .map(str::to_string)
}

#[cfg(target_os = "linux")]
pub(super) fn gsetting(schema: &str, key: &str) -> Option<String> {
    read("/usr/bin/gsettings", &["get", schema, key])
}

#[cfg(target_os = "linux")]
fn user_locale() -> (Option<Vec<String>>, Option<String>, Option<bool>) {
    let mut locales = Vec::<String>::new();
    if let Some(path) = account_user_path() {
        if let Some(primary) = bus_property(
            "org.freedesktop.Accounts",
            &path,
            "org.freedesktop.Accounts.User",
            "Language",
            false,
        )
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| language_tag(value).is_some())
        {
            locales.push(primary);
            if let Some(extra) = bus_property(
                "org.freedesktop.Accounts",
                &path,
                "org.freedesktop.Accounts.User",
                "Languages",
                false,
            )
            .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
            {
                for locale in extra {
                    if language_tag(&locale).is_none() {
                        return (None, None, None);
                    }
                    if !locales
                        .iter()
                        .any(|prior| prior.eq_ignore_ascii_case(&locale))
                    {
                        locales.push(locale);
                    }
                }
            }
        }
    }
    let gnome = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .contains("gnome");
    let region = if gnome {
        gsetting("org.gnome.system.locale", "region")
            .and_then(|value| {
                value
                    .strip_prefix('\'')
                    .and_then(|text| text.strip_suffix('\''))
                    .map(str::to_string)
            })
            .and_then(|locale| {
                if locale.is_empty() {
                    locales.first().cloned()
                } else {
                    Some(locale)
                }
            })
    } else {
        None
    };
    let automatic = if gnome {
        gsetting("org.gnome.desktop.datetime", "automatic-timezone").and_then(|value| {
            match value.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            }
        })
    } else {
        None
    };
    (
        if locales.is_empty() {
            None
        } else {
            locales.iter().map(|value| language_tag(value)).collect()
        },
        region,
        automatic,
    )
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
    let (languages, user_locale, automatic_timezone) = user_locale();
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
        apple_locale: user_locale,
        languages,
        automatic_timezone,
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
            "GNOME org.gnome.desktop.datetime automatic-timezone",
        ),
        (
            "language.preferred",
            "AccountsService current-UID Language/Languages (session preference)",
        ),
        (
            "language.region",
            "GNOME org.gnome.system.locale region; current-user language inheritance",
        ),
        ("language.locale", "GNOME current-user regional locale"),
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
        assert_eq!(language_tag("en_US.UTF-8").as_deref(), Some("en-US"));
        assert_eq!(
            language_tag("zh_Hant_TW@calendar=chinese").as_deref(),
            Some("zh-Hant-TW")
        );
        for locale in ["C", "C.UTF-8", "POSIX", ""] {
            assert!(language_tag(locale).is_none());
        }
    }
}
