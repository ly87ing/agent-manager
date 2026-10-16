use super::Target;
use crate::Request;
use anyhow::{bail, Result};
use serde_json::{json, Value};

fn settings_url(args: &[String]) -> Result<&'static str> {
    if args.len() != 2 || args[0] != "--pane" {
        bail!("Choose a supported system settings pane.");
    }
    match args[1].as_str() {
        "date-time" => Ok(if cfg!(windows) {
            "ms-settings:dateandtime"
        } else {
            "x-apple.systempreferences:com.apple.Date-Time-Settings.extension"
        }),
        "language-region" => Ok(if cfg!(windows) {
            "ms-settings:regionlanguage"
        } else {
            "x-apple.systempreferences:com.apple.Localization-Settings.extension"
        }),
        _ => bail!("Choose a supported system settings pane."),
    }
}

#[cfg(any(target_os = "linux", test))]
fn kde_settings_command(pane: &str, listed: &str) -> Option<(&'static str, Vec<&'static str>)> {
    let modules: std::collections::BTreeSet<_> = listed
        .lines()
        .filter_map(|line| line.split_once(" - ").map(|(id, _)| id.trim()))
        .collect();
    match pane {
        "date-time" if modules.contains("kcm_clock") => {
            Some(("/usr/bin/systemsettings", vec!["kcm_clock"]))
        }
        "language-region" if modules.contains("kcm_regionandlang") => {
            Some(("/usr/bin/systemsettings", vec!["kcm_regionandlang"]))
        }
        "language-region"
            if modules.contains("kcm_translations") && modules.contains("kcm_formats") =>
        {
            Some((
                "/usr/bin/kcmshell5",
                vec!["kcm_translations", "kcm_formats"],
            ))
        }
        _ => None,
    }
}

pub(super) fn open_settings(req: &Request) -> Result<Value> {
    let url = settings_url(&req.args)?;
    #[cfg(target_os = "macos")]
    {
        super::macos::command("/usr/bin/open", &[url])
            .ok_or_else(|| anyhow::anyhow!("Could not open System Settings."))?;
        Ok(json!({"pane":req.args[1]}))
    }
    #[cfg(windows)]
    {
        super::windows::script(&format!("Start-Process '{url}' -ErrorAction Stop"))
            .ok_or_else(|| anyhow::anyhow!("Could not open Windows Settings."))?;
        Ok(json!({"pane":req.args[1]}))
    }
    #[cfg(target_os = "linux")]
    {
        let _ = url;
        let kde = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains("kde");
        let (program, panes) = if kde {
            let listed =
                super::command::read("/usr/bin/systemsettings", &["--list"]).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Could not discover KDE settings modules. Open System Settings manually."
                    )
                })?;
            kde_settings_command(&req.args[1], &listed)
                .ok_or_else(|| anyhow::anyhow!("The requested KDE settings modules are unavailable. Open System Settings manually."))?
        } else {
            (
                "/usr/bin/gnome-control-center",
                vec![if req.args[1] == "date-time" {
                    "datetime"
                } else {
                    "region"
                }],
            )
        };
        crate::process::launch_command(program).args(&panes).stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn()
            .map_err(|_| anyhow::anyhow!("Could not open the desktop's date/time or region settings. Use your desktop settings application."))?;
        Ok(json!({"pane":req.args[1]}))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = url;
        bail!("Environment actions require a supported desktop.");
    }
}

#[cfg(any(target_os = "macos", test))]
const READ_TIME_SETTINGS: &str = r#"try
    return do shell script "/usr/bin/env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin LC_ALL=C /usr/sbin/systemsetup -getusingnetworktime" with administrator privileges
on error messageText number errorNumber
    if errorNumber is -128 then return "cancelled"
    return "failed"
end try"#;

#[cfg(any(target_os = "macos", test))]
fn time_settings_result(
    output: Option<&str>,
    make_report: impl FnOnce(bool) -> Result<Value>,
) -> Result<Value> {
    match output.map(str::trim) {
        Some("Network Time: On") => Ok(json!({"status":"read", "report":make_report(true)?})),
        Some("Network Time: Off") => Ok(json!({"status":"read", "report":make_report(false)?})),
        Some("cancelled") => Ok(json!({"status":"cancelled", "report":null})),
        _ => Ok(json!({"status":"failed", "report":null})),
    }
}

pub(super) fn check_time_settings(target: &Target, probe: bool) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        use std::{process::Stdio, time::Duration};
        let mut command = crate::process::launch_command("/usr/bin/osascript");
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LC_ALL", "C")
            .args(["-e", READ_TIME_SETTINGS])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output =
            crate::process::capture_group(&mut command, Duration::from_secs(150), 16 * 1024)
                .ok()
                .filter(|value| value.status.success() && value.stdout.len() < 16 * 1024)
                .and_then(|value| String::from_utf8(value.stdout).ok());
        time_settings_result(output.as_deref(), |automatic| {
            super::report_with_automatic_time(target, probe, Some(automatic))
        })
    }
    #[cfg(any(windows, target_os = "linux"))]
    {
        Ok(json!({"status":"read", "report":super::report(target, probe)?}))
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = (target, probe);
        bail!("Environment actions require a supported desktop.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_destinations_are_fixed_and_reject_arbitrary_urls_and_extra_flags() {
        for pane in ["date-time", "language-region"] {
            assert!(settings_url(&["--pane".into(), pane.into()])
                .unwrap()
                .starts_with(if cfg!(windows) {
                    "ms-settings:"
                } else {
                    "x-apple.systempreferences:"
                }));
        }
        for args in [
            vec![],
            vec!["--pane", "https://example.org"],
            vec!["--pane", "date-time", "--confirm"],
        ] {
            assert!(
                settings_url(&args.into_iter().map(str::to_string).collect::<Vec<_>>()).is_err()
            );
        }
    }

    #[test]
    fn kde_settings_use_only_complete_discovered_module_ids() {
        let legacy = "Modules:\nkcm_clock        - Date and Time\nkcm_translations - Language\nkcm_formats      - Formats\n";
        assert_eq!(
            kde_settings_command("language-region", legacy),
            Some((
                "/usr/bin/kcmshell5",
                vec!["kcm_translations", "kcm_formats"]
            ))
        );
        assert_eq!(
            kde_settings_command("date-time", legacy),
            Some(("/usr/bin/systemsettings", vec!["kcm_clock"]))
        );
        assert_eq!(
            kde_settings_command(
                "language-region",
                &format!("{legacy}kcm_regionandlang - 语言与区域\n")
            ),
            Some(("/usr/bin/systemsettings", vec!["kcm_regionandlang"]))
        );
        for listed in [
            "",
            "kcm_translations - Language",
            "kcm_formats - Formats",
            "fake - kcm_regionandlang",
            "fake_kcm_regionandlang - Region",
            "kcm_regionandlang_suffix - Region",
        ] {
            assert!(kde_settings_command("language-region", listed).is_none());
        }
        assert!(kde_settings_command("date-time", "fake - kcm_clock").is_none());
        assert!(kde_settings_command("arbitrary", legacy).is_none());
    }

    #[test]
    fn authorized_inspection_only_reports_exact_read_results() {
        for (text, enabled) in [("Network Time: On\n", true), ("Network Time: Off", false)] {
            let value = time_settings_result(Some(text), |automatic| {
                assert_eq!(automatic, enabled);
                Ok(json!({"automatic_time":automatic}))
            })
            .unwrap();
            assert_eq!(value["status"], "read");
            assert_eq!(value["report"]["automatic_time"], enabled);
        }
        for (text, status) in [
            (Some("cancelled"), "cancelled"),
            (Some("failed"), "failed"),
            (None, "failed"),
            (Some("Network Time: Unknown"), "failed"),
        ] {
            let value = time_settings_result(text, |_| {
                panic!("Failed authorization must not fabricate a report")
            })
            .unwrap();
            assert_eq!(value["status"], status);
            assert!(value["report"].is_null());
        }
        assert!(READ_TIME_SETTINGS.contains("-getusingnetworktime"));
        assert!(!READ_TIME_SETTINGS.contains("-set"));
        assert!(!READ_TIME_SETTINGS.contains("sntp"));
    }
}
