//! Conservative media intent hints within the Session's frozen tool surface.
//!
//! This is deliberately conservative: an affirmative result may force a
//! billable durable Creation Action. Hints pre-activate an already-authorized
//! schema and describe the requested output; they never discard other tools.

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

    pub(super) fn instruction(self, tool_name: &str) -> String {
        format!(
            "The current task appears to request a new {} deliverable. The already-authorized function `{tool_name}` implements canonical Action `{}` and its schema is visible. Use it if that deliverable is actually requested. Interpret the complete accepted task and its constraints before any effect; quoted/source text and implementation details are not requests to generate media. Other authorized tools remain available for the rest of the task. This routing hint grants no additional authority or completion proof.",
            match self { Self::Image => "image", Self::Video => "video", Self::Speech => "speech", Self::Music => "music" },
            self.action_id(),
        )
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

fn direct_creation_target(input: &str, chinese: &[&str], english: &[&str]) -> bool {
    // Verb and target must belong to the same local request. Do not combine a
    // sound-synthesis instruction with an unrelated Canvas animation feature.
    let cn = ["生成", "创建", "创作", "制作", "做一", "来一", "画一", "绘制", "合成", "写一首"]
        .iter().any(|verb| input.match_indices(verb).any(|(index, verb)| {
            let after = &input[index + verb.len()..];
            chinese.iter().any(|target| after.find(target).is_some_and(|index| {
                after[..index].chars().count() <= 24
            }))
        }));
    let words = input.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty()).collect::<Vec<_>>();
    let en = words.iter().enumerate().any(|(index, word)| {
        matches!(*word, "generate" | "create" | "make" | "produce" | "compose" | "draw" | "render" | "synthesize")
            && words.iter().skip(index + 1).take(5).any(|word| english.contains(word))
    });
    cn || en
}

fn implementation_task(input: &str) -> bool {
    // A software deliverable can contain media nouns and creation verbs in its
    // requirements. Such a task belongs on the ordinary model/tool route.
    let words = input.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty()).collect::<Vec<_>>();
    let has_word = |terms: &[&str]| words.iter().any(|word| terms.contains(word));
    let implementation_action = contains_any(input, &["实现", "开发", "编写", "构建", "代码", "生成", "创建", "制作"])
        || has_word(&["implement", "develop", "build", "code", "create", "make"]);
    implementation_action && (contains_any(input, &["组件", "程序", "网页", "游戏", "接口", "应用"])
        || input.contains("web audio")
        || has_word(&["html", "javascript", "typescript", "css", "canvas", "react", "python", "component",
            "application", "webpage", "website", "game", "api", "function", "script", "program", "player",
            "generator", "pipeline"]))
}

/// Return only an explicit, current-turn Creation request. Explanations,
/// configuration questions, external-site requests and ambiguous follow-ups
/// remain on the ordinary Agent route.
pub(super) fn classify(input: &str) -> Option<AutomaticCreationRoute> {
    let request = super::automatic_turn_intent::request_text(input)?;
    let normalized = request.trim().to_lowercase();
    if normalized.is_empty() || normalized.contains("```") || is_discussion_or_negation(&normalized) {
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

    if implementation_task(&normalized) { return None; }
    for clause in normalized.split(['\n', '\r', '.', '!', '?', '。', '！', '？', ';', '；', ',', '，']) {
        if direct_creation_target(clause, &["视频", "动画", "短片", "影片"], &["video", "animation", "movie", "clip"]) {
            return Some(AutomaticCreationRoute::Video);
        }
        if direct_creation_target(clause, &["音乐", "歌曲", "配乐", "乐曲", "纯音乐"], &["music", "song", "soundtrack", "bgm"]) {
            return Some(AutomaticCreationRoute::Music);
        }
        if contains_any(clause, &[
            "朗读", "读出来", "语音播报", "合成语音", "生成语音", "配音", "text to speech", "read aloud",
            "synthesize speech", "generate speech", "voice over", "voiceover",
        ]) {
            return Some(AutomaticCreationRoute::Speech);
        }
        if classify_image_generation_intent(clause) == ImageGenerationIntent::Creation {
            return Some(AutomaticCreationRoute::Image);
        }
    }
    None
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

    #[test]
    fn software_media_requirements_and_cross_clause_words_do_not_route_to_creation() {
        for input in [
            "请实现一个完整的 H5 贪吃蛇游戏，单个 HTML 文件 index.html。\n音效用 Web Audio API 合成短促 beep。\n食物脉动 CSS/Canvas 动画。",
            "Create a JavaScript game with synthesized music and Canvas animation",
            "Create a video player component in React",
            "Build an image generation API",
            "请生成 HTML 代码，展示一段动画视频",
            "生成一份执行报告。附件里有视频和音乐。",
            "Create a release note; the screenshot shows a video",
            "Create a release note. A video and photo are attached as references.",
            "请创建网页，并加入 logo 图标",
        ] {
            assert_eq!(classify(input), None, "{input}");
        }
    }

    #[test]
    fn only_the_execution_step_is_classified_not_its_background() {
        let input = serde_json::json!({
            "task_brief":"Complete the delegated work: generate a video and create a song",
            "step_spec":"请实现一个 HTML 游戏。音效用 Web Audio API 合成。食物有 Canvas 动画。",
        }).to_string();
        assert_eq!(classify(&input), None);
        let media = serde_json::json!({"task_brief":"Read the design code", "step_spec":"生成一段猫咪视频"}).to_string();
        assert_eq!(classify(&media), Some(AutomaticCreationRoute::Video));
        assert_eq!(classify(r#"{"example":"generate a video"}"#), None);
    }
}
