//! Windows PowerShell reads only the current user's regional settings and OS state.
use super::Observation;
use serde_json::Value;

#[cfg(windows)]
pub(super) fn powershell() -> Option<String> {
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buffer = [0u16; 32768];
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() {
        return None;
    }
    Some(format!(
        "{}\\WindowsPowerShell\\v1.0\\powershell.exe",
        String::from_utf16(&buffer[..length]).ok()?
    ))
}

#[cfg(windows)]
pub(super) fn script(text: &str) -> Option<String> {
    script_timeout(text, std::time::Duration::from_secs(8))
}

#[cfg(windows)]
pub(super) fn script_timeout(text: &str, timeout: std::time::Duration) -> Option<String> {
    super::command::bounded(
        &powershell()?,
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("$env:PSModulePath=[IO.Path]::Combine($PSHOME,'Modules'); [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); {text}"),
        ],
        timeout,
    )
}

#[cfg(windows)]
pub(super) fn resolve(host: &str) -> anyhow::Result<std::net::IpAddr> {
    // valid_host excludes PowerShell syntax and quotes; still pass a quoted literal.
    if !super::time_probe::valid_host(host) {
        anyhow::bail!("Invalid time server.");
    }
    let output = script_timeout(&format!("Resolve-DnsName -Name '{host}' -DnsOnly -QuickTimeout -ErrorAction Stop | ForEach-Object {{ if ($_.IPAddress) {{ $_.IPAddress }} }}"), std::time::Duration::from_secs(3))
        .ok_or_else(|| anyhow::anyhow!("Name lookup did not finish."))?;
    output
        .lines()
        .find_map(|line| line.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("No time-server address was found."))
}

#[cfg(windows)]
pub(super) fn zone_id(iana: &str) -> Option<String> {
    icu_zone(iana, false)
}

#[cfg(windows)]
pub(super) fn iana_zone(windows: &str) -> Option<String> {
    icu_zone(windows, true)
}

#[cfg(windows)]
pub(super) fn geo_name() -> Option<String> {
    let mut buffer = [0u16; 16];
    let length = unsafe {
        windows_sys::Win32::Globalization::GetUserDefaultGeoName(
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    if length < 2 || length as usize > buffer.len() {
        return None;
    }
    String::from_utf16(&buffer[..length as usize - 1]).ok()
}

#[cfg(windows)]
pub(super) fn set_geo_name(region: &str) -> bool {
    if region.len() != 2 || !region.bytes().all(|value| value.is_ascii_uppercase()) {
        return false;
    }
    let value: Vec<_> = region.encode_utf16().chain(Some(0)).collect();
    unsafe { windows_sys::Win32::Globalization::SetUserGeoName(value.as_ptr()) != 0 }
}

#[cfg(windows)]
fn icu_zone(input: &str, from_windows: bool) -> Option<String> {
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };
    let name: Vec<u16> = "icu.dll\0".encode_utf16().collect();
    // Resolve ICU only in the system directory, never beside an assistant executable.
    let library = unsafe {
        LoadLibraryExW(
            name.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if library.is_null() {
        return None;
    }
    let input: Vec<u16> = input.encode_utf16().collect();
    let mut output = [0u16; 128];
    let mut status = 0i32;
    let length = unsafe {
        if from_windows {
            type Map =
                unsafe extern "C" fn(*const u16, i32, *const i8, *mut u16, i32, *mut i32) -> i32;
            GetProcAddress(library, c"ucal_getTimeZoneIDForWindowsID".as_ptr().cast()).map(
                |address| {
                    let map: Map = std::mem::transmute(address);
                    map(
                        input.as_ptr(),
                        input.len() as i32,
                        std::ptr::null(),
                        output.as_mut_ptr(),
                        output.len() as i32,
                        &mut status,
                    )
                },
            )
        } else {
            type Map = unsafe extern "C" fn(*const u16, i32, *mut u16, i32, *mut i32) -> i32;
            GetProcAddress(library, c"ucal_getWindowsTimeZoneID".as_ptr().cast()).map(|address| {
                let map: Map = std::mem::transmute(address);
                map(
                    input.as_ptr(),
                    input.len() as i32,
                    output.as_mut_ptr(),
                    output.len() as i32,
                    &mut status,
                )
            })
        }
    };
    unsafe { FreeLibrary(library) };
    length
        .filter(|length| status <= 0 && *length > 0 && (*length as usize) < output.len())
        .and_then(|length| String::from_utf16(&output[..length as usize]).ok())
}

#[cfg(windows)]
const READ: &str = r#"
$result=@{}
try { $result.zone=(Get-TimeZone -ErrorAction Stop).Id } catch {}
try { $result.languages=@((Get-WinUserLanguageList -ErrorAction Stop).LanguageTag) } catch {}
try { $result.locale=(Get-Culture -ErrorAction Stop).Name } catch {}
try { $result.formats=Get-ItemProperty 'HKCU:\Control Panel\International' -ErrorAction Stop | Select-Object sShortDate,sLongDate,sTimeFormat,sDecimal,sThousand,sCurrency } catch {}
try { $service=Get-Service W32Time -ErrorAction Stop; $result.service=($service.Status -eq 'Running') } catch {}
try { $key=Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\W32Time\Parameters' -ErrorAction Stop; $result.ntp=$key.NtpServer; $result.type=$key.Type } catch {}
try { $result.policy=Test-Path 'HKLM:\SOFTWARE\Policies\Microsoft\W32Time' -ErrorAction Stop; $result.ntp_enabled=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\W32Time\TimeProviders\NtpClient' -ErrorAction Stop).Enabled; $result.time_start=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\W32Time' -ErrorAction Stop).Start } catch {}
try { $result.zone_start=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\tzautoupdate' -ErrorAction Stop).Start } catch {}
$result.local_time=(Get-Date).ToString('yyyy-MM-dd HH:mm:ss zzz')
$result.os=[Environment]::OSVersion.Version.ToString()
$result | ConvertTo-Json -Depth 5 -Compress
"#;

pub(super) fn decode(value: &Value) -> Observation {
    let text = |key: &str| value[key].as_str().map(str::to_string);
    let unmanaged = value["policy"] == false;
    let server = text("ntp")
        .filter(|_| unmanaged && value["type"] == "NTP" && value["ntp_enabled"] == 1)
        .and_then(|value| {
            let mut peers = value.split_whitespace();
            let server = peers.next()?.split(',').next()?;
            (peers.next().is_none() && super::time_probe::valid_host(server))
                .then(|| server.trim_end_matches('.').to_ascii_lowercase())
        });
    let mut observed = Observation {
        platform: Some("windows"),
        apple_locale: text("locale"),
        user_region: text("region"),
        languages: serde_json::from_value(value["languages"].clone()).ok(),
        format_overrides: value.get("formats").cloned(),
        ntp_server: server,
        // Registry Type is configuration; it does not prove W32Time synchronized.
        automatic_time: if !unmanaged {
            None
        } else if value["type"] == "NoSync" || value["ntp_enabled"] == 0 || value["time_start"] == 4
        {
            Some(false)
        } else {
            match (
                value["type"].as_str(),
                value["ntp_enabled"].as_u64(),
                value["time_start"].as_u64(),
            ) {
                (Some("NTP" | "NT5DS" | "AllSync"), Some(1), Some(2 | 3)) => Some(true),
                _ => None,
            }
        },
        automatic_timezone: match value["zone_start"].as_u64() {
            Some(3) => Some(true),
            Some(4) => Some(false),
            _ => None,
        },
        time_service: value["service"].as_bool(),
        local_time: text("local_time"),
        os_version: text("os"),
        ..Observation::default()
    };
    observed.sources.extend([
        ("ntp.actual_source", "W32Time live peer unavailable"),
        ("os.version", "Windows OSVersion"),
        ("time.local", "Get-Date without TZ override"),
        ("time.timezone", "Get-TimeZone; ICU IANA mapping"),
        ("time.automatic_timezone", "tzautoupdate Start"),
        (
            "language.preferred",
            "Get-WinUserLanguageList (current user)",
        ),
        ("language.region", "GetUserDefaultGeoName (current user)"),
        ("language.locale", "Get-Culture (current user)"),
        ("language.formats", "HKCU Control Panel International"),
        ("ntp.server", "W32Time Parameters (manual NTP only)"),
        ("ntp.automatic_time", "W32Time Parameters Type"),
        ("ntp.service", "Get-Service W32Time"),
        ("ntp.synchronized", "W32Time synchronization unavailable"),
        ("ntp.kernel_max_error", "kernel error unavailable"),
        ("ntp.kernel_estimated_error", "kernel error unavailable"),
    ]);
    observed
}

#[cfg(windows)]
pub(super) fn collect() -> Observation {
    let value = script(READ)
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or(Value::Null);
    let mut observed = decode(&value);
    observed.user_region = geo_name();
    observed.timezone = value["zone"].as_str().and_then(|zone| icu_zone(zone, true));
    observed.env = super::command::process_environment();
    observed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_reads_preserve_unknowns_and_do_not_infer_sync_from_a_running_service() {
        let read = decode(
            &serde_json::json!({"policy":false,"ntp_enabled":1,"time_start":3,"type":"NT5DS","ntp":"example.org,0x9","service":true,"languages":["en-US","zh-CN"]}),
        );
        assert_eq!(read.automatic_time, Some(true));
        assert_eq!(read.time_synchronized, None);
        assert_eq!(read.ntp_server, None);
        assert_eq!(read.languages.unwrap(), ["en-US", "zh-CN"]);
        assert!(decode(&Value::Null).automatic_time.is_none());
        let read = decode(
            &serde_json::json!({"policy":false,"ntp_enabled":1,"time_start":3,"type":"NTP","ntp":"Example.ORG.,0x9"}),
        );
        assert_eq!(read.ntp_server.as_deref(), Some("example.org"));
        for policy in [Value::Null, Value::Bool(true)] {
            let read = decode(
                &serde_json::json!({"policy":policy,"type":"NTP","ntp_enabled":1,"time_start":3,"ntp":"example.org"}),
            );
            assert!(read.automatic_time.is_none());
            assert!(read.ntp_server.is_none());
        }
        assert_eq!(
            decode(
                &serde_json::json!({"policy":false,"type":"NTP","ntp_enabled":0,"time_start":3})
            )
            .automatic_time,
            Some(false)
        );
    }
    #[cfg(windows)]
    #[test]
    fn installed_windows_icu_maps_both_builtin_timezones() {
        assert_eq!(
            zone_id("America/Los_Angeles").as_deref(),
            Some("Pacific Standard Time")
        );
        assert_eq!(
            zone_id("Asia/Singapore").as_deref(),
            Some("Singapore Standard Time")
        );
        assert_eq!(
            icu_zone("Pacific Standard Time", true).as_deref(),
            Some("America/Los_Angeles")
        );
        assert!(zone_id("Invalid/Zone").is_none());
    }
}
