//! Best-effort 文本脱敏：把文本里常见的 API key / token / 私钥 / `key=value`
//! 形式的 secret 替换成 `[REDACTED_SECRET]`。用于「记忆 / 日志 / 事件 写入磁盘
//! 前」堵泄漏面。移植自 codex `secrets/src/sanitizer.rs`。
//!
//! 注意：这是「尽力而为」的正则脱敏，不保证抓全；它**不是**加密，也**不**替代
//! 「不要把密钥写进会被持久化的文本」这一原则。

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

const PLACEHOLDER: &str = "[REDACTED_SECRET]";

static OPENAI_KEY_REGEX: LazyLock<Regex> = LazyLock::new(|| compile(r"sk-[A-Za-z0-9]{20,}"));
static AWS_ACCESS_KEY_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| compile(r"\bAKIA[0-9A-Z]{16}\b"));
static BEARER_TOKEN_REGEX: LazyLock<Regex> =
    LazyLock::new(|| compile(r"(?i)\bBearer\s+[A-Za-z0-9._\-]{16,}\b"));
static SECRET_ASSIGNMENT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    compile(r#"(?i)\b(api[_-]?key|token|secret|password)\b(\s*[:=]\s*)(["']?)[^\s"']{8,}"#)
});
// nomi 增量第 5 条：PEM 私钥块。命中即把整块 BEGIN..END 抹掉。
static PEM_PRIVATE_KEY_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    compile(
        r"(?s)-----BEGIN (?:RSA |EC |OPENSSH |DSA |PGP )?PRIVATE KEY-----.*?-----END (?:RSA |EC |OPENSSH |DSA |PGP )?PRIVATE KEY-----",
    )
});

/// 对 `input` 做 best-effort 脱敏。无任何命中时返回 `Cow::Borrowed`（零分配）。
pub fn redact_secrets(input: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(input);
    out = apply(out, &OPENAI_KEY_REGEX, PLACEHOLDER);
    out = apply(out, &AWS_ACCESS_KEY_ID_REGEX, PLACEHOLDER);
    out = apply(out, &BEARER_TOKEN_REGEX, "Bearer [REDACTED_SECRET]");
    out = apply(out, &SECRET_ASSIGNMENT_REGEX, "$1$2$3[REDACTED_SECRET]");
    out = apply(out, &PEM_PRIVATE_KEY_REGEX, PLACEHOLDER);
    out
}

/// codex 同形签名（String -> String），便于 `.map(redact_secrets_owned)` 链式调用。
pub fn redact_secrets_owned(input: String) -> String {
    match redact_secrets(&input) {
        Cow::Borrowed(_) => input, // 无命中，原样归还，零拷贝
        Cow::Owned(s) => s,
    }
}

/// 对 Cow 链式应用一条正则：保持「无命中不分配」的语义。
fn apply<'a>(input: Cow<'a, str>, re: &Regex, repl: &str) -> Cow<'a, str> {
    match input {
        Cow::Borrowed(s) => re.replace_all(s, repl),
        Cow::Owned(s) => Cow::Owned(re.replace_all(&s, repl).into_owned()),
    }
}

fn compile(pattern: &str) -> Regex {
    // Panic 由 `compiles_all_patterns` 测试兜底，保证不会带病发布。
    Regex::new(pattern).unwrap_or_else(|e| panic!("invalid regex `{pattern}`: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_all_patterns() {
        let _ = redact_secrets("warmup");
    }

    #[test]
    fn redacts_openai_key() {
        let out = redact_secrets("key is sk-ABCDEFGHIJ0123456789xyz here");
        assert!(out.contains("[REDACTED_SECRET]"));
        assert!(!out.contains("sk-ABCDEFGHIJ"));
    }

    #[test]
    fn redacts_aws_access_key_id() {
        let out = redact_secrets("AKIAIOSFODNN7EXAMPLE in config");
        assert!(out.contains("[REDACTED_SECRET]"));
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn redacts_bearer_token_keeps_prefix() {
        let out = redact_secrets("Authorization: Bearer abcdef0123456789ABCDEF");
        assert!(out.contains("Bearer [REDACTED_SECRET]"));
        assert!(!out.contains("abcdef0123456789ABCDEF"));
    }

    #[test]
    fn redacts_assignment_keeps_key_and_separator() {
        let out = redact_secrets(r#"password = "hunter2supersecret""#);
        assert!(out.starts_with(r#"password = "[REDACTED_SECRET]"#));
        assert!(!out.contains("hunter2supersecret"));
    }

    #[test]
    fn redacts_api_key_variants() {
        for s in [
            "api_key=sk_live_0123456789abcdef",
            "API-KEY: 0123456789abcdef",
            "token:abcdefgh12345678",
        ] {
            assert!(redact_secrets(s).contains("[REDACTED_SECRET]"), "miss: {s}");
        }
    }

    #[test]
    fn redacts_pem_private_key_block() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIabc...\nxyz==\n-----END RSA PRIVATE KEY-----";
        let out = redact_secrets(pem);
        assert!(out.contains("[REDACTED_SECRET]"));
        assert!(!out.contains("MIIabc"));
    }

    #[test]
    fn leaves_normal_text_untouched() {
        for s in [
            "主人喜欢用 Rust 写后端，偏好 tokio。",
            "今天修了 3 个编译错误，心情不错。",
            "项目部署在 docker-compose 上，端口 8080。",
            "短值不脱敏：token=abc",
            "提到 password 这个词但没赋值",
            "skirt 和 skill 不是 sk- key",
        ] {
            let out = redact_secrets(s);
            assert!(!out.contains("[REDACTED_SECRET]"), "false positive: {s}");
            assert!(matches!(out, Cow::Borrowed(_)), "应零分配: {s}");
        }
    }

    #[test]
    fn cow_borrowed_when_clean_owned_when_hit() {
        assert!(matches!(redact_secrets("clean text"), Cow::Borrowed(_)));
        assert!(matches!(
            redact_secrets("sk-ABCDEFGHIJ0123456789xyz"),
            Cow::Owned(_)
        ));
    }

    #[test]
    fn owned_signature_roundtrips() {
        assert_eq!(redact_secrets_owned("clean".into()), "clean");
        assert!(redact_secrets_owned("sk-ABCDEFGHIJ0123456789xyz".into()).contains("[REDACTED_SECRET]"));
    }
}
