// Integration tests for the memory store.
//
// These tests target functional requirements from test-plan.md TC-3 and TC-4,
// treating the public API as a black box.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

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

// -- TC-3.6: Delete existing memory file ------------------------------------

#[test]
fn tc_3_6_delete_existing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let entry = MemoryEntry::build("to delete", "desc", MemoryType::Feedback, "content");
    let path = store::write_memory(tmp.path(), &entry).unwrap();
    assert!(path.exists());

    store::delete_memory(&path).unwrap();
    assert!(!path.exists());
}

// -- TC-3.7: Delete non-existent file returns error -------------------------

#[test]
fn tc_3_7_delete_nonexistent_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("nonexistent.md");

    let result = store::delete_memory(&path);
    assert!(result.is_err());
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

// ===========================================================================
// TC-4: Directory scanning
// ===========================================================================

// -- TC-4.1: Scan directory with multiple memory files ----------------------

#[test]
fn tc_4_1_scan_multiple_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Create 3 memory files + MEMORY.md
    fs::write(
        dir.join("user_role.md"),
        "---\ntype: user\ndescription: role\n---\nBody",
    )
    .unwrap();
    fs::write(
        dir.join("feedback_testing.md"),
        "---\ntype: feedback\n---\nBody",
    )
    .unwrap();
    fs::write(
        dir.join("project_status.md"),
        "---\ntype: project\n---\nBody",
    )
    .unwrap();
    fs::write(dir.join("MEMORY.md"), "# Index\n- [role](user_role.md)").unwrap();

    let headers = store::scan_memory_files(dir).unwrap();

    // MEMORY.md should be excluded
    assert_eq!(headers.len(), 3);
    let filenames: Vec<&str> = headers.iter().map(|h| h.filename.as_str()).collect();
    assert!(!filenames.contains(&"MEMORY.md"));
}

// -- TC-4.2: Scan empty directory -------------------------------------------

#[test]
fn tc_4_2_scan_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let headers = store::scan_memory_files(tmp.path()).unwrap();
    assert!(headers.is_empty());
}

// -- TC-4.3: Scan non-existent directory ------------------------------------

#[test]
fn tc_4_3_scan_nonexistent_dir() {
    let headers = store::scan_memory_files(Path::new("/nonexistent/dir")).unwrap();
    assert!(headers.is_empty());
}

// -- TC-4.4: Sort by modification time (newest first) -----------------------

#[test]
fn tc_4_4_sort_by_mtime() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Write files with small delays to ensure different mtimes
    fs::write(dir.join("old.md"), "---\nname: old\n---\nOld").unwrap();
    thread::sleep(Duration::from_millis(50));
    fs::write(dir.join("mid.md"), "---\nname: mid\n---\nMid").unwrap();
    thread::sleep(Duration::from_millis(50));
    fs::write(dir.join("new.md"), "---\nname: new\n---\nNew").unwrap();

    let headers = store::scan_memory_files(dir).unwrap();
    assert_eq!(headers.len(), 3);

    // Newest first
    assert_eq!(headers[0].filename, "new.md");
    assert_eq!(headers[2].filename, "old.md");
}

// -- TC-4.5: File count cap at 200 -----------------------------------------

#[test]
fn tc_4_5_file_count_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Create 210 files
    for i in 0..210 {
        fs::write(
            dir.join(format!("mem_{i:03}.md")),
            format!("---\nname: mem{i}\n---\nBody {i}"),
        )
        .unwrap();
    }

    let headers = store::scan_memory_files(dir).unwrap();
    assert_eq!(headers.len(), 200);
}

// -- TC-4.6: Non-.md files are ignored --------------------------------------

#[test]
fn tc_4_6_non_md_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    fs::write(dir.join("memory.md"), "---\nname: valid\n---\nBody").unwrap();
    fs::write(dir.join("notes.txt"), "text file").unwrap();
    fs::write(dir.join("data.json"), "{}").unwrap();
    fs::write(dir.join("script.py"), "pass").unwrap();

    let headers = store::scan_memory_files(dir).unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].filename, "memory.md");
}

// -- TC-4.7: Format memory manifest -----------------------------------------

#[test]
fn tc_4_7_format_manifest() {
    use chrono::{TimeZone, Utc};

    let headers = vec![
        nomi_memory::types::MemoryHeader {
            filename: "user_role.md".into(),
            file_path: "/mem/user_role.md".into(),
            mtime: Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap(),
            description: Some("User role info".into()),
            memory_type: Some(MemoryType::User),
        },
        nomi_memory::types::MemoryHeader {
            filename: "notes.md".into(),
            file_path: "/mem/notes.md".into(),
            mtime: Utc.with_ymd_and_hms(2026, 4, 9, 8, 0, 0).unwrap(),
            description: None,
            memory_type: None,
        },
    ];

    let manifest = store::format_memory_manifest(&headers);
    let lines: Vec<&str> = manifest.lines().collect();

    assert_eq!(lines.len(), 2);
    // First: has type and description
    assert!(lines[0].contains("[user]"));
    assert!(lines[0].contains("user_role.md"));
    assert!(lines[0].contains("User role info"));
    // Second: no type, no description
    assert!(!lines[1].contains("["));
    assert!(lines[1].contains("notes.md"));
}
