use std::sync::OnceLock;

use regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionClass {
    Options,
    OpenQuestion,
    Sensitive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionOption {
    pub key: String,
    pub text: String,
    pub recommended: bool,
}

impl DecisionOption {
    pub fn reply(&self) -> String {
        if self.key.trim().is_empty() {
            self.text.clone()
        } else {
            self.key.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionPrompt {
    pub class: DecisionClass,
    pub question: String,
    pub options: Vec<DecisionOption>,
}

fn option_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*(?:(?<number>\d{1,2})\s*[\.\)\]、:]|(?<letter>[a-z])\s*[\.\)\]、:]|[-*•])\s*(?<text>\S.*)$",
        )
        .expect("IDMM option regex is static")
    })
}

fn has_question_cue(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "请选择",
        "请回复",
        "请输入",
        "请提供",
        "请问",
        "你希望",
        "您希望",
        "你的选择",
        "您的选择",
        "是否要",
        "需要我",
        "选哪",
        "选择一个",
        "等待你的",
        "which option",
        "which approach",
        "which one",
        "what would you",
        "how would you",
        "your choice",
        "please choose",
        "please select",
        "please enter",
        "please provide",
        "reply with",
        "would you like",
        "do you want",
        "waiting for your",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
}

fn is_sensitive_question(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "password",
        "api key",
        "secret",
        "credential",
        "验证码",
        "密码",
        "密钥",
        "凭据",
        "付款",
        "支付",
        "购买",
        "授权访问",
        "grant permission",
        "allow access",
        "administrator permission",
        "sudo password",
    ]
    .iter()
    .any(|signal| lower.contains(signal))
}

pub(crate) fn is_destructive(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "rm -rf",
        "rm -fr",
        "drop table",
        "drop database",
        "truncate table",
        "delete from",
        "force push",
        "push --force",
        "reset --hard",
        "git clean -",
        "永久删除",
        "彻底删除",
        "删除所有",
        "删除全部",
        "清空数据",
        "格式化磁盘",
        "覆盖原文件",
        "发布到生产",
        "直接上线",
        "提交并推送",
        "发送邮件",
        "overwrite all",
        "remove all",
        "deploy to production",
        "send email",
    ]
    .iter()
    .any(|signal| lower.contains(signal))
}

fn is_cancel_option(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "取消",
        "放弃",
        "跳过",
        "稍后",
        "暂不",
        "退出",
        "什么都不",
        "都不选",
        "cancel",
        "skip",
        "abort",
        "quit",
        "none of",
        "do nothing",
        "never mind",
    ]
    .iter()
    .any(|signal| lower.contains(signal))
}

pub(crate) fn safe_option(option: &DecisionOption) -> bool {
    !is_destructive(&option.text)
        && !is_cancel_option(&option.text)
        && !is_sensitive_question(&option.text)
}

pub(crate) fn rule_answer(
    prompt: &DecisionPrompt,
    prefer_recommended: bool,
) -> Option<String> {
    if prompt.class != DecisionClass::Options {
        return None;
    }
    let safe = prompt.options.iter().filter(|option| safe_option(option));
    if prefer_recommended
        && let Some(option) = safe.clone().find(|option| option.recommended)
    {
        return Some(option.reply());
    }
    safe.into_iter().next().map(DecisionOption::reply)
}

/// Detect only an explicit hand-off to the user. Ordinary prose containing a
/// list is not a decision merely because it has numbered lines.
pub fn detect_decision(text: &str) -> Option<DecisionPrompt> {
    let boundary = text
        .char_indices()
        .rev()
        .take_while(|(index, _)| text.len().saturating_sub(*index) <= 32_000)
        .last()
        .map_or(0, |(index, _)| index);
    let bounded = &text[boundary..];
    if !has_question_cue(bounded) {
        return None;
    }
    let lines = bounded.lines().collect::<Vec<_>>();
    let mut options = Vec::new();
    for line in &lines {
        let Some(captures) = option_regex().captures(line) else {
            continue;
        };
        let text = captures
            .name("text")
            .map(|value| value.as_str().trim())
            .unwrap_or_default();
        if text.is_empty() || text.len() > 1_000 {
            continue;
        }
        let key = captures
            .name("number")
            .or_else(|| captures.name("letter"))
            .map(|value| value.as_str().to_owned())
            .unwrap_or_default();
        let lower = text.to_lowercase();
        options.push(DecisionOption {
            key,
            text: text.to_owned(),
            recommended: lower.contains("推荐")
                || lower.contains("recommended")
                || lower.contains("default"),
        });
    }
    let question = lines
        .iter()
        .rev()
        .find(|line| has_question_cue(line))
        .copied()
        .unwrap_or(bounded)
        .trim()
        .chars()
        .take(2_000)
        .collect::<String>();
    if is_sensitive_question(bounded) {
        return Some(DecisionPrompt {
            class: DecisionClass::Sensitive,
            question,
            options,
        });
    }
    Some(DecisionPrompt {
        class: if options.len() >= 2 {
            DecisionClass::Options
        } else {
            DecisionClass::OpenQuestion
        },
        question,
        options,
    })
}

pub(crate) fn is_retryable_provider_fault(error: &str) -> bool {
    let lower = error.to_lowercase();
    [
        "429",
        "rate limit",
        "too many requests",
        "timeout",
        "timed out",
        "network",
        "connection reset",
        "connection refused",
        "connection closed",
        "gateway",
        "502",
        "503",
        "504",
        "overloaded",
        "temporarily unavailable",
        "provider unavailable",
        "empty response",
        "stream ended",
        "runtime dispatch failed",
        "模型供应商",
        "限流",
        "网络",
        "连接中断",
        "超时",
        "服务暂不可用",
    ]
    .iter()
    .any(|signal| lower.contains(signal))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_recommended_safe_option() {
        let prompt = detect_decision(
            "请选择下一步：\n1. 删除数据库\n2. 继续分析（推荐）\n3. 取消",
        )
        .unwrap();
        assert_eq!(prompt.class, DecisionClass::Options);
        assert_eq!(rule_answer(&prompt, true).as_deref(), Some("2"));
    }

    #[test]
    fn a_plain_numbered_list_is_not_a_question() {
        assert!(detect_decision("完成内容：\n1. 编译\n2. 测试\n3. 打包").is_none());
    }

    #[test]
    fn a_rhetorical_question_does_not_start_an_automatic_turn() {
        assert!(detect_decision("Everything is complete. Anything else?").is_none());
    }

    #[test]
    fn credentials_are_never_auto_answered() {
        let prompt = detect_decision("请输入 API key 以继续？").unwrap();
        assert_eq!(prompt.class, DecisionClass::Sensitive);
        assert!(rule_answer(&prompt, true).is_none());
    }

    #[test]
    fn detects_provider_failures_without_treating_semantic_failures_as_transient() {
        assert!(is_retryable_provider_fault("provider returned 429 rate limit"));
        assert!(is_retryable_provider_fault("network connection reset"));
        assert!(!is_retryable_provider_fault("completion evidence is incomplete"));
    }
}
