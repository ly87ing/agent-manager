//! Bounded, connection-close HTTP for the ephemeral loopback diagnostic origin.
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub(super) struct Request {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub(super) fn read(stream: &mut TcpStream, session_deadline: Instant) -> Result<Request> {
    // Darwin accepted sockets inherit the listener's nonblocking mode.
    stream.set_nonblocking(false)?;
    let deadline = session_deadline.min(Instant::now() + Duration::from_secs(2));
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    let (header_end, mut request, length) = loop {
        if Instant::now() >= deadline || bytes.len() > 32 * 1024 {
            bail!("Request limit reached.");
        }
        stream.set_read_timeout(Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        ))?;
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            bail!("Incomplete request.");
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            if end > 16 * 1024 {
                bail!("Request headers too large.");
            }
            let text = std::str::from_utf8(&bytes[..end])?;
            let mut lines = text.split("\r\n");
            let parts: Vec<_> = lines.next().unwrap_or("").split(' ').collect();
            if parts.len() != 3 || parts[2] != "HTTP/1.1" || !parts[1].starts_with('/') {
                bail!("Invalid request line.");
            }
            let mut headers = BTreeMap::new();
            for line in lines {
                let (key, value) = line
                    .split_once(':')
                    .ok_or_else(|| anyhow::anyhow!("Invalid header."))?;
                if key.is_empty()
                    || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || value.trim().chars().any(char::is_control)
                    || headers
                        .insert(key.to_ascii_lowercase(), value.trim().to_string())
                        .is_some()
                {
                    bail!("Invalid or duplicate header.");
                }
            }
            if headers.contains_key("transfer-encoding") {
                bail!("Transfer encoding is unsupported.");
            }
            let length = headers
                .get("content-length")
                .map(|v| v.parse::<usize>())
                .transpose()?
                .unwrap_or(0);
            if length > 16 * 1024 || (parts[0] == "POST" && !headers.contains_key("content-length"))
            {
                bail!("Invalid body length.");
            }
            break (
                end + 4,
                Request {
                    method: parts[0].into(),
                    path: parts[1].into(),
                    headers,
                    body: vec![],
                },
                length,
            );
        }
    };
    while bytes.len() < header_end + length {
        if Instant::now() >= deadline {
            bail!("Request expired.");
        }
        stream.set_read_timeout(Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        ))?;
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            bail!("Incomplete body.");
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    if bytes.len() != header_end + length {
        bail!("Unexpected trailing bytes.");
    }
    request.body = bytes[header_end..].to_vec();
    Ok(request)
}

pub(super) fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Forbidden",
    };
    write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'\r\nAccept-CH: Sec-CH-UA-Full-Version-List, Sec-CH-UA-Platform-Version, Sec-CH-UA-Arch, Sec-CH-UA-Bitness\r\n\r\n{body}", body.len())?;
    Ok(())
}
