//! Pure browser action effect classification.
//!
//! The classifier is descriptive only. Selected Browser actions execute
//! directly; the classification remains useful for concurrency and completion
//! effect accounting.

use nomi_protocol::events::ToolCategory;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionEffect {
    Info,
    Edit,
    Exec,
    Irreversible,
}

impl ActionEffect {
    pub fn to_category(self) -> ToolCategory {
        match self {
            Self::Info => ToolCategory::Info,
            Self::Edit => ToolCategory::Edit,
            Self::Exec => ToolCategory::Exec,
            Self::Irreversible => ToolCategory::Irreversible,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ActionContext {
    pub element_accname: Option<String>,
    pub element_role: Option<String>,
    pub is_submit_control: bool,
    pub is_cross_origin_post: bool,
    pub enter_submits_form: bool,
    pub reload_resubmits_post: bool,
}

const IRREVERSIBLE_EN_WORDS: &[&str] = &[
    "pay",
    "purchase",
    "checkout",
    "buy",
    "order now",
    "place order",
    "submit",
    "confirm",
    "delete",
    "remove",
    "send",
    "transfer",
    "withdraw",
    "subscribe",
    "sign contract",
    "agree and",
];

const IRREVERSIBLE_CN_WORDS: &[&str] = &[
    "付款",
    "支付",
    "删除",
    "移除",
    "发送",
    "发布",
    "确认",
    "提交",
    "购买",
    "下单",
    "结账",
    "结算",
    "转账",
    "提现",
    "订阅",
    "立即购买",
    "确定支付",
    "同意并",
];

const EN_FALSE_POSITIVE_HINTS: &[&str] = &["display", "replay", "repaper"];

pub fn accname_is_irreversible(accname: &str) -> bool {
    let trimmed = accname.trim();
    if trimmed.is_empty() {
        return false;
    }
    if IRREVERSIBLE_CN_WORDS
        .iter()
        .any(|word| trimmed.contains(word))
    {
        return true;
    }

    let lower = trimmed.to_lowercase();
    if !IRREVERSIBLE_EN_WORDS
        .iter()
        .any(|word| lower.contains(word))
    {
        return false;
    }
    if EN_FALSE_POSITIVE_HINTS
        .iter()
        .any(|word| lower.contains(word))
    {
        return IRREVERSIBLE_EN_WORDS
            .iter()
            .filter(|word| **word != "pay")
            .any(|word| lower.contains(word));
    }
    true
}

pub fn classify_action(action: &str, context: &ActionContext) -> ActionEffect {
    if context.is_cross_origin_post {
        return ActionEffect::Irreversible;
    }

    match action {
        "navigate"
        | "observe"
        | "screenshot"
        | "capabilities"
        | "get_page_text"
        | "search_page"
        | "find_elements"
        | "get_dropdown_options"
        | "cursor"
        | "wait"
        | "wait_for"
        | "tabs"
        | "extract"
        | "get_console_logs"
        | "get_page_errors"
        | "get_network_log" => ActionEffect::Info,
        "click" => {
            if context.is_submit_control
                || context
                    .element_accname
                    .as_deref()
                    .is_some_and(accname_is_irreversible)
            {
                ActionEffect::Irreversible
            } else {
                ActionEffect::Exec
            }
        }
        "press_key" if context.enter_submits_form => ActionEffect::Irreversible,
        "reload" if context.reload_resubmits_post => ActionEffect::Irreversible,
        _ => ActionEffect::Exec,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_accessible_names_are_irreversible() {
        for name in [
            "Pay now",
            "Complete purchase",
            "Submit order",
            "Delete account",
            "Send message",
            "立即支付",
            "删除账户",
            "提交订单",
        ] {
            assert!(accname_is_irreversible(name), "{name:?}");
        }
    }

    #[test]
    fn benign_accessible_names_remain_non_irreversible() {
        for name in ["Display details", "Replay video", "Show more", "Cancel"] {
            assert!(!accname_is_irreversible(name), "{name:?}");
        }
    }

    #[test]
    fn classifier_preserves_effect_categories() {
        assert_eq!(
            classify_action("observe", &ActionContext::default()),
            ActionEffect::Info
        );
        assert_eq!(
            classify_action("click", &ActionContext::default()),
            ActionEffect::Exec
        );
        assert_eq!(
            classify_action(
                "click",
                &ActionContext {
                    element_accname: Some("Pay now".into()),
                    ..Default::default()
                }
            ),
            ActionEffect::Irreversible
        );
        assert_eq!(
            classify_action(
                "press_key",
                &ActionContext {
                    enter_submits_form: true,
                    ..Default::default()
                }
            ),
            ActionEffect::Irreversible
        );
        assert_eq!(
            classify_action(
                "reload",
                &ActionContext {
                    reload_resubmits_post: true,
                    ..Default::default()
                }
            ),
            ActionEffect::Irreversible
        );
    }

    #[test]
    fn effect_maps_to_tool_category() {
        assert_eq!(ActionEffect::Info.to_category(), ToolCategory::Info);
        assert_eq!(ActionEffect::Edit.to_category(), ToolCategory::Edit);
        assert_eq!(ActionEffect::Exec.to_category(), ToolCategory::Exec);
        assert_eq!(
            ActionEffect::Irreversible.to_category(),
            ToolCategory::Irreversible
        );
    }
}
