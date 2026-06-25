// Integration tests for memory system prompt construction (TC-6).
//
// These are black-box tests that exercise the public API of the prompt
// module against the functional requirements in test-plan.md.

use std::fs;
use std::path::Path;

use nomi_memory::prompt::{
    build_memory_instructions, build_memory_prompt, memory_type_descriptions,
};

// ---------------------------------------------------------------------------
// TC-6.1: Complete prompt contains all required sections
// ---------------------------------------------------------------------------

#[test]
fn tc_6_1_prompt_contains_all_required_parts() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");
    fs::create_dir_all(&mem_dir).unwrap();

    let prompt = build_memory_prompt(&mem_dir);

    // Memory system introduction
    assert!(
        prompt.contains("persistent, file-based memory system"),
        "should contain memory system introduction"
    );

    // 4 type definitions
    for ty in ["user", "feedback", "project", "reference"] {
        assert!(
            prompt.contains(&format!("<name>{ty}</name>")),
            "should contain type definition for: {ty}"
        );
    }

    // What not to save
    assert!(
        prompt.contains("What NOT to save"),
        "should contain what-not-to-save section"
    );

    // Save steps
    assert!(
        prompt.contains("How to save memories"),
        "should contain save instructions"
    );

    // When to access
    assert!(
        prompt.contains("When to access memories"),
        "should contain access guidance"
    );

    // MEMORY.md content or empty-state message
    assert!(
        prompt.contains("MEMORY.md"),
        "should reference MEMORY.md entrypoint"
    );
}

// ---------------------------------------------------------------------------
// TC-6.2: Prompt includes the memory directory path
// ---------------------------------------------------------------------------

#[test]
fn tc_6_2_prompt_includes_memory_dir_path() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("my_project_memory");
    fs::create_dir_all(&mem_dir).unwrap();

    let prompt = build_memory_prompt(&mem_dir);

    assert!(
        prompt.contains(&mem_dir.display().to_string()),
        "prompt should contain the memory directory path"
    );
}

// ---------------------------------------------------------------------------
// TC-6.3: With MEMORY.md present, prompt includes its content
// ---------------------------------------------------------------------------

#[test]
fn tc_6_3_prompt_includes_memory_md_content() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");
    fs::create_dir_all(&mem_dir).unwrap();

    let index_content = "- [User Role](user_role.md) \u{2014} senior engineer\n\
                         - [Test Policy](feedback_tests.md) \u{2014} always use real DB\n";
    fs::write(mem_dir.join("MEMORY.md"), index_content).unwrap();

    let prompt = build_memory_prompt(&mem_dir);

    assert!(
        prompt.contains("user_role.md"),
        "prompt should contain index entry references"
    );
    assert!(
        prompt.contains("senior engineer"),
        "prompt should contain index entry summaries"
    );
    assert!(
        prompt.contains("feedback_tests.md"),
        "prompt should contain all index entries"
    );
}

// ---------------------------------------------------------------------------
// TC-6.4: Without MEMORY.md, prompt shows empty-state message
// ---------------------------------------------------------------------------

#[test]
fn tc_6_4_no_memory_md_shows_empty_message() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");
    fs::create_dir_all(&mem_dir).unwrap();
    // No MEMORY.md file created

    let prompt = build_memory_prompt(&mem_dir);

    assert!(
        prompt.contains("currently empty"),
        "should indicate MEMORY.md is empty when file doesn't exist"
    );
}

#[test]
fn tc_6_4_empty_memory_md_shows_empty_message() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");
    fs::create_dir_all(&mem_dir).unwrap();
    fs::write(mem_dir.join("MEMORY.md"), "").unwrap();

    let prompt = build_memory_prompt(&mem_dir);

    assert!(
        prompt.contains("currently empty"),
        "should indicate MEMORY.md is empty when file is blank"
    );
}

#[test]
fn tc_6_4_whitespace_only_memory_md_shows_empty_message() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");
    fs::create_dir_all(&mem_dir).unwrap();
    fs::write(mem_dir.join("MEMORY.md"), "   \n\n  ").unwrap();

    let prompt = build_memory_prompt(&mem_dir);

    assert!(
        prompt.contains("currently empty"),
        "should indicate MEMORY.md is empty when file is whitespace-only"
    );
}

// ---------------------------------------------------------------------------
// TC-6.5: No bb brand identifiers in prompt
// ---------------------------------------------------------------------------

#[test]
fn tc_6_5_no_bb_brand_in_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");
    fs::create_dir_all(&mem_dir).unwrap();
    fs::write(
        mem_dir.join("MEMORY.md"),
        "- [Test](test.md) \u{2014} entry\n",
    )
    .unwrap();

    let prompt = build_memory_prompt(&mem_dir);

    assert!(
        !prompt.contains("~/.claude"),
        "prompt must not contain bb brand path ~/.claude"
    );
    assert!(
        !prompt.contains("CLAUDE.md"),
        "prompt must not reference CLAUDE.md"
    );
    // Allow "claude" in lowercase only in non-brand contexts (e.g. general English).
    // The key check is no bb-specific identifiers.
}

#[test]
fn tc_6_5_no_bb_brand_in_instructions() {
    let lines = build_memory_instructions(Path::new("/test/memory"));
    let joined = lines.join("\n");

    assert!(!joined.contains("~/.claude"));
    assert!(!joined.contains("CLAUDE.md"));
}

#[test]
fn tc_6_5_no_bb_brand_in_type_descriptions() {
    let desc = memory_type_descriptions();

    assert!(!desc.contains("~/.claude"));
    assert!(!desc.contains("CLAUDE.md"));
}

// ---------------------------------------------------------------------------
// TC-6.6: Paths use nomi brand, not hardcoded platform paths
// ---------------------------------------------------------------------------

#[test]
fn tc_6_6_no_hardcoded_platform_paths() {
    let lines = build_memory_instructions(Path::new("/test/memory"));
    let joined = lines.join("\n");

    // Should not contain hardcoded Unix-specific config paths
    assert!(
        !joined.contains("~/.config/nomi"),
        "should not hardcode platform-specific config path"
    );

    // Path should come from the memory_dir argument, not hardcoded
    assert!(
        joined.contains("/test/memory"),
        "should use the provided memory_dir path"
    );
}

// ---------------------------------------------------------------------------
// Additional integration tests beyond TC-6
// ---------------------------------------------------------------------------

#[test]
fn instructions_are_well_structured_vec() {
    let lines = build_memory_instructions(Path::new("/test/memory"));

    // Should be a non-empty vec
    assert!(!lines.is_empty());

    // First line should be the title
    assert!(lines[0].starts_with("# "));
}

#[test]
fn prompt_with_large_index_includes_truncation_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");
    fs::create_dir_all(&mem_dir).unwrap();

    // Create 250-line index (exceeds 200-line limit)
    let content: String = (0..250)
        .map(|i| format!("- [Item {i}](item_{i}.md) \u{2014} summary for item {i}\n"))
        .collect();
    fs::write(mem_dir.join("MEMORY.md"), &content).unwrap();

    let prompt = build_memory_prompt(&mem_dir);

    assert!(
        prompt.contains("WARNING"),
        "should include truncation warning for large index"
    );
    assert!(
        prompt.contains("250 lines"),
        "warning should mention original line count"
    );
}

#[test]
fn type_descriptions_standalone() {
    let desc = memory_type_descriptions();

    assert!(desc.contains("<types>"));
    assert!(desc.contains("</types>"));

    // All four types present
    for ty in ["user", "feedback", "project", "reference"] {
        assert!(
            desc.contains(&format!("<name>{ty}</name>")),
            "type_descriptions should include: {ty}"
        );
    }

    // Each type has description and examples
    assert!(desc.contains("<description>"));
    assert!(desc.contains("<examples>"));
    assert!(desc.contains("<when_to_save>"));
    assert!(desc.contains("<how_to_use>"));
}

#[test]
fn prompt_nonexistent_dir_succeeds() {
    // build_memory_prompt should not panic even if the directory doesn't exist
    // (read_index returns empty string for missing files)
    let result = build_memory_prompt(Path::new("/nonexistent/path/memory"));
    assert!(result.contains("currently empty"));
}

#[test]
fn prompt_sections_appear_in_correct_order() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");
    fs::create_dir_all(&mem_dir).unwrap();
    fs::write(mem_dir.join("MEMORY.md"), "- [A](a.md) \u{2014} test\n").unwrap();

    let prompt = build_memory_prompt(&mem_dir);

    // Verify section ordering
    let positions = [
        ("# auto memory", prompt.find("# auto memory")),
        ("## Types of memory", prompt.find("## Types of memory")),
        ("## What NOT to save", prompt.find("## What NOT to save")),
        ("## How to save", prompt.find("## How to save")),
        ("## When to access", prompt.find("## When to access")),
        (
            "## Before recommending",
            prompt.find("## Before recommending"),
        ),
        (
            "## Memory and other forms",
            prompt.find("## Memory and other forms"),
        ),
        ("## MEMORY.md", prompt.find("## MEMORY.md")),
    ];

    for (name, pos) in &positions {
        assert!(pos.is_some(), "section missing: {name}");
    }

    // Verify monotonically increasing positions
    let nums: Vec<usize> = positions.iter().map(|(_, p)| p.unwrap()).collect();
    for i in 1..nums.len() {
        assert!(
            nums[i] > nums[i - 1],
            "section '{}' should appear after '{}', but positions are {} vs {}",
            positions[i].0,
            positions[i - 1].0,
            nums[i],
            nums[i - 1]
        );
    }
}
