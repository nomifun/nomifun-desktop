//! Market MCP entry resolution: turn a ranked MCP item (SkillHub MCP or
//! MCPWorld) into an importable `mcpServers` JSON config by fetching its
//! readme/detail page and selecting the most portable fenced config block.

use std::sync::LazyLock;

use nomifun_api_types::SkillMarketMcpConfigRequest;
use nomifun_common::AppError;
use regex::Regex;

use super::client::{build_market_client, read_market_body};
use super::parse::{is_market_slug, last_url_segment, market_ref_suffix};
use super::{MCPWORLD_SOURCE, SKILLHUB_MCP_SOURCE};

static CODE_FENCE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)```(?:json|javascript|js)?\s*(.*?)```").expect("valid mcp code-fence regex")
});

static PLACEHOLDER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)
        \$\{[^}]+\}
        | <[^>\r\n]+>
        | x{4,}
        | (?:your|replace|insert)[\s_-]+[a-z0-9][a-z0-9\s_-]*
        | appbuilder[\s_-]+api[\s_-]+key
        | (?:api[\s_-]*key|token|secret)[\s_-]+here
        ",
    )
    .expect("valid MCP placeholder regex")
});

/// Resolve a market MCP entry into an importable `mcpServers` JSON value.
/// Only `skillhub_mcp` and `mcpworld` items carry resolvable configs.
pub async fn resolve_market_mcp_config(req: SkillMarketMcpConfigRequest) -> Result<serde_json::Value, AppError> {
    let client = build_market_client()?;
    match req.source.as_str() {
        SKILLHUB_MCP_SOURCE => {
            let slug = market_ref_suffix(&req.id, SKILLHUB_MCP_SOURCE)
                .or_else(|| last_url_segment(&req.url))
                .ok_or_else(|| AppError::BadRequest("invalid SkillHub MCP market id".into()))?;
            if !is_market_slug(&slug) {
                return Err(AppError::BadRequest("invalid SkillHub MCP slug".into()));
            }
            let body = read_market_body(
                &client,
                &format!("https://api.skillhub.cn/api/v1/mcp/servers/{slug}/readme"),
            )
            .await?;
            extract_mcp_config_from_markdown(&body)
                .ok_or_else(|| AppError::BadGateway("MCP config block not found".into()))
        }
        MCPWORLD_SOURCE => {
            let id = market_ref_suffix(&req.id, MCPWORLD_SOURCE)
                .or_else(|| last_url_segment(&req.url))
                .ok_or_else(|| AppError::BadRequest("invalid MCPWorld market id".into()))?;
            if !is_market_slug(&id) {
                return Err(AppError::BadRequest("invalid MCPWorld id".into()));
            }
            let body = read_market_body(
                &client,
                &format!("https://www.mcpworld.com/api/mcp-market/server/detail?id={id}&lg=zh"),
            )
            .await?;
            let value = serde_json::from_str::<serde_json::Value>(&body)
                .map_err(|e| AppError::BadGateway(format!("MCPWorld detail JSON parse failed: {e}")))?;
            let detail_text = value
                .pointer("/data/detail/abstract")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.get("value").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            extract_mcp_config_from_markdown(&detail_text)
                .ok_or_else(|| AppError::BadGateway("MCP config block not found".into()))
        }
        other => Err(AppError::BadRequest(format!("unsupported MCP market source: {other}"))),
    }
}

/// Find the most portable importable JSON value with an `mcpServers` key.
///
/// Market readmes commonly list Docker or globally installed commands before
/// an `npx`/`uvx` alternative, and sometimes put a placeholder endpoint before
/// a package-based config. Treating the first example as an install artifact
/// made a large part of the market fail deterministically. Bare JSON remains
/// authoritative; markdown candidates are ranked without executing anything.
fn extract_mcp_config_from_markdown(markdown: &str) -> Option<serde_json::Value> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(markdown.trim())
        && is_importable_mcp_config(&value)
    {
        return Some(value);
    }

    let mut best: Option<(i64, serde_json::Value)> = None;
    for cap in CODE_FENCE_RE.captures_iter(markdown) {
        let Some(block) = cap.get(1).map(|m| m.as_str().trim()) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(block)
            && is_importable_mcp_config(&value)
        {
            let score = mcp_config_score(&value);
            if best.as_ref().is_none_or(|(best_score, _)| score > *best_score) {
                best = Some((score, value));
            }
        }
    }
    best.map(|(_, value)| value)
}

fn is_importable_mcp_config(value: &serde_json::Value) -> bool {
    let Some(servers) = value.get("mcpServers").and_then(serde_json::Value::as_object) else {
        return false;
    };
    !servers.is_empty() && servers.values().all(is_importable_server)
}

fn is_importable_server(value: &serde_json::Value) -> bool {
    let Some(server) = value.as_object() else {
        return false;
    };
    let transport = server
        .get("transport")
        .and_then(serde_json::Value::as_object)
        .unwrap_or(server);
    config_string(transport, server, &["command"]).is_some()
        || config_string(
            transport,
            server,
            &["url", "baseUrl", "base_url", "endpoint", "serverUrl", "server_url"],
        )
        .is_some()
}

fn mcp_config_score(value: &serde_json::Value) -> i64 {
    let scores = value["mcpServers"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(_, server)| mcp_server_score(server))
        .collect::<Vec<_>>();
    // Compare average portability so a snippet does not win merely because it
    // bundles more unrelated servers than another snippet.
    scores.iter().sum::<i64>() / scores.len().max(1) as i64
}

fn mcp_server_score(value: &serde_json::Value) -> i64 {
    let Some(server) = value.as_object() else {
        return -1_000;
    };
    let transport = server
        .get("transport")
        .and_then(serde_json::Value::as_object)
        .unwrap_or(server);

    if let Some(command) = config_string(transport, server, &["command"]) {
        let first_token = command.split_whitespace().next().unwrap_or(command);
        let command_name = first_token
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(first_token)
            .to_ascii_lowercase();
        let command_name = command_name
            .strip_suffix(".exe")
            .or_else(|| command_name.strip_suffix(".cmd"))
            .unwrap_or(&command_name);
        let mut score = match command_name {
            "npx" | "pnpx" | "bunx" | "uvx" => 90,
            "npm" | "bun" | "uv" | "python" | "python3" | "deno" | "node" => 65,
            "docker" | "podman" => 10,
            _ => 35,
        };
        if contains_placeholder(command) {
            score -= 250;
        }
        if let Some(args) = config_value(transport, server, &["args"]).and_then(serde_json::Value::as_array) {
            if matches!(command_name, "npx" | "pnpx")
                && args.iter().any(|arg| matches!(arg.as_str(), Some("-y" | "--yes")))
            {
                score += 10;
            }
            score -= args
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|arg| contains_placeholder(arg))
                .count() as i64
                * 80;
        }
        score -= placeholder_record_count(config_value(transport, server, &["env"])) * 30;
        return score;
    }

    let url = config_string(
        transport,
        server,
        &["url", "baseUrl", "base_url", "endpoint", "serverUrl", "server_url"],
    );
    let Some(url) = url else {
        return -1_000;
    };
    let lower = url.to_ascii_lowercase();
    let mut score = if lower.starts_with("https://") {
        75
    } else if lower.starts_with("http://localhost") || lower.starts_with("http://127.0.0.1") {
        55
    } else if lower.starts_with("http://") {
        -80
    } else {
        -120
    };
    if contains_placeholder(url) {
        score -= 250;
    }
    score -= placeholder_record_count(config_value(transport, server, &["headers"])) * 30;
    score
}

fn config_value<'a>(
    transport: &'a serde_json::Map<String, serde_json::Value>,
    server: &'a serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a serde_json::Value> {
    keys.iter()
        .find_map(|key| transport.get(*key))
        .or_else(|| keys.iter().find_map(|key| server.get(*key)))
}

fn config_string<'a>(
    transport: &'a serde_json::Map<String, serde_json::Value>,
    server: &'a serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a str> {
    config_value(transport, server, keys)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn placeholder_record_count(value: Option<&serde_json::Value>) -> i64 {
    value
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(_, value)| value.as_str())
        .filter(|value| contains_placeholder(value))
        .count() as i64
}

fn contains_placeholder(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty() || PLACEHOLDER_RE.is_match(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_mcp_config_from_markdown_finds_mcpservers_block() {
        let markdown = r#"
```json
{
  "mcpServers": {
    "playwright": {
      "command": "npx",
      "args": ["@playwright/mcp@latest"]
    }
  }
}
```
"#;
        let config = extract_mcp_config_from_markdown(markdown).unwrap();
        assert!(config.get("mcpServers").is_some());
    }

    #[test]
    fn extract_mcp_config_from_markdown_accepts_bare_json_and_skips_noise() {
        let bare = r#"{ "mcpServers": { "x": { "command": "npx" } } }"#;
        assert!(extract_mcp_config_from_markdown(bare).is_some());

        let nested_with_root_command =
            r#"{ "mcpServers": { "x": { "transport": { "type": "stdio" }, "command": "npx" } } }"#;
        assert!(extract_mcp_config_from_markdown(nested_with_root_command).is_some());

        // Non-mcpServers and empty configs are skipped; a later usable block wins.
        let mixed = "```json\n{ \"other\": 1 }\n```\n```js\n{ \"mcpServers\": { \"x\": { \"command\": \"npx\", \"args\": [\"demo\"] } } }\n```";
        assert!(extract_mcp_config_from_markdown(mixed).is_some());

        assert!(extract_mcp_config_from_markdown("{ \"mcpServers\": {} }").is_none());
        assert!(extract_mcp_config_from_markdown("no config here").is_none());
    }

    #[test]
    fn prefers_portable_launcher_over_docker_or_global_command() {
        let memory = r#"
```json
{"mcpServers":{"memory":{"command":"docker","args":["run","memory"]}}}
```
```json
{"mcpServers":{"memory":{"command":"npx","args":["-y","@modelcontextprotocol/server-memory"]}}}
```
"#;
        let selected = extract_mcp_config_from_markdown(memory).unwrap();
        assert_eq!(selected["mcpServers"]["memory"]["command"], "npx");

        let rednote = r#"
```json
{"mcpServers":{"rednote":{"command":"rednote-mcp","args":["--stdio"]}}}
```
```json
{"mcpServers":{"rednote":{"command":"npx","args":["rednote-mcp","--stdio"]}}}
```
"#;
        let selected = extract_mcp_config_from_markdown(rednote).unwrap();
        assert_eq!(selected["mcpServers"]["rednote"]["command"], "npx");
    }

    #[test]
    fn prefers_https_alias_and_editable_credentials_over_placeholder_endpoint() {
        let ai_search = r#"
```json
{"mcpServers":{"AISearch":{"url":"http://appbuilder.baidu.com/v2/ai_search/mcp/sse?api_key=AppBuilder API Key"}}}
```
```json
{"mcpServers":{"AISearch":{"type":"streamableHttp","baseUrl":"https://qianfan.baidubce.com/v2/ai_search/mcp","headers":{"Authorization":"Bearer xxxxx"}}}}
```
"#;
        let selected = extract_mcp_config_from_markdown(ai_search).unwrap();
        assert_eq!(
            selected["mcpServers"]["AISearch"]["baseUrl"],
            "https://qianfan.baidubce.com/v2/ai_search/mcp"
        );

        let memos = r#"
```json
{"mcpServers":{"MemOS":{"type":"streamable_http","url":"https://mcp.example.com/xxxxx/mcp"},"md5":{"command":"npx","args":["md5-mcp"]}}}
```
```json
{"mcpServers":{"MemOS":{"command":"npx","args":["-y","@memtensor/memos-api-mcp"],"env":{"MEMOS_API_KEY":"xxxxx","MEMOS_USER_ID":"your-user-id"}},"md5":{"command":"npx","args":["md5-mcp"]}}}
```
"#;
        let selected = extract_mcp_config_from_markdown(memos).unwrap();
        assert_eq!(selected["mcpServers"]["MemOS"]["command"], "npx");
    }

    #[tokio::test]
    async fn resolve_market_mcp_config_rejects_unsupported_source_and_bad_slug() {
        let err = resolve_market_mcp_config(SkillMarketMcpConfigRequest {
            source: "clawhub".into(),
            id: "clawhub:owner/skill".into(),
            url: "https://clawhub.ai/owner/skills/skill".into(),
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("unsupported MCP market source"));

        // Traversal-shaped slug is rejected before any network fetch.
        let err = resolve_market_mcp_config(SkillMarketMcpConfigRequest {
            source: SKILLHUB_MCP_SOURCE.into(),
            id: format!("{SKILLHUB_MCP_SOURCE}:../etc"),
            url: String::new(),
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("invalid SkillHub MCP slug"));
    }
}
