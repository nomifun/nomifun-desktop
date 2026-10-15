//! Pure acceptance classification shared by the live runner and portable tests.
//! A challenge is an observed site result, never proof of normal compatibility.
#[derive(Default)]
pub(crate) struct Evidence<'a> {
    pub text: &'a str,
    pub url: &'a str,
    pub site: &'a str,
    pub native_failed: bool,
    pub current_document_finished: bool,
    pub meaningful_document: bool,
    pub loop_suspected: bool,
    pub late_page_navigations: u64,
    pub late_unknown_navigations: u64,
    pub routing_lost: bool,
    pub host_commands: Option<u64>,
}

pub(crate) fn classify(evidence: &Evidence<'_>) -> (&'static str, bool) {
    let text = evidence.text;
    let lower = text.to_ascii_lowercase();
    if text.contains("安全验证") || text.contains("验证码") || text.contains("人机验证")
        || lower.contains("captcha") || lower.contains("verify you are human") {
        return ("verification_document_observed", false);
    }
    if text.contains("浏览器版本过低") || text.contains("浏览器下载建议")
        || evidence.url.contains("activity-CjJbuaD7Xw") {
        return ("browser_version_rejected", false);
    }
    if evidence.native_failed { return ("native_load_failure", false); }
    if evidence.loop_suspected {
        return ("persistent_page_navigation", false);
    }
    if evidence.late_page_navigations > 0 {
        return ("navigation_not_stable_during_observation", false);
    }
    if evidence.routing_lost || evidence.late_unknown_navigations > 0
        || evidence.host_commands.is_none() || (evidence.site != "manual_result" && evidence.host_commands != Some(1))
        || !evidence.current_document_finished
        || !evidence.meaningful_document {
        return ("insufficient_current_document_evidence", false);
    }
    match evidence.site {
        "baidu_homepage" if text.contains("百度")
            && (text.contains("百度一下") || text.contains("新闻") || text.contains("搜索")) =>
            ("homepage_document_observed", true),
        "baidu_search" if text.contains("百度") && (lower.contains("nomifun")
            || text.contains("搜索工具") || text.contains("下一页")) =>
            ("search_document_observed", true),
        "bilibili" if text.contains("哔哩哔哩") || lower.contains("bilibili") =>
            ("homepage_document_observed", true),
        "manual_result" => ("manual_document_observed", true),
        _ => ("unclassified_site_document", false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled(text: &str) -> Evidence<'_> {
        Evidence { text, site: "baidu_search", current_document_finished: true,
            meaningful_document: true, host_commands: Some(1), ..Default::default() }
    }

    #[test]
    fn challenge_with_search_branding_is_never_normal_success() {
        assert_eq!(classify(&settled("百度 nomifun 搜索工具 安全验证 验证码")),
            ("verification_document_observed", false));
    }

    #[test]
    fn homepage_and_search_have_separate_required_markers() {
        let mut evidence = settled("百度一下 新闻");
        assert_eq!(classify(&evidence), ("unclassified_site_document", false));
        evidence.site = "baidu_homepage";
        assert_eq!(classify(&evidence), ("homepage_document_observed", true));
    }

    #[test]
    fn a_green_final_document_does_not_hide_continuing_reload() {
        let mut evidence = settled("百度 nomifun 搜索工具");
        evidence.late_page_navigations = 1;
        assert_eq!(classify(&evidence), ("navigation_not_stable_during_observation", false));
    }

    #[test]
    fn unknown_or_lost_routing_cannot_prove_stability() {
        let mut evidence = settled("百度 nomifun 搜索工具");
        evidence.routing_lost = true;
        assert_eq!(classify(&evidence), ("insufficient_current_document_evidence", false));
        evidence.routing_lost = false;
        evidence.late_unknown_navigations = 1;
        assert_eq!(classify(&evidence), ("insufficient_current_document_evidence", false));
    }

    #[test]
    fn stable_search_requires_exactly_one_entry_command() {
        let mut evidence = settled("百度 nomifun 搜索工具");
        assert_eq!(classify(&evidence), ("search_document_observed", true));
        evidence.host_commands = Some(2);
        assert_eq!(classify(&evidence), ("insufficient_current_document_evidence", false));
    }

    #[test]
    fn manual_final_document_cannot_reuse_the_normal_idle_outcome() {
        let mut evidence = settled("百度 nomifun 搜索工具");
        evidence.site = "manual_result";
        evidence.host_commands = Some(3);
        assert_eq!(classify(&evidence), ("manual_document_observed", true));
        evidence.text = "安全验证 验证码";
        assert_eq!(classify(&evidence), ("verification_document_observed", false));
        evidence.text = "Visible retained search document";
        evidence.native_failed = true;
        assert_eq!(classify(&evidence), ("native_load_failure", false));
    }

    #[test]
    fn native_loop_warning_and_single_slow_reload_have_distinct_outcomes() {
        let mut evidence = settled("百度 nomifun 搜索工具");
        evidence.loop_suspected = true;
        assert_eq!(classify(&evidence), ("persistent_page_navigation", false));
        evidence.loop_suspected = false;
        evidence.late_page_navigations = 1;
        assert_eq!(classify(&evidence), ("navigation_not_stable_during_observation", false));
    }
}
