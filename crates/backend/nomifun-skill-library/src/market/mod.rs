//! Skill market integration: live ranking sync across six public
//! marketplaces and MCP config resolution.
//!
//! Route handlers in [`crate::skill_routes`] stay thin and delegate here.
//! Submodules:
//! - [`client`] — allowlist-guarded HTTP client (custom redirect policy) and
//!   size-capped body readers.
//! - [`parse`] — per-source ranking parsers and shared text/JSON helpers.
//! - [`mcp`] — market MCP entry → importable `mcpServers` JSON.

mod client;
mod mcp;
mod parse;

pub use mcp::resolve_market_mcp_config;

use std::collections::HashSet;
use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nomifun_api_types::{SkillMarketItemResponse, SkillMarketSyncResponse};
use nomifun_common::AppError;

use client::{build_market_client, read_market_body, read_market_json_post};
use parse::{
    parse_clawhub_plugins, parse_clawhub_rankings, parse_loophub_rankings, parse_mcpworld_rankings,
    parse_skillhub_mcp_rankings, parse_skillhub_rankings,
};

// ---------------------------------------------------------------------------
// Sources & ranking endpoints
// ---------------------------------------------------------------------------

const CLAWHUB_SOURCE: &str = "clawhub";
const SKILLHUB_SOURCE: &str = "skillhub";
const LOOPHUB_SOURCE: &str = "loophub";
const SKILLHUB_MCP_SOURCE: &str = "skillhub_mcp";
const MCPWORLD_SOURCE: &str = "mcpworld";
const CLAWHUB_PLUGINS_SOURCE: &str = "clawhub_plugins";

const CLAWHUB_RANKING_URL: &str = "https://clawhub.ai/skills?tab=new";
const CLAWHUB_CONVEX_QUERY_URL: &str = "https://wry-manatee-359.convex.cloud/api/query";
const SKILLHUB_RANKING_URL: &str = "https://api.skillhub.cn/api/skills?page=1&pageSize=100&sortBy=score&order=desc";
const SKILLHUB_HTML_FALLBACK_URL: &str = "https://www.skills.sh/trending/";
const LOOPHUB_RANKING_URL: &str =
    "https://api.cocoloop.cn/api/v1/store/skills?page=1&page_size=100&sort=downloads&tab=overall";
const SKILLHUB_MCP_RANKING_URL: &str =
    "https://api.skillhub.cn/api/v1/mcp/servers?page=1&pageSize=100&sortBy=updated_at&order=desc";
const MCPWORLD_RANKING_URL: &str =
    "https://www.mcpworld.com/api/mcp-market/servers?wd=most_popular&type=tag&pn=0&lg=zh&pl=100";
const CLAWHUB_PLUGINS_API_URL: &str = "https://clawhub.ai/api/v1/plugins?limit=100&sort=recommended";
const CLAWHUB_PLUGINS_URL: &str = "https://clawhub.ai/plugins";

/// Ranking cap applied per source after parsing.
const MAX_MARKET_ITEMS_PER_SOURCE: usize = 200;

/// Outer per-source budget. Must exceed the worst case of one primary plus
/// one fallback request (2 × the 12s per-request client timeout = 24s), so a
/// slow primary can never starve the fallback of its whole window.
const MARKET_SOURCE_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Fetch coordination
// ---------------------------------------------------------------------------

/// Fetch live rankings for the selected `sources` (empty = all supported),
/// concurrently with a per-source timeout. Per-source failures are reported
/// in [`SkillMarketSyncResponse::errors`] instead of failing the whole sync.
pub async fn fetch_skill_market_rankings(sources: Vec<String>) -> Result<SkillMarketSyncResponse, AppError> {
    let selected = normalize_market_sources(sources)?;
    let client = build_market_client()?;

    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut tasks = tokio::task::JoinSet::new();
    for source in selected {
        let client = client.clone();
        tasks.spawn(async move { fetch_market_source_with_timeout(&client, source).await });
    }

    while let Some(joined) = tasks.join_next().await {
        let (source, result) = joined.map_err(|e| AppError::Internal(format!("skill market task failed: {e}")))?;
        match result {
            Ok(mut source_items) => items.append(&mut source_items),
            Err(error) => errors.push(format!("{source}: {error}")),
        }
    }

    Ok(SkillMarketSyncResponse {
        fetched_at: now_epoch_ms(),
        items,
        errors,
    })
}

async fn fetch_market_source_with_timeout(
    client: &reqwest::Client,
    source: &'static str,
) -> (&'static str, Result<Vec<SkillMarketItemResponse>, AppError>) {
    let result = match tokio::time::timeout(MARKET_SOURCE_TIMEOUT, fetch_market_source(client, source)).await {
        Ok(result) => result,
        Err(_) => Err(AppError::Timeout(format!(
            "skill market source timed out after {}s",
            MARKET_SOURCE_TIMEOUT.as_secs()
        ))),
    };
    (source, result)
}

/// Validate and dedupe the requested source slugs. Empty input selects all
/// supported sources; an unknown slug is a 400.
fn normalize_market_sources(sources: Vec<String>) -> Result<Vec<&'static str>, AppError> {
    if sources.is_empty() {
        return Ok(vec![
            CLAWHUB_SOURCE,
            LOOPHUB_SOURCE,
            SKILLHUB_SOURCE,
            SKILLHUB_MCP_SOURCE,
            MCPWORLD_SOURCE,
            CLAWHUB_PLUGINS_SOURCE,
        ]);
    }

    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for source in sources {
        let normalized = source.trim().to_ascii_lowercase();
        let source = match normalized.as_str() {
            CLAWHUB_SOURCE => CLAWHUB_SOURCE,
            SKILLHUB_SOURCE => SKILLHUB_SOURCE,
            LOOPHUB_SOURCE => LOOPHUB_SOURCE,
            SKILLHUB_MCP_SOURCE => SKILLHUB_MCP_SOURCE,
            MCPWORLD_SOURCE => MCPWORLD_SOURCE,
            CLAWHUB_PLUGINS_SOURCE => CLAWHUB_PLUGINS_SOURCE,
            other => return Err(AppError::BadRequest(format!("unsupported skill market source: {other}"))),
        };
        if seen.insert(source) {
            selected.push(source);
        }
    }
    Ok(selected)
}

async fn fetch_market_source(
    client: &reqwest::Client,
    source: &'static str,
) -> Result<Vec<SkillMarketItemResponse>, AppError> {
    match source {
        CLAWHUB_SOURCE => fetch_clawhub_rankings(client).await,
        SKILLHUB_SOURCE => fetch_skillhub_rankings(client).await,
        CLAWHUB_PLUGINS_SOURCE => fetch_clawhub_plugins(client).await,
        LOOPHUB_SOURCE => Ok(parse_loophub_rankings(&read_market_body(client, LOOPHUB_RANKING_URL).await?)),
        SKILLHUB_MCP_SOURCE => Ok(parse_skillhub_mcp_rankings(
            &read_market_body(client, SKILLHUB_MCP_RANKING_URL).await?,
        )),
        MCPWORLD_SOURCE => Ok(parse_mcpworld_rankings(&read_market_body(client, MCPWORLD_RANKING_URL).await?)),
        other => Err(AppError::BadRequest(format!("unsupported skill market source: {other}"))),
    }
}

// ---------------------------------------------------------------------------
// Primary/fallback fetchers
// ---------------------------------------------------------------------------

/// Run `primary`, falling back to `fallback` only when the primary produced
/// no items. Error contract: `Ok(vec![])` is returned only when at least one
/// fetch genuinely succeeded and neither errored while both were empty; if no
/// items were produced and any fetch failed, the error surfaces (preferring
/// the primary's) instead of being masked as an empty success.
async fn fetch_with_fallback(
    primary: impl Future<Output = Result<Vec<SkillMarketItemResponse>, AppError>>,
    fallback: impl Future<Output = Result<Vec<SkillMarketItemResponse>, AppError>>,
) -> Result<Vec<SkillMarketItemResponse>, AppError> {
    let primary = primary.await;
    if let Ok(items) = &primary
        && !items.is_empty()
    {
        return primary;
    }

    let fallback = fallback.await;
    match (primary, fallback) {
        // A non-empty fallback rescues an empty or failed primary.
        (_, Ok(items)) if !items.is_empty() => Ok(items),
        // Both fetches succeeded but found nothing: genuinely empty.
        (Ok(items), Ok(_)) => Ok(items),
        // No usable items and at least one failure: surface an error,
        // preferring the primary's.
        (Err(primary_error), _) => Err(primary_error),
        (Ok(_), Err(fallback_error)) => Err(fallback_error),
    }
}

async fn fetch_clawhub_rankings(client: &reqwest::Client) -> Result<Vec<SkillMarketItemResponse>, AppError> {
    fetch_with_fallback(
        async {
            let body = read_market_json_post(
                client,
                CLAWHUB_CONVEX_QUERY_URL,
                serde_json::json!({
                    "path": "skills:listPublicPageV4",
                    "format": "convex_encoded_json",
                    "args": [{
                        "dir": "desc",
                        "numItems": 100,
                        "sort": "newest"
                    }]
                }),
            )
            .await?;
            Ok(parse_clawhub_rankings(&body))
        },
        async { Ok(parse_clawhub_rankings(&read_market_body(client, CLAWHUB_RANKING_URL).await?)) },
    )
    .await
}

async fn fetch_skillhub_rankings(client: &reqwest::Client) -> Result<Vec<SkillMarketItemResponse>, AppError> {
    fetch_with_fallback(
        async { Ok(parse_skillhub_rankings(&read_market_body(client, SKILLHUB_RANKING_URL).await?)) },
        async {
            Ok(parse_skillhub_rankings(
                &read_market_body(client, SKILLHUB_HTML_FALLBACK_URL).await?,
            ))
        },
    )
    .await
}

async fn fetch_clawhub_plugins(client: &reqwest::Client) -> Result<Vec<SkillMarketItemResponse>, AppError> {
    fetch_with_fallback(
        async { Ok(parse_clawhub_plugins(&read_market_body(client, CLAWHUB_PLUGINS_API_URL).await?)) },
        async { Ok(parse_clawhub_plugins(&read_market_body(client, CLAWHUB_PLUGINS_URL).await?)) },
    )
    .await
}

fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_market_sources_rejects_unknown_source() {
        let err = normalize_market_sources(vec!["unknown".into()]).unwrap_err();
        assert!(err.to_string().contains("unsupported skill market source"));
    }

    #[test]
    fn normalize_market_sources_accepts_new_market_sources() {
        let sources = normalize_market_sources(vec![
            LOOPHUB_SOURCE.into(),
            SKILLHUB_MCP_SOURCE.into(),
            MCPWORLD_SOURCE.into(),
            CLAWHUB_PLUGINS_SOURCE.into(),
        ])
        .unwrap();
        assert_eq!(
            sources,
            vec![
                LOOPHUB_SOURCE,
                SKILLHUB_MCP_SOURCE,
                MCPWORLD_SOURCE,
                CLAWHUB_PLUGINS_SOURCE,
            ]
        );
    }

    #[test]
    fn normalize_market_sources_defaults_to_all_six() {
        let sources = normalize_market_sources(Vec::new()).unwrap();
        assert_eq!(sources.len(), 6);
    }

    #[test]
    fn clawhub_market_uses_skills_ranking_page() {
        assert!(CLAWHUB_RANKING_URL.ends_with("/skills?tab=new"));
    }

    fn item(name: &str) -> SkillMarketItemResponse {
        SkillMarketItemResponse {
            id: format!("clawhub:owner/{name}"),
            source: CLAWHUB_SOURCE.into(),
            rank: 1,
            name: name.into(),
            description: String::new(),
            url: format!("https://clawhub.ai/owner/skills/{name}"),
            install_command: format!("openclaw skills install @owner/{name}"),
            tags: vec![],
            audience_tags: vec![],
            scenario_tags: vec![],
            stats: None,
        }
    }

    #[tokio::test]
    async fn fetch_with_fallback_prefers_non_empty_primary() {
        let items = fetch_with_fallback(async { Ok(vec![item("primary")]) }, async {
            panic!("fallback must not run when primary has items")
        })
        .await
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "primary");
    }

    #[tokio::test]
    async fn fetch_with_fallback_uses_fallback_when_primary_empty_or_failed() {
        let items = fetch_with_fallback(async { Ok(vec![]) }, async { Ok(vec![item("fallback")]) })
            .await
            .unwrap();
        assert_eq!(items[0].name, "fallback");

        let items = fetch_with_fallback(
            async { Err(AppError::BadGateway("primary down".into())) },
            async { Ok(vec![item("fallback")]) },
        )
        .await
        .unwrap();
        assert_eq!(items[0].name, "fallback");
    }

    /// Fix for the empty-success masking bug: when neither fetch produced
    /// items and at least one errored, an error must surface (preferring the
    /// primary's) — never a silent `Ok(vec![])`.
    #[tokio::test]
    async fn fetch_with_fallback_surfaces_errors_instead_of_empty_success() {
        // Primary error + empty fallback → primary error.
        let err = fetch_with_fallback(
            async { Err(AppError::BadGateway("primary down".into())) },
            async { Ok(vec![]) },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("primary down"), "{err}");

        // Both errored → primary error preferred.
        let err = fetch_with_fallback(
            async { Err(AppError::BadGateway("primary down".into())) },
            async { Err(AppError::BadGateway("fallback down".into())) },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("primary down"), "{err}");

        // Empty primary + fallback error → fallback error.
        let err = fetch_with_fallback(async { Ok(vec![]) }, async {
            Err(AppError::BadGateway("fallback down".into()))
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("fallback down"), "{err}");

        // Both genuinely empty successes → empty success.
        let items = fetch_with_fallback(async { Ok(vec![]) }, async { Ok(vec![]) })
            .await
            .unwrap();
        assert!(items.is_empty());
    }

    /// Manual contract smoke test for the two original third-party pages.
    /// Kept ignored in normal CI because it requires public network access
    /// and those sites are outside NomiFun's availability control.
    #[tokio::test]
    #[ignore = "requires public ClawHub and SkillHub access"]
    async fn live_market_pages_still_match_the_ranking_contract() {
        let response = fetch_skill_market_rankings(vec![CLAWHUB_SOURCE.into(), SKILLHUB_SOURCE.into()])
            .await
            .unwrap();

        assert!(response.errors.is_empty(), "live fetch errors: {:?}", response.errors);
        assert!(response.items.iter().any(|item| item.source == CLAWHUB_SOURCE));
        assert!(response.items.iter().any(|item| item.source == SKILLHUB_SOURCE));
        assert!(response.items.iter().all(|item| {
            item.url.starts_with("https://")
                && (item.install_command.starts_with("openclaw skills install @")
                    || item.install_command.starts_with("npx skills add "))
        }));
    }

    /// Manual contract smoke test for the four newer sources. Ignored for the
    /// same public-network reason as above.
    #[tokio::test]
    #[ignore = "requires public LoopHub, SkillHub, and MCPWorld access"]
    async fn live_new_market_sources_return_ranked_items() {
        let response = fetch_skill_market_rankings(vec![
            LOOPHUB_SOURCE.into(),
            SKILLHUB_MCP_SOURCE.into(),
            MCPWORLD_SOURCE.into(),
            CLAWHUB_PLUGINS_SOURCE.into(),
        ])
        .await
        .unwrap();

        assert!(response.errors.is_empty(), "live fetch errors: {:?}", response.errors);
        assert!(!response.items.is_empty());
        assert!(
            response
                .items
                .iter()
                .all(|item| item.rank >= 1 && item.url.starts_with("https://"))
        );
    }

}
