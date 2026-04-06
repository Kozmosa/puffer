use puffer_transport_anthropic::{
    build_messages_request, AnthropicAuth, AnthropicMessage, AnthropicModelRequest,
    AnthropicRequestConfig,
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn claude_bare_request_matches_expected_header_shape() {
    let Some(claude_bin) = which_claude() else {
        return;
    };

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
    let port = listener.local_addr().expect("local addr").port();
    let capture_thread = thread::spawn(move || capture_single_request(listener));

    let mut child = Command::new(claude_bin)
        .args(["--print", "--bare", "hello"])
        .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}"))
        .env("ANTHROPIC_API_KEY", "dummy")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn claude");

    let captured = match capture_thread.join().expect("join capture thread") {
        Ok(captured) => captured,
        Err(_) => {
            let _ = wait_for_exit_or_kill(&mut child, Duration::from_secs(1));
            return;
        }
    };
    let _ = wait_for_exit_or_kill(&mut child, Duration::from_secs(5));

    let header_names = captured
        .lines()
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':').map(|(name, _)| name.to_string()))
        .collect::<Vec<_>>();

    let session_id = header_value(&captured, "X-Claude-Code-Session-Id")
        .unwrap_or("session-test")
        .to_string();

    let built = build_messages_request(
        &AnthropicRequestConfig {
            base_url: format!("http://127.0.0.1:{port}"),
            session_id,
            custom_headers: Default::default(),
            remote_container_id: None,
            remote_session_id: None,
            client_app: None,
            entrypoint: "cli".to_string(),
            user_type: "external".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            workload: None,
            additional_protection: false,
            cch_enabled: true,
            auth: AnthropicAuth::ApiKey("dummy".to_string()),
            beta_header: None,
            client_request_id: None,
        },
        &AnthropicModelRequest {
            model: "claude-sonnet-4-5".to_string(),
            max_tokens: 1024,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
        },
    )
    .expect("build request");

    let expected_names = built
        .headers
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| header_names.iter().any(|captured| captured.eq_ignore_ascii_case(name)))
        .collect::<Vec<_>>();

    assert_starts_with(&header_names, &expected_names);
    let user_agent = header_value(&captured, "User-Agent").expect("user agent");
    assert!(user_agent.starts_with("claude-cli/"));
    assert!(user_agent.contains("(external, cli"));
    assert!(captured.starts_with("POST /v1/messages HTTP/1.1"));
}

fn capture_single_request(listener: TcpListener) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|err| err.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(10);

    loop {
        let (mut stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("timed out waiting for POST /v1/messages".to_string());
                }
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|err| err.to_string())?;

        let request = read_request(&mut stream)?;
        let request_line = request.lines().next().unwrap_or_default().to_string();

        if request_line.starts_with("HEAD / ") {
            write_simple_response(&mut stream, b"")?;
        } else if request_line.starts_with("POST /v1/messages ") {
            let body = br#"{"id":"msg_test","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}"#;
            write_json_response(&mut stream, body)?;
            return Ok(request);
        } else {
            write_simple_response(&mut stream, b"")?;
        }

    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Result<String, String> {
    let mut buffer = Vec::new();
    let mut temp = [0u8; 4096];
    let mut header_end = None;
    let mut content_length = 0usize;

    while header_end.is_none() {
        let read = stream.read(&mut temp).map_err(|err| err.to_string())?;
        if read == 0 {
            return Err("connection closed before headers".to_string());
        }
        buffer.extend_from_slice(&temp[..read]);
        if let Some(index) = find_header_end(&buffer) {
            header_end = Some(index);
            let headers = String::from_utf8_lossy(&buffer[..index]);
            content_length = parse_content_length(&headers).unwrap_or(0);
        }
    }

    let header_end = header_end.expect("header end");
    while buffer.len() < header_end + 4 + content_length {
        let read = stream.read(&mut temp).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&temp[..read]);
    }

    String::from_utf8(buffer).map_err(|err| err.to_string())
}

fn write_simple_response(stream: &mut std::net::TcpStream, body: &[u8]) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|err| err.to_string())
}

fn write_json_response(stream: &mut std::net::TcpStream, body: &[u8]) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|err| err.to_string())
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (header, value) = line.split_once(':')?;
        if header.eq_ignore_ascii_case(name) {
            Some(value.trim())
        } else {
            None
        }
    })
}

fn assert_starts_with(actual: &[String], expected: &[&str]) {
    for (index, expected_name) in expected.iter().enumerate() {
        assert_eq!(
            actual.get(index).map(String::as_str),
            Some(*expected_name),
            "header order mismatch at index {index}"
        );
    }
}

fn which_claude() -> Option<String> {
    let output = Command::new("which").arg("claude").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn wait_for_exit_or_kill(child: &mut std::process::Child, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                child.kill().map_err(|err| err.to_string())?;
                let _ = child.wait();
                return Ok(());
            }
            Err(err) => return Err(err.to_string()),
        }
    }
}
