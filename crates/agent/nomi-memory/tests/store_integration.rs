// Integration tests for the memory store.
//
// These tests target functional requirements from test-plan.md TC-3,
// treating the public API as a black box.

use std::fs;

use nomi_memory::store;
use nomi_memory::types::{MemoryEntry, MemoryFrontmatter, MemoryType};

// ===========================================================================
// TC-3: Memory file read/write
// ===========================================================================

// -- TC-3.1: Write then read full memory ------------------------------------

#[test]
fn tc_3_1_write_then_read_full_memory() {
    let tmp = tempfile::tempdir().unwrap();
    let entry = MemoryEntry::build(
        "test memory",
        "a test description",
        MemoryType::User,
        "Body content here",
    );

    let path = store::write_memory(tmp.path(), &entry).unwrap();
    let read_back = store::read_memory(&path).unwrap();

    assert_eq!(read_back.frontmatter.name, entry.frontmatter.name);
    assert_eq!(
        read_back.frontmatter.description,
        entry.frontmatter.description
    );
    assert_eq!(
        read_back.frontmatter.memory_type,
        entry.frontmatter.memory_type
    );
    assert_eq!(read_back.content, entry.content);
}

// -- TC-3.2: Read file with frontmatter -------------------------------------

#[test]
fn tc_3_2_read_with_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.md");
    fs::write(
        &path,
        "---\nname: test memory\ndescription: a test\ntype: feedback\n---\nBody content here",
    )
    .unwrap();

    let entry = store::read_memory(&path).unwrap();
    assert_eq!(entry.frontmatter.name.as_deref(), Some("test memory"));
    assert_eq!(entry.frontmatter.description.as_deref(), Some("a test"));
    assert_eq!(entry.frontmatter.memory_type, Some(MemoryType::Feedback));
    assert_eq!(entry.content, "Body content here");
}

// -- TC-3.3: Read file without frontmatter ----------------------------------

#[test]
fn tc_3_3_read_without_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("plain.md");
    fs::write(&path, "Just plain text").unwrap();

    let entry = store::read_memory(&path).unwrap();
    assert_eq!(entry.frontmatter.name, None);
    assert_eq!(entry.frontmatter.description, None);
    assert_eq!(entry.frontmatter.memory_type, None);
    assert_eq!(entry.content, "Just plain text");
}

// -- TC-3.4: Read empty file ------------------------------------------------

#[test]
fn tc_3_4_read_empty_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("empty.md");
    fs::write(&path, "").unwrap();

    let entry = store::read_memory(&path).unwrap();
    assert_eq!(entry.frontmatter, MemoryFrontmatter::default());
    assert_eq!(entry.content, "");
}

// -- TC-3.5: Read incomplete frontmatter ------------------------------------

#[test]
fn tc_3_5_read_incomplete_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("incomplete.md");
    fs::write(&path, "---\nname: orphan\nno closing delimiter").unwrap();

    // Should not panic, should degrade gracefully
    let entry = store::read_memory(&path).unwrap();
    // Entire content treated as body since frontmatter is incomplete
    assert_eq!(entry.frontmatter, MemoryFrontmatter::default());
    assert!(entry.content.contains("orphan"));
}

// -- TC-3.8: Written filename format ----------------------------------------

#[test]
fn tc_3_8_filename_format() {
    let tmp = tempfile::tempdir().unwrap();
    let entry = MemoryEntry::build("My Role", "desc", MemoryType::User, "content");

    let path = store::write_memory(tmp.path(), &entry).unwrap();
    let filename = path.file_name().unwrap().to_str().unwrap();

    // Should be lowercase, safe characters
    assert_eq!(filename, "user_my_role.md");
    assert!(
        filename
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    );
}
