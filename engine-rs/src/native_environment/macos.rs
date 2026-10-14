use super::Observation;
use std::process::Stdio;
use std::time::Duration;

const PREFERENCES: &str = r#"ObjC.import('Foundation');
var d=ObjC.deepUnwrap($.NSUserDefaults.standardUserDefaults.persistentDomainForName($.NSGlobalDomain)) || {};
var out={};
['AppleLocale','AppleLanguages','AppleICUForce24HourTime','AppleMeasurementUnits','AppleMetricUnits','AppleTemperatureUnit','AppleFirstWeekday','AppleICUDateFormatStrings','AppleICUTimeFormatStrings','AppleICUNumberFormatStrings'].forEach(function(k){if(d[k] !== undefined) out[k]=d[k];});
JSON.stringify(out)"#;

pub(super) fn command(program: &str, args: &[&str]) -> Option<String> {
    let mut command = crate::process::launch_command(program);
    command
        .args(args)
        .env("LC_ALL", "C")
        .env_remove("TZ")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output =
        crate::process::capture_group(&mut command, Duration::from_secs(3), 16 * 1024).ok()?;
    if !output.status.success() || output.stdout.len() >= 16 * 1024 {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|text| text.trim().to_string())
}

pub(super) fn configured_server(text: &str) -> Option<String> {
    let servers: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('#').next()?.split_whitespace();
            (fields.next()? == "server")
                .then(|| {
                    fields
                        .next()
                        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
                })
                .flatten()
        })
        .collect();
    if servers.len() == 1 && super::time_probe::valid_host(&servers[0]) {
        servers.into_iter().next()
    } else {
        None
    }
}

pub(super) fn collect() -> Observation {
    let preferences = command(
        "/usr/bin/osascript",
        &["-l", "JavaScript", "-e", PREFERENCES],
    )
    .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok());
    let mut observed = Observation {
        timezone: std::fs::read_link("/etc/localtime").ok().and_then(|path| {
            path.to_str()
                .and_then(|text| text.split("/zoneinfo/").nth(1))
                .map(str::to_string)
        }),
        languages: preferences
            .as_ref()
            .and_then(|value| value.get("AppleLanguages"))
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        apple_locale: preferences
            .as_ref()
            .and_then(|value| value.get("AppleLocale"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        automatic_timezone: command(
            "/usr/bin/defaults",
            &[
                "read",
                "/Library/Preferences/com.apple.timezone.auto",
                "Active",
            ],
        )
        .and_then(|value| match value.as_str() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }),
        automatic_time: command("/usr/sbin/systemsetup", &["-getusingnetworktime"]).and_then(
            |value| match value.as_str() {
                "Network Time: On" => Some(true),
                "Network Time: Off" => Some(false),
                _ => None,
            },
        ),
        ntp_server: std::fs::read_to_string("/etc/ntp.conf")
            .ok()
            .filter(|value| value.len() <= 16 * 1024)
            .and_then(|value| configured_server(&value)),
        time_service: command("/bin/launchctl", &["print", "system/com.apple.timed"])
            .map(|value| value.lines().any(|line| line.trim() == "state = running")),
        local_time: command("/bin/date", &["+%Y-%m-%d %H:%M:%S %Z %z"]),
        os_version: command("/usr/bin/sw_vers", &["-productVersion"]),
        ..Observation::default()
    };
    observed.format_overrides = preferences.map(|mut value| {
        if let Some(map) = value.as_object_mut() {
            map.remove("AppleLocale");
            map.remove("AppleLanguages");
        }
        value
    });
    for key in ["TZ", "LANG", "LC_ALL"]
        .iter()
        .chain(super::LOCALE_CATEGORIES.iter())
    {
        if let Some(value) = std::env::var_os(key) {
            observed.env.insert(
                (*key).into(),
                value.to_string_lossy().chars().take(512).collect(),
            );
        }
    }
    observed
}
