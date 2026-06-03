use super::*;

#[test]
fn execute_user_prompt_streaming_parses_headerless_sse() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_log = Arc::clone(&requests);

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 8192];
        let bytes = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
        request_log.lock().unwrap().push(request);

        let body = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"headerless \"}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n",
            "event: response.output_item.done\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"headerless ok\"}]}}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}}\n\n"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let mut registry = ProviderRegistry::new();
    registry.register(openai_provider(format!("http://{address}")));
    let mut auth_store = AuthStore::default();
    auth_store.set_api_key("openai", "sk-openai");
    let mut state = state();
    state.current_provider = Some("openai".to_string());
    state.current_model = Some("openai/gpt-5".to_string());

    let mut deltas = Vec::new();
    let turn = execute_user_prompt_streaming(
        &mut state,
        &LoadedResources::default(),
        &registry,
        &mut auth_store,
        "hello",
        |event| {
            if let TurnStreamEvent::TextDelta(delta) = event {
                deltas.push(delta);
            }
        },
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(turn.assistant_text, "headerless ok");
    assert_eq!(deltas, vec!["headerless ".to_string(), "ok".to_string()]);

    let requests = requests.lock().unwrap();
    let request = requests[0].to_ascii_lowercase();
    assert!(request.contains("accept: text/event-stream"));
    assert!(request.contains("authorization: bearer sk-openai"));
}

#[test]
fn execute_user_prompt_streaming_retries_truncated_responses_sse() {
    let _guard = env_lock();
    std::env::set_var("PUFFER_OPENAI_STREAM_MAX_ATTEMPTS", "2");
    std::env::set_var("PUFFER_OPENAI_STREAM_RETRY_DELAY_MS", "0");
    std::env::set_var("PUFFER_OPENAI_HTTP_MAX_ATTEMPTS", "1");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_log = Arc::clone(&requests);

    let server = thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut accepted = 0;
        while accepted < 2 && std::time::Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("accept failed: {error}"),
            };
            accepted += 1;
            stream.set_nonblocking(false).unwrap();
            let mut buffer = [0_u8; 8192];
            let bytes = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
            request_log.lock().unwrap().push(request);

            let body = if accepted == 1 {
                concat!(
                    "event: response.created\n",
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_truncated\"}}\n\n",
                    "event: response.output_text.delta\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n"
                )
            } else {
                concat!(
                    "event: response.created\n",
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_ok\"}}\n\n",
                    "event: response.output_text.delta\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"pong\"}\n\n",
                    "event: response.completed\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_ok\",\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n"
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });

    let mut registry = ProviderRegistry::new();
    registry.register(openai_provider(format!("http://{address}")));
    let mut auth_store = AuthStore::default();
    auth_store.set_api_key("openai", "sk-openai");
    let mut state = state();
    state.current_provider = Some("openai".to_string());
    state.current_model = Some("openai/gpt-5".to_string());

    let mut events = Vec::new();
    let result = execute_user_prompt_streaming(
        &mut state,
        &LoadedResources::default(),
        &registry,
        &mut auth_store,
        "hello",
        |event| events.push(event),
    );
    server.join().unwrap();
    std::env::remove_var("PUFFER_OPENAI_STREAM_MAX_ATTEMPTS");
    std::env::remove_var("PUFFER_OPENAI_STREAM_RETRY_DELAY_MS");
    std::env::remove_var("PUFFER_OPENAI_HTTP_MAX_ATTEMPTS");

    let turn = result.unwrap();
    assert_eq!(turn.assistant_text, "pong");
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert!(events.iter().any(|event| matches!(
        event,
        TurnStreamEvent::RetryAttempt {
            attempt: 1,
            max_attempts: 2,
            ..
        }
    )));
}
