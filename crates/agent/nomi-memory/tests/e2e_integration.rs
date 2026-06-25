// End-to-end integration tests for the memory system (TC-8).
//
// These tests exercise the full memory lifecycle across multiple modules,
// verifying that all components work together correctly.

use std::fs;

use nomi_memory::index;
use nomi_memory::paths;
use nomi_memory::prompt::build_memory_prompt;
use nomi_memory::store;
use nomi_memory::types::{MemoryEntry, MemoryType};

// ===========================================================================
// TC-8.1: Complete memory lifecycle
// ===========================================================================

#[test]
fn tc_8_1_complete_memory_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");

    // 1. Ensure memory directory exists
    paths::ensure_memory_dir(&mem_dir).unwrap();
    assert!(mem_dir.is_dir());

    // 2. Write a feedback-type memory
    let entry = MemoryEntry::build(
        "test policy",
        "integration tests must hit real DB",
        MemoryType::Feedback,
        "Never mock the database in integration tests.\n\n\
         **Why:** mocked tests once passed but prod migration failed.\n\n\
         **How to apply:** use testcontainers for all DB tests.",
    );
    let written_path = store::write_memory(&mem_dir, &entry).unwrap();
    assert!(written_path.exists());

    // 3. Append index entry to MEMORY.md
    let index_path = paths::memory_entrypoint(&mem_dir);
    let filename = written_path.file_name().unwrap().to_str().unwrap();
    index::append_index_entry(
        &index_path,
        "Test Policy",
        filename,
        "integration tests must hit real DB",
    )
    .unwrap();

    // 4. Scan directory — the memory should appear
    let headers = store::scan_memory_files(&mem_dir).unwrap();
    assert_eq!(
        headers.len(),
        1,
        "should find exactly 1 memory file (MEMORY.md excluded)"
    );
    assert_eq!(headers[0].filename, filename);
    assert_eq!(headers[0].memory_type, Some(MemoryType::Feedback));
    assert_eq!(
        headers[0].description.as_deref(),
        Some("integration tests must hit real DB")
    );

    // 5. Build prompt — should include MEMORY.md content
    let prompt = build_memory_prompt(&mem_dir);
    assert!(
        prompt.contains(filename),
        "prompt should reference the memory file"
    );
    assert!(
        prompt.contains("integration tests must hit real DB"),
        "prompt should contain the index summary"
    );

    // 6. Read back the memory file — verify content integrity
    let read_back = store::read_memory(&written_path).unwrap();
    assert_eq!(read_back.frontmatter.name.as_deref(), Some("test policy"));
    assert_eq!(
        read_back.frontmatter.memory_type,
        Some(MemoryType::Feedback)
    );
    assert!(read_back.content.contains("testcontainers"));

    // 7. Delete the memory file
    store::delete_memory(&written_path).unwrap();
    assert!(!written_path.exists());

    // 8. Re-scan — should be empty
    let headers_after = store::scan_memory_files(&mem_dir).unwrap();
    assert!(
        headers_after.is_empty(),
        "should find no memory files after deletion"
    );
}

// ===========================================================================
// TC-8.2: Chinese content memory
// ===========================================================================

#[test]
fn tc_8_2_chinese_content_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");
    paths::ensure_memory_dir(&mem_dir).unwrap();

    let entry = MemoryEntry::build(
        "用户角色",
        "资深后端工程师",
        MemoryType::User,
        "用户是一位有十年经验的后端工程师，熟悉 Rust 和 Go。\n\
         偏好函数式编程风格，不喜欢过度抽象。",
    );

    // Write
    let path = store::write_memory(&mem_dir, &entry).unwrap();
    assert!(path.exists());

    // Read back — Chinese content should be intact
    let read_back = store::read_memory(&path).unwrap();
    assert_eq!(read_back.frontmatter.name.as_deref(), Some("用户角色"));
    assert_eq!(
        read_back.frontmatter.description.as_deref(),
        Some("资深后端工程师")
    );
    assert_eq!(read_back.frontmatter.memory_type, Some(MemoryType::User));
    assert!(read_back.content.contains("十年经验"));
    assert!(read_back.content.contains("函数式编程"));

    // Scan — header should preserve Chinese description
    let headers = store::scan_memory_files(&mem_dir).unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].description.as_deref(), Some("资深后端工程师"));

    // Index — append Chinese title and verify
    let index_path = paths::memory_entrypoint(&mem_dir);
    let filename = path.file_name().unwrap().to_str().unwrap();
    index::append_index_entry(&index_path, "用户角色", filename, "资深后端工程师").unwrap();

    let index_content = fs::read_to_string(&index_path).unwrap();
    assert!(index_content.contains("用户角色"));
    assert!(index_content.contains("资深后端工程师"));

    // Prompt — should include Chinese index content
    let prompt = build_memory_prompt(&mem_dir);
    assert!(prompt.contains("用户角色"));
    assert!(prompt.contains("资深后端工程师"));
}

// ===========================================================================
// TC-8.3: Special character handling
// ===========================================================================

#[test]
fn tc_8_3_special_characters_in_name() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");
    paths::ensure_memory_dir(&mem_dir).unwrap();

    // Name with special characters: spaces, slashes, colons, emoji
    let entry = MemoryEntry::build(
        "My Role / Senior: 🚀",
        "role with special chars",
        MemoryType::User,
        "Body with special chars: <tag>, \"quotes\", 'apostrophes' & ampersands",
    );

    let path = store::write_memory(&mem_dir, &entry).unwrap();

    // Filename should be safe (no slashes, colons, etc.)
    let filename = path.file_name().unwrap().to_str().unwrap();
    assert!(
        !filename.contains('/'),
        "filename should not contain slash: {filename}"
    );
    assert!(
        !filename.contains(':'),
        "filename should not contain colon: {filename}"
    );
    assert!(
        filename.ends_with(".md"),
        "filename should end with .md: {filename}"
    );

    // Content should round-trip correctly
    let read_back = store::read_memory(&path).unwrap();
    assert!(read_back.content.contains("<tag>"));
    assert!(read_back.content.contains("\"quotes\""));
    assert!(read_back.content.contains("& ampersands"));
}

#[test]
fn tc_8_3_name_with_only_special_chars() {
    let tmp = tempfile::tempdir().unwrap();
    let mem_dir = tmp.path().join("memory");
    paths::ensure_memory_dir(&mem_dir).unwrap();

    // Edge case: name is entirely special characters / non-ASCII
    let entry = MemoryEntry::build(
        "🔥💡✨",
        "emoji only name",
        MemoryType::Feedback,
        "Some body",
    );

    let path = store::write_memory(&mem_dir, &entry).unwrap();
    assert!(path.exists());

    // Should still produce a valid filename (hash fallback)
    let filename = path.file_name().unwrap().to_str().unwrap();
    assert!(filename.ends_with(".md"));

    // Content should round-trip
    let read_back = store::read_memory(&path).unwrap();
    assert_eq!(read_back.content, "Some body");
}
