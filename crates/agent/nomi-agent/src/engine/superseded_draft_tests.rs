//! A pre-tool text draft that the same round then writes to disk must not stay
//! in engine history at full length: it is re-sent to the provider on every
//! later turn of the task, so a single 5 KB draft costs thousands of tokens per
//! turn for the rest of a long session.

use super::supersede_written_draft;
use nomi_types::message::ContentBlock;

/// A body long enough to clear the minimum, with distinctive non-trivial lines.
fn code_body(marker: &str) -> String {
    let mut body = String::new();
    for i in 0..30 {
        body.push_str(&format!(
            "it(\"{marker} case {i} satisfies the documented contract\", () => {{\n  \
             expect(runCli([\"list\", \"--db\", \"tasks.json\"]).exitCode).toBe(0);\n}});\n"
        ));
    }
    body
}

fn text(text: &str) -> ContentBlock {
    ContentBlock::Text {
        text: text.to_string(),
    }
}

fn write_call(content: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: "t1".to_string(),
        name: "Write".to_string(),
        input: serde_json::json!({ "file_path": "tests/cli.test.ts", "content": content }),
        extra: None,
    }
}

fn text_of(content: &[ContentBlock]) -> &str {
    content
        .iter()
        .find_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .expect("a text block is kept in place")
}

#[test]
fn a_draft_written_verbatim_in_the_same_round_is_replaced_by_a_marker() {
    let draft = code_body("verbatim");
    let mut content = vec![text(&draft), write_call(&draft)];

    assert!(supersede_written_draft(&mut content));

    let text = text_of(&content);
    assert!(
        !text.contains("satisfies the documented contract"),
        "every substantive draft line is dropped: {text}"
    );
    assert!(
        text.chars().count() * 4 < draft.chars().count(),
        "the block shrinks dramatically: {} -> {} chars",
        draft.chars().count(),
        text.chars().count()
    );
    assert!(
        text.contains("tests/cli.test.ts"),
        "the marker names the file that superseded it: {text}"
    );
    assert!(
        matches!(&content[1], ContentBlock::ToolUse { name, .. } if name == "Write"),
        "the tool call itself is untouched: {content:?}"
    );
}

/// The critical false-positive guard: the engine coalesces a round's text deltas
/// into ONE block, so an explanation and a draft routinely share a block.
/// Collapsing the block wholesale would silently destroy the explanation, and the
/// model would have no record on any later turn that it ever knew it.
#[test]
fn prose_sharing_a_block_with_a_written_draft_is_preserved() {
    let warning = "IMPORTANT: also bump the schema version in migrations/007.sql \
                   or the deploy fails silently.";
    let draft = code_body("mixed");
    let block = format!("{warning}\n{draft}");
    let mut content = vec![text(&block), write_call(&draft)];

    assert!(supersede_written_draft(&mut content));

    let text = text_of(&content);
    assert!(
        text.contains(warning),
        "prose the write never persisted must survive: {text}"
    );
    assert!(
        !text.contains("satisfies the documented contract"),
        "the written lines are still dropped: {text}"
    );
}

/// Short narration before a tool call is legitimate and must stay verbatim.
#[test]
fn short_narration_before_a_write_is_preserved() {
    let narration = "Let me write the CLI test file now.";
    let mut content = vec![text(narration), write_call(&code_body("n"))];

    assert!(!supersede_written_draft(&mut content));
    assert_eq!(text_of(&content), narration);
}

/// A long explanation that the write does NOT contain is real content.
#[test]
fn a_long_text_the_write_does_not_contain_is_preserved() {
    let explanation = "Here is my reasoning about the contract, at length. ".repeat(30);
    let mut content = vec![text(&explanation), write_call("something entirely different")];

    assert!(!supersede_written_draft(&mut content));
    assert_eq!(text_of(&content), explanation);
}

/// Only file-writing tools supersede a draft: a Bash echo of the same text is a
/// command, not a persisted artifact, so the text may still be the answer.
#[test]
fn a_non_write_tool_does_not_supersede_a_draft() {
    let draft = code_body("bash");
    let mut content = vec![
        text(&draft),
        ContentBlock::ToolUse {
            id: "t1".to_string(),
            name: "Bash".to_string(),
            input: serde_json::json!({ "command": draft.clone() }),
            extra: None,
        },
    ];

    assert!(!supersede_written_draft(&mut content));
    assert_eq!(text_of(&content), draft);
}

/// A text-only round is a final answer and must never be rewritten.
#[test]
fn a_round_without_tool_calls_is_never_superseded() {
    let answer = "The task is complete. Here is a long summary. ".repeat(30);
    let mut content = vec![text(&answer)];

    assert!(!supersede_written_draft(&mut content));
    assert_eq!(text_of(&content), answer);
}

/// Whitespace-only reindentation between draft and write still counts as the
/// same content: models routinely reformat while emitting tool arguments.
#[test]
fn reindented_draft_still_matches_the_write() {
    let draft = code_body("reindent");
    let reformatted = draft.replace("  ", "\t");
    let mut content = vec![text(&draft), write_call(&reformatted)];

    assert!(supersede_written_draft(&mut content));
}

/// The observed bad case: the model revised the draft while emitting the tool
/// arguments (a sync helper became async). Line-level matching keeps the
/// unchanged bulk collapsible while the revised lines are preserved as content.
#[test]
fn a_revised_draft_is_still_superseded() {
    let signature =
        "function runCli(args: string[]): { stdout: string; exitCode: number | null } {";
    let draft = format!("{signature}\n{}", code_body("revised"));
    let written = draft.replace(
        signature,
        "async function runCli(args: string[]): Promise<{ stdout: string }> {",
    );
    assert_ne!(written, draft);

    let mut content = vec![text(&draft), write_call(&written)];
    assert!(
        supersede_written_draft(&mut content),
        "the unchanged bulk of a revised draft must still be collapsed"
    );
    assert!(
        text_of(&content).contains(signature),
        "the line the write actually changed is not persisted, so it is kept"
    );
}

/// Edit tools carry the body under a different key and must be covered too.
#[test]
fn an_edit_new_string_also_supersedes_a_draft() {
    let draft = code_body("edit");
    let mut content = vec![
        text(&draft),
        ContentBlock::ToolUse {
            id: "t1".to_string(),
            name: "Edit".to_string(),
            input: serde_json::json!({
                "file_path": "src/store.ts",
                "old_string": "stub",
                "new_string": draft.clone(),
            }),
            extra: None,
        },
    ];

    assert!(supersede_written_draft(&mut content));
    assert!(text_of(&content).contains("src/store.ts"));
}

/// ApplyPatch nests its bodies under `files[]`; the tool the model is steered
/// toward for multi-file work must not be a blind spot.
#[test]
fn an_apply_patch_body_supersedes_a_draft() {
    let draft = code_body("patch");
    let mut content = vec![
        text(&draft),
        ContentBlock::ToolUse {
            id: "t1".to_string(),
            name: "ApplyPatch".to_string(),
            input: serde_json::json!({
                "files": [{ "file_path": "src/cli.ts", "content": draft.clone() }],
            }),
            extra: None,
        },
    ];

    assert!(supersede_written_draft(&mut content));
    assert!(text_of(&content).contains("src/cli.ts"));
}

/// The marker itself must never be re-collapsed on a later pass.
#[test]
fn superseding_is_idempotent() {
    let draft = code_body("idempotent");
    let mut content = vec![text(&draft), write_call(&draft)];
    assert!(supersede_written_draft(&mut content));
    let after_first = format!("{content:?}");
    assert!(!supersede_written_draft(&mut content));
    assert_eq!(after_first, format!("{content:?}"));
}

/// Trivial lines recur in unrelated files, so matching them must not by itself
/// collapse anything.
#[test]
fn trivial_shared_lines_alone_do_not_supersede() {
    let prose = "}\n};\nreturn;\n".repeat(60);
    let mut content = vec![text(&prose), write_call("}\n};\nreturn;\n")];

    assert!(!supersede_written_draft(&mut content));
}
