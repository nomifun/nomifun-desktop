use nomifun_channel::formatter::format_text_for_platform;
use nomifun_channel::types::PluginType;

// ── Telegram: escape HTML, then convert markdown to HTML tags ────

#[test]
fn telegram_bold_and_code() {
    let input = "**bold** and `code`";
    let result = format_text_for_platform(input, PluginType::Telegram);
    assert!(result.contains("<b>bold</b>"), "got: {result}");
    assert!(result.contains("<code>code</code>"), "got: {result}");
}

#[test]
fn telegram_escapes_raw_html() {
    let input = "<script>alert(1)</script>";
    let result = format_text_for_platform(input, PluginType::Telegram);
    assert!(!result.contains("<script>"), "got: {result}");
    assert!(result.contains("&lt;script&gt;"), "got: {result}");
}

#[test]
fn telegram_code_block() {
    let input = "```rust\nfn main() {}\n```";
    let result = format_text_for_platform(input, PluginType::Telegram);
    assert!(result.contains("<pre><code>"), "got: {result}");
    assert!(result.contains("fn main()"), "got: {result}");
}

#[test]
fn telegram_link() {
    let input = "[click](https://example.com)";
    let result = format_text_for_platform(input, PluginType::Telegram);
    assert!(
        result.contains(r#"<a href="https://example.com">click</a>"#),
        "got: {result}"
    );
}

// ── Lark / DingTalk: HTML tags → markdown syntax ─────────────────

#[test]
fn lark_bold_and_code() {
    let input = "<b>bold</b> and <code>code</code>";
    let result = format_text_for_platform(input, PluginType::Lark);
    assert!(result.contains("**bold**"), "got: {result}");
    assert!(result.contains("`code`"), "got: {result}");
}

#[test]
fn lark_link_with_protocol_whitelist() {
    let input = r#"<a href="https://ok.com">safe</a> <a href="javascript:void(0)">evil</a>"#;
    let result = format_text_for_platform(input, PluginType::Lark);
    assert!(result.contains("[safe](https://ok.com)"), "got: {result}");
    assert!(!result.contains("javascript:"), "got: {result}");
}

#[test]
fn lark_strips_unknown_tags() {
    let input = "<div><b>bold</b></div>";
    let result = format_text_for_platform(input, PluginType::Lark);
    assert!(result.contains("**bold**"), "got: {result}");
    assert!(!result.contains("<div>"), "got: {result}");
}

#[test]
fn dingtalk_same_output_as_lark() {
    let input = "<b>bold</b> and <i>italic</i>";
    let lark = format_text_for_platform(input, PluginType::Lark);
    let ding = format_text_for_platform(input, PluginType::Dingtalk);
    assert_eq!(lark, ding);
}

// ── WeChat: strip all HTML ───────────────────────────────────────

#[test]
fn weixin_strips_all_html() {
    let input = "<b>bold</b> and <a href=\"url\">link</a>";
    let result = format_text_for_platform(input, PluginType::Weixin);
    assert!(!result.contains('<'), "got: {result}");
    assert!(!result.contains('>'), "got: {result}");
    assert!(result.contains("bold"), "got: {result}");
    assert!(result.contains("link"), "got: {result}");
}

#[test]
fn weixin_decodes_entities() {
    let input = "&amp; &lt;tag&gt;";
    let result = format_text_for_platform(input, PluginType::Weixin);
    assert_eq!(result.trim(), "& tag");
}

#[test]
fn weixin_nested_tags() {
    let input = "<scr<script>ipt>alert(1)</scr</script>ipt>";
    let result = format_text_for_platform(input, PluginType::Weixin);
    assert!(!result.contains('<'), "got: {result}");
}

// ── Fallback: escape HTML ────────────────────────────────────────

#[test]
fn fallback_escapes_html() {
    let input = "<b>bold</b>";
    let result = format_text_for_platform(input, PluginType::Slack);
    assert!(result.contains("&lt;b&gt;"), "got: {result}");
}
