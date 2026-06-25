//! 技能起草器 / 评审器的提示词与严格 JSON 解析（design §5.2 / §5.3）。
//!
//! 两个阶段都走 `one_shot_completion(tools:[])`（选 model，不切 agent）。解析容错完全
//! 镜像 `crate::prompt::{parse_learn_output, extract_json_object}`：容忍 ```json 围栏与
//! 周围散文，抽最外层 `{...}`。

use serde::Deserialize;

use super::miner::MinedPattern;

/// 起草器输出：一份技能的 frontmatter 字段 + 正文。
#[derive(Debug, Clone, Deserialize)]
pub struct DraftOutput {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub when_to_use: Option<String>,
    #[serde(default)]
    pub body: String,
}

/// 评审器裁决。
#[derive(Debug, Clone, Deserialize)]
pub struct CriticVerdict {
    #[serde(default)]
    pub approve: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

/// 起草器 system：只产 JSON，禁围栏/散文，给精确骨架。
pub const DRAFT_SYSTEM: &str = "你是技能起草器。用户反复做某套多步操作,你要把它固化成一个可复用技能(SKILL.md)。\
只输出一个 JSON 对象,不要任何解释、不要代码围栏。字段:\n\
{\"name\":\"kebab-case 短名\",\"description\":\"一句话说明这个技能做什么(必填,非空)\",\
\"when_to_use\":\"什么情况下该用它(一句话)\",\"body\":\"## 步骤\\n1. ...\\n2. ... 的 markdown 操作手册\"}\n\
要求:name 只含小写字母数字和连字符;description 必须非空;body 给出可照做的步骤。";

/// 评审器 system：判断草稿是否一个足够通用、可复用的好技能。
pub const CRITIC_SYSTEM: &str = "你是技能评审器。判断给定技能草稿是否一个足够通用、可复用、安全的好技能。\
只输出一个 JSON 对象,不要解释、不要围栏:\n\
{\"approve\":true|false,\"reason\":\"一句话理由\"}\n\
拒绝条件:过于具体只适用一次、description 空洞、含危险/破坏性操作而无防护、与常识矛盾。";

/// 合并/演化 system：给定一个已有技能正文和一份新证据,产出改进后的同名技能(升版本)。
pub const MERGE_SYSTEM: &str = "你是技能演化器。已有一个技能,又观察到相关的新做法。\
把两者合并成一份**改进版**技能,保留原优点、补充新步骤、去重。只输出一个 JSON 对象,不要解释、不要围栏:\n\
{\"name\":\"沿用原 kebab-case 名\",\"description\":\"一句话说明(必填,非空)\",\"when_to_use\":\"何时用\",\"body\":\"改进后的 markdown 操作手册\"}";

/// 起草提示：给模型工具序列 + 真实操作转录(已脱敏,可空),要它产出技能字段。
pub fn build_draft_prompt(p: &MinedPattern, transcript: &[String]) -> String {
    let steps = p.steps.join(" → ");
    let mut s = format!(
        "主人在 {} 个不同会话里反复做了这套 {} 步操作(共 {} 次):\n{}\n\n",
        p.distinct_sessions,
        p.steps.len(),
        p.count,
        steps
    );
    if !transcript.is_empty() {
        s.push_str("这是其中一次的实际操作过程(已脱敏,据此提炼可复用的做法,不要照抄一次性细节):\n");
        for r in transcript.iter().take(40) {
            s.push_str("- ");
            s.push_str(r);
            s.push('\n');
        }
        s.push('\n');
    }
    s.push_str("把它固化成一个可复用技能。按 system 要求只输出 JSON。");
    s
}

/// 评审提示：给模型草稿 + 来源套路。
pub fn build_critic_prompt(d: &DraftOutput, p: &MinedPattern) -> String {
    format!(
        "技能草稿:\nname: {}\ndescription: {}\nwhen_to_use: {}\nbody:\n{}\n\n来源:主人在 {} 个会话重复了 {} 次。\n按 system 要求只输出 JSON 裁决。",
        d.name,
        d.description,
        d.when_to_use.as_deref().unwrap_or(""),
        d.body,
        p.distinct_sessions,
        p.count
    )
}

/// 合并提示：给模型已有技能正文 + 新证据,要它产出改进版。
pub fn build_merge_prompt(existing_body: &str, draft: &DraftOutput, p: &MinedPattern) -> String {
    format!(
        "已有技能正文:\n{}\n\n新观察到的相关做法(步骤: {}):\n{}\n\n请合并成改进版(沿用原名),按 system 要求只输出 JSON。",
        existing_body,
        p.steps.join(" → "),
        draft.body
    )
}

/// 解析起草器输出（容忍围栏/散文）。
pub fn parse_draft_output(raw: &str) -> Result<DraftOutput, String> {
    let cleaned = extract_json_object(raw).ok_or_else(|| "no JSON object found in draft output".to_owned())?;
    serde_json::from_str(cleaned).map_err(|e| format!("invalid draft JSON: {e}"))
}

/// 解析评审器输出。
pub fn parse_critic_output(raw: &str) -> Result<CriticVerdict, String> {
    let cleaned = extract_json_object(raw).ok_or_else(|| "no JSON object found in critic output".to_owned())?;
    serde_json::from_str(cleaned).map_err(|e| format!("invalid critic JSON: {e}"))
}

/// 抽最外层 `{...}`（与 `crate::prompt::extract_json_object` 同语义）。
fn extract_json_object(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(&raw[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_fenced_draft() {
        let plain = r#"{"name":"weekly-report","description":"汇总周报","when_to_use":"周五","body":"步骤:\n1. 收集"}"#;
        let d = parse_draft_output(plain).unwrap();
        assert_eq!(d.name, "weekly-report");
        assert_eq!(d.description, "汇总周报");

        let fenced = format!("好的：\n```json\n{plain}\n```\n以上。");
        let d2 = parse_draft_output(&fenced).unwrap();
        assert_eq!(d2.name, "weekly-report");
    }

    #[test]
    fn empty_description_draft_still_parses() {
        // 解析层不拒空 description（由 create_skill/critic 后续拒绝），仅保证可解析。
        let d = parse_draft_output(r#"{"name":"x","description":"","body":"y"}"#).unwrap();
        assert_eq!(d.description, "");
    }

    #[test]
    fn malformed_draft_errors() {
        assert!(parse_draft_output("not json at all").is_err());
        assert!(parse_draft_output(r#"{"name": }"#).is_err());
    }

    #[test]
    fn parses_critic_verdict() {
        let approve = parse_critic_output(r#"{"approve":true,"reason":"通用"}"#).unwrap();
        assert!(approve.approve);
        let reject = parse_critic_output("裁决如下 {\"approve\":false} 完毕").unwrap();
        assert!(!reject.approve);
        // 缺字段走 serde default → approve=false
        let missing = parse_critic_output(r#"{"reason":"x"}"#).unwrap();
        assert!(!missing.approve);
    }

    #[test]
    fn build_prompts_include_steps() {
        let p = MinedPattern {
            signature: "grep\u{1f}read".into(),
            steps: vec!["grep".into(), "read".into()],
            count: 4,
            distinct_sessions: 3,
            example_event_ids: vec![],
            anchor: Default::default(),
        };
        let dp = build_draft_prompt(&p, &["在仓库里查 TODO".to_string()]);
        assert!(dp.contains("grep → read"));
        assert!(dp.contains("3 个不同会话"));
        let d = DraftOutput { name: "x".into(), description: "d".into(), when_to_use: None, body: "b".into() };
        let cp = build_critic_prompt(&d, &p);
        assert!(cp.contains("name: x"));
    }
}
