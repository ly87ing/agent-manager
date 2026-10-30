use super::*;

fn sample() -> Sample {
    serde_json::from_value(json!({"origin":"http://127.0.0.1:1234", "timezone":"America/Los_Angeles", "languages":["en-US","en"],
        "intl_locale":"en-US", "user_agent":"Chrome fixture", "platform":"MacIntel",
        "client_hints":{"brands":[{"brand":"Google Chrome","version":"130"},{"brand":"Not A Brand","version":"99"}],"mobile":false,"platform":"macOS",
            "architecture":"arm","bitness":"64","platform_version":"15.0.0","full_version_list":null},
        "screen":{"width":1440,"height":900,"available_width":1440,"available_height":875,"inner_width":1000,"inner_height":650,"outer_width":1000,"outer_height":750,"device_pixel_ratio":2.0,"visual_scale":1.0,"color_depth":24}})).unwrap()
}

fn headers() -> BTreeMap<String, String> {
    [
        ("user-agent", "Chrome fixture"),
        ("accept-language", "en-US,en;q=0.9"),
        (
            "sec-ch-ua",
            "\"Not A Brand\";v=\"99\", \"Google Chrome\";v=\"130\"",
        ),
        ("sec-ch-ua-platform", "\"macOS\""),
        ("sec-ch-ua-mobile", "?0"),
        ("sec-ch-ua-arch", "\"arm\""),
        ("sec-ch-ua-bitness", "\"64\""),
        ("sec-ch-ua-platform-version", "\"15.0.0\""),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}

#[test]
fn compares_real_browser_criteria_and_http_without_requiring_equal_brand_order() {
    let mut target = Target {
        timezone: Some("America/Los_Angeles".into()),
        languages: vec!["en-us".into(), "en".into()],
        ..Default::default()
    };
    let observed = sample();
    let rows = evaluation::evaluate(&target, Some(&observed), &headers());
    for id in [
        "browser.timezone",
        "browser.languages",
        "browser.ua_http",
        "browser.language_http",
        "browser.ch_brands",
        "browser.ch_platform",
        "browser.ch_mobile",
        "browser.ch_architecture",
    ] {
        assert_eq!(
            rows.iter().find(|r| r.id == id).unwrap().state,
            "match",
            "{id}"
        );
    }
    assert_eq!(
        rows.iter()
            .find(|r| r.id == "browser.ch_full_versions")
            .unwrap()
            .state,
        "unknown"
    );
    target.timezone = Some("Asia/Singapore".into());
    target.languages = vec!["en-SG".into()];
    let rows = evaluation::evaluate(&target, Some(&observed), &headers());
    assert_eq!(rows[0].state, "mismatch");
    assert_eq!(rows[1].state, "mismatch");
    let mut changed = headers();
    changed.insert("sec-ch-ua-platform".into(), "\"Windows\"".into());
    let rows = evaluation::evaluate(&target, Some(&observed), &changed);
    assert_eq!(
        rows.iter()
            .find(|r| r.id == "browser.ch_platform")
            .unwrap()
            .state,
        "mismatch"
    );
}

#[test]
fn language_quality_and_empty_hint_values_do_not_produce_false_matches() {
    let mut observed = sample();
    let mut http = headers();
    http.insert("accept-language".into(), "en-US;q=0,de-DE;q=1".into());
    let rows = evaluation::evaluate(&Target::default(), Some(&observed), &http);
    assert_eq!(
        rows.iter()
            .find(|r| r.id == "browser.language_http")
            .unwrap()
            .observed
            .as_deref(),
        Some("de-de")
    );
    assert_eq!(
        rows.iter()
            .find(|r| r.id == "browser.language_http")
            .unwrap()
            .state,
        "mismatch"
    );
    http.insert("accept-language".into(), "*;q=1,en-US;q=0.5".into());
    let hints = observed.client_hints.as_mut().unwrap();
    hints.platform.clear();
    hints.brands.clear();
    hints.full_version_list = Some(vec![]);
    http.insert(
        "sec-ch-ua-full-version-list".into(),
        "\"Google Chrome\";v=\"130.0.0.0\"".into(),
    );
    let rows = evaluation::evaluate(&Target::default(), Some(&observed), &http);
    for id in [
        "browser.language_http",
        "browser.ch_platform",
        "browser.ch_brands",
        "browser.ch_full_versions",
        "browser.ua_version",
    ] {
        assert_eq!(
            rows.iter().find(|r| r.id == id).unwrap().state,
            "unknown",
            "{id}"
        );
    }
}

#[test]
fn unavailable_hidden_and_malformed_evidence_never_counts_as_a_match() {
    let target = Target {
        timezone: Some("Asia/Singapore".into()),
        ..Default::default()
    };
    let report = outcome(&target, "timeout", None, "fixture", None, &headers());
    assert_eq!(report["summary"]["matched"], 0);
    assert_eq!(report["summary"]["status"], "incomplete");
    assert!(!report["not_checked"].as_array().unwrap().is_empty());
    let mut observed = sample();
    observed.user_agent = "Chrome/130.0.0.0".into();
    observed.client_hints = None;
    let rows = evaluation::evaluate(&target, Some(&observed), &headers());
    assert!(rows
        .iter()
        .filter(|r| r.id.starts_with("browser.ch_"))
        .all(|r| r.state == "unknown"));
    assert_eq!(
        rows.iter()
            .find(|r| r.id == "browser.ua_version")
            .unwrap()
            .state,
        "unknown"
    );
    assert!(observed.validate("http://127.0.0.1:9999").is_err());
    observed.screen.device_pixel_ratio = f64::NAN;
    assert!(observed.validate(&observed.origin).is_err());
}

#[cfg(unix)]
#[test]
fn http_reader_waits_for_request_bytes_and_rejects_ambiguous_or_unbounded_frames() {
    use std::io::Write;
    let read = |frame: Vec<u8>| {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        let sender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            let _ = client.write_all(&frame);
        });
        let result = http::read(&mut stream, Instant::now() + Duration::from_millis(500));
        sender.join().unwrap();
        result
    };
    assert!(read(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".to_vec()).is_ok());
    for headers in [
        "Host: one\r\nHost: two\r\n",
        "Content-Length: 0\r\nContent-Length: 1\r\n",
        "Transfer-Encoding: chunked\r\n",
        "Content-Length: 20000\r\n",
    ] {
        assert!(read(format!("POST / HTTP/1.1\r\n{headers}\r\n").into_bytes()).is_err());
    }
    assert!(
        read(format!("GET / HTTP/1.1\r\nX-Large: {}\r\n\r\n", "x".repeat(17000)).into_bytes())
            .is_err()
    );
}

#[cfg(unix)]
fn request(url: &str, method: &str, suffix: &str, extra: &str, body: &str) -> String {
    use std::io::{Read, Write};
    let (host, path) = url
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap();
    let mut stream = std::net::TcpStream::connect(host).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(stream, "{method} /{path}{suffix} HTTP/1.1\r\nHost: {host}\r\nSec-Fetch-Site: same-origin\r\n{extra}Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[cfg(unix)]
#[test]
fn local_protocol_refuses_cross_origin_and_replay_and_preserves_only_allowlisted_headers() {
    let target = Target::default();
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        collect(&target, Duration::from_secs(5), |url| {
            send.send(url.to_string()).unwrap();
            Ok(())
        })
        .unwrap()
    });
    let url = receive.recv().unwrap();
    let (host, _) = url
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap();
    let origin = format!("http://{host}");
    assert!(request(&url, "GET", "", "", "").contains("Cache-Control: no-store"));
    assert!(request(
        &url,
        "POST",
        "result",
        "Origin: https://example.invalid\r\nContent-Type: application/json\r\n",
        "{}"
    )
    .starts_with("HTTP/1.1 403"));
    assert!(request(&url, "GET", "headers", "Cookie: YOUR_PRIVATE_COOKIE\r\nAuthorization: YOUR_PRIVATE_AUTH\r\nUser-Agent: Chrome fixture\r\n", "").starts_with("HTTP/1.1 200"));
    assert!(request(
        &url,
        "POST",
        "result",
        &format!("Origin: {origin}\r\nContent-Type: application/json\r\n"),
        "{}"
    )
    .starts_with("HTTP/1.1 400"));
    let mut observed = sample();
    observed.origin = origin.clone();
    let body = serde_json::to_string(&observed).unwrap();
    let response = request(
        &url,
        "POST",
        "result",
        &format!("Origin: {origin}\r\nContent-Type: application/json\r\n"),
        &body,
    );
    assert!(response.starts_with("HTTP/1.1 200"));
    let result = worker.join().unwrap();
    assert_eq!(result["status"], "received");
    assert_eq!(result["headers"].as_object().unwrap().len(), 1);
    assert!(!result.to_string().contains("YOUR_PRIVATE"));
    assert!(!result.to_string().contains(url.split('/').nth(3).unwrap()));
    assert!(
        std::net::TcpStream::connect(host).is_err(),
        "Listener must close after completion"
    );
}

#[cfg(unix)]
#[test]
fn cancellation_timeout_and_launch_failure_return_unknown_and_close_the_listener() {
    let target = Target::default();
    assert_eq!(
        collect(&target, Duration::from_millis(10), |_| Ok(())).unwrap()["status"],
        "timeout"
    );
    assert_eq!(
        collect(&target, Duration::from_secs(1), |_| bail!(
            "fixture failure"
        ))
        .unwrap()["status"],
        "unavailable"
    );
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        collect(&target, Duration::from_secs(3), |url| {
            send.send(url.to_string()).unwrap();
            Ok(())
        })
        .unwrap()
    });
    let url = receive.recv().unwrap();
    let host = url
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    assert!(request(
        &url,
        "POST",
        "cancel",
        &format!("Origin: http://{host}\r\nContent-Type: application/json\r\n"),
        "{}"
    )
    .starts_with("HTTP/1.1 200"));
    let result = worker.join().unwrap();
    assert_eq!(result["status"], "cancelled");
    assert_eq!(result["summary"]["matched"], 0);
    assert!(std::net::TcpStream::connect(host).is_err());
}
