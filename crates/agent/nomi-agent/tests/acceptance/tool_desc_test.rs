// Acceptance test for tool usage guidance in the system prompt (TC-A4-01).
//
// This is a LOCAL test — no LLM call required.

use nomi_agent::context::{SystemPromptCache, build_system_prompt};

/// TC-A4-01: System prompt contains tool guidance.
///
/// Calls `build_system_prompt` with minimal arguments and verifies that the
/// returned prompt includes the tool-usage guidance section with all expected
/// content: heading, Bash prohibition mappings, parallel call guidance,
/// Edit-over-Write preference, and Read-before-Edit rule.
#[test]
fn system_prompt_contains_tool_guidance() {
    let prompt = build_system_prompt(
        &mut SystemPromptCache::new(),
        None,
        "/tmp",
        "test-model",
        &[],
        None,
        None,
        false,
        false,
        false,
    );

    // 1. Heading
    assert!(
        prompt.contains("# Using your tools"),
        "system prompt must contain the '# Using your tools' heading"
    );

    // 2. Bash prohibition mappings — dedicated tool replacements
    assert!(
        prompt.contains("Glob"),
        "should mention Glob as replacement for find/ls"
    );
    assert!(
        prompt.contains("Grep"),
        "should mention Grep as replacement for grep/rg"
    );
    assert!(
        prompt.contains("Read"),
        "should mention Read as replacement for cat/head/tail"
    );
    assert!(
        prompt.contains("Edit"),
        "should mention Edit as replacement for sed/awk"
    );
    assert!(
        prompt.contains("Write"),
        "should mention Write as replacement for echo redirection"
    );

    // 3. Parallel call guidance
    assert!(
        prompt.contains("parallel"),
        "should contain parallel call guidance"
    );

    // 4. Edit-over-Write preference
    assert!(
        prompt.contains("Prefer Edit over Write"),
        "should contain Edit-over-Write preference"
    );

    // 5. Read-before-Edit rule
    assert!(
        prompt.contains("Read a file before editing"),
        "should contain Read-before-Edit rule"
    );
}
