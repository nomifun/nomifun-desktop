//! Market MCP entry resolution: turn a ranked MCP item (SkillHub MCP or
//! MCPWorld) into an importable `mcpServers` JSON config by fetching its
//! readme/detail page and extracting the first fenced config block.

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

/// Find the first JSON value with an `mcpServers` key: the whole document
/// when it is bare JSON, otherwise the first fenced code block that parses
/// to one.
fn extract_mcp_config_from_markdown(markdown: &str) -> Option<serde_json::Value> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(markdown.trim())
        && value.get("mcpServers").is_some()
    {
        return Some(value);
    }

    for cap in CODE_FENCE_RE.captures_iter(markdown) {
        let Some(block) = cap.get(1).map(|m| m.as_str().trim()) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(block)
            && value.get("mcpServers").is_some()
        {
            return Some(value);
        }
    }
    None
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

        // Non-mcpServers fenced blocks are skipped; a later matching block wins.
        let mixed = "```json\n{ \"other\": 1 }\n```\n```js\n{ \"mcpServers\": {} }\n```";
        assert!(extract_mcp_config_from_markdown(mixed).is_some());

        assert!(extract_mcp_config_from_markdown("no config here").is_none());
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
