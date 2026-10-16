//! Read manual desktop proxy settings without loading proxy credentials or PAC.
use super::{bypass_hosts, SystemProxy};

fn quoted(value: &str) -> Option<String> {
    let text = value.strip_prefix('\'')?.strip_suffix('\'')?;
    (!text.contains(['\\', '\'', '"']) && !text.chars().any(char::is_control))
        .then(|| text.to_string())
}

fn endpoint(host: &str, port: &str, scheme: &str) -> Option<Option<String>> {
    if host.is_empty() || port == "0" {
        return Some(None);
    }
    let port = port.parse::<u16>().ok().filter(|port| *port != 0)?;
    let host = if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else if host
        .bytes()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'-'))
    {
        host.into()
    } else {
        return None;
    };
    Some(Some(format!("{scheme}://{host}:{port}")))
}

fn gnome(mut get: impl FnMut(&str, &str) -> Option<String>) -> Option<SystemProxy> {
    if quoted(&get("", "mode")?)? != "manual" || get(".http", "use-authentication")? != "false" {
        return None;
    }
    let ignore = get("", "ignore-hosts")?;
    let ignore = ignore
        .strip_prefix("@as ")
        .unwrap_or(&ignore)
        .trim()
        .strip_prefix('[')?
        .strip_suffix(']')?;
    let hosts = ignore
        .split(',')
        .filter(|host| !host.trim().is_empty())
        .map(|host| quoted(host.trim()))
        .collect::<Option<Vec<_>>>()?;
    let bypass = bypass_hosts(hosts.iter().map(String::as_str))?;
    let mut proxy =
        |kind, scheme| endpoint(&quoted(&get(kind, "host")?)?, &get(kind, "port")?, scheme);
    let http = proxy(".http", "http")?;
    let https = proxy(".https", "http")?.or_else(|| http.clone());
    let socks = proxy(".socks", "socks5h")?;
    let default = match get("", "use-same-proxy")?.as_str() {
        "true" => http.clone(),
        "false" => None,
        _ => return None,
    };
    Some(SystemProxy {
        http,
        https,
        socks: socks.or(default),
        bypass,
    })
}

fn kde(mut get: impl FnMut(&str, &str) -> Option<String>) -> Option<SystemProxy> {
    if get("ProxyType", "0")? != "1" || get("ReversedException", "false")? != "false" {
        return None;
    }
    let bypass = get("NoProxyFor", "")?;
    let bypass = bypass_hosts(bypass.split(','))?;
    let mut proxy = |key, scheme: &str| -> Option<Option<String>> {
        let value = get(key, "")?;
        if value.is_empty() {
            return Some(None);
        }
        let value = value
            .strip_prefix(&format!("{scheme}://"))
            .or_else(|| {
                (scheme == "socks5")
                    .then(|| value.strip_prefix("socks://"))
                    .flatten()
            })
            .unwrap_or(&value);
        // KConfig historically stores "http://host port"; newer writers use host:port.
        let (host, port) = value.rsplit_once(' ').or_else(|| value.rsplit_once(':'))?;
        endpoint(host.trim_matches(['[', ']']), port, scheme)
    };
    Some(SystemProxy {
        http: proxy("httpProxy", "http")?,
        https: proxy("httpsProxy", "http")?,
        socks: proxy("socksProxy", "socks5")?,
        bypass,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn read() -> SystemProxy {
    use std::{path::Path, process::Stdio, time::Duration};
    let started = std::time::Instant::now();
    let read = |program: &str, args: &[&str]| {
        let remaining = Duration::from_secs(2).checked_sub(started.elapsed())?;
        let mut command = super::launch_command(program);
        if program == "/usr/bin/gsettings" {
            super::deny_settings_file_writes(&mut command);
        }
        command
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output = super::capture_group(
            &mut command,
            remaining.min(Duration::from_secs(1)),
            16 * 1024,
        )
        .ok()?;
        (output.status.success() && output.stdout.len() < 16 * 1024)
            .then(|| {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|text| text.trim().to_string())
            })
            .flatten()
    };
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if desktop
        .split(':')
        .any(|part| part == "gnome" || part.ends_with("-gnome"))
    {
        return gnome(|suffix, key| {
            read(
                "/usr/bin/gsettings",
                &["get", &format!("org.gnome.system.proxy{suffix}"), key],
            )
        })
        .unwrap_or_default();
    }
    if desktop.split(':').any(|part| part == "kde") {
        if let Some(program) = ["/usr/bin/kreadconfig6", "/usr/bin/kreadconfig5"]
            .into_iter()
            .find(|path| Path::new(path).is_file())
        {
            return kde(|key, default| {
                read(
                    program,
                    &[
                        "--file",
                        "kioslaverc",
                        "--group",
                        "Proxy Settings",
                        "--key",
                        key,
                        "--default",
                        default,
                    ],
                )
            })
            .unwrap_or_default();
        }
    }
    SystemProxy::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gnome_manual_settings_keep_protocol_precedence_and_fail_closed() {
        let get = |suffix: &str, key: &str| {
            Some(
                match (suffix, key) {
                    ("", "mode") => "'manual'",
                    ("", "ignore-hosts") => "['localhost', '*.example.invalid']",
                    ("", "use-same-proxy") => "true",
                    (".http", "use-authentication") => "false",
                    (".http", "host") => "'localhost'",
                    (".http", "port") => "8080",
                    (".https", "host") => "''",
                    (".socks", "host") => "'::1'",
                    (_, "port") => "1080",
                    _ => panic!("unexpected key"),
                }
                .to_string(),
            )
        };
        let proxy = gnome(get).unwrap();
        assert_eq!(proxy.http.as_deref(), Some("http://localhost:8080"));
        assert_eq!(proxy.https, proxy.http);
        assert_eq!(proxy.socks.as_deref(), Some("socks5h://[::1]:1080"));
        assert_eq!(proxy.bypass.as_deref(), Some("localhost,.example.invalid"));
        for (suffix, key, value) in [
            ("", "mode", "'auto'"),
            (".http", "use-authentication", "true"),
            ("", "ignore-hosts", "['<local>']"),
            (".http", "host", "'user@host'"),
            (".http", "port", "99999"),
        ] {
            assert!(gnome(|s, k| if s == suffix && k == key {
                Some(value.into())
            } else {
                get(s, k)
            })
            .is_none());
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn native_gsettings_reads_isolated_schemas_without_creating_home() {
        const CHILD: &str = "AM_PROXY_NATIVE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let proxy = read();
            assert_eq!(proxy.http.as_deref(), Some("http://localhost:8080"));
            assert_eq!(proxy.https, proxy.http);
            assert_eq!(proxy.bypass.as_deref(), Some("localhost,.example.invalid"));
            assert!(!std::path::Path::new(&std::env::var("HOME").unwrap()).exists());
            assert!(!std::path::Path::new(&std::env::var("XDG_CONFIG_HOME").unwrap()).exists());
            return;
        }
        let root = std::env::temp_dir().join(format!("am-proxy-{}", crate::store::uuid_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut xml = String::from("<schemalist>");
        for (suffix, keys) in [
            (
                "",
                vec![
                    ("mode", "s", "'manual'"),
                    ("ignore-hosts", "as", "['localhost', '*.example.invalid']"),
                    ("use-same-proxy", "b", "false"),
                ],
            ),
            (
                ".http",
                vec![
                    ("host", "s", "'localhost'"),
                    ("port", "i", "8080"),
                    ("use-authentication", "b", "false"),
                ],
            ),
            (".https", vec![("host", "s", "''"), ("port", "i", "0")]),
            (".socks", vec![("host", "s", "''"), ("port", "i", "0")]),
        ] {
            xml.push_str(&format!(
                "<schema id=\"org.gnome.system.proxy{suffix}\" path=\"/am/proxy/{}/\">",
                if suffix.is_empty() {
                    "root".into()
                } else {
                    suffix.replace('.', "")
                }
            ));
            for (key, kind, value) in keys {
                xml.push_str(&format!(
                    "<key name=\"{key}\" type=\"{kind}\"><default>{value}</default></key>"
                ));
            }
            xml.push_str("</schema>");
        }
        xml.push_str("</schemalist>");
        std::fs::write(root.join("am.gschema.xml"), xml).unwrap();
        assert!(std::process::Command::new("/usr/bin/glib-compile-schemas")
            .arg(&root)
            .status()
            .unwrap()
            .success());
        let output = std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact", "process::linux_proxy::tests::native_gsettings_reads_isolated_schemas_without_creating_home", "--nocapture"])
            .env(CHILD, "1").env("GSETTINGS_SCHEMA_DIR", &root).env("GSETTINGS_BACKEND", "keyfile").env("XDG_CURRENT_DESKTOP", "GNOME").env("HOME", root.join("missing-home")).env("XDG_CONFIG_HOME", root.join("missing-config")).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn kde_manual_settings_reject_reverse_bypass_and_credentials() {
        let get = |key: &str, default: &str| {
            Some(
                match key {
                    "ProxyType" => "1",
                    "httpProxy" => "http://localhost 8080",
                    "NoProxyFor" => "localhost,*.example.invalid",
                    _ => default,
                }
                .to_string(),
            )
        };
        assert_eq!(
            kde(get).unwrap().http.as_deref(),
            Some("http://localhost:8080")
        );
        for (key, value) in [
            ("ProxyType", "2"),
            ("ReversedException", "true"),
            ("httpProxy", "http://user:password@localhost:8080"),
        ] {
            assert!(kde(|k, d| if k == key {
                Some(value.into())
            } else {
                get(k, d)
            })
            .is_none());
        }
    }
}
