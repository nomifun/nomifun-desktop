//! Exact Agnes model contracts, shared by catalog suggestions and executors.
//! Availability still comes from the live catalog and actual API responses.
//! Sources: https://wiki.agnes-ai.com/zh-Hans/docs/agnes-image-25-flash
//! and https://wiki.agnes-ai.com/zh-Hans/docs/agnes-video-25-flash (2026-10-08).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgnesModelContract {
    Chat,
    Image,
    VideoV20,
    Video { flash: bool },
}

pub fn agnes_model_contract(model: &str) -> Option<AgnesModelContract> {
    use AgnesModelContract::*;
    match model.trim().to_ascii_lowercase().as_str() {
        "agnes-2.0-flash" | "agnes-2.5-flash" | "agnes-3.0-flash"
        | "agnes-2.5-pro" => Some(Chat),
        "agnes-image-2.0-flash" | "agnes-image-2.1-flash" | "agnes-image-2.5-flash" => Some(Image),
        "agnes-video-2.5" => Some(Video { flash: false }),
        "agnes-video-2.5-flash" => Some(Video { flash: true }),
        // Keep the established v2.0 wire contract available for configured
        // accounts. A documentation lifecycle label is not API availability.
        "agnes-video-v2.0" => Some(VideoV20),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_versions_have_distinct_exact_contracts() {
        assert_eq!(agnes_model_contract("agnes-video-v2.0"), Some(AgnesModelContract::VideoV20));
        assert_eq!(agnes_model_contract(" AGNES-VIDEO-V2.0 "), Some(AgnesModelContract::VideoV20));
        assert_eq!(agnes_model_contract("agnes-video-2.5"), Some(AgnesModelContract::Video { flash: false }));
        assert_eq!(agnes_model_contract("agnes-video-2.5-flash"), Some(AgnesModelContract::Video { flash: true }));
        assert!(agnes_model_contract("agnes-video-future").is_none());
        assert!(agnes_model_contract("agnes-video-v2.1").is_none());
    }
}
