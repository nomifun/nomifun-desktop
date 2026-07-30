//! Constant rule table for the Nomi chat-path platform mapping.
//!
//! ONE place that says, per DB `providers.platform` value, (a) which nomi
//! provider implementation serves the chat request and (b) how the configured
//! `base_url` is turned into the request URL. Before P2 Task 6 these lived as
//! scattered special-cases inside `factory/nomi.rs` (`map_nomi_provider`'s
//! match + `resolve_nomi_url_and_compat`'s if-ladder and the
//! `uses_configured_openai_chat_base` whitelist); the byte-exact behavior is
//! locked by `factory::nomi::platform_chat_snapshot` (220-row matrix) — extend
//! the table, then extend the snapshot, never the other way around.
//!
//! Deliberately NOT in the table:
//! - The `is_full_url` provider flag: it bypasses every platform rule (URL is
//!   used verbatim, `api_path = ""`) and is checked before the lookup in
//!   `resolve_nomi_url_and_compat`.
//! - The new-api per-model protocol override: it is keyed off the MODEL row's
//!   `protocol`, not the platform, and stays verbatim in `map_nomi_provider`.
//! - The `api.openai.com` → `max_completion_tokens` compat override: it is
//!   host-gated (and gated on the MAPPED provider being "openai"), so it
//!   belongs to the default URL rule, not to any platform row.
//! - A per-platform compat column: every compat override today is fully
//!   determined by the URL rule (`api_path`) or by the host rule above; a
//!   `compat` field would be `None` on every row (dead config). Add one only
//!   when a platform actually needs a platform-constant override.

/// How a platform's configured `base_url` becomes the chat request base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UrlRule {
    /// Default row: strip trailing `/`s and a trailing `/v1` — nomi appends
    /// its own suffix (`/v1/chat/completions`, `/v1/messages`). When the
    /// mapped provider is `openai` and the host is `api.openai.com`, the
    /// `max_tokens_field = max_completion_tokens` compat override applies.
    StripTrailingV1,
    /// Gemini OpenAI-compat endpoint: append `/v1beta/openai` to the trimmed
    /// base and pin `api_path` to `/chat/completions`.
    GeminiOpenAiCompat,
    /// OpenAI-compatible vendors with a NONSTANDARD version segment
    /// (`/api/v3`, `/v2`, `/api/paas/v4`, `/step_plan/v1`, ...): keep the
    /// configured base verbatim (minus trailing `/`) and pin `api_path` to
    /// `/chat/completions` so nomi does not prepend `/v1`.
    ConfiguredChatBase,
}

/// One row of the chat-path platform table.
pub(crate) struct PlatformChatRule {
    /// nomi provider implementation: `"openai"`, `"anthropic"`, `"bedrock"`
    /// or `"vertex"` (the strings `nomi_providers::create_provider` matches).
    pub nomi_provider: &'static str,
    /// URL construction rule for the chat endpoint.
    pub url_rule: UrlRule,
}

/// Platforms that deviate from [`DEFAULT_CHAT_RULE`]. Everything else —
/// `custom`, `new-api`, `deepseek`, `moonshot-*`, `siliconflow`, `minimax*`,
/// `mimo*`, `dashscope` (compatible-mode), `hunyuan`, `lingyi`,
/// `nomifun-free-model`, unknown platforms — is an OpenAI-compatible `/v1`
/// endpoint served by the default row.
pub(crate) static PLATFORM_CHAT_RULES: &[(&str, PlatformChatRule)] = &[
    // Non-OpenAI provider implementations.
    (
        "anthropic",
        PlatformChatRule { nomi_provider: "anthropic", url_rule: UrlRule::StripTrailingV1 },
    ),
    (
        "bedrock",
        PlatformChatRule { nomi_provider: "bedrock", url_rule: UrlRule::StripTrailingV1 },
    ),
    (
        "gemini-vertex-ai",
        PlatformChatRule { nomi_provider: "vertex", url_rule: UrlRule::StripTrailingV1 },
    ),
    // Gemini official API, spoken through its OpenAI-compat surface.
    (
        "gemini",
        PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::GeminiOpenAiCompat },
    ),
    // Domestic OpenAI-compatible vendors whose version path is not `/v1`:
    // the configured base is already the full prefix, only
    // `/chat/completions` is appended.
    ("ark", PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::ConfiguredChatBase }),
    (
        "ark-coding-plan",
        PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::ConfiguredChatBase },
    ),
    (
        "ark-agent-plan",
        PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::ConfiguredChatBase },
    ),
    (
        "stepfun",
        PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::ConfiguredChatBase },
    ),
    (
        "stepfun-plan",
        PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::ConfiguredChatBase },
    ),
    (
        "dashscope-coding",
        PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::ConfiguredChatBase },
    ),
    (
        "zhipu",
        PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::ConfiguredChatBase },
    ),
    (
        "glm-coding-plan",
        PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::ConfiguredChatBase },
    ),
    (
        "qianfan",
        PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::ConfiguredChatBase },
    ),
    (
        "qianfan-coding-plan",
        PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::ConfiguredChatBase },
    ),
];

/// Documented default row: any platform without an entry above is an
/// OpenAI-compatible endpoint at `<base>/v1` (nomi appends the version
/// segment itself, hence [`UrlRule::StripTrailingV1`]).
pub(crate) static DEFAULT_CHAT_RULE: PlatformChatRule =
    PlatformChatRule { nomi_provider: "openai", url_rule: UrlRule::StripTrailingV1 };

/// Table lookup with the default-row fallback.
pub(crate) fn platform_chat_rule(platform: &str) -> &'static PlatformChatRule {
    PLATFORM_CHAT_RULES
        .iter()
        .find(|(key, _)| *key == platform)
        .map(|(_, rule)| rule)
        .unwrap_or(&DEFAULT_CHAT_RULE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_keys_are_unique() {
        for (index, (key, _)) in PLATFORM_CHAT_RULES.iter().enumerate() {
            assert!(
                !PLATFORM_CHAT_RULES[index + 1..].iter().any(|(other, _)| other == key),
                "duplicate platform key in PLATFORM_CHAT_RULES: {key}"
            );
        }
    }

    #[test]
    fn providers_are_known_nomi_implementations() {
        for (key, rule) in PLATFORM_CHAT_RULES {
            assert!(
                matches!(rule.nomi_provider, "openai" | "anthropic" | "bedrock" | "vertex"),
                "unknown nomi provider for platform {key}: {}",
                rule.nomi_provider
            );
        }
    }

    #[test]
    fn unknown_platform_falls_back_to_default_row() {
        let rule = platform_chat_rule("some-future-platform");
        assert_eq!(rule.nomi_provider, "openai");
        assert_eq!(rule.url_rule, UrlRule::StripTrailingV1);
    }
}
