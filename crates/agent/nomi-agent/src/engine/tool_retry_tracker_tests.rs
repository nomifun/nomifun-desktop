use super::{
    SKIPPED_AFTER_PRIOR_ERROR, ToolRetryTracker,
    confirmed_predispatch_schema_invalid_call_ids,
};
use nomi_types::message::ContentBlock;
use serde_json::json;
use std::collections::HashSet;

fn call(id: &str, name: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id.to_owned(),
        name: name.to_owned(),
        input: json!({ "tasks": ["invalid"] }),
        extra: None,
    }
}

#[test]
fn inherits_only_one_immediately_previous_same_name_schema_failure() {
    let mut tracker = ToolRetryTracker::default();
    let first = vec![call("call-1", "nomi_delegate")];
    let first_contexts = tracker.assign(&first).unwrap();
    tracker.observe_invalid_arguments(
        &first,
        &first_contexts,
        &HashSet::from(["call-1".to_owned()]),
    );

    let second = vec![call("call-2", "nomi_delegate")];
    let second_contexts = tracker.assign(&second).unwrap();
    let retry = &second_contexts["call-2"].retry;
    assert_eq!(retry.retry_group_id, "call-1");
    assert_eq!(retry.attempt_no, 2);
    assert_eq!(retry.retry_of_call_id.as_deref(), Some("call-1"));

    // An intervening model round with no calls expires the candidate.
    tracker.observe_invalid_arguments(
        &second,
        &second_contexts,
        &HashSet::from(["call-2".to_owned()]),
    );
    assert!(tracker.assign(&[]).unwrap().is_empty());
    let third = tracker
        .assign(&[call("call-3", "nomi_delegate")])
        .unwrap();
    assert_eq!(third["call-3"].retry.retry_group_id, "call-3");
}

#[test]
fn parallel_same_name_calls_never_inherit() {
    let mut tracker = ToolRetryTracker::default();
    let first = vec![call("call-1", "nomi_delegate")];
    let contexts = tracker.assign(&first).unwrap();
    tracker.observe_invalid_arguments(
        &first,
        &contexts,
        &HashSet::from(["call-1".to_owned()]),
    );

    let parallel = vec![
        call("call-2", "nomi_delegate"),
        call("call-3", "nomi_delegate"),
    ];
    let contexts = tracker.assign(&parallel).unwrap();
    assert_eq!(contexts["call-2"].retry.retry_group_id, "call-2");
    assert_eq!(contexts["call-3"].retry.retry_group_id, "call-3");
}

#[test]
fn previous_round_same_name_parallel_calls_never_inherit() {
    let mut tracker = ToolRetryTracker::default();
    let previous = vec![
        call("call-1", "nomi_delegate"),
        call("call-2", "nomi_delegate"),
    ];
    let contexts = tracker.assign(&previous).unwrap();
    tracker.observe_invalid_arguments(
        &previous,
        &contexts,
        &HashSet::from(["call-1".to_owned()]),
    );

    let next = tracker
        .assign(&[call("call-3", "nomi_delegate")])
        .unwrap();
    assert_eq!(next["call-3"].retry.retry_group_id, "call-3");
    assert_eq!(next["call-3"].retry.attempt_no, 1);
    assert_eq!(next["call-3"].retry.retry_of_call_id, None);
}

#[test]
fn rejects_provider_call_id_reuse_across_rounds_before_self_retry_assignment() {
    let mut tracker = ToolRetryTracker::default();
    let first = vec![call("call-1", "nomi_delegate")];
    let contexts = tracker.assign(&first).unwrap();
    tracker.observe_invalid_arguments(
        &first,
        &contexts,
        &HashSet::from(["call-1".to_owned()]),
    );

    assert_eq!(
        tracker.assign(&[call("call-1", "nomi_delegate")]),
        Err("call-1".to_owned())
    );
}

fn result(id: &str, is_error: bool) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_use_id: id.to_owned(),
        content: "opaque structured outcome".to_owned(),
        is_error,
        images: Vec::new(),
    }
}

#[test]
fn confirms_only_schema_invalid_outcomes_before_the_error_barrier() {
    let preflight =
        HashSet::from(["invalid-first".to_owned(), "invalid-skipped".to_owned()]);
    let confirmed = confirmed_predispatch_schema_invalid_call_ids(
        &preflight,
        &[
            result("ok", false),
            result("invalid-first", true),
            result("invalid-skipped", true),
        ],
    );

    assert_eq!(confirmed, HashSet::from(["invalid-first".to_owned()]));
}

#[test]
fn skipped_schema_invalid_call_after_runtime_error_is_not_a_retry_candidate() {
    let preflight = HashSet::from(["invalid-skipped".to_owned()]);
    let confirmed = confirmed_predispatch_schema_invalid_call_ids(
        &preflight,
        &[
            result("runtime-error", true),
            ContentBlock::ToolResult {
                tool_use_id: "invalid-skipped".to_owned(),
                content: SKIPPED_AFTER_PRIOR_ERROR.to_owned(),
                is_error: true,
                images: Vec::new(),
            },
        ],
    );

    assert!(confirmed.is_empty());
}
