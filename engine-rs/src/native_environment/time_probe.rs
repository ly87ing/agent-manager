use anyhow::{bail, Result};
use serde::Serialize;
use std::net::IpAddr;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const EPOCH: f64 = 2_208_988_800.0;
static LAST_QUERY: Mutex<Option<Instant>> = Mutex::new(None);

pub(super) fn wait_for_interval() {
    let remaining = LAST_QUERY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .map(|time| Duration::from_secs(4).saturating_sub(time.elapsed()))
        .unwrap_or_default();
    std::thread::sleep(remaining);
}

pub(super) fn mark_query() {
    *LAST_QUERY.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}

#[derive(Serialize)]
pub(super) struct Sample {
    pub server: String,
    pub peer_address: String,
    pub special_use_address: bool,
    pub stratum: u8,
    pub offset_ms: f64,
    pub round_trip_ms: f64,
    pub uncertainty_ms: f64,
    #[cfg(any(target_os = "linux", windows))]
    #[serde(skip)]
    pub captured: Instant,
    #[cfg(any(target_os = "linux", windows))]
    #[serde(skip)]
    pub captured_wall: f64,
}

pub(super) fn valid_host(host: &str) -> bool {
    if host.parse::<IpAddr>().is_ok() {
        return true;
    }
    !host.is_empty()
        && host.len() <= 253
        && host.trim_end_matches('.').split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}

pub(super) fn query(host: &str) -> Result<Sample> {
    if !valid_host(host) {
        bail!("Invalid time server.");
    }
    {
        let mut last = LAST_QUERY.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|time| time.elapsed() < Duration::from_secs(4)) {
            bail!("Wait four seconds before querying again.");
        }
        *last = Some(Instant::now());
    }
    let address = if let Ok(ip) = host.parse::<IpAddr>() {
        ip
    } else {
        #[cfg(target_os = "macos")]
        let output =
            super::macos::command("/usr/bin/dscacheutil", &["-q", "host", "-a", "name", host])
                .ok_or_else(|| anyhow::anyhow!("Name lookup did not finish."))?;
        #[cfg(target_os = "macos")]
        let address = output
            .lines()
            .find_map(|line| {
                line.strip_prefix("ip_address: ")
                    .or_else(|| line.strip_prefix("ipv6_address: "))
                    .and_then(|value| value.parse().ok())
            })
            .ok_or_else(|| anyhow::anyhow!("No time-server address was found."))?;
        #[cfg(target_os = "linux")]
        let address =
            super::command::bounded("/usr/bin/getent", &["ahosts", host], Duration::from_secs(3))
                .and_then(|text| {
                    text.lines()
                        .find_map(|line| line.split_whitespace().next()?.parse::<IpAddr>().ok())
                })
                .ok_or_else(|| anyhow::anyhow!("No time-server address was found."))?;
        #[cfg(target_os = "windows")]
        let address = super::windows::resolve(host)?;
        address
    };
    query_address(host, SocketAddr::new(address, 123), Duration::from_secs(3))
}

fn seconds() -> Result<f64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}

fn timestamp(value: f64) -> [u8; 8] {
    let ntp = value + EPOCH;
    let mut out = [0; 8];
    out[..4].copy_from_slice(&(ntp.floor() as u64 as u32).to_be_bytes());
    out[4..].copy_from_slice(&((ntp.fract() * 4_294_967_296.0) as u32).to_be_bytes());
    out
}

fn decode(bytes: &[u8], reference: f64) -> f64 {
    let seconds = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as f64;
    let fraction = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as f64 / 4_294_967_296.0;
    let era = ((reference + EPOCH - seconds) / 4_294_967_296.0).round();
    seconds + era * 4_294_967_296.0 + fraction - EPOCH
}

fn query_address(host: &str, address: SocketAddr, timeout: Duration) -> Result<Sample> {
    let socket = UdpSocket::bind(if address.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })?;
    socket.connect(address)?;
    socket.set_read_timeout(Some(timeout))?;
    socket.set_write_timeout(Some(timeout))?;
    let sent_at = seconds()?;
    let start = Instant::now();
    let mut request = [0u8; 48];
    request[0] = 0x23; // NTPv4 client mode.
    request[40..48].copy_from_slice(&timestamp(sent_at));
    socket.send(&request)?;
    let mut response = [0u8; 512];
    let length = socket.recv(&mut response)?;
    let received_at = seconds()?;
    #[cfg(any(target_os = "linux", windows))]
    let captured = Instant::now();
    if (received_at - sent_at - start.elapsed().as_secs_f64()).abs() > 0.1 {
        bail!("The local clock changed during the query.");
    }
    if length < 48
        || response[0] & 7 != 4
        || !matches!((response[0] >> 3) & 7, 3 | 4)
        || response[0] >> 6 == 3
        || !(1..=15).contains(&response[1])
        || response[24..32] != request[40..48]
        || response[32..40] == [0; 8]
        || response[40..48] == [0; 8]
    {
        bail!("The time server did not return a usable matching response.");
    }
    let server_received = decode(&response[32..40], sent_at);
    let server_sent = decode(&response[40..48], sent_at);
    let round_trip = (received_at - sent_at) - (server_sent - server_received);
    if round_trip < -0.001 || server_sent < server_received {
        bail!("Invalid time-server timestamps.");
    }
    let root_delay = i32::from_be_bytes(response[4..8].try_into().unwrap()) as f64 / 65536.0;
    let dispersion = u32::from_be_bytes(response[8..12].try_into().unwrap()) as f64 / 65536.0;
    let precision = 2_f64.powi(response[3] as i8 as i32);
    let uncertainty =
        (round_trip.max(0.0) + root_delay.max(0.0)) / 2.0 + dispersion + precision + 0.001;
    let special_use_address = match address.ip() {
        IpAddr::V4(ip) => {
            let bytes = ip.octets();
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || (bytes[0] == 198 && matches!(bytes[1], 18 | 19))
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.segments()[0] & 0xfe00 == 0xfc00
                || ip.segments()[0] & 0xffc0 == 0xfe80
        }
    };
    Ok(Sample {
        server: host.into(),
        peer_address: address.ip().to_string(),
        special_use_address,
        stratum: response[1],
        offset_ms: ((server_received - sent_at) + (server_sent - received_at)) * 500.0,
        round_trip_ms: round_trip.max(0.0) * 1000.0,
        uncertainty_ms: uncertainty * 1000.0,
        #[cfg(any(target_os = "linux", windows))]
        captured,
        #[cfg(any(target_os = "linux", windows))]
        captured_wall: received_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_reply_identity_and_handles_timestamp_eras() {
        for value in [1_800_000_000.25, 2_300_000_000.75] {
            assert!((decode(&timestamp(value), value) - value).abs() < 0.00001);
        }
        for defect in [
            "none",
            "origin",
            "short",
            "unsynchronized",
            "refused",
            "transmit",
        ] {
            let server = UdpSocket::bind("127.0.0.1:0").unwrap();
            server
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let address = server.local_addr().unwrap();
            let worker = std::thread::spawn(move || {
                let mut request = [0; 48];
                let (_, client) = server.recv_from(&mut request).unwrap();
                let mut response = [0; 48];
                response[0] = 0x24;
                response[1] = 1;
                response[3] = (-20_i8) as u8;
                if defect != "origin" {
                    response[24..32].copy_from_slice(&request[40..48]);
                }
                response[32..40].copy_from_slice(&timestamp(seconds().unwrap() + 0.25));
                response[40..48].copy_from_slice(&timestamp(seconds().unwrap() + 0.25));
                if defect == "unsynchronized" {
                    response[0] |= 0xc0;
                }
                if defect == "refused" {
                    response[1] = 0;
                }
                if defect == "transmit" {
                    response[40..48].fill(0);
                }
                server
                    .send_to(
                        if defect == "short" {
                            &response[..16]
                        } else {
                            &response
                        },
                        client,
                    )
                    .unwrap();
            });
            let result = query_address("fixture", address, Duration::from_secs(1));
            worker.join().unwrap();
            if defect == "none" {
                assert!((result.unwrap().offset_ms - 250.0).abs() < 100.0);
            } else {
                assert!(result.is_err());
            }
        }
    }

    #[test]
    fn times_out_and_rejects_non_host_inputs() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        assert!(query_address(
            "fixture",
            server.local_addr().unwrap(),
            Duration::from_millis(30)
        )
        .is_err());
        for host in ["https://example.com", "-server", "server:123", "x\n", ""] {
            assert!(!valid_host(host));
        }
    }
}
