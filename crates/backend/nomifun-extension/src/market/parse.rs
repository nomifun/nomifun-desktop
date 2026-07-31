//! Per-source ranking parsers and shared extraction helpers for the skill
//! market. Each `parse_*` function takes a raw response body (JSON or HTML)
//! and yields at most [`MAX_MARKET_ITEMS_PER_SOURCE`] ranked items; sources
//! with both an API and an HTML shape try the API JSON first and fall back
//! to anchor scraping on the same body.

use std::collections::HashSet;
use std::sync::LazyLock;

use nomifun_api_types::SkillMarketItemResponse;
use regex::Regex;

use super::{
    CLAWHUB_PLUGINS_SOURCE, CLAWHUB_SOURCE, LOOPHUB_SOURCE, MAX_MARKET_ITEMS_PER_SOURCE, MCPWORLD_SOURCE,
    SKILLHUB_MCP_SOURCE, SKILLHUB_PACKAGES_SOURCE, SKILLHUB_SOURCE,
};

static ANCHOR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<a\b[^>]*\bhref=["']([^"']+)["'][^>]*>(.*?)</a>"#).expect("valid market anchor regex")
});
static STATS_CAPTURE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\d+(?:\.\d+)?\s*[km]?\+?\s*(?:installs?|downloads?|uses?|stars?)?)")
        .expect("valid market stats-capture regex")
});
static STATS_STRIP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\d+(?:\.\d+)?\s*[km]?\+?\s*(?:installs?|downloads?|uses?|stars?)?")
        .expect("valid market stats-strip regex")
});
static HTML_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<[^>]+>").expect("valid html tag regex"));

// ---------------------------------------------------------------------------
// Ranking helpers
// ---------------------------------------------------------------------------

/// Cap parsed items at [`MAX_MARKET_ITEMS_PER_SOURCE`] and assign 1-based ranks.
fn ranked(items: impl Iterator<Item = SkillMarketItemResponse>) -> Vec<SkillMarketItemResponse> {
    items
        .take(MAX_MARKET_ITEMS_PER_SOURCE)
        .enumerate()
        .map(|(index, mut item)| {
            item.rank = index + 1;
            item
        })
        .collect()
}

// ---------------------------------------------------------------------------
// ClawHub skills (API + HTML fallback)
// ---------------------------------------------------------------------------

pub(super) fn parse_clawhub_rankings(body: &str) -> Vec<SkillMarketItemResponse> {
    if let Ok(root) = serde_json::from_str::<serde_json::Value>(body) {
        let items = root
            .pointer("/value/items")
            .or_else(|| root.pointer("/value/page"))
            .and_then(serde_json::Value::as_array);
        if let Some(items) = items {
            let parsed = ranked(items.iter().filter_map(parse_clawhub_api_item));
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }

    parse_clawhub_html_rankings(body)
}

fn parse_clawhub_api_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let skill = item.get("skill")?;
    if skill
        .get("isSuspicious")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let owner = json_text(item, "ownerHandle", 96)
        .or_else(|| item.get("owner").and_then(|owner| json_text(owner, "handle", 96)))?;
    let slug = json_text(skill, "slug", 96)?;
    valid_owner_slug(&owner, &slug)?;
    let name = json_text(skill, "displayName", 96).unwrap_or_else(|| title_from_slug(&slug));
    let description = json_text(skill, "summary", 220).unwrap_or_default();
    let mut tags = json_string_array(skill.get("topics"), 40);
    tags.extend(json_string_array(skill.get("categories"), 40));
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let stats = market_count_stats(
        skill.get("stats"),
        &[("downloads", "downloads"), ("installs", "installs"), ("stars", "stars")],
    );

    Some(SkillMarketItemResponse {
        id: format!("{CLAWHUB_SOURCE}:{owner}/{slug}"),
        source: CLAWHUB_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://clawhub.ai/{owner}/skills/{slug}"),
        install_command: format!("openclaw skills install @{owner}/{slug}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

fn parse_clawhub_html_rankings(html: &str) -> Vec<SkillMarketItemResponse> {
    let mut seen = HashSet::new();
    let mut parsed = Vec::new();

    for (href, text) in market_anchors(html) {
        let Some(url) = market_url(CLAWHUB_SOURCE, &href) else {
            continue;
        };
        let Some((owner, slug)) = clawhub_owner_slug(&url) else {
            continue;
        };
        let id = format!("{CLAWHUB_SOURCE}:{owner}/{slug}");
        if !seen.insert(id.clone()) {
            continue;
        }

        let name = extract_clawhub_name(&text, &owner, &slug);
        let description = extract_clawhub_description(&text, &owner, &name);
        let stats = extract_stats(&text);
        let (tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
        let rank = parsed.len() + 1;
        parsed.push(SkillMarketItemResponse {
            id,
            source: CLAWHUB_SOURCE.into(),
            rank,
            name,
            description,
            url: format!("https://clawhub.ai/{owner}/skills/{slug}"),
            install_command: format!("openclaw skills install @{owner}/{slug}"),
            tags,
            audience_tags,
            scenario_tags,
            stats,
        });
        if parsed.len() >= MAX_MARKET_ITEMS_PER_SOURCE {
            break;
        }
    }

    parsed
}

// ---------------------------------------------------------------------------
// SkillHub skills (API + HTML fallback)
// ---------------------------------------------------------------------------

pub(super) fn parse_skillhub_rankings(body: &str) -> Vec<SkillMarketItemResponse> {
    if let Ok(root) = serde_json::from_str::<serde_json::Value>(body) {
        let items = root.pointer("/data/skills").and_then(serde_json::Value::as_array);
        if let Some(items) = items {
            let parsed = ranked(items.iter().filter_map(parse_skillhub_api_item));
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }

    parse_skillhub_html_rankings(body)
}

fn parse_skillhub_api_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let namespace = item.get("namespace")?;
    let canonical = json_text(namespace, "canonicalName", 160);
    let (owner, slug) = canonical
        .as_deref()
        .and_then(skillhub_canonical_owner_slug)
        .or_else(|| {
            let owner = json_text(namespace, "handle", 96)?;
            let slug = json_text(namespace, "publicSlug", 96).or_else(|| json_text(item, "slug", 96))?;
            valid_owner_slug(&owner, &slug)
        })?;
    let name = json_text(item, "name", 96).unwrap_or_else(|| title_from_slug(&slug));
    let description = json_text(item, "description_zh", 220)
        .or_else(|| json_text(item, "description", 220))
        .unwrap_or_default();
    let mut tags = Vec::new();
    if item
        .pointer("/labels/requires_api_key")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        tags.push("requires_api_key".into());
    } else {
        tags.push("no_api_key".into());
    }
    tags.extend(
        item.get("subCategories")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|category| json_text(category, "key", 40)),
    );
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let stats = market_count_stats(
        Some(item),
        &[("downloads", "downloads"), ("installs", "installs"), ("stars", "stars")],
    );

    Some(SkillMarketItemResponse {
        id: format!("{SKILLHUB_SOURCE}:{owner}/skills/{slug}"),
        source: SKILLHUB_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://skillhub.cn/skills/{owner}/{slug}"),
        install_command: format!("npx skills add @{owner}/{slug}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

fn parse_skillhub_html_rankings(html: &str) -> Vec<SkillMarketItemResponse> {
    let mut seen = HashSet::new();
    let mut parsed = Vec::new();

    for (href, text) in market_anchors(html) {
        let Some(url) = market_url(SKILLHUB_SOURCE, &href) else {
            continue;
        };
        let Some((owner, slug)) = skillhub_owner_slug(&url) else {
            continue;
        };
        let id = format!("{SKILLHUB_SOURCE}:{owner}/skills/{slug}");
        if !seen.insert(id.clone()) {
            continue;
        }

        let name = extract_skillhub_name(&text, &owner, &slug);
        let stats = extract_stats(&text);
        let description = extract_skillhub_description(&text, &owner, &name, stats.as_deref());
        let (tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
        let install_command = if owner.contains('.') {
            format!("npx skills add https://www.skills.sh/{owner}/skills/{slug}")
        } else {
            format!("npx skills add https://github.com/{owner}/skills --skill {slug}")
        };
        let rank = parsed.len() + 1;
        parsed.push(SkillMarketItemResponse {
            id,
            source: SKILLHUB_SOURCE.into(),
            rank,
            name,
            description,
            url: format!("https://www.skills.sh/{owner}/skills/{slug}"),
            install_command,
            tags,
            audience_tags,
            scenario_tags,
            stats,
        });
        if parsed.len() >= MAX_MARKET_ITEMS_PER_SOURCE {
            break;
        }
    }

    parsed
}

// ---------------------------------------------------------------------------
// LoopHub skills
// ---------------------------------------------------------------------------

pub(super) fn parse_loophub_rankings(body: &str) -> Vec<SkillMarketItemResponse> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(items) = root.pointer("/data/items").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    ranked(items.iter().filter_map(parse_loophub_item))
}

fn parse_loophub_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let id = item.get("id")?.as_i64()?;
    let download_url = json_text(item, "download_url", 260)?;
    if !download_url.starts_with("https://dl.cocoloop.cn/bss/skills/") {
        return None;
    }
    let name = json_text(item, "name", 96).unwrap_or_else(|| format!("LoopHub Skill {id}"));
    let subtitle = json_text(item, "subtitle", 160).unwrap_or_default();
    let brief = json_text(item, "brief", 220).unwrap_or_default();
    let description = if !brief.is_empty() { brief } else { subtitle };
    let stats = json_text(item, "downloads", 60).map(|downloads| format!("{downloads} downloads"));
    let mut tags = json_text(item, "category", 40).into_iter().collect::<Vec<_>>();
    tags.extend(json_text(item, "security_level", 20).map(|value| format!("security-{value}")));
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    Some(SkillMarketItemResponse {
        id: format!("{LOOPHUB_SOURCE}:{id}"),
        source: LOOPHUB_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://hub.cocoloop.cn/skills/{id}"),
        install_command: format!("loophub skill download {download_url}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

// ---------------------------------------------------------------------------
// SkillHub MCP servers
// ---------------------------------------------------------------------------

pub(super) fn parse_skillhub_mcp_rankings(body: &str) -> Vec<SkillMarketItemResponse> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(items) = root.get("items").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    ranked(items.iter().filter_map(parse_skillhub_mcp_item))
}

fn parse_skillhub_mcp_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let slug = json_text(item, "slug", 96)?;
    if !is_market_slug(&slug) {
        return None;
    }
    let name = json_text(item, "name", 96)
        .or_else(|| json_text(item, "nameEn", 96))
        .unwrap_or_else(|| title_from_slug(&slug));
    let description = json_text(item, "summaryZh", 220)
        .or_else(|| json_text(item, "summary", 220))
        .unwrap_or_else(|| "SkillHub MCP server.".into());
    let mut tags = json_text(item, "category", 40).into_iter().collect::<Vec<_>>();
    tags.extend(json_string_array(item.get("tags"), 40));
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let stats = item.get("stats").map(|stats| {
        let downloads = stats.get("downloads").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let installs = stats.get("installs").and_then(serde_json::Value::as_u64).unwrap_or(0);
        format!("{downloads} downloads / {installs} installs")
    });
    Some(SkillMarketItemResponse {
        id: format!("{SKILLHUB_MCP_SOURCE}:{slug}"),
        source: SKILLHUB_MCP_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://skillhub.cn/mcp/{slug}"),
        install_command: format!("mcp market add skillhub:{slug}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

// ---------------------------------------------------------------------------
// MCPWorld servers
// ---------------------------------------------------------------------------

pub(super) fn parse_mcpworld_rankings(body: &str) -> Vec<SkillMarketItemResponse> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(lists) = root.pointer("/data/mcpList").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    ranked(
        lists
            .iter()
            .flat_map(|list| list.get("servers").and_then(serde_json::Value::as_array).into_iter().flatten())
            .filter_map(parse_mcpworld_item),
    )
}

fn parse_mcpworld_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let id = json_text(item, "id", 120)?;
    if !is_market_slug(&id) {
        return None;
    }
    let name = json_text(item, "serverName", 96).unwrap_or_else(|| format!("MCP {id}"));
    let description = json_text(item, "description", 220).unwrap_or_else(|| "MCP World server.".into());
    let mut tags = json_string_array(item.get("labels"), 40);
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let stats = item.get("star").and_then(serde_json::Value::as_u64).map(|stars| format!("{stars} stars"));
    Some(SkillMarketItemResponse {
        id: format!("{MCPWORLD_SOURCE}:{id}"),
        source: MCPWORLD_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://www.mcpworld.com/zh/detail/{id}"),
        install_command: format!("mcp market add mcpworld:{id}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

// ---------------------------------------------------------------------------
// ClawHub plugins (API + HTML fallback)
// ---------------------------------------------------------------------------

pub(super) fn parse_clawhub_plugins(body: &str) -> Vec<SkillMarketItemResponse> {
    if let Ok(root) = serde_json::from_str::<serde_json::Value>(body) {
        let items = root.get("items").and_then(serde_json::Value::as_array);
        if let Some(items) = items {
            let parsed = ranked(items.iter().filter_map(parse_clawhub_plugin_api_item));
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }

    parse_clawhub_plugins_html(body)
}

fn parse_clawhub_plugin_api_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let canonical_name = json_text(item, "name", 160)?;
    let (owner, slug) = skillhub_canonical_owner_slug(&canonical_name).or_else(|| {
        let owner = json_text(item, "ownerHandle", 96)?;
        let slug = json_text(item, "runtimeId", 96)?;
        valid_owner_slug(&owner, &slug)
    })?;
    let name = json_text(item, "displayName", 96).unwrap_or_else(|| title_from_slug(&slug));
    let description = json_text(item, "summary", 220).unwrap_or_default();
    let mut tags = json_string_array(item.get("topics"), 40);
    tags.extend(json_string_array(item.get("categories"), 40));
    tags.extend(json_text(item, "family", 40));
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let stats = market_count_stats(
        item.get("stats"),
        &[("downloads", "downloads"), ("installs", "installs"), ("stars", "stars")],
    );

    Some(SkillMarketItemResponse {
        id: format!("{CLAWHUB_PLUGINS_SOURCE}:{owner}/{slug}"),
        source: CLAWHUB_PLUGINS_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://clawhub.ai/{owner}/plugins/{slug}"),
        install_command: format!("openclaw plugins install clawhub:@{owner}/{slug}"),
        tags,
        audience_tags,
        scenario_tags,
        stats,
    })
}

fn parse_clawhub_plugins_html(html: &str) -> Vec<SkillMarketItemResponse> {
    let mut seen = HashSet::new();
    let mut parsed = Vec::new();

    for (href, text) in market_anchors(html) {
        let Some(url) = market_url(CLAWHUB_PLUGINS_SOURCE, &href) else {
            continue;
        };
        let Some((owner, slug)) = clawhub_plugin_owner_slug(&url) else {
            continue;
        };
        let id = format!("{CLAWHUB_PLUGINS_SOURCE}:{owner}/{slug}");
        if !seen.insert(id.clone()) {
            continue;
        }

        let name = extract_clawhub_name(&text, &owner, &slug);
        let description = extract_clawhub_description(&text, &owner, &name);
        let stats = extract_stats(&text);
        let (tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
        let rank = parsed.len() + 1;
        parsed.push(SkillMarketItemResponse {
            id,
            source: CLAWHUB_PLUGINS_SOURCE.into(),
            rank,
            name,
            description,
            url: format!("https://clawhub.ai/{owner}/plugins/{slug}"),
            install_command: format!("openclaw plugins install clawhub:@{owner}/{slug}"),
            tags,
            audience_tags,
            scenario_tags,
            stats,
        });
        if parsed.len() >= MAX_MARKET_ITEMS_PER_SOURCE {
            break;
        }
    }

    parsed
}

// ---------------------------------------------------------------------------
// SkillHub expert packages
// ---------------------------------------------------------------------------

pub(super) fn parse_skillhub_packages(body: &str) -> Vec<SkillMarketItemResponse> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(items) = root.get("skillSets").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    ranked(items.iter().filter_map(parse_skillhub_package_item))
}

fn parse_skillhub_package_item(item: &serde_json::Value) -> Option<SkillMarketItemResponse> {
    let slug = json_text(item, "slug", 96)?;
    if !is_market_slug(&slug) {
        return None;
    }
    let name = json_text(item, "displayName", 96)
        .or_else(|| json_text(item, "displayNameEn", 96))
        .unwrap_or_else(|| title_from_slug(&slug));
    let description = json_text(item, "summary", 220)
        .or_else(|| json_text(item, "summaryEn", 220))
        .unwrap_or_else(|| "SkillHub expert package.".into());
    let skill_slugs = json_string_array(item.get("skillSlugs"), 40);
    let mut tags = skill_slugs.clone();
    tags.extend(json_text(item, "scene", 40));
    tags.extend(json_text(item, "subScene", 40));
    let (inferred_tags, audience_tags, scenario_tags) = infer_market_tags(&format!("{name} {description}"));
    tags.extend(inferred_tags);
    dedup_strings(&mut tags);
    let skill_count = item
        .get("skillCount")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(skill_slugs.len() as u64);
    Some(SkillMarketItemResponse {
        id: format!("{SKILLHUB_PACKAGES_SOURCE}:{slug}"),
        source: SKILLHUB_PACKAGES_SOURCE.into(),
        rank: 0,
        name,
        description,
        url: format!("https://skillhub.cn/skillspackage/{slug}"),
        install_command: format!("skillhub package add {slug}"),
        tags,
        audience_tags,
        scenario_tags,
        stats: Some(format!("{skill_count} skills")),
    })
}

// ---------------------------------------------------------------------------
// HTML anchor & URL helpers
// ---------------------------------------------------------------------------

fn market_anchors(html: &str) -> Vec<(String, String)> {
    ANCHOR_RE
        .captures_iter(html)
        .filter_map(|cap| {
            let href = cap.get(1)?.as_str().trim();
            let inner = cap.get(2)?.as_str();
            let text = clean_market_text(&strip_html_tags(inner), 360);
            if href.is_empty() || text.is_empty() {
                return None;
            }
            Some((href.to_string(), text))
        })
        .collect()
}

fn market_url(source: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.starts_with('#') || href.starts_with("mailto:") || href.starts_with("javascript:") {
        return None;
    }
    let url = if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if href.starts_with('/') {
        match source {
            CLAWHUB_SOURCE | CLAWHUB_PLUGINS_SOURCE => format!("https://clawhub.ai{href}"),
            SKILLHUB_SOURCE => format!("https://www.skills.sh{href}"),
            LOOPHUB_SOURCE => format!("https://hub.cocoloop.cn{href}"),
            SKILLHUB_MCP_SOURCE | SKILLHUB_PACKAGES_SOURCE => format!("https://skillhub.cn{href}"),
            MCPWORLD_SOURCE => format!("https://www.mcpworld.com{href}"),
            _ => return None,
        }
    } else {
        return None;
    };

    match source {
        CLAWHUB_SOURCE if url.starts_with("https://clawhub.ai/") => Some(url),
        CLAWHUB_PLUGINS_SOURCE if url.starts_with("https://clawhub.ai/") => Some(url),
        SKILLHUB_SOURCE if url.starts_with("https://www.skills.sh/") || url.starts_with("https://skills.sh/") => {
            Some(url.replacen("https://skills.sh/", "https://www.skills.sh/", 1))
        }
        LOOPHUB_SOURCE if url.starts_with("https://hub.cocoloop.cn/") => Some(url),
        SKILLHUB_MCP_SOURCE | SKILLHUB_PACKAGES_SOURCE if url.starts_with("https://skillhub.cn/") => Some(url),
        MCPWORLD_SOURCE if url.starts_with("https://www.mcpworld.com/") => Some(url),
        _ => None,
    }
}

fn clawhub_owner_slug(url: &str) -> Option<(String, String)> {
    let segments = market_path_segments(url, "https://clawhub.ai")?;
    let reserved = ["skills", "plugins", "docs", "about", "login", "sign-in", "search"];
    if segments.len() >= 3 && segments.get(1).is_some_and(|s| s == "skills") {
        return valid_owner_slug(&segments[0], &segments[2]);
    }
    if segments.len() == 2 && !reserved.contains(&segments[0].as_str()) && !reserved.contains(&segments[1].as_str()) {
        return valid_owner_slug(&segments[0], &segments[1]);
    }
    None
}

fn skillhub_owner_slug(url: &str) -> Option<(String, String)> {
    let segments = market_path_segments(url, "https://www.skills.sh")?;
    if segments.len() >= 3 && segments.get(1).is_some_and(|s| s == "skills") {
        return valid_owner_slug(&segments[0], &segments[2]);
    }
    None
}

fn clawhub_plugin_owner_slug(url: &str) -> Option<(String, String)> {
    let segments = market_path_segments(url, "https://clawhub.ai")?;
    if segments.len() >= 3 && segments.get(1).is_some_and(|s| s == "plugins") {
        return valid_owner_slug(&segments[0], &segments[2]);
    }
    None
}

fn market_path_segments(url: &str, origin: &str) -> Option<Vec<String>> {
    let rest = url.strip_prefix(origin)?;
    let path = rest
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_matches('/');
    if path.is_empty() {
        return None;
    }
    Some(path.split('/').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
}

fn valid_owner_slug(owner: &str, slug: &str) -> Option<(String, String)> {
    if is_market_slug(owner) && is_market_slug(slug) {
        Some((owner.to_string(), slug.to_string()))
    } else {
        None
    }
}

fn skillhub_canonical_owner_slug(canonical_name: &str) -> Option<(String, String)> {
    let value = canonical_name.trim().trim_start_matches('@');
    let mut parts = value.split('/');
    let owner = parts.next()?;
    let slug = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    valid_owner_slug(owner, slug)
}

// ---------------------------------------------------------------------------
// Slug & id helpers
// ---------------------------------------------------------------------------

/// Validate a market slug/id used in URLs and temp-file names. Only ASCII
/// alphanumerics plus internal `-`/`_`/`.` separators are allowed; the value
/// must start and end alphanumeric (so `..`, `.git`-style names, and
/// leading/trailing separators are rejected), be non-empty, contain no `..`
/// run, and be at most 96 chars.
pub(crate) fn is_market_slug(value: &str) -> bool {
    let bytes = value.as_bytes();
    !value.is_empty()
        && value.len() <= 96
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && !value.contains("..")
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Strip a `"{source}:"` prefix from a market item id.
pub(crate) fn market_ref_suffix(id: &str, source: &str) -> Option<String> {
    id.strip_prefix(&format!("{source}:"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Last non-empty path segment of a URL (query/fragment stripped).
pub(crate) fn last_url_segment(url: &str) -> Option<String> {
    url.split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_matches('/')
        .rsplit('/')
        .next()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

// ---------------------------------------------------------------------------
// JSON extraction helpers
// ---------------------------------------------------------------------------

/// Read a string field, whitespace-collapsed and capped at `max_chars`.
pub(crate) fn json_text(item: &serde_json::Value, key: &str, max_chars: usize) -> Option<String> {
    item.get(key)
        .and_then(serde_json::Value::as_str)
        .map(|value| clean_market_text(value, max_chars))
        .filter(|value| !value.is_empty())
}

/// Read a string field verbatim (whitespace preserved), capped at
/// `max_chars`. Logs a warning when the cap actually truncates — package
/// instructions silently losing their tail would be hard to diagnose.
pub(crate) fn json_text_preserve(item: &serde_json::Value, key: &str, max_chars: usize) -> Option<String> {
    let value = item.get(key)?.as_str()?.trim();
    if value.is_empty() {
        return None;
    }
    let truncated: String = value.chars().take(max_chars).collect();
    if truncated.chars().count() < value.chars().count() {
        tracing::warn!(
            key,
            max_chars,
            original_chars = value.chars().count(),
            "market text field truncated to the char cap"
        );
    }
    Some(truncated)
}

pub(crate) fn json_string_array(value: Option<&serde_json::Value>, max_chars: usize) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(|value| clean_market_text(value, max_chars))
        .filter(|value| !value.is_empty())
        .collect()
}

fn market_count_stats(value: Option<&serde_json::Value>, fields: &[(&str, &str)]) -> Option<String> {
    let value = value?;
    let stats = fields
        .iter()
        .filter_map(|(key, label)| {
            let count = value
                .get(*key)
                .and_then(|value| {
                    value
                        .as_u64()
                        .or_else(|| value.as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n as u64))
                        .or_else(|| value.as_str().and_then(|s| s.parse::<u64>().ok()))
                })?;
            Some(format!("{count} {label}"))
        })
        .collect::<Vec<_>>();
    if stats.is_empty() {
        None
    } else {
        Some(stats.join(" · "))
    }
}

// ---------------------------------------------------------------------------
// Text extraction helpers
// ---------------------------------------------------------------------------

fn extract_clawhub_name(text: &str, owner: &str, slug: &str) -> String {
    let owner_marker = format!("@ {owner}");
    let before_owner = text
        .split(&owner_marker)
        .next()
        .unwrap_or(text)
        .split('@')
        .next()
        .unwrap_or(text);
    let candidate = clean_market_text(before_owner.trim_matches(|c: char| c == '#' || c.is_ascii_digit()), 80);
    if candidate.len() >= 2 {
        candidate
    } else {
        title_from_slug(slug)
    }
}

fn extract_clawhub_description(text: &str, owner: &str, name: &str) -> String {
    let owner_marker = format!("@ {owner}");
    let tail = text.split(&owner_marker).nth(1).unwrap_or(text);
    let cleaned = strip_known_stats(tail);
    let cleaned = clean_market_text(&cleaned.replace(name, ""), 180);
    if cleaned.len() >= 12 {
        cleaned
    } else {
        "Trending ClawHub skill package.".into()
    }
}

fn extract_skillhub_name(text: &str, owner: &str, slug: &str) -> String {
    let repo_marker = format!("{owner}/skills");
    let before_repo = text.split(&repo_marker).next().unwrap_or(text);
    let candidate = clean_market_text(
        before_repo.trim_matches(|c: char| c == '#' || c.is_ascii_digit() || c == '.'),
        80,
    );
    if candidate.len() >= 2 && !candidate.eq_ignore_ascii_case("skill") {
        candidate
    } else {
        title_from_slug(slug)
    }
}

fn extract_skillhub_description(text: &str, owner: &str, name: &str, stats: Option<&str>) -> String {
    let without_stats = stats.map_or_else(|| text.to_string(), |s| text.replace(s, ""));
    let without_repo = without_stats.replace(&format!("{owner}/skills"), "");
    let cleaned = clean_market_text(&without_repo.replace(name, ""), 180);
    if cleaned.len() >= 18 {
        cleaned
    } else {
        format!("Ranked SkillHub skill from {owner}/skills.")
    }
}

fn extract_stats(text: &str) -> Option<String> {
    let mut matches = STATS_CAPTURE_RE
        .captures_iter(text)
        .filter_map(|cap| cap.get(1).map(|m| clean_market_text(m.as_str(), 40)))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    matches.dedup();
    matches.last().cloned()
}

fn strip_known_stats(text: &str) -> String {
    STATS_STRIP_RE.replace_all(text, " ").to_string()
}

fn strip_html_tags(html: &str) -> String {
    HTML_TAG_RE.replace_all(html, " ").to_string()
}

/// Decode common HTML entities, collapse whitespace/control runs to single
/// spaces, and cap the result at `max_chars` chars.
pub(crate) fn clean_market_text(text: &str, max_chars: usize) -> String {
    let decoded = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    let mut out = String::new();
    let mut last_was_space = false;
    for ch in decoded.chars() {
        let is_space = ch.is_whitespace() || ch.is_control();
        if is_space {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(ch);
            last_was_space = false;
        }
        if out.chars().count() >= max_chars {
            break;
        }
    }
    out.trim().to_string()
}

pub(crate) fn title_from_slug(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Tag inference
// ---------------------------------------------------------------------------

fn infer_market_tags(text: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let lower = text.to_ascii_lowercase();
    let mut audience = Vec::new();
    let mut scenario = Vec::new();

    if contains_any(&lower, &["code", "github", "git", "api", "cli", "npm", "python", "typescript", "developer"]) {
        audience.push("developer".to_string());
        scenario.push("coding".to_string());
    }
    if contains_any(&lower, &["doc", "pdf", "word", "office", "excel", "sheet", "ppt", "slide"]) {
        audience.push("office".to_string());
        if contains_any(&lower, &["excel", "sheet", "spreadsheet"]) {
            scenario.push("spreadsheet".to_string());
        }
        if contains_any(&lower, &["ppt", "slide", "presentation"]) {
            scenario.push("presentation".to_string());
        }
        if contains_any(&lower, &["doc", "pdf", "word"]) {
            scenario.push("document".to_string());
        }
    }
    if contains_any(&lower, &["design", "image", "figma", "ui", "ux"]) {
        audience.push("designer".to_string());
        scenario.push("design".to_string());
    }
    if contains_any(&lower, &["research", "paper", "academic", "web search"]) {
        audience.push("student".to_string());
        scenario.push("research".to_string());
    }
    if contains_any(&lower, &["write", "blog", "copy", "content"]) {
        scenario.push("writing".to_string());
    }
    if contains_any(&lower, &["plan", "project", "task", "calendar"]) {
        scenario.push("planning".to_string());
    }
    if contains_any(&lower, &["social", "tweet", "x.com", "marketing"]) {
        audience.push("marketing".to_string());
        scenario.push("social".to_string());
    }
    if contains_any(&lower, &["setup", "install", "configure", "config"]) {
        scenario.push("setup".to_string());
    }

    dedup_strings(&mut audience);
    dedup_strings(&mut scenario);
    let mut tags = audience.clone();
    tags.extend(scenario.clone());
    dedup_strings(&mut tags);
    (tags, audience, scenario)
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

/// Case-sensitive first-occurrence dedup, preserving order.
pub(crate) fn dedup_strings(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clawhub_rankings_extracts_safe_install_command() {
        let html = r#"
          <a href="/pskoett/skills/self-improving-agent">
            <span>self-improving agent</span>
            <span>@<!-- -->pskoett</span>
            <p>Captures discoveries from agent sessions into reusable skills.</p>
            <span>468k installs</span>
          </a>
        "#;
        let items = parse_clawhub_rankings(html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, CLAWHUB_SOURCE);
        assert_eq!(items[0].install_command, "openclaw skills install @pskoett/self-improving-agent");
        assert!(items[0].url.starts_with("https://clawhub.ai/"));
    }

    #[test]
    fn parse_clawhub_rankings_extracts_convex_api_items() {
        let body = r#"{
          "status": "success",
          "value": {
            "page": [{
              "ownerHandle": "steipete",
              "skill": {
                "displayName": "Github",
                "slug": "github",
                "summary": "Interact with GitHub using the gh CLI.",
                "stats": { "downloads": 194199, "installs": 7620, "stars": 659 },
                "topics": ["GitHub"],
                "categories": ["integrations"]
              }
            }]
          }
        }"#;
        let items = parse_clawhub_rankings(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, CLAWHUB_SOURCE);
        assert_eq!(items[0].url, "https://clawhub.ai/steipete/skills/github");
        assert_eq!(items[0].install_command, "openclaw skills install @steipete/github");
        assert_eq!(items[0].stats.as_deref(), Some("194199 downloads · 7620 installs · 659 stars"));
    }

    #[test]
    fn parse_skillhub_rankings_extracts_skills_command() {
        let html = r#"
          <a href="/vercel-labs/skills/find-skills">
            <span>find-skills</span>
            <span>vercel-labs/skills</span>
            <span>2.5M installs</span>
          </a>
        "#;
        let items = parse_skillhub_rankings(html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, SKILLHUB_SOURCE);
        assert_eq!(
            items[0].install_command,
            "npx skills add https://github.com/vercel-labs/skills --skill find-skills"
        );
    }

    #[test]
    fn parse_skillhub_rankings_extracts_api_items() {
        let body = r#"{
          "code": 0,
          "data": {
            "skills": [{
              "name": "web-tools-guide",
              "slug": "web-tools-guide",
              "description_zh": "上网检索工具指南",
              "downloads": 196303,
              "installs": 3459,
              "stars": 168,
              "namespace": {
                "canonicalName": "@user_ec205dbb/web-tools-guide",
                "handle": "user_ec205dbb",
                "publicSlug": "web-tools-guide"
              },
              "labels": { "requires_api_key": "false" },
              "subCategories": [{ "key": "knowledge-retrieval", "name": "信息检索" }]
            }]
          }
        }"#;
        let items = parse_skillhub_rankings(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, SKILLHUB_SOURCE);
        assert_eq!(items[0].url, "https://skillhub.cn/skills/user_ec205dbb/web-tools-guide");
        assert_eq!(items[0].install_command, "npx skills add @user_ec205dbb/web-tools-guide");
        assert!(items[0].tags.contains(&"no_api_key".into()));
    }

    #[test]
    fn parse_loophub_rankings_extracts_download_package() {
        let body = r#"{
          "code": 0,
          "data": {
            "items": [{
              "id": 12277,
              "author": "pskoett",
              "name": "Self-Improving Agent",
              "subtitle": "Keeps lessons",
              "brief": "Records fixes and best practices.",
              "downloads": "419.4k",
              "category": "productivity",
              "security_level": "A",
              "download_url": "https://dl.cocoloop.cn/bss/skills/pskoett-self-improving-agent.zip"
            }]
          }
        }"#;
        let items = parse_loophub_rankings(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, LOOPHUB_SOURCE);
        assert_eq!(items[0].id, "loophub:12277");
        assert_eq!(
            items[0].install_command,
            "loophub skill download https://dl.cocoloop.cn/bss/skills/pskoett-self-improving-agent.zip"
        );
    }

    #[test]
    fn parse_skillhub_mcp_rankings_extracts_market_add_handle() {
        let body = r#"{
          "items": [{
            "slug": "playwright",
            "name": "Playwright MCP",
            "summary": "Browser automation server",
            "category": "browser",
            "tags": ["automation"],
            "stats": { "downloads": 12, "installs": 8 }
          }]
        }"#;
        let items = parse_skillhub_mcp_rankings(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, SKILLHUB_MCP_SOURCE);
        assert_eq!(items[0].url, "https://skillhub.cn/mcp/playwright");
        assert_eq!(items[0].install_command, "mcp market add skillhub:playwright");
    }

    #[test]
    fn parse_mcpworld_rankings_extracts_detail_url() {
        let body = r#"{
          "code": 0,
          "data": {
            "mcpList": [{
              "servers": [{
                "id": "c7897f8abf0350fbbf5a7fccc3e79bb8",
                "serverName": "Playwright MCP",
                "description": "Browser automation",
                "star": 68302,
                "labels": ["local", "browser"]
              }]
            }]
          }
        }"#;
        let items = parse_mcpworld_rankings(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, MCPWORLD_SOURCE);
        assert_eq!(
            items[0].url,
            "https://www.mcpworld.com/zh/detail/c7897f8abf0350fbbf5a7fccc3e79bb8"
        );
    }

    #[test]
    fn parse_clawhub_plugins_extracts_openclaw_plugin_command() {
        let html = r#"
          <a href="/openclaw/plugins/whatsapp">
            <span>WhatsApp MCP Plugin</span>
            <span>@<!-- -->openclaw</span>
            <p>WhatsApp chat integration.</p>
            <code>openclaw plugins install clawhub:@openclaw/whatsapp</code>
          </a>
        "#;
        let items = parse_clawhub_plugins(html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, CLAWHUB_PLUGINS_SOURCE);
        assert_eq!(
            items[0].install_command,
            "openclaw plugins install clawhub:@openclaw/whatsapp"
        );
    }

    #[test]
    fn parse_clawhub_plugins_extracts_api_items() {
        let body = r#"{
          "items": [{
            "categories": ["channels"],
            "displayName": "WhatsApp",
            "family": "code-plugin",
            "name": "@openclaw/whatsapp",
            "ownerHandle": "openclaw",
            "runtimeId": "whatsapp",
            "stats": { "downloads": 160061, "installs": 597, "stars": 0 },
            "summary": "OpenClaw WhatsApp channel plugin for WhatsApp Web chats.",
            "topics": ["WhatsApp"]
          }],
          "totalCount": 1609
        }"#;
        let items = parse_clawhub_plugins(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, CLAWHUB_PLUGINS_SOURCE);
        assert_eq!(items[0].url, "https://clawhub.ai/openclaw/plugins/whatsapp");
        assert_eq!(
            items[0].install_command,
            "openclaw plugins install clawhub:@openclaw/whatsapp"
        );
        assert!(items[0].stats.as_deref().unwrap_or_default().contains("downloads"));
    }

    #[test]
    fn parse_skillhub_packages_extracts_expert_package() {
        let body = r#"{
          "skillSets": [{
            "slug": "tech-test-automation",
            "displayName": "Test Automation",
            "summary": "End-to-end automated testing workflow.",
            "skillCount": 6,
            "skillSlugs": ["superpowers-tdd", "test-case-generator"]
          }],
          "total": 1
        }"#;
        let items = parse_skillhub_packages(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, SKILLHUB_PACKAGES_SOURCE);
        assert_eq!(items[0].install_command, "skillhub package add tech-test-automation");
        assert!(items[0].stats.as_deref().unwrap_or_default().contains("6 skills"));
    }

    #[test]
    fn is_market_slug_requires_alphanumeric_bounds() {
        // Valid slugs: internal separators are fine.
        for good in ["superpowers-tdd", "a.b-c_d", "a", "x1", "playwright", "user_ec205dbb"] {
            assert!(is_market_slug(good), "{good:?} must be accepted");
        }

        // Traversal shapes, dot-only strings, boundary separators, path
        // separators, and `..` runs are all rejected.
        for bad in ["", ".", "..", "...", "../x", "x/..", "owner/skill", "a..b", ".hidden", "trailing.", "-lead", "trail-", "_x", "x_"] {
            assert!(!is_market_slug(bad), "{bad:?} must be rejected");
        }

        // Length cap: 96 ok, 97 rejected.
        let max = "a".repeat(96);
        assert!(is_market_slug(&max));
        let over = "a".repeat(97);
        assert!(!is_market_slug(&over));
    }

    #[test]
    fn json_text_preserve_keeps_whitespace_and_caps_chars() {
        let value = serde_json::json!({ "content": "line one\n\nline two" });
        assert_eq!(
            json_text_preserve(&value, "content", 1000).as_deref(),
            Some("line one\n\nline two")
        );
        // Truncation applies the char cap (and logs a warning).
        assert_eq!(json_text_preserve(&value, "content", 4).as_deref(), Some("line"));
        // Missing / empty fields yield None.
        assert!(json_text_preserve(&value, "missing", 10).is_none());
        let blank = serde_json::json!({ "content": "   " });
        assert!(json_text_preserve(&blank, "content", 10).is_none());
    }
}
