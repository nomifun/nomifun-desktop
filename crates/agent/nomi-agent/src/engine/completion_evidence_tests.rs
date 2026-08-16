//! A model reporting spec-driven work as delivered must have looked at the spec
//! since it started editing. The observed failure read README/QA_TASK in the
//! first few messages, then wrote code for 57 more without re-reading either,
//! invented its own CLI interface, wrote tests around the inventions, and
//! truthfully reported that those tests passed — `bun test` really did exit 0,
//! while an independent contract verifier scored 0/10.

use super::{SPEC_RECHECK_NUDGE, unbacked_completion_claim};
use nomi_types::message::{ContentBlock, Message, Role};

fn read(path: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: "r".to_string(),
        name: "Read".to_string(),
        input: serde_json::json!({ "file_path": path }),
        extra: None,
    }
}

fn write(path: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: "w".to_string(),
        name: "Write".to_string(),
        input: serde_json::json!({ "file_path": path, "content": "x" }),
        extra: None,
    }
}

fn round(block: ContentBlock) -> Message {
    Message::new(Role::Assistant, vec![block])
}

/// The exact reported shape: spec read early, then implementation only.
fn the_bad_case() -> Vec<Message> {
    vec![
        round(read("workspace/README.md")),
        round(read("workspace/QA_TASK.md")),
        round(write("workspace/src/csv.ts")),
        round(write("workspace/tests/cli.test.ts")),
    ]
}

/// The observed final summary, verbatim in spirit: truthful about its own tests,
/// wrong about the contract.
const FALSE_GREEN: &str = "交付总结：bun test 20 pass / 0 fail，现有测试全部保留且通过，\
项目当前处于可交付状态。";

#[test]
fn the_reported_false_green_delivery_is_gated() {
    assert_eq!(
        unbacked_completion_claim(FALSE_GREEN, &the_bad_case()),
        Some(SPEC_RECHECK_NUDGE)
    );
}

#[test]
fn re_reading_the_spec_after_editing_satisfies_the_gate() {
    // The remedy the nudge asks for. Doing it must end the turn normally,
    // otherwise the gate would loop forever.
    let mut messages = the_bad_case();
    messages.push(round(read("workspace/README.md")));
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn an_english_completion_claim_is_gated_too() {
    for claim in [
        "All tests pass. The project is deliverable.",
        "20 pass / 0 fail and the work is ready to ship.",
    ] {
        assert!(
            unbacked_completion_claim(claim, &the_bad_case()).is_some(),
            "must gate: {claim}"
        );
    }
}

#[test]
fn an_ordinary_answer_is_never_gated() {
    for answer in [
        "Here is a summary of what the CSV parser does.",
        "The contract requires --db; I have not implemented it yet.",
        "I could not get the tests to run; the runner is missing.",
        "",
    ] {
        assert_eq!(
            unbacked_completion_claim(answer, &the_bad_case()),
            None,
            "must not gate: {answer}"
        );
    }
}

#[test]
fn a_turn_with_no_spec_to_check_is_never_gated() {
    // Ad-hoc work with no contract has nothing to re-read, so the gate must not
    // fire and demand a file that does not exist.
    let messages = vec![round(write("src/main.rs"))];
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn a_read_only_turn_is_never_gated() {
    // Nothing was changed, so there is no implementation to have drifted.
    let messages = vec![round(read("README.md")), round(read("src/cli.ts"))];
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn a_spec_read_only_after_the_last_write_still_counts() {
    // Order is what matters, not how many times the spec was read.
    let messages = vec![
        round(write("src/cli.ts")),
        round(read("docs/REQUIREMENTS.md")),
    ];
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn non_spec_markdown_does_not_count_as_a_contract() {
    // Editing a changelog is not consulting a contract.
    let messages = vec![
        round(read("CHANGELOG.md")),
        round(write("src/cli.ts")),
    ];
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn a_batch_read_of_the_spec_is_recognized() {
    // Read accepts file_paths for several files at once.
    let messages = vec![
        round(write("src/cli.ts")),
        round(ContentBlock::ToolUse {
            id: "r".to_string(),
            name: "Read".to_string(),
            input: serde_json::json!({ "file_paths": ["src/store.ts", "README.md"] }),
            extra: None,
        }),
    ];
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn the_nudge_names_the_concrete_remedy() {
    // A gate that only objects would just make the model rephrase the claim.
    assert!(SPEC_RECHECK_NUDGE.contains("Re-read the spec"), "{SPEC_RECHECK_NUDGE}");
    assert!(
        SPEC_RECHECK_NUDGE.contains("each required behavior"),
        "{SPEC_RECHECK_NUDGE}"
    );
}
