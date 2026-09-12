use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{McpTransport, sse::SseTransport, streamable_http::StreamableHttpTransport};
use crate::protocol::JsonRpcRequest;

async fn read_request(stream: &mut TcpStream) {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let len = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + len {
                return;
            }
        }
    }
}

async fn serve_once(response: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        read_request(&mut stream).await;
        stream.write_all(&response).await.unwrap();
    });
    (url, task)
}

fn json_response(body: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes()
}

#[tokio::test]
async fn invalid_header_diagnostics_never_echo_the_credential() {
    let secret = "Bearer review-private-token\ninvalid";
    let headers = HashMap::from([("Authorization".into(), secret.into())]);
    let http = StreamableHttpTransport::connect("http://127.0.0.1:1", &headers)
        .await
        .err()
        .unwrap();
    let sse = SseTransport::connect("http://127.0.0.1:1", &headers)
        .await
        .err()
        .unwrap();
    for error in [http, sse] {
        assert!(!error.to_string().contains("review-private-token"));
    }
}

#[tokio::test]
async fn notification_http_failure_is_not_reported_as_success() {
    let (url, server) = serve_once(
        b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
    )
    .await;
    let transport = StreamableHttpTransport::connect(&url, &HashMap::new())
        .await
        .unwrap();
    assert!(
        transport
            .notify(&JsonRpcRequest::notification(
                "notifications/initialized",
                None
            ))
            .await
            .is_err()
    );
    server.await.unwrap();
}

#[tokio::test]
async fn direct_http_response_must_match_the_request_id_and_version() {
    for body in [
        r#"{"jsonrpc":"2.0","id":8,"result":{}}"#,
        r#"{"jsonrpc":"1.0","id":7,"result":{}}"#,
    ] {
        let (url, server) = serve_once(json_response(body)).await;
        let transport = StreamableHttpTransport::connect(&url, &HashMap::new())
            .await
            .unwrap();
        assert!(
            transport
                .request(&JsonRpcRequest::new(7, "tools/list", None))
                .await
                .is_err()
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn invalid_json_error_does_not_echo_response_body() {
    let (url, server) = serve_once(json_response("private-response-payload")).await;
    let transport = StreamableHttpTransport::connect(&url, &HashMap::new())
        .await
        .unwrap();
    let error = transport
        .request(&JsonRpcRequest::new(7, "tools/list", None))
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("private-response-payload"));
    server.await.unwrap();
}

#[tokio::test]
async fn sse_notifications_are_skipped_until_the_matching_response() {
    let body = "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let (url, server) = serve_once(response.into_bytes()).await;
    let transport = StreamableHttpTransport::connect(&url, &HashMap::new())
        .await
        .unwrap();
    let result = transport
        .request(&JsonRpcRequest::new(7, "tools/list", None))
        .await
        .unwrap();
    assert_eq!(result.id, Some(7));
    assert_eq!(result.result.unwrap()["ok"], true);
    server.await.unwrap();
}

#[tokio::test]
async fn utf8_sse_payload_survives_http_chunks_split_inside_characters() {
    let event = "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"text\":\"记忆🦀\"}}\n\n";
    let mut wire = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    for byte in event.as_bytes() {
        wire.extend_from_slice(&[b'1', b'\r', b'\n', *byte, b'\r', b'\n']);
    }
    wire.extend_from_slice(b"0\r\n\r\n");
    let (url, server) = serve_once(wire).await;
    let transport = StreamableHttpTransport::connect(&url, &HashMap::new())
        .await
        .unwrap();
    let result = transport
        .request(&JsonRpcRequest::new(7, "tools/list", None))
        .await
        .unwrap();
    assert_eq!(result.result.unwrap()["text"], "记忆🦀");
    server.await.unwrap();
}

#[tokio::test]
async fn dropping_legacy_sse_transport_closes_its_listener() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/sse", listener.local_addr().unwrap());
    let accept = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        let event = "event: endpoint\ndata: /messages\n\n";
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{event}\r\n", event.len()).as_bytes()).await.unwrap();
        stream
    };
    let headers = HashMap::new();
    let (transport, mut stream) = tokio::join!(SseTransport::connect(&url, &headers), accept);
    drop(transport.unwrap());
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte))
            .await
            .expect("listener survived transport drop")
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn legacy_endpoint_cannot_redirect_configured_credentials_to_another_origin() {
    let event = "event: endpoint\ndata: http://127.0.0.1:1/steal\n\n";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{event}",
        event.len()
    );
    let (url, server) = serve_once(response.into_bytes()).await;
    let headers = HashMap::from([("Authorization".into(), "Bearer test-private-token".into())]);
    let result = SseTransport::connect(&url, &headers).await;
    assert!(result.is_err());
    server.await.unwrap();
}

#[test]
fn sse_data_parser_preserves_spaces_and_accepts_bare_cr() {
    assert_eq!(
        super::parse_sse_event("event: message\rdata:  leading space\rdata: trailing space "),
        ("message".into(), " leading space\ntrailing space ".into())
    );
}

#[tokio::test]
async fn cross_origin_redirect_is_not_followed_with_custom_credentials() {
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let response = format!(
        "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{}/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        target.local_addr().unwrap()
    );
    let (url, server) = serve_once(response.into_bytes()).await;
    let transport = StreamableHttpTransport::connect(
        &url,
        &HashMap::from([("x-private-key".into(), "private-token".into())]),
    )
    .await
    .unwrap();
    let notification = JsonRpcRequest::notification("notifications/initialized", None);
    tokio::select! {
        result = transport.notify(&notification) => assert!(result.is_err()),
        _ = target.accept() => panic!("custom credential request followed an unconfigured origin"),
        _ = tokio::time::sleep(Duration::from_secs(3)) => panic!("redirect check timed out"),
    }
    server.await.unwrap();
}
