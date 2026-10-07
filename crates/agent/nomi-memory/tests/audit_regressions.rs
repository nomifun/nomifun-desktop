use std::fs;
use std::sync::{Arc, Barrier};

use chrono::Utc;
use nomi_memory::{
    index, store,
    types::{MemoryEntry, MemoryFrontmatter, MemoryType},
};

#[test]
fn unicode_index_truncation_keeps_valid_prefix() {
    for text in [
        "记".repeat(10_000),
        format!("first\n{}", "🦀".repeat(7_000)),
    ] {
        let result = index::truncate_index(&text);
        let prefix = result.content.split("\n\n> WARNING:").next().unwrap();
        assert!(result.was_truncated);
        assert!(prefix.len() <= index::MAX_INDEX_BYTES);
        assert!(text.starts_with(prefix));
        if text.starts_with("first\n") {
            assert_eq!(prefix, "first");
        }
    }
}

#[test]
fn malformed_frontmatter_preserves_the_entire_document() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("user_bad.md");
    for raw in [
        "---\n: :\n  :\n---\nBody",
        "---not-a-delimiter\nname: title\n---\nBody",
    ] {
        fs::write(&path, raw).unwrap();
        let entry = store::read_memory(&path).unwrap();
        assert_eq!(entry.frontmatter, MemoryFrontmatter::default());
        assert_eq!(entry.content, raw);
    }
}

#[test]
fn crlf_and_padded_delimiters_do_not_leak_into_the_body() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("user_crlf.md");
    fs::write(&path, "---\r\nname: test\r\n  ---  \r\n\r\nBody\r\nnext").unwrap();
    let entry = store::read_memory(&path).unwrap();
    assert_eq!(entry.frontmatter.name.as_deref(), Some("test"));
    assert_eq!(entry.content, "Body\r\nnext");
}

#[test]
fn usage_count_saturates_instead_of_panicking() {
    let temp = tempfile::tempdir().unwrap();
    let mut entry = MemoryEntry::build("max", "desc", MemoryType::User, "body");
    entry.frontmatter.usage_count = Some(u64::MAX);
    let path = store::write_memory(temp.path(), &entry).unwrap();
    store::bump_memory_usage(temp.path(), "user_max.md", Utc::now()).unwrap();
    assert_eq!(
        store::read_memory(&path).unwrap().frontmatter.usage_count,
        Some(u64::MAX)
    );
}

#[test]
fn append_preserves_existing_non_utf8_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("MEMORY.md");
    let original = b"old contents: \xff\n";
    fs::write(&path, original).unwrap();
    index::append_index_entry(&path, "New", "new.md", "summary").unwrap();
    assert!(fs::read(&path).unwrap().starts_with(original));
}

#[test]
fn concurrent_index_appends_do_not_lose_entries() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("MEMORY.md");
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|scope| {
        for worker in 0..8 {
            let path = &path;
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                for entry in 0..16 {
                    index::append_index_entry(
                        path,
                        &format!("{worker}-{entry}"),
                        "entry.md",
                        "summary",
                    )
                    .unwrap();
                }
            });
        }
    });
    let text = fs::read_to_string(&path).unwrap();
    let entries: std::collections::HashSet<_> = text.lines().collect();
    assert_eq!(entries.len(), 128);
    for worker in 0..8 {
        for entry in 0..16 {
            assert!(entries.contains(format!("- [{worker}-{entry}](entry.md) — summary").as_str()));
        }
    }
}

#[test]
fn citation_updates_preserve_unknown_metadata_and_exact_body() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("user_custom.md");
    let body = "\r\n\r\n  indented body\r\n\n";
    fs::write(
        &path,
        format!("---\r\nname: custom\r\ncustom: keep-me\r\n---\r\n{body}"),
    )
    .unwrap();
    store::bump_memory_usage(temp.path(), "user_custom.md", Utc::now()).unwrap();
    let updated = fs::read_to_string(path).unwrap();
    assert!(updated.contains("custom: keep-me"));
    assert!(updated.ends_with(body));
}

#[test]
fn citation_does_not_rewrite_nonmemory_or_malformed_files() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("user_bad.md");
    for raw in [
        "ordinary text",
        "---\n: :\n  :\n---\nBody",
        "---\nusage_count: invalid\n---\nBody",
    ] {
        fs::write(&path, raw).unwrap();
        store::bump_memory_usage(temp.path(), "user_bad.md", Utc::now()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), raw);
    }
}

#[test]
fn concurrent_citations_keep_all_usage_increments() {
    let temp = tempfile::tempdir().unwrap();
    let entry = MemoryEntry::build("shared", "desc", MemoryType::User, "body");
    let path = store::write_memory(temp.path(), &entry).unwrap();
    let barrier = Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                barrier.wait();
                for _ in 0..16 {
                    store::bump_memory_usage(temp.path(), "user_shared.md", Utc::now()).unwrap();
                }
            });
        }
    });
    assert_eq!(
        store::read_memory(&path).unwrap().frontmatter.usage_count,
        Some(128)
    );
}

#[cfg(unix)]
#[test]
fn citation_does_not_follow_a_symlink_outside_the_memory_directory() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside.md");
    let memory = temp.path().join("memory");
    fs::create_dir(&memory).unwrap();
    let original = "---\nname: outside\n---\nprivate";
    fs::write(&outside, original).unwrap();
    std::os::unix::fs::symlink(&outside, memory.join("user_link.md")).unwrap();
    store::bump_memory_usage(&memory, "user_link.md", Utc::now()).unwrap();
    assert_eq!(fs::read_to_string(outside).unwrap(), original);
}
