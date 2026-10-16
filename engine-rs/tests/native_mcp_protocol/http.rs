//! Actual system curl and loopback HTTP acceptance; no external service is used.
use super::{database_bytes, Env, OTHER_TOKEN, TOKEN};
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Request {
    headers: Vec<(String, String)>,
    body: Value,
}

struct BoundedSocket {
    stream: std::net::TcpStream,
    stop: Arc<AtomicBool>,
    deadline: Instant,
}

impl BoundedSocket {
    fn remaining(&self) -> std::io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero() && !self.stop.load(Ordering::Relaxed))
            .map(|left| left.min(Duration::from_secs(2)))
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "HTTP fixture deadline")
            })
    }
}

impl Read for BoundedSocket {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(bytes)
    }
}

impl Write for BoundedSocket {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.remaining()?;
        self.stream.flush()
    }
}

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<Vec<Request>>>>,
}

impl Server {
    fn start(mode: &'static str, lifetime: Duration) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            let deadline = Instant::now() + lifetime;
            let expected = match mode {
                "invalid-init" => 1,
                "no-tools" => 2,
                _ => 3,
            };
            while requests.len() < expected && !stopped.load(Ordering::Relaxed) {
                ensure!(Instant::now() < deadline, "HTTP fixture accept deadline");
                let (stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                };
                let mut reader = BufReader::new(BoundedSocket {
                    stream,
                    stop: Arc::clone(&stopped),
                    deadline,
                });
                let mut headers = Vec::new();
                let mut length = None;
                {
                    let mut limited = (&mut reader).take(16 * 1024);
                    let mut request_line = String::new();
                    ensure!(limited.read_line(&mut request_line)? > 0);
                    ensure!(request_line.trim_end_matches(['\r', '\n']) == "POST /mcp HTTP/1.1");
                    loop {
                        let mut line = String::new();
                        ensure!(limited.read_line(&mut line)? > 0, "Incomplete HTTP headers");
                        let line = line.trim_end_matches(['\r', '\n']);
                        if line.is_empty() {
                            break;
                        }
                        let (name, value) = line
                            .split_once(':')
                            .ok_or_else(|| anyhow::anyhow!("Malformed HTTP fixture header"))?;
                        let name = name.to_ascii_lowercase();
                        let value = value.trim();
                        if name == "content-length" {
                            ensure!(length.is_none(), "Duplicate content length");
                            length = Some(value.trim().parse::<usize>()?);
                        }
                        headers.push((name, value.to_string()));
                    }
                }
                let length = length.ok_or_else(|| anyhow::anyhow!("Missing content length"))?;
                ensure!(length <= 16 * 1024, "Oversized HTTP fixture body");
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes)?;
                let body: Value = serde_json::from_slice(&bytes)?;
                let method = body["method"].as_str().unwrap_or_default();
                let (status, content_type, session, response) = match method {
                    "initialize" => {
                        let capabilities = if mode == "no-tools" {
                            json!({})
                        } else {
                            json!({"tools":{}})
                        };
                        let result = if mode == "invalid-init" {
                            json!({"capabilities":[],"serverInfo":{}})
                        } else {
                            json!({"protocolVersion":"2025-06-18","capabilities":capabilities,"serverInfo":{"name":"native-http","version":"1","echo":TOKEN}})
                        };
                        (
                            "200 OK",
                            "application/json",
                            "Mcp-Session-Id: native-session-42\r\n",
                            json!({"jsonrpc":"2.0","id":1,"result":result}).to_string(),
                        )
                    }
                    "notifications/initialized" => {
                        ("202 Accepted", "application/json", "", String::new())
                    }
                    "tools/list" => {
                        let response = match mode {
                            "tools-error" => json!({"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":format!("rejected {TOKEN}")}}),
                            "missing-tools" => json!({"jsonrpc":"2.0","id":2,"result":{}}),
                            "malformed-tools" => json!({"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"valid","inputSchema":{"type":"object"}},{"name":7,"inputSchema":{"type":"object"}}]}}),
                            "missing-result" => json!({"jsonrpc":"2.0","id":999,"result":{"tools":[]}}),
                            _ => json!({"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":format!("remote_{TOKEN}"),"inputSchema":{"type":"object"}}]}}),
                        }.to_string();
                        if mode == "http-error" {
                            (
                                "401 Unauthorized",
                                "text/plain",
                                "",
                                format!("rejected {TOKEN}"),
                            )
                        } else if mode == "sse" {
                            (
                                "200 OK",
                                "text/event-stream",
                                "",
                                format!("event: message\ndata: {response}\n\n"),
                            )
                        } else {
                            ("200 OK", "application/json", "", response)
                        }
                    }
                    _ => anyhow::bail!("Unexpected HTTP method: {method}"),
                };
                requests.push(Request { headers, body });
                let headers = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{session}Content-Length: {}\r\nConnection: close\r\n\r\n", response.len());
                let stream = reader.get_mut();
                stream.write_all(headers.as_bytes())?;
                stream.write_all(response.as_bytes())?;
                stream.flush()?;
            }
            Ok(requests)
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }

    fn receipts(mut self) -> Vec<Request> {
        self.thread.take().unwrap().join().unwrap().unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) fn run() {
    for mode in [
        "json",
        "sse",
        "no-tools",
        "invalid-init",
        "tools-error",
        "missing-tools",
        "malformed-tools",
        "missing-result",
        "http-error",
    ] {
        let env = Env::new(mode);
        env.config("ok", false);
        let server = Server::start(mode, Duration::from_secs(30));
        std::fs::write(env.home.join(".claude.json"), json!({"mcpServers":{"native":{"type":"http","url":server.url,"headers":{"Authorization":"Bearer ${AM_NATIVE_MCP_TOKEN}"},"startup_timeout_ms":10000,"tool_timeout_sec":2}}}).to_string()).unwrap();
        for name in [".curlrc", "_curlrc"] {
            std::fs::write(env.home.join(name), "header = \"X-Wrong-Account: YOUR_FAKE_CURLRC_TOKEN\"\nurl = \"http://127.0.0.1:1/never\"\n").unwrap();
        }
        let config = std::fs::read(env.home.join(".claude.json")).unwrap();
        let source = std::fs::read(env.home.join("secrets.env")).unwrap();
        let db = database_bytes(&env.data);
        let result = env.run(&[
            "mcp:check",
            "--only",
            "mcp:native",
            "--agent",
            "claude-code",
        ]);
        assert_eq!(result["ok"], true, "{result}");
        let data = &result["data"];
        let success = matches!(mode, "json" | "sse" | "no-tools");
        assert_eq!(data["ok"], success, "{result}");
        assert_eq!(
            data["stage"],
            if success {
                "done"
            } else if mode == "invalid-init" {
                "initialize"
            } else {
                "tools"
            },
            "{result}"
        );
        if success {
            assert_eq!(data["server_info"]["echo"], "***");
            assert_eq!(
                data["tools"],
                if mode == "no-tools" {
                    Value::Null
                } else {
                    json!(["remote_***"])
                }
            );
        } else {
            assert_eq!(data["tools"], Value::Null);
        }
        let requests = server.receipts();
        let expected = if mode == "invalid-init" {
            1
        } else if mode == "no-tools" {
            2
        } else {
            3
        };
        assert_eq!(requests.len(), expected, "{requests:?}");
        for (index, request) in requests.iter().enumerate() {
            let values = |name: &str| {
                request
                    .headers
                    .iter()
                    .filter(|(key, _)| key == name)
                    .map(|(_, value)| value.clone())
                    .collect::<Vec<_>>()
            };
            assert_eq!(values("authorization"), vec![format!("Bearer {TOKEN}")]);
            assert_eq!(
                values("mcp-protocol-version"),
                vec!["2025-06-18".to_string()]
            );
            assert!(request
                .headers
                .iter()
                .all(|(name, value)| name != "x-wrong-account"
                    && !value.contains("YOUR_FAKE_CURLRC_TOKEN")
                    && !value.contains(OTHER_TOKEN)));
            assert_eq!(
                values("mcp-session-id"),
                if index == 0 {
                    vec![]
                } else {
                    vec!["native-session-42".to_string()]
                }
            );
            assert!(!request.body.to_string().contains(OTHER_TOKEN));
            assert_eq!(request.body["jsonrpc"], "2.0");
            assert_eq!(
                request.body["method"],
                ["initialize", "notifications/initialized", "tools/list"][index]
            );
            if index == 1 {
                assert!(request.body.get("id").is_none());
            } else {
                assert_eq!(request.body["id"], if index == 0 { 1 } else { 2 });
            }
        }
        assert_eq!(
            std::fs::read(env.home.join(".claude.json")).unwrap(),
            config
        );
        assert_eq!(std::fs::read(env.home.join("secrets.env")).unwrap(), source);
        assert_eq!(database_bytes(&env.data), db);
        println!("native MCP HTTP {mode}: passed");
    }
}
