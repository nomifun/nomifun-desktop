//! Shared helpers for the `nomicore mcp-*-stdio` subcommands.
//!
//! The guide / requirement / gateway stdio bridges each forward MCP tool calls
//! as an HTTP `POST /tool` to their in-process server. They shared an identical
//! connect-retry loop and response-parsing shape; only the request-body fields,
//! the log prefix, and (for gateway) the handling of a non-string `result`
//! differ. This module centralizes the loop so the three bridges stay in sync.

use std::time::Duration;

/// Build the `reqwest::Client` the stdio bridges use to reach their in-process
/// HTTP server: no idle-connection pooling (each bridge is short-lived and makes
/// few requests), a short connect timeout, and a generous overall timeout.
pub fn build_bridge_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Forward an MCP tool call as `POST http://127.0.0.1:{port}/tool` with bearer
/// auth, retrying on transient failures (the in-process server may not be ready
/// yet right after a session-resume spawns this process). Returns the tool
/// result string, or an `Error: ...` string on failure.
///
/// `body` is built by the caller — each bridge injects its own context fields.
/// `stringify_non_string_result` controls how a non-string `result` JSON value
/// is rendered: `false` ignores it (guide / requirement), `true` pretty-prints
/// it (gateway).
pub async fn forward_tool_http(
    http_client: &reqwest::Client,
    port: u16,
    token: &str,
    log_prefix: &str,
    body: &serde_json::Value,
    stringify_non_string_result: bool,
) -> String {
    let url = format!("http://127.0.0.1:{port}/tool");

    // Retry with backoff — the in-process HTTP server may not be fully ready
    // immediately after a session resume spawns this process.
    let delays_ms: &[u64] = &[0, 1000, 2000, 3000];
    let mut last_error = String::new();
    for (attempt, &delay_ms) in delays_ms.iter().enumerate() {
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            eprintln!("[{log_prefix}] retrying (attempt {})...", attempt + 1);
        }
        match http_client
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(body)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                match resp.text().await {
                    Ok(text) => {
                        eprintln!("[{log_prefix}] POST /tool → status={status}");
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                            match v.get("result") {
                                Some(serde_json::Value::String(s)) => return s.clone(),
                                Some(other) if stringify_non_string_result => {
                                    return serde_json::to_string_pretty(other)
                                        .unwrap_or_else(|_| other.to_string());
                                }
                                _ => {}
                            }
                            if let Some(error) = v.get("error") {
                                return format!("Error: {error}");
                            }
                        }
                        return text;
                    }
                    Err(e) => {
                        last_error = format!("failed to read response: {e}");
                        eprintln!("[{log_prefix}] HTTP FAILED: {last_error}");
                    }
                }
            }
            Err(e) => {
                last_error = format!("{e:#}");
                eprintln!("[{log_prefix}] HTTP FAILED: {last_error}");
            }
        }
    }
    format!("Error: {last_error}")
}
