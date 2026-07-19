//! Conservative task-level artifact requirements for Agent execution steps.
//!
//! `ExecutionStep` does not currently persist a typed artifact schema. Until it
//! does, only an explicit output verb followed by an artifact noun/format in
//! the immutable step spec creates an obligation. This deliberately does not
//! infer an output from input-oriented phrases such as "analyse 3 images".

use std::collections::BTreeSet;
use std::path::Path;

const MAX_ARTIFACT_TERM_DISTANCE: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ArtifactMatcher {
    Any,
    Image,
    Audio,
    Video,
    Animation,
    Document,
    Spreadsheet,
    Presentation,
    Archive,
    Exact(&'static str),
}

impl ArtifactMatcher {
    fn accepts(self, extension: Option<&str>) -> bool {
        let Some(extension) = extension else {
            return self == Self::Any;
        };
        match self {
            Self::Any => true,
            Self::Image => matches!(
                extension,
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "avif" | "svg"
            ),
            Self::Audio => matches!(
                extension,
                "mp3" | "wav" | "flac" | "ogg" | "oga" | "m4a" | "aac" | "opus"
            ),
            Self::Video => matches!(
                extension,
                "mp4" | "mov" | "webm" | "mkv" | "avi" | "m4v"
            ),
            Self::Animation => matches!(
                extension,
                "gif" | "webp" | "mp4" | "mov" | "webm" | "mkv" | "avi" | "m4v"
            ),
            Self::Document => matches!(
                extension,
                "pdf" | "doc" | "docx" | "odt" | "rtf" | "txt" | "md"
            ),
            Self::Spreadsheet => {
                matches!(extension, "xls" | "xlsx" | "csv" | "tsv" | "ods")
            }
            Self::Presentation => matches!(extension, "ppt" | "pptx" | "odp" | "pdf"),
            Self::Archive => matches!(
                extension,
                "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar"
            ),
            Self::Exact("jpg" | "jpeg") => matches!(extension, "jpg" | "jpeg"),
            Self::Exact("tif" | "tiff") => matches!(extension, "tif" | "tiff"),
            Self::Exact(expected) => extension == expected,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Any => "artifact",
            Self::Image => "image artifact",
            Self::Audio => "audio artifact",
            Self::Video => "video artifact",
            Self::Animation => "animation artifact",
            Self::Document => "document artifact",
            Self::Spreadsheet => "spreadsheet artifact",
            Self::Presentation => "presentation artifact",
            Self::Archive => "archive artifact",
            Self::Exact(extension) => extension,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtifactRequirement {
    matcher: ArtifactMatcher,
    minimum_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedArtifactContract {
    requirements: Vec<ArtifactRequirement>,
}

/// Both normal settlement and manual adoption call this function with paths
/// projected from committed, re-verified conversation receipts. It does not
/// treat assistant prose or an unverified path mentioned in prose as evidence.
pub(crate) fn validate_required_artifacts(
    step_spec: &str,
    verified_output_files: &[String],
) -> Result<(), String> {
    let Some(contract) = infer_expected_artifacts(step_spec) else {
        return Ok(());
    };

    // A repeated receipt must never satisfy an explicit count. Production
    // projections already deduplicate canonical paths, and this keeps mocked
    // or recovered inputs under the same rule.
    let unique_files = verified_output_files
        .iter()
        .filter(|path| !path.trim().is_empty())
        .map(|path| {
            if cfg!(windows) {
                path.replace('/', "\\").to_lowercase()
            } else {
                path.to_owned()
            }
        })
        .collect::<BTreeSet<_>>();

    for requirement in contract.requirements {
        let delivered = unique_files
            .iter()
            .filter(|path| {
                let extension = Path::new(path)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(str::to_ascii_lowercase);
                requirement.matcher.accepts(extension.as_deref())
            })
            .count();
        if delivered < requirement.minimum_count {
            return Err(format!(
                "step explicitly requires at least {} verified {}, but only {} matching output file(s) were delivered",
                requirement.minimum_count,
                requirement.matcher.label(),
                delivered
            ));
        }
    }
    Ok(())
}

fn infer_expected_artifacts(step_spec: &str) -> Option<ExpectedArtifactContract> {
    let normalized = step_spec.to_lowercase();
    let mut requirements = Vec::new();

    for clause in normalized.split(['\n', '\r', ';', '；', '!', '！', '?', '？', '。']) {
        let verbs = matching_terms(clause, OUTPUT_VERBS)
            .into_iter()
            .map(|(position, verb)| (position, position + verb.len(), verb))
            .collect::<Vec<_>>();
        for (verb_index, (verb_start, verb_end, verb)) in verbs.iter().copied().enumerate() {
            if verb_is_nested_capability(clause, verb_start) {
                continue;
            }
            let next_verb = verbs.get(verb_index + 1).map(|(start, _, _)| *start);
            let artifact_search_start = if is_packaging_verb(verb) {
                match packaging_output_start(clause, verb_end, next_verb) {
                    Some(target_start) => target_start,
                    None => {
                        // `package files` still promises one archive even when
                        // its container format is omitted. Source-side counts
                        // describe inputs and must never become output counts.
                        merge_requirement(
                            &mut requirements,
                            ArtifactRequirement {
                                matcher: ArtifactMatcher::Archive,
                                minimum_count: 1,
                            },
                        );
                        continue;
                    }
                }
            } else {
                let Some(target_start) = transform_output_start(clause, verb, verb_end) else {
                    continue;
                };
                target_start
            };
            for (artifact_start, artifact_end, matcher) in output_artifacts_after(
                clause,
                artifact_search_start,
                next_verb,
            ) {
                let lead_in = local_target_lead(&clause[artifact_search_start..artifact_start]);
                let minimum_count = explicit_count_before(lead_in)
                    .or_else(|| explicit_count_after(&clause[artifact_end..]))
                    .unwrap_or(1);
                if minimum_count == 0 {
                    continue;
                }
                let matcher = explicit_format_after(clause, artifact_end).unwrap_or(matcher);
                merge_requirement(
                    &mut requirements,
                    ArtifactRequirement {
                        matcher,
                        minimum_count,
                    },
                );
            }
        }
    }

    (!requirements.is_empty()).then_some(ExpectedArtifactContract { requirements })
}

fn is_packaging_verb(verb: &str) -> bool {
    matches!(verb, "package" | "bundle" | "打包")
}

fn packaging_output_start(clause: &str, verb_end: usize, before: Option<usize>) -> Option<usize> {
    let suffix = &clause[verb_end..before.unwrap_or(clause.len())];
    let directional = [" into ", " as ", " to ", " -> ", "→", "为", "成"]
        .iter()
        .filter_map(|delimiter| {
            suffix
                .find(delimiter)
                .map(|offset| verb_end + offset + delimiter.len())
        })
        .min();
    if directional.is_some() {
        return directional;
    }

    // Direct target forms (`Package a ZIP`, `打包压缩包`) have no source-side
    // artifact before the archive name, so scanning from the verb is safe.
    ["zip", "archive", "压缩包", "归档文件"]
        .iter()
        .any(|target| !matching_term_positions(suffix, target).is_empty())
        .then_some(verb_end)
}

fn transform_output_start(clause: &str, verb: &str, verb_end: usize) -> Option<usize> {
    const TRANSFORM_VERBS: &[&str] = &["convert", "transform", "转换"];
    // `转为PDF` includes its output direction in the verb itself.
    if verb == "转为" {
        return Some(verb_end);
    }
    if !TRANSFORM_VERBS.contains(&verb) {
        return Some(verb_end);
    }

    // For `convert PDF to CSV`, PDF is an input and CSV is the required
    // deliverable. Restrict the delimiter search to the current clause so an
    // unrelated later preposition cannot manufacture an obligation.
    let suffix = &clause[verb_end..];
    [
        " into ", " to ", " as ", " -> ", "→", "转换为", "转为", "为", "成",
    ]
        .iter()
        .filter_map(|delimiter| suffix.find(delimiter).map(|offset| verb_end + offset + delimiter.len()))
        .min()
}

fn verb_is_nested_capability(clause: &str, verb_start: usize) -> bool {
    let prefix = clause[..verb_start].trim_end();
    if prefix.ends_with("用于") || prefix.ends_with("以便") {
        return true;
    }
    let Some(before_to) = prefix.strip_suffix("to").map(str::trim_end) else {
        return false;
    };
    if before_to.is_empty() {
        return false;
    }
    [
        "implement", "build", "design", "develop", "refactor", "add", "support", "tool",
        "function", "system", "service", "workflow", "pipeline", "feature", "app",
    ]
    .iter()
    .any(|cue| matching_term_positions(before_to, cue).last().is_some())
}

fn output_artifacts_after(
    clause: &str,
    after: usize,
    before: Option<usize>,
) -> Vec<(usize, usize, ArtifactMatcher)> {
    let mut candidates = ARTIFACT_TERMS
        .iter()
        .flat_map(|(term, matcher)| {
            matching_term_positions(clause, term)
                .into_iter()
                .filter(move |position| {
                    *position >= after && before.is_none_or(|limit| *position < limit)
                })
                .map(move |position| (position, position + term.len(), *matcher))
        })
        .filter(|(start, end, _)| {
            clause[after..*start].chars().count() <= MAX_ARTIFACT_TERM_DISTANCE
                && !artifact_is_input_context(&clause[after..*start])
                && !artifact_is_modifier_context(&clause[*end..])
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| right.2.cmp(&left.2))
    });
    candidates.dedup_by(|left, right| left.0 == right.0 && left.2 == right.2);
    candidates
}

fn local_target_lead(lead_in: &str) -> &str {
    [" and ", " plus ", ",", "，", "、", "和", "及"]
        .iter()
        .filter_map(|separator| lead_in.rfind(separator).map(|position| position + separator.len()))
        .max()
        .map_or(lead_in, |start| &lead_in[start..])
}

fn artifact_is_input_context(lead_in: &str) -> bool {
    let normalized = lead_in.trim();
    [
        "based on",
        "based upon",
        "from the",
        "from an",
        "from a",
        "using the",
        "using an",
        "using a",
        "using ",
        "according to",
        "about the",
        "about ",
        "from ",
        " from ",
        "for ",
        " for ",
        "with ",
        " with ",
        "of ",
        " of ",
        "containing",
        "contains",
        "that analyzes",
        "that analyses",
        "that compares",
        "to analyze",
        "to analyse",
        "to process",
        "to classify",
        "to inspect",
        "to read",
        "analyze ",
        "analyse ",
        "review ",
        "inspect ",
        "read ",
        "for the provided",
        "for provided",
        "基于",
        "根据",
        "参考",
        "用于",
        "包含",
        "分析",
        "处理",
        "分类",
        "检查",
        "审查",
        "读取",
        "使用已有",
        "使用现有",
        "针对已有",
        "针对现有",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn artifact_is_modifier_context(suffix: &str) -> bool {
    let suffix = suffix.trim_start_matches(|character: char| {
        character.is_whitespace()
            || matches!(character, '-' | '_' | '/' | '\\' | ':' | '：' | '(' | '（')
    });
    let modifier_window = suffix.chars().take(64).collect::<String>();
    if suffix.starts_with("to-")
        && [
            " converter",
            " parser",
            " tool",
            " pipeline",
            " workflow",
            " service",
        ]
        .iter()
        .any(|modifier| modifier_window.contains(modifier))
    {
        return true;
    }
    [
        "analyzer",
        "analyser",
        "analysis",
        "architecture",
        "classifier",
        "classification",
        "decoder",
        "encoder",
        "parser",
        "processor",
        "processing",
        "model",
        "dataset",
        "pipeline",
        "workflow",
        "tool",
        "generator",
        "generation",
        "converter",
        "transcoder",
        "importer",
        "exporter",
        "transcription",
        "recognition",
        "service",
        "system",
        "api",
        "function",
        "component",
        "editor",
        "viewer",
        "loader",
        "player",
        "support",
        "handling",
        "feature",
        "prompt",
        "outline",
        "plan",
        "script",
        "transcript",
        "copy",
        "reference",
        "input",
        "test",
        "gallery",
        "search",
        "export feature",
        "based",
        "分析",
        "分类",
        "解码",
        "编码",
        "解析",
        "处理",
        "模型",
        "数据集",
        "管线",
        "流水线",
        "工作流",
        "工具",
        "生成器",
        "生成",
        "转换器",
        "转码器",
        "导入器",
        "导出器",
        "转录",
        "识别",
        "服务",
        "系统",
        "接口",
        "函数",
        "组件",
        "编辑器",
        "查看器",
        "加载器",
        "播放器",
        "支持",
        "功能",
        "提示词",
        "架构",
        "大纲",
        "计划",
        "脚本",
        "转录稿",
        "文案",
        "参考",
        "输入",
        "测试",
        "检索",
    ]
    .iter()
    .any(|modifier| suffix.starts_with(modifier))
}

fn explicit_format_after(clause: &str, artifact_end: usize) -> Option<ArtifactMatcher> {
    let suffix = clause[artifact_end..].chars().take(40).collect::<String>();
    let format_marker = [
        " as ", " in ", " format", "格式", "为", "以", "（", "(",
    ]
    .iter()
    .filter_map(|marker| suffix.find(marker))
    .min()?;
    let format_area = &suffix[format_marker..];
    ARTIFACT_TERMS
        .iter()
        .filter_map(|(term, matcher)| match matcher {
            ArtifactMatcher::Exact(_) => matching_term_positions(format_area, term)
                .into_iter()
                .next()
                .map(|position| (position, *matcher)),
            _ => None,
        })
        .min_by_key(|(position, _)| *position)
        .map(|(_, matcher)| matcher)
}

fn merge_requirement(
    requirements: &mut Vec<ArtifactRequirement>,
    incoming: ArtifactRequirement,
) {
    if let Some(existing) = requirements
        .iter_mut()
        .find(|existing| existing.matcher == incoming.matcher)
    {
        // Repeated wording commonly restates the same deliverable. Taking the
        // strongest stated minimum is deterministic without double-counting.
        existing.minimum_count = existing.minimum_count.max(incoming.minimum_count);
    } else {
        requirements.push(incoming);
    }
}

fn matching_terms<'a>(text: &str, terms: &'a [&'a str]) -> Vec<(usize, &'a str)> {
    let mut matches = terms
        .iter()
        .flat_map(|term| {
            matching_term_positions(text, term)
                .into_iter()
                .map(|position| (position, *term))
        })
        .collect::<Vec<_>>();
    matches.sort_unstable_by_key(|(position, _)| *position);
    matches
}

fn matching_term_positions(text: &str, term: &str) -> Vec<usize> {
    text.match_indices(term)
        .filter_map(|(position, _)| {
            ascii_word_boundaries_match(text, position, term).then_some(position)
        })
        .collect()
}

fn ascii_word_boundaries_match(text: &str, position: usize, term: &str) -> bool {
    let requires_boundary = term
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if !requires_boundary {
        return true;
    }
    let before = text[..position].chars().next_back();
    let after = text[position + term.len()..].chars().next();
    !before.is_some_and(is_ascii_word_char) && !after.is_some_and(is_ascii_word_char)
}

fn is_ascii_word_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn explicit_count_before(segment: &str) -> Option<usize> {
    let digit_runs = ascii_digit_runs(segment)
        .into_iter()
        .filter(|(start, end)| {
            !digit_run_is_dimension(segment, *start, *end)
                && digit_run_has_count_context(segment, *start, *end)
        })
        .collect::<Vec<_>>();
    // Multiple unrelated numbers are ambiguous (dimensions, versions, page
    // ranges); defaulting to one is safer than inventing a huge count.
    if digit_runs.len() != 1 {
        return None;
    }
    parse_count(&segment[digit_runs[0].0..digit_runs[0].1])
}

fn digit_run_has_count_context(segment: &str, start: usize, end: usize) -> bool {
    let immediate_before = segment[..start].chars().next_back();
    let immediate_after = segment[end..].chars().next();
    if immediate_before.is_some_and(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
    }) || immediate_after.is_some_and(|character| matches!(character, '_' | '-'))
    {
        return false;
    }

    let prefix = segment[..start].trim().to_ascii_lowercase();
    if ["版本", "第", "尺寸", "分辨率"]
        .iter()
        .any(|marker| prefix.contains(marker))
    {
        return false;
    }
    prefix
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .all(|token| {
            matches!(
                token,
                "a" | "an" | "the" | "exactly" | "at" | "least" | "minimum" | "min" | "up" | "to"
            )
        })
}

fn explicit_count_after(segment: &str) -> Option<usize> {
    // Handle explicit suffix counters such as `图片3张` / `images: 3 variants`
    // without interpreting later dimensions or unrelated numbered prose.
    let prefix = segment.chars().take(24).collect::<String>();
    let (start, end) = *ascii_digit_runs(&prefix).first()?;
    let before = prefix[..start].trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, ':' | '：' | '(' | '（')
    });
    if !before.is_empty() || digit_run_is_dimension(&prefix, start, end) {
        return None;
    }
    let suffix = prefix[end..].trim_start();
    let has_counter = suffix.starts_with(['张', '个', '份', '段'])
        || ["variant", "variants", "file", "files", "copy", "copies"]
            .iter()
            .any(|counter| suffix.starts_with(counter));
    has_counter
        .then(|| parse_count(&prefix[start..end]))
        .flatten()
}

fn ascii_digit_runs(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut runs = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if !bytes[cursor].is_ascii_digit() {
            cursor += 1;
            continue;
        }
        let start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        runs.push((start, cursor));
    }
    runs
}

fn digit_run_is_dimension(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].trim_end().chars().next_back();
    let after = text[end..].trim_start().chars().next();
    matches!(before, Some('x' | '×')) || matches!(after, Some('x' | '×'))
}

fn parse_count(raw: &str) -> Option<usize> {
    // An explicit but unrepresentably large count must fail closed rather than
    // silently degrade to the default requirement of one artifact.
    Some(raw.parse::<usize>().unwrap_or(usize::MAX))
}

const OUTPUT_VERBS: &[&str] = &[
    "create",
    "generate",
    "produce",
    "render",
    "draw",
    "design",
    "convert",
    "transform",
    "record",
    "capture",
    "photograph",
    "encode",
    "transcode",
    "synthesize",
    "synthesise",
    "illustrate",
    "paint",
    "download",
    "compose",
    "package",
    "bundle",
    "export",
    "save",
    "make",
    "deliver",
    "build",
    "write",
    "生成",
    "创建",
    "制作",
    "渲染",
    "导出",
    "保存",
    "输出",
    "产出",
    "交付",
    "绘制",
    "设计",
    "转换",
    "转为",
    "录制",
    "截取",
    "拍摄",
    "编码",
    "转码",
    "下载",
    "合成",
    "打包",
    "写入",
    "编写",
];

// Exact formats are intentionally listed before family nouns. A format name
// is accepted as an obligation only after an output verb; `.json` mentioned as
// an input before the verb therefore cannot create a false task requirement.
const ARTIFACT_TERMS: &[(&str, ArtifactMatcher)] = &[
    (".png", ArtifactMatcher::Exact("png")),
    (".jpg", ArtifactMatcher::Exact("jpg")),
    (".jpeg", ArtifactMatcher::Exact("jpeg")),
    (".webp", ArtifactMatcher::Exact("webp")),
    (".gif", ArtifactMatcher::Exact("gif")),
    (".bmp", ArtifactMatcher::Exact("bmp")),
    (".tif", ArtifactMatcher::Exact("tif")),
    (".tiff", ArtifactMatcher::Exact("tiff")),
    (".svg", ArtifactMatcher::Exact("svg")),
    (".pdf", ArtifactMatcher::Exact("pdf")),
    (".doc", ArtifactMatcher::Exact("doc")),
    (".docx", ArtifactMatcher::Exact("docx")),
    (".odt", ArtifactMatcher::Exact("odt")),
    (".rtf", ArtifactMatcher::Exact("rtf")),
    (".txt", ArtifactMatcher::Exact("txt")),
    (".md", ArtifactMatcher::Exact("md")),
    (".xls", ArtifactMatcher::Exact("xls")),
    (".xlsx", ArtifactMatcher::Exact("xlsx")),
    (".tsv", ArtifactMatcher::Exact("tsv")),
    (".ods", ArtifactMatcher::Exact("ods")),
    (".ppt", ArtifactMatcher::Exact("ppt")),
    (".pptx", ArtifactMatcher::Exact("pptx")),
    (".odp", ArtifactMatcher::Exact("odp")),
    (".csv", ArtifactMatcher::Exact("csv")),
    (".mp3", ArtifactMatcher::Exact("mp3")),
    (".wav", ArtifactMatcher::Exact("wav")),
    (".flac", ArtifactMatcher::Exact("flac")),
    (".ogg", ArtifactMatcher::Exact("ogg")),
    (".m4a", ArtifactMatcher::Exact("m4a")),
    (".aac", ArtifactMatcher::Exact("aac")),
    (".opus", ArtifactMatcher::Exact("opus")),
    (".mp4", ArtifactMatcher::Exact("mp4")),
    (".mov", ArtifactMatcher::Exact("mov")),
    (".webm", ArtifactMatcher::Exact("webm")),
    (".mkv", ArtifactMatcher::Exact("mkv")),
    (".avi", ArtifactMatcher::Exact("avi")),
    (".m4v", ArtifactMatcher::Exact("m4v")),
    (".zip", ArtifactMatcher::Exact("zip")),
    (".tar", ArtifactMatcher::Exact("tar")),
    (".gz", ArtifactMatcher::Exact("gz")),
    (".tgz", ArtifactMatcher::Exact("tgz")),
    (".bz2", ArtifactMatcher::Exact("bz2")),
    (".xz", ArtifactMatcher::Exact("xz")),
    (".7z", ArtifactMatcher::Exact("7z")),
    (".rar", ArtifactMatcher::Exact("rar")),
    (".json", ArtifactMatcher::Exact("json")),
    (".html", ArtifactMatcher::Exact("html")),
    (".htm", ArtifactMatcher::Exact("htm")),
    (".xml", ArtifactMatcher::Exact("xml")),
    (".yaml", ArtifactMatcher::Exact("yaml")),
    (".yml", ArtifactMatcher::Exact("yml")),
    ("png", ArtifactMatcher::Exact("png")),
    ("pngs", ArtifactMatcher::Exact("png")),
    ("bmp", ArtifactMatcher::Exact("bmp")),
    ("tiff", ArtifactMatcher::Exact("tiff")),
    ("tif", ArtifactMatcher::Exact("tif")),
    ("avif", ArtifactMatcher::Exact("avif")),
    ("jpeg", ArtifactMatcher::Exact("jpeg")),
    ("jpegs", ArtifactMatcher::Exact("jpeg")),
    ("jpg", ArtifactMatcher::Exact("jpg")),
    ("jpgs", ArtifactMatcher::Exact("jpg")),
    ("webp", ArtifactMatcher::Exact("webp")),
    ("gif", ArtifactMatcher::Exact("gif")),
    ("svg", ArtifactMatcher::Exact("svg")),
    ("pdf", ArtifactMatcher::Exact("pdf")),
    ("pdfs", ArtifactMatcher::Exact("pdf")),
    ("doc", ArtifactMatcher::Exact("doc")),
    ("docx", ArtifactMatcher::Exact("docx")),
    ("docxs", ArtifactMatcher::Exact("docx")),
    ("odt", ArtifactMatcher::Exact("odt")),
    ("rtf", ArtifactMatcher::Exact("rtf")),
    ("xls", ArtifactMatcher::Exact("xls")),
    ("xlsx", ArtifactMatcher::Exact("xlsx")),
    ("xlsxs", ArtifactMatcher::Exact("xlsx")),
    ("csv", ArtifactMatcher::Exact("csv")),
    ("csvs", ArtifactMatcher::Exact("csv")),
    ("tsv", ArtifactMatcher::Exact("tsv")),
    ("ods", ArtifactMatcher::Exact("ods")),
    ("ppt", ArtifactMatcher::Exact("ppt")),
    ("pptx", ArtifactMatcher::Exact("pptx")),
    ("pptxs", ArtifactMatcher::Exact("pptx")),
    ("odp", ArtifactMatcher::Exact("odp")),
    ("mp3", ArtifactMatcher::Exact("mp3")),
    ("mp3s", ArtifactMatcher::Exact("mp3")),
    ("wav", ArtifactMatcher::Exact("wav")),
    ("flac", ArtifactMatcher::Exact("flac")),
    ("ogg", ArtifactMatcher::Exact("ogg")),
    ("m4a", ArtifactMatcher::Exact("m4a")),
    ("aac", ArtifactMatcher::Exact("aac")),
    ("opus", ArtifactMatcher::Exact("opus")),
    ("mp4", ArtifactMatcher::Exact("mp4")),
    ("mp4s", ArtifactMatcher::Exact("mp4")),
    ("mov", ArtifactMatcher::Exact("mov")),
    ("webm", ArtifactMatcher::Exact("webm")),
    ("mkv", ArtifactMatcher::Exact("mkv")),
    ("avi", ArtifactMatcher::Exact("avi")),
    ("m4v", ArtifactMatcher::Exact("m4v")),
    ("zip", ArtifactMatcher::Exact("zip")),
    ("zips", ArtifactMatcher::Exact("zip")),
    ("tar", ArtifactMatcher::Exact("tar")),
    ("tgz", ArtifactMatcher::Exact("tgz")),
    ("7z", ArtifactMatcher::Exact("7z")),
    ("rar", ArtifactMatcher::Exact("rar")),
    ("json file", ArtifactMatcher::Exact("json")),
    ("html file", ArtifactMatcher::Exact("html")),
    ("xml file", ArtifactMatcher::Exact("xml")),
    ("yaml file", ArtifactMatcher::Exact("yaml")),
    ("text file", ArtifactMatcher::Exact("txt")),
    ("markdown file", ArtifactMatcher::Exact("md")),
    ("images", ArtifactMatcher::Image),
    ("image", ArtifactMatcher::Image),
    ("pictures", ArtifactMatcher::Image),
    ("picture", ArtifactMatcher::Image),
    ("photos", ArtifactMatcher::Image),
    ("photo", ArtifactMatcher::Image),
    ("illustrations", ArtifactMatcher::Image),
    ("illustration", ArtifactMatcher::Image),
    ("posters", ArtifactMatcher::Image),
    ("poster", ArtifactMatcher::Image),
    ("screenshots", ArtifactMatcher::Image),
    ("screenshot", ArtifactMatcher::Image),
    ("infographics", ArtifactMatcher::Image),
    ("infographic", ArtifactMatcher::Image),
    ("diagrams", ArtifactMatcher::Image),
    ("diagram", ArtifactMatcher::Image),
    ("flowcharts", ArtifactMatcher::Image),
    ("flowchart", ArtifactMatcher::Image),
    ("charts", ArtifactMatcher::Image),
    ("chart", ArtifactMatcher::Image),
    ("logos", ArtifactMatcher::Image),
    ("logo", ArtifactMatcher::Image),
    ("icons", ArtifactMatcher::Image),
    ("icon", ArtifactMatcher::Image),
    ("thumbnails", ArtifactMatcher::Image),
    ("thumbnail", ArtifactMatcher::Image),
    ("mockups", ArtifactMatcher::Image),
    ("mockup", ArtifactMatcher::Image),
    ("图片", ArtifactMatcher::Image),
    ("图像", ArtifactMatcher::Image),
    ("照片", ArtifactMatcher::Image),
    ("插图", ArtifactMatcher::Image),
    ("海报", ArtifactMatcher::Image),
    ("截图", ArtifactMatcher::Image),
    ("信息图", ArtifactMatcher::Image),
    ("流程图", ArtifactMatcher::Image),
    ("图表", ArtifactMatcher::Image),
    ("徽标", ArtifactMatcher::Image),
    ("图标", ArtifactMatcher::Image),
    ("缩略图", ArtifactMatcher::Image),
    ("样机", ArtifactMatcher::Image),
    ("audio", ArtifactMatcher::Audio),
    ("podcast", ArtifactMatcher::Audio),
    ("narration", ArtifactMatcher::Audio),
    ("voiceover", ArtifactMatcher::Audio),
    ("soundtrack", ArtifactMatcher::Audio),
    ("songs", ArtifactMatcher::Audio),
    ("song", ArtifactMatcher::Audio),
    ("recordings", ArtifactMatcher::Audio),
    ("recording", ArtifactMatcher::Audio),
    ("speech file", ArtifactMatcher::Audio),
    ("音频", ArtifactMatcher::Audio),
    ("播客", ArtifactMatcher::Audio),
    ("旁白", ArtifactMatcher::Audio),
    ("配音", ArtifactMatcher::Audio),
    ("配乐", ArtifactMatcher::Audio),
    ("语音文件", ArtifactMatcher::Audio),
    ("videos", ArtifactMatcher::Video),
    ("video", ArtifactMatcher::Video),
    ("视频", ArtifactMatcher::Video),
    ("animations", ArtifactMatcher::Animation),
    ("animation", ArtifactMatcher::Animation),
    ("动画", ArtifactMatcher::Animation),
    ("spreadsheets", ArtifactMatcher::Spreadsheet),
    ("spreadsheet", ArtifactMatcher::Spreadsheet),
    ("workbooks", ArtifactMatcher::Spreadsheet),
    ("workbook", ArtifactMatcher::Spreadsheet),
    ("excel file", ArtifactMatcher::Spreadsheet),
    ("电子表格", ArtifactMatcher::Spreadsheet),
    ("工作簿", ArtifactMatcher::Spreadsheet),
    ("presentations", ArtifactMatcher::Presentation),
    ("presentation", ArtifactMatcher::Presentation),
    ("slides", ArtifactMatcher::Presentation),
    ("slide", ArtifactMatcher::Presentation),
    ("slide decks", ArtifactMatcher::Presentation),
    ("slide deck", ArtifactMatcher::Presentation),
    ("powerpoint file", ArtifactMatcher::Presentation),
    ("powerpoint", ArtifactMatcher::Presentation),
    ("演示文稿", ArtifactMatcher::Presentation),
    ("幻灯片", ArtifactMatcher::Presentation),
    ("archives", ArtifactMatcher::Archive),
    ("archive", ArtifactMatcher::Archive),
    ("压缩包", ArtifactMatcher::Archive),
    ("归档文件", ArtifactMatcher::Archive),
    ("documents", ArtifactMatcher::Document),
    ("document", ArtifactMatcher::Document),
    ("report file", ArtifactMatcher::Document),
    ("报告文件", ArtifactMatcher::Document),
    ("文档", ArtifactMatcher::Document),
    ("datasets", ArtifactMatcher::Any),
    ("dataset", ArtifactMatcher::Any),
    ("数据集", ArtifactMatcher::Any),
    ("artifacts", ArtifactMatcher::Any),
    ("artifact", ArtifactMatcher::Any),
    ("deliverables", ArtifactMatcher::Any),
    ("deliverable", ArtifactMatcher::Any),
    ("files", ArtifactMatcher::Any),
    ("file", ArtifactMatcher::Any),
    ("产物", ArtifactMatcher::Any),
    ("交付物", ArtifactMatcher::Any),
    ("文件", ArtifactMatcher::Any),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn explicit_english_count_and_format_are_enforced() {
        let spec = "Generate 3 PNG images and return their saved paths.";
        assert!(validate_required_artifacts(spec, &files(&["/w/1.png", "/w/2.png"])).is_err());
        assert!(
            validate_required_artifacts(
                spec,
                &files(&["/w/1.png", "/w/2.png", "/w/wrong.jpg"])
            )
            .is_err()
        );
        assert!(
            validate_required_artifacts(
                spec,
                &files(&["/w/1.png", "/w/2.PNG", "/w/3.png"])
            )
            .is_ok()
        );
    }

    #[test]
    fn explicit_chinese_count_and_image_family_are_enforced() {
        let spec = "请生成2张图片，并在会话中展示保存路径";
        assert!(validate_required_artifacts(spec, &files(&["C:\\w\\one.webp"])).is_err());
        assert!(
            validate_required_artifacts(
                spec,
                &files(&["C:\\w\\one.webp", "C:\\w\\two.jpeg"])
            )
            .is_ok()
        );
    }

    #[test]
    fn duplicate_paths_do_not_satisfy_explicit_count() {
        let spec = "Create 2 image files";
        assert!(validate_required_artifacts(spec, &files(&["/w/a.png", "/w/a.png"])).is_err());
    }

    #[test]
    fn non_image_artifact_families_and_counts_are_enforced() {
        assert!(
            validate_required_artifacts("Export 2 PDF documents", &files(&["/w/a.pdf"]))
                .is_err()
        );
        assert!(
            validate_required_artifacts(
                "Export 2 PDF documents",
                &files(&["/w/a.pdf", "/w/b.pdf"])
            )
            .is_ok()
        );
        assert!(
            validate_required_artifacts("创建1个电子表格", &files(&["/w/result.xlsx"]))
                .is_ok()
        );
    }

    #[test]
    fn input_artifacts_and_chat_only_work_do_not_create_an_obligation() {
        let specs = [
            "Analyze 3 images and write a concise summary in chat.",
            "Review the existing PDF and explain the findings.",
            "Create a written analysis based on the provided images.",
            "Write a report from the existing PDF in the conversation.",
            "读取 input.xlsx，然后回答用户的问题",
            "创建一份基于现有图片的文字分析并直接回复",
            "Refactor the image decoder implementation and run tests.",
            "Design an image classifier and explain the architecture.",
            "Build a PDF parser and return the code review in chat.",
            "Build a tool to generate images.",
            "创建一个用于生成图片的工具",
            "Draw conclusions from the supplied images.",
            "Record observations about an existing video.",
            "Write a comparison of the two PDF files in chat.",
            "Build a PDF-to-CSV converter.",
            "Write a CSV parser.",
        ];
        for spec in specs {
            assert!(validate_required_artifacts(spec, &[]).is_ok(), "{spec}");
        }
    }

    #[test]
    fn image_dimensions_are_not_mistaken_for_a_count() {
        assert!(
            validate_required_artifacts(
                "Generate a 1920x1080 image",
                &files(&["/w/hero.png"])
            )
            .is_ok()
        );
        assert!(
            validate_required_artifacts(
                "Save the output to report-v2.json",
                &files(&["/w/report-v2.json"])
            )
            .is_ok()
        );
        assert!(
            validate_required_artifacts(
                "Export report version 2 as PDF",
                &files(&["/w/report-v2.pdf"])
            )
            .is_ok()
        );
    }

    #[test]
    fn explicit_format_after_family_noun_is_enforced() {
        let spec = "Generate 2 images in JPEG format";
        assert!(
            validate_required_artifacts(spec, &files(&["/w/a.jpg", "/w/b.jpeg"])).is_ok()
        );
        assert!(
            validate_required_artifacts(spec, &files(&["/w/a.jpg", "/w/b.png"])).is_err()
        );
        assert!(
            validate_required_artifacts("生成1张图片，格式为PNG", &files(&["/w/a.jpg"]))
                .is_err()
        );
    }

    #[test]
    fn adoption_uses_the_same_required_artifact_contract() {
        let spec = "生成3张图片";
        let adopted = files(&["/w/a.png", "/w/b.webp"]);
        let error = validate_required_artifacts(spec, &adopted).unwrap_err();
        assert!(error.contains("at least 3 verified image artifact"));
    }

    #[test]
    fn common_imperative_artifact_actions_cannot_complete_with_prose_only() {
        let cases = [
            ("Draw a poster", "/w/poster.png", "/w/poster.docx"),
            (
                "Design a presentation",
                "/w/deck.pptx",
                "/w/deck.png",
            ),
            (
                "Convert the report to PDF",
                "/w/report.pdf",
                "/w/report.docx",
            ),
            ("Record a podcast", "/w/show.mp3", "/w/show.png"),
            ("Download a CSV", "/w/data.csv", "/w/data.pdf"),
            ("设计一张海报", "/w/poster.jpg", "/w/poster.docx"),
            ("Capture a screenshot", "/w/screen.webp", "/w/screen.mp3"),
            ("Compose a narration", "/w/voice.flac", "/w/voice.png"),
            ("Render an animation", "/w/demo.gif", "/w/demo.pdf"),
            ("Create slides", "/w/slides.ppt", "/w/slides.wav"),
        ];
        for (spec, matching, mismatched) in cases {
            assert!(validate_required_artifacts(spec, &[]).is_err(), "{spec}");
            assert!(
                validate_required_artifacts(spec, &files(&[mismatched])).is_err(),
                "{spec}: {mismatched}"
            );
            assert!(
                validate_required_artifacts(spec, &files(&[matching])).is_ok(),
                "{spec}: {matching}"
            );
        }
    }

    #[test]
    fn transform_contract_targets_the_output_format_not_the_input() {
        let spec = "Convert PDF to CSV";
        assert!(validate_required_artifacts(spec, &files(&["/w/input.pdf"])).is_err());
        assert!(validate_required_artifacts(spec, &files(&["/w/output.csv"])).is_ok());
        assert!(validate_required_artifacts("Convert PDF", &[]).is_ok());
        assert!(
            validate_required_artifacts("将PDF转换为CSV", &files(&["/w/output.csv"]))
                .is_ok()
        );
    }

    #[test]
    fn common_standalone_format_names_are_exact_file_contracts() {
        let cases = [
            ("Export 2 PDFs", vec!["/w/a.pdf", "/w/b.pdf"]),
            ("Download 2 CSVs", vec!["/w/a.csv", "/w/b.csv"]),
            ("Create a DOCX", vec!["/w/a.docx"]),
            ("Create an XLSX", vec!["/w/a.xlsx"]),
            ("Create a PPTX", vec!["/w/a.pptx"]),
            ("Record an MP3", vec!["/w/a.mp3"]),
            ("Render an MP4", vec!["/w/a.mp4"]),
            ("Package a ZIP", vec!["/w/a.zip"]),
            ("Write a JSON file", vec!["/w/a.json"]),
        ];
        for (spec, paths) in cases {
            assert!(validate_required_artifacts(spec, &[]).is_err(), "{spec}");
            assert!(
                validate_required_artifacts(spec, &files(&paths)).is_ok(),
                "{spec}"
            );
        }
    }

    #[test]
    fn packaging_contract_counts_only_the_archive_target() {
        let spec = "Package 3 images into ZIP";
        assert!(validate_required_artifacts(spec, &[]).is_err());
        assert!(
            validate_required_artifacts(
                spec,
                &files(&["/w/a.png", "/w/b.jpg", "/w/c.webp"])
            )
            .is_err()
        );
        assert!(validate_required_artifacts(spec, &files(&["/w/bundle.zip"])).is_ok());

        let chinese = "把3张图片打包成ZIP";
        assert!(validate_required_artifacts(chinese, &files(&["/w/bundle.zip"])).is_ok());
        assert!(validate_required_artifacts(chinese, &files(&["/w/a.png"])).is_err());

        // Omitting the container format still promises one archive, never one
        // output per source file.
        assert!(validate_required_artifacts("Bundle 3 files", &[]).is_err());
        assert!(
            validate_required_artifacts("Bundle 3 files", &files(&["/w/bundle.tar"])).is_ok()
        );
    }

    #[test]
    fn one_explicit_action_can_require_multiple_targets_and_counts() {
        let spec = "Generate 2 images and 3 MP4 videos";
        assert!(
            validate_required_artifacts(
                spec,
                &files(&[
                    "/w/a.png",
                    "/w/b.webp",
                    "/w/1.mp4",
                    "/w/2.mp4",
                    "/w/3.mp4",
                ])
            )
            .is_ok()
        );
        assert!(
            validate_required_artifacts(
                spec,
                &files(&["/w/a.png", "/w/b.webp", "/w/1.mp4", "/w/2.mp4"])
            )
            .is_err()
        );

        let two_formats = "Export PDF and DOCX files";
        assert!(
            validate_required_artifacts(two_formats, &files(&["/w/a.pdf", "/w/a.docx"]))
                .is_ok()
        );
        assert!(
            validate_required_artifacts(two_formats, &files(&["/w/a.pdf"])).is_err()
        );

        // The document is the output; its embedded/reference images are input
        // context and must not manufacture a second three-image obligation.
        let document_with_images = "Create a document containing 3 reference images";
        assert!(validate_required_artifacts(document_with_images, &[]).is_err());
        assert!(
            validate_required_artifacts(document_with_images, &files(&["/w/report.docx"]))
                .is_ok()
        );
    }
}
