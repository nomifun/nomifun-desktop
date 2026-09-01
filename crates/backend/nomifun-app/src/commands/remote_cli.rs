//! Canonical Remote CLI adapter.
//!
//! This client mirrors the four `/api/remote/*` operations exactly. It never
//! reads the legacy Gateway registry, chooses a profile/domain set, or infers a
//! product Session from transport state.

use std::process::ExitCode;

use nomifun_api_types::{
    RemoteCancelRequestDto, RemoteOpenRequestDto, RemoteTurnRequestDto,
};
use serde_json::Value;

use crate::cli::RemoteCommand;

const DEFAULT_URL: &str = "http://127.0.0.1:25808";

pub async fn run_remote(operation: &RemoteCommand) -> ExitCode {
    let request = match build_request(operation) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let endpoint = request.endpoint.clone();
    match request.send().await {
        Ok(response) => print_response(response).await,
        Err(error) => {
            eprintln!(
                "request to {endpoint} failed: {error} (is NomiFun running and NOMIFUN_URL correct?)"
            );
            ExitCode::from(1)
        }
    }
}

struct RemoteRequest {
    endpoint: String,
    request: reqwest::RequestBuilder,
}

impl RemoteRequest {
    async fn send(self) -> Result<reqwest::Response, reqwest::Error> {
        self.request.send().await
    }
}

fn build_request(operation: &RemoteCommand) -> Result<RemoteRequest, String> {
    let client = reqwest::Client::new();
    match operation {
        RemoteCommand::Open {
            binding_id,
            initial_input,
            idempotency_key,
            url,
            token,
        } => {
            let (base, token) = resolve_endpoint(url.clone(), token.clone())?;
            let idempotency_key = resolve_idempotency_key(idempotency_key, "remote-open")?;
            let initial_input = initial_input
                .as_deref()
                .map(|input| parse_json(input, "initial input"))
                .transpose()?;
            let endpoint = format!("{base}/api/remote/open");
            Ok(RemoteRequest {
                request: client
                    .post(&endpoint)
                    .bearer_auth(token)
                    .json(&RemoteOpenRequestDto {
                        binding_id: binding_id.clone(),
                        idempotency_key,
                        initial_input,
                    }),
                endpoint,
            })
        }
        RemoteCommand::Turn {
            agent_session_id,
            input,
            idempotency_key,
            url,
            token,
        } => {
            let (base, token) = resolve_endpoint(url.clone(), token.clone())?;
            let idempotency_key = resolve_idempotency_key(idempotency_key, "remote-turn")?;
            let endpoint = format!("{base}/api/remote/turn");
            Ok(RemoteRequest {
                request: client
                    .post(&endpoint)
                    .bearer_auth(token)
                    .json(&RemoteTurnRequestDto {
                        agent_session_id: agent_session_id.clone(),
                        input: parse_json(input, "turn input")?,
                        idempotency_key,
                    }),
                endpoint,
            })
        }
        RemoteCommand::Observe {
            agent_session_id,
            after_seq,
            limit,
            url,
            token,
        } => {
            if *limit == 0 {
                return Err("--limit must be greater than zero".to_owned());
            }
            let (base, token) = resolve_endpoint(url.clone(), token.clone())?;
            let endpoint = format!("{base}/api/remote/observe");
            Ok(RemoteRequest {
                request: client.get(&endpoint).bearer_auth(token).query(&[
                    ("agent_session_id", agent_session_id.as_str().to_owned()),
                    ("after_seq", after_seq.to_string()),
                    ("limit", limit.to_string()),
                ]),
                endpoint,
            })
        }
        RemoteCommand::Cancel {
            agent_session_id,
            idempotency_key,
            url,
            token,
        } => {
            let (base, token) = resolve_endpoint(url.clone(), token.clone())?;
            let idempotency_key = resolve_idempotency_key(idempotency_key, "remote-cancel")?;
            let endpoint = format!("{base}/api/remote/cancel");
            Ok(RemoteRequest {
                request: client
                    .post(&endpoint)
                    .bearer_auth(token)
                    .json(&RemoteCancelRequestDto {
                        agent_session_id: agent_session_id.clone(),
                        idempotency_key,
                    }),
                endpoint,
            })
        }
    }
}

fn resolve_endpoint(
    url: Option<String>,
    token: Option<String>,
) -> Result<(String, String), String> {
    let base = url
        .or_else(|| std::env::var("NOMIFUN_URL").ok())
        .map(|url| url.trim_end_matches('/').to_owned())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_owned());
    let parsed = reqwest::Url::parse(&base)
        .map_err(|error| format!("invalid NomiFun base URL {base:?}: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("NomiFun base URL must use http or https".to_owned());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("NomiFun base URL must not contain a query or fragment".to_owned());
    }

    let token = token
        .or_else(|| std::env::var("NOMIFUN_ACCESS_TOKEN").ok())
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| {
            "no access token: pass --token or set NOMIFUN_ACCESS_TOKEN (mint the installation \
             token in NomiFun via POST /api/webui/access-token)"
                .to_owned()
        })?;
    Ok((base, token))
}

fn resolve_idempotency_key(
    value: &Option<String>,
    prefix: &str,
) -> Result<String, String> {
    if let Some(value) = value {
        if value.trim().is_empty() || value.trim() != value {
            return Err("--idempotency-key must be non-empty without surrounding whitespace".into());
        }
        return Ok(value.clone());
    }
    let generated = format!("{prefix}-{}", uuid::Uuid::new_v4());
    eprintln!("using generated idempotency key: {generated}");
    Ok(generated)
}

fn parse_json(value: &str, label: &str) -> Result<Value, String> {
    serde_json::from_str(value).map_err(|error| format!("invalid {label} JSON: {error}"))
}

async fn print_response(response: reqwest::Response) -> ExitCode {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    match serde_json::from_str::<Value>(&text) {
        Ok(value) => println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
        ),
        Err(_) => println!("{text}"),
    }
    if status.is_success() {
        ExitCode::SUCCESS
    } else {
        eprintln!("HTTP {status}");
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_building_uses_only_canonical_remote_paths() {
        let open = build_request(&RemoteCommand::Open {
            binding_id: "binding-1".to_owned(),
            initial_input: Some(r#"{"text":"hello"}"#.to_owned()),
            idempotency_key: Some("open-1".to_owned()),
            url: Some("http://127.0.0.1:25808".to_owned()),
            token: Some("secret".to_owned()),
        })
        .unwrap();
        assert_eq!(open.endpoint, "http://127.0.0.1:25808/api/remote/open");

        let observe = build_request(&RemoteCommand::Observe {
            agent_session_id: "0190f5fe-7c00-7a00-8000-000000000001".to_owned(),
            after_seq: 7,
            limit: 25,
            url: Some("https://example.test/base/".to_owned()),
            token: Some("secret".to_owned()),
        })
        .unwrap();
        assert_eq!(
            observe.endpoint,
            "https://example.test/base/api/remote/observe"
        );
    }

    #[test]
    fn invalid_json_idempotency_and_base_url_fail_before_network_io() {
        assert!(parse_json("{", "turn input").is_err());
        assert!(resolve_idempotency_key(&Some(" padded ".to_owned()), "remote-turn").is_err());
        assert!(
            resolve_endpoint(
                Some("ftp://example.test".to_owned()),
                Some("secret".to_owned())
            )
            .is_err()
        );
        assert!(
            resolve_endpoint(
                Some("https://example.test?profile=agent".to_owned()),
                Some("secret".to_owned())
            )
            .is_err()
        );
    }
}
