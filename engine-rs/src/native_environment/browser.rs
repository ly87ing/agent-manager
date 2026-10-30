//! One explicitly requested, short-lived Chrome observation. No profile reads or writes.
mod evaluation;
mod http;

use super::Target;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Brand {
    brand: String,
    version: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClientHints {
    brands: Vec<Brand>,
    mobile: bool,
    platform: String,
    architecture: Option<String>,
    bitness: Option<String>,
    platform_version: Option<String>,
    full_version_list: Option<Vec<Brand>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Screen {
    width: u32,
    height: u32,
    available_width: u32,
    available_height: u32,
    inner_width: u32,
    inner_height: u32,
    outer_width: u32,
    outer_height: u32,
    device_pixel_ratio: f64,
    visual_scale: Option<f64>,
    color_depth: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Sample {
    origin: String,
    timezone: Option<String>,
    languages: Vec<String>,
    intl_locale: Option<String>,
    user_agent: String,
    platform: String,
    client_hints: Option<ClientHints>,
    screen: Screen,
}

impl Sample {
    fn validate(&self, origin: &str) -> Result<()> {
        let text = |s: &str| s.len() <= 1024 && !s.chars().any(char::is_control);
        if self.origin != origin
            || self.languages.is_empty()
            || self.languages.len() > 16
            || self.languages.iter().any(|s| s.is_empty() || !text(s))
            || !text(&self.user_agent)
            || !text(&self.platform)
            || self
                .timezone
                .iter()
                .chain(self.intl_locale.iter())
                .any(|s| !text(s))
            || !self.screen.device_pixel_ratio.is_finite()
            || self.screen.device_pixel_ratio <= 0.0
            || self.screen.device_pixel_ratio > 32.0
            || self
                .screen
                .visual_scale
                .is_some_and(|s| !s.is_finite() || s <= 0.0 || s > 32.0)
            || [
                self.screen.width,
                self.screen.height,
                self.screen.available_width,
                self.screen.available_height,
                self.screen.inner_width,
                self.screen.inner_height,
                self.screen.outer_width,
                self.screen.outer_height,
            ]
            .iter()
            .any(|v| *v > 100_000)
        {
            bail!("Invalid browser observation.");
        }
        if let Some(hints) = &self.client_hints {
            if !text(&hints.platform)
                || hints
                    .architecture
                    .iter()
                    .chain(hints.bitness.iter())
                    .chain(hints.platform_version.iter())
                    .any(|s| !text(s))
                || [
                    &hints.brands,
                    hints.full_version_list.as_ref().unwrap_or(&hints.brands),
                ]
                .iter()
                .any(|list| {
                    list.len() > 16 || list.iter().any(|b| !text(&b.brand) || !text(&b.version))
                })
            {
                bail!("Invalid browser client hints.");
            }
        }
        Ok(())
    }
}

fn outcome(
    target: &Target,
    status: &str,
    origin: Option<&str>,
    session: &str,
    sample: Option<&Sample>,
    headers: &BTreeMap<String, String>,
) -> Value {
    let checks = evaluation::evaluate(target, sample, headers);
    let now = chrono::Utc::now();
    json!({"schema_version":1, "status":status, "session_id":session,
        "checked_at":now.to_rfc3339(), "expires_at":(now + chrono::Duration::minutes(5)).to_rfc3339(),
        "origin":origin, "source":"explicit Chrome diagnostic page; same-origin HTTP request",
        "scope":"this diagnostic origin and browser context only; browser statements, not binary attestation",
        "sample":sample, "headers":headers, "checks":checks,
        "summary":super::summarize(&checks),
        "not_checked":["geoip","webrtc","canvas_webgl","fonts","clock_drift","tls_fingerprint"]})
}

pub(super) fn run(target: &Target) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        collect(target, Duration::from_secs(45), |url| {
            let mut command = crate::process::launch_command("/usr/bin/open");
            command
                .args(["-b", "com.google.Chrome", url])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let output = crate::process::capture_group(&mut command, Duration::from_secs(5), 1024)?;
            if !output.status.success() {
                bail!("Chrome could not open the local diagnostic page.");
            }
            Ok(())
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = target;
        bail!("Chrome inspection currently requires macOS.");
    }
}

#[cfg(any(target_os = "macos", test))]
fn collect(
    target: &Target,
    timeout: Duration,
    open: impl FnOnce(&str) -> Result<()>,
) -> Result<Value> {
    use std::io::Read;
    // A bearer URL requires OS entropy, never the general UUID clock fallback.
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let session = crate::native_mcp::model::sha256_hex(&token);
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let host = listener.local_addr()?.to_string();
    let origin = format!("http://{host}");
    let prefix = format!("/{token}/");
    let deadline = Instant::now() + timeout;
    if open(&format!("{origin}{prefix}")).is_err() {
        return Ok(outcome(
            target,
            "unavailable",
            None,
            &session,
            None,
            &BTreeMap::new(),
        ));
    }
    let mut headers = None;
    while Instant::now() < deadline {
        let (mut stream, _) = match listener.accept() {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(15));
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let request = match http::read(&mut stream, deadline) {
            Ok(request) if request.headers.get("host") == Some(&host) => request,
            _ => {
                let _ = http::respond(&mut stream, 400, "text/plain", "Invalid request");
                continue;
            }
        };
        let suffix = request.path.strip_prefix(&prefix);
        let same_origin =
            request.headers.get("sec-fetch-site").map(String::as_str) == Some("same-origin");
        match (request.method.as_str(), suffix) {
            ("GET", Some("")) => {
                http::respond(
                    &mut stream,
                    200,
                    "text/html; charset=utf-8",
                    include_str!("browser/page.html"),
                )?;
            }
            ("GET", Some("collect.js")) if same_origin => {
                http::respond(
                    &mut stream,
                    200,
                    "text/javascript; charset=utf-8",
                    include_str!("browser/collect.js"),
                )?;
            }
            ("GET", Some("headers")) if same_origin => {
                // Only retain allowlisted diagnostic headers, never cookies or authorization.
                headers = Some(
                    request
                        .headers
                        .into_iter()
                        .filter(|(key, _)| {
                            matches!(
                                key.as_str(),
                                "user-agent"
                                    | "accept-language"
                                    | "sec-ch-ua"
                                    | "sec-ch-ua-mobile"
                                    | "sec-ch-ua-platform"
                                    | "sec-ch-ua-full-version-list"
                                    | "sec-ch-ua-platform-version"
                                    | "sec-ch-ua-arch"
                                    | "sec-ch-ua-bitness"
                            )
                        })
                        .collect(),
                );
                http::respond(&mut stream, 200, "application/json", "{}")?;
            }
            ("POST", Some(action @ ("result" | "cancel")))
                if same_origin
                    && request.headers.get("origin") == Some(&origin)
                    && request.headers.get("content-type").map(String::as_str)
                        == Some("application/json") =>
            {
                if action == "cancel" {
                    http::respond(&mut stream, 200, "application/json", "{}")?;
                    return Ok(outcome(
                        target,
                        "cancelled",
                        Some(&origin),
                        &session,
                        None,
                        &BTreeMap::new(),
                    ));
                }
                let sample = serde_json::from_slice::<Sample>(&request.body)
                    .ok()
                    .filter(|s| s.validate(&origin).is_ok());
                if let (Some(sample), Some(headers)) = (sample, headers.as_ref()) {
                    if Instant::now() >= deadline {
                        break;
                    }
                    let result = outcome(
                        target,
                        "received",
                        Some(&origin),
                        &session,
                        Some(&sample),
                        headers,
                    );
                    http::respond(
                        &mut stream,
                        200,
                        "application/json",
                        &serde_json::to_string(&result)?,
                    )?;
                    return Ok(result);
                }
                http::respond(
                    &mut stream,
                    400,
                    "text/plain",
                    "Invalid or incomplete observation",
                )?;
            }
            _ => {
                let _ = http::respond(&mut stream, 403, "text/plain", "Request refused");
            }
        }
    }
    Ok(outcome(
        target,
        "timeout",
        Some(&origin),
        &session,
        None,
        &BTreeMap::new(),
    ))
}

#[cfg(test)]
mod tests;
