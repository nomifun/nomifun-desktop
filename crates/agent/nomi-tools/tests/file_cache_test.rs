//! Integration tests for FileStateCache (TC-5.2 series from test-plan.md).
//!
//! Black-box tests targeting the public API of FileStateCache without
//! depending on internal implementation details.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use nomi_config::file_cache::FileCacheConfig;
use nomi_tools::file_cache::{FileStateCache, file_mtime_ms, update_cache_after_write};
use nomi_types::file_state::FileState;

fn default_config() -> FileCacheConfig {
    FileCacheConfig {
        max_entries: 100,
        max_size_bytes: 25 * 1024 * 1024,
        enabled: true,
    }
}

fn make_state(content: &str, mtime_ms: u64) -> FileState {
    FileState {
        content: content.to_string(),
        mtime_ms,
        offset: None,
        limit: None,
    }
}

/// TC-5.2-01: Insert and retrieve a file state entry.
#[test]
fn tc_5_2_01_insert_and_retrieve() {
    let mut cache = FileStateCache::new(&default_config());

    let path = PathBuf::from("/home/user/project/main.rs");
    let state = make_state("     1\tfn main() {}", 1_700_000_000_000);

    cache.insert(path.clone(), state);

    let retrieved = cache.get(&path).expect("entry should exist");
    assert_eq!(retrieved.content, "     1\tfn main() {}");
    assert_eq!(retrieved.mtime_ms, 1_700_000_000_000);
    assert!(retrieved.offset.is_none());
    assert!(retrieved.limit.is_none());
}

/// TC-5.2-02: Getting a non-existent key returns None.
#[test]
fn tc_5_2_02_nonexistent_key() {
    let mut cache = FileStateCache::new(&default_config());
    assert!(cache.get(Path::new("/no/such/file.rs")).is_none());
}

/// TC-5.2-03: LRU eviction when count exceeds max_entries.
#[test]
fn tc_5_2_03_lru_count_eviction() {
    let config = FileCacheConfig {
        max_entries: 3,
        max_size_bytes: 10_000_000,
        enabled: true,
    };
    let mut cache = FileStateCache::new(&config);

    cache.insert(PathBuf::from("/f1"), make_state("1", 1));
    cache.insert(PathBuf::from("/f2"), make_state("2", 2));
    cache.insert(PathBuf::from("/f3"), make_state("3", 3));

    // 4th insert should evict /f1 (the LRU)
    cache.insert(PathBuf::from("/f4"), make_state("4", 4));

    assert!(
        cache.get(Path::new("/f1")).is_none(),
        "/f1 should be evicted"
    );
    assert!(cache.get(Path::new("/f2")).is_some());
    assert!(cache.get(Path::new("/f3")).is_some());
    assert!(cache.get(Path::new("/f4")).is_some());
    assert_eq!(cache.len(), 3);
}

/// TC-5.2-04: Path normalization ensures equivalent paths hit the same slot.
#[test]
fn tc_5_2_04_path_normalization() {
    let mut cache = FileStateCache::new(&default_config());

    // Insert with redundant `..` in path
    cache.insert(
        PathBuf::from("/project/src/../lib/file.rs"),
        make_state("content", 100),
    );

    // Retrieve using canonical-style path
    let got = cache
        .get(Path::new("/project/lib/file.rs"))
        .expect("normalized path should hit cache");
    assert_eq!(got.content, "content");

    // Only one entry in cache
    assert_eq!(cache.len(), 1);
}

/// TC-5.2-05: clear() removes all entries and resets size accounting.
#[test]
fn tc_5_2_05_clear() {
    let mut cache = FileStateCache::new(&default_config());

    cache.insert(PathBuf::from("/a"), make_state("aaa", 1));
    cache.insert(PathBuf::from("/b"), make_state("bbb", 2));
    cache.insert(PathBuf::from("/c"), make_state("ccc", 3));
    assert_eq!(cache.len(), 3);
    assert!(cache.current_size_bytes() > 0);

    cache.clear();
    assert_eq!(cache.len(), 0);
    assert!(cache.is_empty());
    assert_eq!(cache.current_size_bytes(), 0);
    assert!(cache.get(Path::new("/a")).is_none());
    assert!(cache.get(Path::new("/b")).is_none());
    assert!(cache.get(Path::new("/c")).is_none());
}

/// TC-5.2-06: remove() deletes a specific entry and returns it.
#[test]
fn tc_5_2_06_remove() {
    let mut cache = FileStateCache::new(&default_config());

    cache.insert(PathBuf::from("/target"), make_state("data", 1));
    cache.insert(PathBuf::from("/keep"), make_state("keep", 2));

    let removed = cache.remove(Path::new("/target"));
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().content, "data");

    assert!(cache.get(Path::new("/target")).is_none());
    assert!(cache.get(Path::new("/keep")).is_some());
    assert_eq!(cache.len(), 1);
}

/// TC-5.2-07: Byte-size limit triggers LRU eviction of old entries.
#[test]
fn tc_5_2_07_byte_size_eviction() {
    let config = FileCacheConfig {
        max_entries: 100,
        max_size_bytes: 15, // tight byte budget
        enabled: true,
    };
    let mut cache = FileStateCache::new(&config);

    // Insert two 6-byte entries: total = 12, within budget
    cache.insert(PathBuf::from("/a"), make_state("aaaaaa", 1)); // 6 bytes
    cache.insert(PathBuf::from("/b"), make_state("bbbbbb", 2)); // 6 bytes
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.current_size_bytes(), 12);

    // Insert 6-byte entry: 12 + 6 = 18 > 15 -> evicts /a (LRU), total = 12
    cache.insert(PathBuf::from("/c"), make_state("cccccc", 3));
    assert!(cache.get(Path::new("/a")).is_none(), "/a should be evicted");
    assert!(cache.get(Path::new("/b")).is_some());
    assert!(cache.get(Path::new("/c")).is_some());
    assert!(cache.current_size_bytes() <= 15);
}

/// TC-5.2-08: Inserting the same path twice updates (overwrites) the entry.
#[test]
fn tc_5_2_08_overwrite_update() {
    let mut cache = FileStateCache::new(&default_config());

    cache.insert(PathBuf::from("/file"), make_state("version1", 100));
    cache.insert(PathBuf::from("/file"), make_state("version2-updated", 200));

    let got = cache.get(Path::new("/file")).expect("entry should exist");
    assert_eq!(got.content, "version2-updated");
    assert_eq!(got.mtime_ms, 200);
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.current_size_bytes(), "version2-updated".len());
}

/// Supplementary: LRU promotion via get() prevents eviction of accessed entries.
#[test]
fn lru_promotion_via_get() {
    let config = FileCacheConfig {
        max_entries: 3,
        max_size_bytes: 10_000_000,
        enabled: true,
    };
    let mut cache = FileStateCache::new(&config);

    cache.insert(PathBuf::from("/oldest"), make_state("o", 1));
    cache.insert(PathBuf::from("/middle"), make_state("m", 2));
    cache.insert(PathBuf::from("/newest"), make_state("n", 3));

    // Access /oldest to promote it; /middle becomes the new LRU
    cache.get(Path::new("/oldest"));

    // Insert /extra -> evicts /middle (now the LRU)
    cache.insert(PathBuf::from("/extra"), make_state("e", 4));

    assert!(
        cache.get(Path::new("/oldest")).is_some(),
        "/oldest was promoted and should survive"
    );
    assert!(
        cache.get(Path::new("/middle")).is_none(),
        "/middle should be evicted as the new LRU"
    );
}

/// Supplementary: remove on a non-existent key returns None without panic.
#[test]
fn remove_nonexistent_returns_none() {
    let mut cache = FileStateCache::new(&default_config());
    assert!(cache.remove(Path::new("/ghost")).is_none());
}

/// Supplementary: partial read state (offset + limit) is preserved.
#[test]
fn partial_read_state_round_trip() {
    let mut cache = FileStateCache::new(&default_config());

    let state = FileState {
        content: "partial".to_string(),
        mtime_ms: 999,
        offset: Some(10),
        limit: Some(20),
    };
    cache.insert(PathBuf::from("/partial"), state);

    let got = cache.get(Path::new("/partial")).unwrap();
    assert_eq!(got.offset, Some(10));
    assert_eq!(got.limit, Some(20));
}

// ==========================================================================
// TC-5.5: update_cache_after_write helper and config integration
// ==========================================================================

/// TC-5.5-01: Cache created with custom max_entries has correct capacity.
#[test]
fn tc_5_5_01_custom_capacity() {
    let config = FileCacheConfig {
        max_entries: 50,
        max_size_bytes: 25 * 1024 * 1024,
        enabled: true,
    };
    let mut cache = FileStateCache::new(&config);

    // Insert 50 entries: all should fit.
    for i in 0..50 {
        cache.insert(PathBuf::from(format!("/f{}", i)), make_state("x", i));
    }
    assert_eq!(cache.len(), 50);

    // 51st entry evicts the LRU.
    cache.insert(PathBuf::from("/f50"), make_state("x", 50));
    assert_eq!(cache.len(), 50);
    assert!(
        cache.get(Path::new("/f0")).is_none(),
        "/f0 should be evicted at capacity 50"
    );
}

/// update_cache_after_write stores line-numbered content with correct mtime.
#[test]
fn update_cache_after_write_stores_numbered_content() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("helper_test.txt");
    let content = "line one\nline two\nline three";
    std::fs::write(&file, content).unwrap();

    let cache_arc = Arc::new(RwLock::new(FileStateCache::new(&default_config())));
    update_cache_after_write(&cache_arc, &file, content);

    let mut cache = cache_arc.write().unwrap();
    let cached = cache.get(&file).expect("entry should exist after update");

    // Content should be line-numbered.
    assert!(cached.content.contains("     1\tline one"));
    assert!(cached.content.contains("     2\tline two"));
    assert!(cached.content.contains("     3\tline three"));

    // Mtime should match disk.
    let disk_mtime = file_mtime_ms(&file).unwrap();
    assert_eq!(cached.mtime_ms, disk_mtime);

    // Offset and limit should be None (full file).
    assert!(cached.offset.is_none());
    assert!(cached.limit.is_none());
}

/// update_cache_after_write handles empty content.
#[test]
fn update_cache_after_write_empty_content() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("empty.txt");
    std::fs::write(&file, "").unwrap();

    let cache_arc = Arc::new(RwLock::new(FileStateCache::new(&default_config())));
    update_cache_after_write(&cache_arc, &file, "");

    let mut cache = cache_arc.write().unwrap();
    let cached = cache.get(&file).expect("entry should exist");
    assert_eq!(cached.content, "");
}

/// update_cache_after_write overwrites previous entry.
#[test]
fn update_cache_after_write_overwrites_previous() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("overwrite.txt");

    std::fs::write(&file, "v1").unwrap();
    let cache_arc = Arc::new(RwLock::new(FileStateCache::new(&default_config())));
    update_cache_after_write(&cache_arc, &file, "v1");

    // Brief delay for mtime change.
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(&file, "v2 updated").unwrap();
    update_cache_after_write(&cache_arc, &file, "v2 updated");

    let mut cache = cache_arc.write().unwrap();
    let cached = cache.get(&file).unwrap();
    assert!(cached.content.contains("v2 updated"));
    assert_eq!(cached.mtime_ms, file_mtime_ms(&file).unwrap());
}
