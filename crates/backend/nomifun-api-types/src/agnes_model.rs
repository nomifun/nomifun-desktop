//! Exact Agnes model contracts, shared by catalog suggestions and executors.
//! Availability still comes from the live catalog and actual API responses.
//! Sources: https://wiki.agnes-ai.com/zh-Hans/docs/agnes-image-25-flash
//! and https://wiki.agnes-ai.com/zh-Hans/docs/agnes-video-25-flash (2026-10-08).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgnesModelContract {
    Chat,
    Image,
    Video { flash: bool },
    RetiredVideo,
}

pub fn agnes_model_contract(model: &str) -> Option<AgnesModelContract> {
    use AgnesModelContract::*;
    match model.trim().to_ascii_lowercase().as_str() {
        "agnes-2.0-flash" | "agnes-2.5-flash" | "agnes-3.0-flash"
        | "agnes-2.5-pro" => Some(Chat),
        "agnes-image-2.0-flash" | "agnes-image-2.1-flash" | "agnes-image-2.5-flash" => Some(Image),
        "agnes-video-2.5" => Some(Video { flash: false }),
        "agnes-video-2.5-flash" => Some(Video { flash: true }),
        "agnes-video-v2.0" => Some(RetiredVideo),
        _ => None,
    }
}

/// Confirmed retirement is distinct from a temporarily unavailable API or an
/// unknown future model. Only the owning provider's documented ID is hidden.
/// https://wiki.agnes-ai.com/zh-Hans/docs/agnes-video-v20
pub fn is_retired_provider_model(platform: &str, model: &str) -> bool {
    platform.trim().eq_ignore_ascii_case("agnes")
        && agnes_model_contract(model) == Some(AgnesModelContract::RetiredVideo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retirement_is_exact_provider_scoped_and_not_an_active_model_allowlist() {
        assert!(is_retired_provider_model("agnes", "agnes-video-v2.0"));
        assert!(is_retired_provider_model(" Agnes ", " AGNES-VIDEO-V2.0 "));
        for model in ["agnes-video-2.5", "agnes-video-2.5-flash", "agnes-video-future",
            "agnes-image-2.0-flash", "agnes-2.0-flash"] {
            assert!(!is_retired_provider_model("agnes", model), "{model}");
        }
        assert!(!is_retired_provider_model("custom", "agnes-video-v2.0"));
    }
}
