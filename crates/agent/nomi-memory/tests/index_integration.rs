// Integration tests for the MEMORY.md index management.
//
// These tests target functional requirements from test-plan.md TC-5,
// treating the public API as a black box.

use std::fs;
use std::path::Path;

use nomi_memory::index;

// ===========================================================================
// TC-5.1: Truncation — under limits, no truncation
// ===========================================================================

#[test]
fn tc_5_1_no_truncation_under_limits() {
    let content = (0..100)
        .map(|i| format!("- [Memory {i}](mem_{i}.md) \u{2014} summary {i}"))
        .collect::<Vec<_>>()
        .join("\n");

    let result = index::truncate_index(&content);
    assert!(!result.was_truncated);
    assert_eq!(result.line_count, 100);
    assert!(result.byte_count > 0);
    // Content should be the same as input (trimmed)
    assert_eq!(result.content, content.trim());
}

// ===========================================================================
// TC-5.2: Truncation — exceeds line limit
// ===========================================================================

#[test]
fn tc_5_2_line_truncation() {
    let content = (0..250)
        .map(|i| format!("- [Memory {i}](mem_{i}.md) \u{2014} summary {i}"))
        .collect::<Vec<_>>()
        .join("\n");

    let result = index::truncate_index(&content);
    assert!(result.was_truncated);
    assert_eq!(result.line_count, 250);
    // Warning should mention line count
    assert!(result.content.contains("250 lines"));
    assert!(result.content.contains("WARNING"));

    // Only first 200 lines should be present (before warning)
    let before_warning = result.content.split("\n\n> WARNING:").next().unwrap();
    let output_lines: Vec<&str> = before_warning.lines().collect();
    assert_eq!(output_lines.len(), 200);
    assert!(output_lines[0].contains("Memory 0"));
    assert!(output_lines[199].contains("Memory 199"));
}

// ===========================================================================
// TC-5.3: Truncation — exceeds byte limit (lines within limit)
// ===========================================================================

#[test]
fn tc_5_3_byte_truncation() {
    // 100 lines of 300 chars each = 30000 bytes > 25000, but 100 < 200 lines
    let content = (0..100)
        .map(|i| format!("{i:03}: {}", "x".repeat(296)))
        .collect::<Vec<_>>()
        .join("\n");

    let result = index::truncate_index(&content);
    assert!(result.was_truncated);
    assert_eq!(result.line_count, 100);
    // Warning should mention byte size and "too long"
    assert!(result.content.contains("index entries are too long"));
    assert!(result.content.contains("KB"));
}

// ===========================================================================
// TC-5.4: Truncation — both line and byte limits exceeded
// ===========================================================================

#[test]
fn tc_5_4_both_limits() {
    // 300 lines of 200 bytes each = 60000 bytes; both limits exceeded
    let content = (0..300)
        .map(|i| format!("{i:03}: {}", "y".repeat(196)))
        .collect::<Vec<_>>()
        .join("\n");

    let result = index::truncate_index(&content);
    assert!(result.was_truncated);
    assert_eq!(result.line_count, 300);
    // Warning should mention both
    assert!(result.content.contains("300 lines"));
    assert!(result.content.contains("KB"));
}

// ===========================================================================
// TC-5.5: Truncation — empty content
// ===========================================================================

#[test]
fn tc_5_5_empty_content() {
    let result = index::truncate_index("");
    assert!(!result.was_truncated);
    assert_eq!(result.line_count, 0);
    assert_eq!(result.byte_count, 0);
    assert_eq!(result.content, "");
}

// ===========================================================================
// TC-5.6: Truncation — whitespace-only content
// ===========================================================================

#[test]
fn tc_5_6_whitespace_only() {
    let result = index::truncate_index("   \n  \n  ");
    assert!(!result.was_truncated);
    assert_eq!(result.content, "");
}

// ===========================================================================
// TC-5.7: Truncation — exactly at line boundary (200 lines)
// ===========================================================================

#[test]
fn tc_5_7_exactly_200_lines() {
    let content = (0..200)
        .map(|i| format!("- line {i}"))
        .collect::<Vec<_>>()
        .join("\n");

    let result = index::truncate_index(&content);
    assert!(!result.was_truncated);
    assert_eq!(result.line_count, 200);
}

// ===========================================================================
// TC-5.8: Truncation — exactly at byte boundary (25000 bytes)
// ===========================================================================

#[test]
fn tc_5_8_exactly_25000_bytes() {
    // 100 lines under 200 limit, totalling exactly 25000 bytes
    let per_line = (index::MAX_INDEX_BYTES - 99) / 100;
    let remainder = index::MAX_INDEX_BYTES - 99 - per_line * 100;
    let mut lines: Vec<String> = (0..100).map(|_| "x".repeat(per_line)).collect();
    if remainder > 0 {
        lines.last_mut().unwrap().push_str(&"x".repeat(remainder));
    }
    let content = lines.join("\n");
    assert_eq!(content.len(), index::MAX_INDEX_BYTES);

    let result = index::truncate_index(&content);
    assert!(!result.was_truncated);
}

// ===========================================================================
// TC-5.9: Truncation — single long line (no newline to cut at)
// ===========================================================================

#[test]
fn tc_5_9_single_long_line() {
    let content = "z".repeat(30_000);
    let result = index::truncate_index(&content);

    assert!(result.was_truncated);
    // Should truncate at MAX_INDEX_BYTES since there's no newline
    let before_warning = result.content.split("\n\n> WARNING:").next().unwrap();
    assert_eq!(before_warning.len(), index::MAX_INDEX_BYTES);
}

// ===========================================================================
// TC-5.10: Read index — file doesn't exist
// ===========================================================================

#[test]
fn tc_5_10_read_nonexistent() {
    let result = index::read_index(Path::new("/nonexistent/MEMORY.md"));
    assert_eq!(result, "");
}

// ===========================================================================
// TC-5.11: Read index — file exists with content
// ===========================================================================

#[test]
fn tc_5_11_read_existing() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("MEMORY.md");
    let content = "# Index\n- [A](a.md) \u{2014} first\n- [B](b.md) \u{2014} second\n";
    fs::write(&path, content).unwrap();

    let result = index::read_index(&path);
    assert_eq!(result, content);
}

// ===========================================================================
// TC-5.12: Read index — empty file
// ===========================================================================

#[test]
fn tc_5_12_read_empty_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("MEMORY.md");
    fs::write(&path, "").unwrap();

    let result = index::read_index(&path);
    assert_eq!(result, "");
}

// ===========================================================================
// TC-5.13: Append entry — to existing content
// ===========================================================================

#[test]
fn tc_5_13_append_to_existing() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("MEMORY.md");
    fs::write(
        &path,
        "- [A](a.md) \u{2014} first\n- [B](b.md) \u{2014} second\n",
    )
    .unwrap();

    index::append_index_entry(&path, "My Memory", "my_memory.md", "a test").unwrap();

    let content = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[2], "- [My Memory](my_memory.md) \u{2014} a test");
}

// ===========================================================================
// TC-5.14: Append entry — file doesn't exist (auto-create)
// ===========================================================================

#[test]
fn tc_5_14_append_auto_create() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("MEMORY.md");
    assert!(!path.exists());

    index::append_index_entry(&path, "First", "first.md", "the first entry").unwrap();

    assert!(path.exists());
    let content = fs::read_to_string(&path).unwrap();
    assert_eq!(content, "- [First](first.md) \u{2014} the first entry\n");
}

// ===========================================================================
// TC-5.15: Append multiple entries sequentially
// ===========================================================================

#[test]
fn tc_5_15_append_multiple() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("MEMORY.md");

    index::append_index_entry(&path, "A", "a.md", "first").unwrap();
    index::append_index_entry(&path, "B", "b.md", "second").unwrap();
    index::append_index_entry(&path, "C", "c.md", "third").unwrap();

    let content = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("[A]"));
    assert!(lines[1].contains("[B]"));
    assert!(lines[2].contains("[C]"));
}

// ===========================================================================
// TC-5.16: Remove entry — by filename
// ===========================================================================

#[test]
fn tc_5_16_remove_by_filename() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("MEMORY.md");
    fs::write(
        &path,
        "- [A](a.md) \u{2014} first\n- [B](old_memory.md) \u{2014} second\n- [C](c.md) \u{2014} third\n",
    )
    .unwrap();

    index::remove_index_entry(&path, "old_memory.md").unwrap();

    let content = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("[A](a.md)"));
    assert!(lines[1].contains("[C](c.md)"));
    // Removed entry should not be present
    assert!(!content.contains("old_memory.md"));
}

// ===========================================================================
// TC-5.17: Remove entry — target not found
// ===========================================================================

#[test]
fn tc_5_17_remove_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("MEMORY.md");
    let original =
        "- [A](a.md) \u{2014} first\n- [B](b.md) \u{2014} second\n- [C](c.md) \u{2014} third\n";
    fs::write(&path, original).unwrap();

    index::remove_index_entry(&path, "nonexistent.md").unwrap();

    let content = fs::read_to_string(&path).unwrap();
    assert_eq!(content, original);
}

// ===========================================================================
// TC-5.18: Remove entry — file doesn't exist
// ===========================================================================

#[test]
fn tc_5_18_remove_from_nonexistent() {
    let path = Path::new("/nonexistent/MEMORY.md");
    // Should not error — idempotent
    index::remove_index_entry(path, "anything.md").unwrap();
}
