//! High-confidence, host-owned media intent routing for ordinary conversations.
//!
//! This is deliberately conservative: an affirmative result may force a
//! billable durable Creation Action, while `None` simply leaves the ordinary
//! Agent tool surface and model reasoning unchanged.

use nomifun_ai_agent::image_generation::{
    ImageGenerationIntent, classify_image_generation_intent,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AutomaticCreationRoute {
    Image,
    Video,
    Speech,
    Music,
}

impl AutomaticCreationRoute {
    pub(super) const fn action_id(self) -> &'static str {
        match self {
            Self::Image => "creation.media/image",
            Self::Video => "creation.media/video",
            Self::Speech => "creation.media/audio",
            Self::Music => "creation.media/music",
        }
    }

    pub(super) const fn instruction(self) -> &'static str {
        match self {
            Self::Image => "The user explicitly requested a new image now. Invoke the one advertised image Creation Action exactly once with the requested visual description. Do not substitute Browser, web research, code, SVG, or prose for the requested image.",
            Self::Video => "The user explicitly requested a new video now. Invoke the one advertised video Creation Action exactly once. Do not substitute an image, Browser, or prose for the requested video.",
            Self::Speech => "The user explicitly requested synthesized speech now. Invoke the one advertised speech Creation Action exactly once with the text to speak. Do not substitute a written answer for the requested audio.",
            Self::Music => "The user explicitly requested generated music now. Invoke the one advertised music Creation Action exactly once. Do not substitute speech, Browser, or prose for the requested music.",
        }
    }
}

fn contains_any(input: &str, values: &[&str]) -> bool {
    values.iter().any(|value| input.contains(value))
}

fn is_discussion_or_negation(input: &str) -> bool {
    contains_any(
        input,
        &[
            "不要生成", "不要创建", "不要制作", "不用生成", "无需生成", "别生成", "请勿生成",
            "只解释", "仅解释", "讨论", "分析", "比较", "对比", "方案", "能力", "路由", "配置模型",
            "don't generate", "do not generate", "don't create", "do not create", "without generating",
            "explain", "discuss", "compare", "architecture", "capability", "route", "configure",
        ],
    )
}

fn has_creation_verb(input: &str) -> bool {
    contains_any(
        input,
        &[
            "生成", "创建", "创作", "制作", "做一", "来一", "画一", "绘制", "合成", "写一首",
            "generate", "create", "make", "produce", "compose", "draw", "render", "synthesize",
        ],
    )
}

/// Return only an explicit, current-turn Creation request. Explanations,
/// configuration questions, external-site requests and ambiguous follow-ups
/// remain on the ordinary Agent route.
pub(super) fn classify(input: &str) -> Option<AutomaticCreationRoute> {
    let normalized = input.trim().to_lowercase();
    if normalized.is_empty() || is_discussion_or_negation(&normalized) {
        return None;
    }
    if contains_any(
        &normalized,
        &[
            "用浏览器", "通过浏览器", "网站生成", "第三方网站", "use the browser", "using a website",
            "third-party generator", "third party generator",
        ],
    ) {
        return None;
    }

    if has_creation_verb(&normalized)
        && contains_any(
            &normalized,
            &["视频", "动画", "短片", "影片", "video", "animation", "movie", "clip"],
        )
    {
        return Some(AutomaticCreationRoute::Video);
    }
    if has_creation_verb(&normalized)
        && contains_any(
            &normalized,
            &["音乐", "歌曲", "配乐", "乐曲", "纯音乐", "music", "song", "soundtrack", "bgm"],
        )
    {
        return Some(AutomaticCreationRoute::Music);
    }
    if contains_any(
        &normalized,
        &[
            "朗读", "读出来", "语音播报", "合成语音", "生成语音", "配音", "text to speech", "read aloud",
            "synthesize speech", "generate speech", "voice over", "voiceover",
        ],
    ) {
        return Some(AutomaticCreationRoute::Speech);
    }
    match classify_image_generation_intent(&normalized) {
        ImageGenerationIntent::Creation => Some(AutomaticCreationRoute::Image),
        ImageGenerationIntent::None
        | ImageGenerationIntent::ExplicitExternal
        | ImageGenerationIntent::Discussion => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_explicit_media_creation_in_chinese_and_english() {
        for (input, expected) in [
            ("生成一张水彩猫咪海报", AutomaticCreationRoute::Image),
            ("Create a cinematic video of ocean waves", AutomaticCreationRoute::Video),
            ("创作一首轻快的纯音乐", AutomaticCreationRoute::Music),
            ("请把这段话朗读出来", AutomaticCreationRoute::Speech),
        ] {
            assert_eq!(classify(input), Some(expected), "{input}");
        }
    }

    #[test]
    fn discussion_negation_and_external_execution_never_force_creation() {
        for input in [
            "解释一下生图路由怎么设计",
            "不要生成图片，只优化提示词",
            "用浏览器访问第三方网站生成一张图",
            "比较两个视频生成模型",
            "Implement this SVG icon in React",
        ] {
            assert_eq!(classify(input), None, "{input}");
        }
    }
}
