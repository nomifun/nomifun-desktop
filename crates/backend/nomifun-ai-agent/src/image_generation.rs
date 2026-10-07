//! Deterministic image-creation intent classification shared with the host.
//!
//! Durable media execution is owned by the unified Creation actions. This
//! module intentionally contains no alternate image tool, provider discovery,
//! output sink, or task lifecycle.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageGenerationIntent {
    None,
    Creation,
    ExplicitExternal,
    Discussion,
}

/// A conservative shortcut for the host-owned Creation router. Ambiguous text
/// stays on the ordinary Agent route.
pub fn classify_image_generation_intent(input: &str) -> ImageGenerationIntent {
    let normalized = input.trim().to_lowercase();
    if normalized.is_empty() {
        return ImageGenerationIntent::None;
    }
    if discusses_image_generation(&normalized) {
        return ImageGenerationIntent::Discussion;
    }
    if !names_image_generation(&normalized) {
        return ImageGenerationIntent::None;
    }
    if names_external_execution(&normalized) {
        ImageGenerationIntent::ExplicitExternal
    } else {
        ImageGenerationIntent::Creation
    }
}

const DECLINES_GENERATION: &[&str] = &[
    "不要生成图片",
    "不要生成图像",
    "不用生成图片",
    "无需生成图片",
    "别生成图片",
    "请勿生成图片",
    "不要画图",
    "不用画图",
    "别画图",
    "只解释提示词",
    "仅解释提示词",
    "don't generate an image",
    "don't generate images",
    "do not generate an image",
    "do not generate images",
    "without generating an image",
    "without generating images",
    "never generate an image",
    "don't create an image",
    "do not create an image",
    "just explain the prompt",
    "only explain the prompt",
];

const META_REQUESTS: &[&str] = &[
    "生图链路",
    "生图能力",
    "生图入口",
    "生图路由",
    "生图模型不可用",
    "配置生图模型",
    "启用生图模型",
    "生图任务判断",
    "生图参数提取",
    "图片生成能力",
    "图像生成能力",
    "图片生成方案",
    "图像生成方案",
    "生图方案",
    "图片生成系统",
    "图像生成系统",
    "生图系统",
    "图片生成服务",
    "图像生成服务",
    "生图服务",
    "图片生成工具",
    "图像生成工具",
    "生图工具",
    "图片生成流水线",
    "图像生成流水线",
    "图片生成教程",
    "图像生成教程",
    "image generation capability",
    "image generation route",
    "image generation chain",
    "image generation parameters",
    "configure image model",
    "image model unavailable",
    "image generation plan",
    "image generation workflow",
    "image generation tutorial",
    "image generation service",
    "image generation tool",
    "image generation pipeline",
    "image generation system",
    "image-generation service",
    "image-generation tool",
    "image-generation pipeline",
    "image-generation system",
    "image generator",
    "image component",
    "picture component",
    "logo component",
    "图片组件",
    "图像组件",
    "logo 组件",
];

fn discusses_image_generation(text: &str) -> bool {
    if DECLINES_GENERATION.iter().any(|term| text.contains(term))
        || META_REQUESTS.iter().any(|term| text.contains(term))
    {
        return true;
    }
    [
        "如何", "怎么", "为什么", "为何", "是否", "能否", "解释", "说明", "分析", "修复",
        "重构", "优化",
    ]
    .iter()
    .any(|term| text.contains(term))
        && ["生图", "图片生成", "图像生成", "image generation"]
            .iter()
            .any(|term| text.contains(term))
        && !["一张", "一幅", "一只", "一个头像", "一个图标"]
            .iter()
            .any(|term| text.contains(term))
}

fn names_image_generation(text: &str) -> bool {
    const NON_VISUAL_OBJECTS: &[&str] = &[
        "directory named image",
        "directory named images",
        "folder named image",
        "folder named images",
        "image directory",
        "images directory",
        "image folder",
        "images folder",
    ];
    if NON_VISUAL_OBJECTS.iter().any(|term| text.contains(term)) {
        return false;
    }
    const CN_STRONG: &[&str] = &[
        "生图",
        "文生图",
        "画一张",
        "画一幅",
        "画一个",
        "画个",
        "画只",
        "画一只",
        "画张",
        "画幅",
        "绘一张",
        "出一张图",
        "弄张图",
        "弄张配图",
        "再生成一张",
        "再画一张",
        "搜索网页生成",
        "搜索网站生成",
    ];
    const CN_ACTIONS: &[&str] = &[
        "生成", "创建", "创作", "绘制", "设计", "制作", "做", "画", "来", "给我", "弄",
    ];
    const CN_PRODUCTS: &[&str] = &[
        "图片", "图像", "插画", "海报", "照片", "头像", "壁纸", "封面", "图标", "漫画", "logo",
        "猫图", "配图", "效果图", "概念图",
    ];
    if CN_STRONG.iter().any(|term| text.contains(term)) {
        return true;
    }

    let words = ascii_words(text);
    let named_external_generator = (words.iter().any(|word| word == "canva")
        || text.contains("pollinations.ai"))
        && (["生成", "创作", "绘制"]
            .iter()
            .any(|action| text.contains(action))
            || ["generate", "create", "draw", "render"]
                .iter()
                .any(|action| words.iter().any(|word| word == action)));
    if named_external_generator {
        return true;
    }

    let chinese = CN_ACTIONS.iter().any(|term| text.contains(term))
        && CN_PRODUCTS.iter().any(|term| text.contains(term));
    let english_action = [
        "generate", "create", "draw", "render", "design", "make", "produce", "paint", "need",
        "want", "whip",
    ]
    .iter()
    .any(|term| words.iter().any(|word| word == term));
    let english_product = [
        "image",
        "images",
        "picture",
        "pictures",
        "photo",
        "photos",
        "poster",
        "illustration",
        "illustrations",
        "logo",
        "icon",
        "wallpaper",
        "avatar",
        "artwork",
        "graphic",
        "banner",
        "banners",
    ]
    .iter()
    .any(|term| words.iter().any(|word| word == term));
    chinese || (english_action && english_product)
}

fn names_external_execution(text: &str) -> bool {
    const CN_AFFIRMATIVE: &[&str] = &[
        "用浏览器",
        "使用浏览器",
        "通过浏览器",
        "打开浏览器",
        "浏览器打开",
        "打开网页",
        "访问网页",
        "打开网站",
        "访问网站",
        "去网站",
        "搜索第三方",
        "搜第三方",
        "找第三方",
        "打开第三方",
        "用第三方",
        "使用第三方",
        "通过第三方",
        "用在线工具",
        "使用在线工具",
        "搜索网页",
        "搜索网站",
        "搜网页",
        "搜网站",
        "用 canva",
        "使用 canva",
        "通过 canva",
        "通过 http://",
        "通过 https://",
    ];
    if CN_AFFIRMATIVE
        .iter()
        .any(|phrase| has_non_negated_cn_phrase(text, phrase))
    {
        return true;
    }

    let words = ascii_words(text);
    const TARGETS: &[&str] = &[
        "browser",
        "browsers",
        "website",
        "websites",
        "web",
        "third-party",
        "thirdparty",
        "search",
        "canva",
        "pollinations",
    ];
    const EXECUTION_VERBS: &[&str] = &[
        "use", "using", "open", "opening", "visit", "visiting", "access", "search", "find",
        "browse",
    ];
    const NEGATIONS: &[&str] = &[
        "not", "never", "without", "avoid", "avoiding", "dont", "don't", "no", "cannot",
        "cant", "can't",
    ];
    words.iter().enumerate().any(|(index, word)| {
        if !TARGETS.contains(&word.as_str()) {
            return false;
        }
        if words[index.saturating_sub(5)..index]
            .iter()
            .any(|word| NEGATIONS.contains(&word.as_str()))
        {
            return false;
        }
        let context = &words[index.saturating_sub(3)..index];
        word == "search"
            || context
                .iter()
                .any(|word| EXECUTION_VERBS.contains(&word.as_str()))
            || context.iter().any(|word| matches!(word.as_str(), "via" | "through"))
            || (matches!(
                word.as_str(),
                "third-party" | "thirdparty" | "canva" | "pollinations"
            ) && context
                .iter()
                .any(|word| matches!(word.as_str(), "with" | "on" | "in")))
    })
}

fn has_non_negated_cn_phrase(text: &str, phrase: &str) -> bool {
    const NEGATIONS: &[&str] = &[
        "不要", "不用", "无需", "不必", "不需要", "别", "禁止", "避免", "拒绝", "请勿", "勿",
    ];
    text.match_indices(phrase).any(|(index, _)| {
        let context = text[..index]
            .chars()
            .rev()
            .take(8)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        !NEGATIONS.iter().any(|negation| context.contains(negation))
    })
}

fn ascii_words(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '\'')
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_creation_external_discussion_and_unrelated_requests() {
        for request in [
            "请生成一张赛博朋克城市图片",
            "给我一张猫图",
            "帮我画一张橘猫",
            "Create a watercolor illustration of a fox",
            "I need a poster",
            "不要用浏览器，生成一张狐狸图片",
            "create an image for my website",
        ] {
            assert_eq!(
                classify_image_generation_intent(request),
                ImageGenerationIntent::Creation,
                "request={request}"
            );
        }
        for request in [
            "请用浏览器打开网站生成一张图片",
            "搜索第三方网页制作海报",
            "用 Canva 生成",
            "Create an image with a third-party website",
            "Use the browser to generate a picture",
        ] {
            assert_eq!(
                classify_image_generation_intent(request),
                ImageGenerationIntent::ExplicitExternal,
                "request={request}"
            );
        }
        for request in [
            "解释 image generation 的工作原理",
            "制作图片生成方案",
            "create an image generation plan",
            "不要生成图片，只解释提示词",
            "create a logo component that renders this existing image",
            "请重构优化 Agent 的生图链路",
        ] {
            assert_eq!(
                classify_image_generation_intent(request),
                ImageGenerationIntent::Discussion,
                "request={request}"
            );
        }
        for request in [
            "搜索几张猫的图片",
            "create a directory named images",
            "再来一张",
            "今天天气如何",
        ] {
            assert_eq!(
                classify_image_generation_intent(request),
                ImageGenerationIntent::None,
                "request={request}"
            );
        }
    }
}
