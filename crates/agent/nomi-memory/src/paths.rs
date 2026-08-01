// Path resolution and directory management for the memory system.
//
// Provides functions to compute memory directory locations and ensure
// directories exist.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::error::Result;

/// MEMORY.md entrypoint filename.
pub const ENTRYPOINT_NAME: &str = "MEMORY.md";

/// Maximum length for sanitized directory names before truncation.
const MAX_SANITIZED_LENGTH: usize = 200;

// ---------------------------------------------------------------------------
// Base directory resolution
// ---------------------------------------------------------------------------

/// Returns the base directory for memory storage.
///
/// Hosted Nomi resolves this to the effective Nomifun application data root,
/// exported as `NOMIFUN_DATA_DIR`. Standalone callers use
/// `nomi_config::config::app_data_dir()`'s platform fallback. v3 deliberately
/// has no independent memory-root override: otherwise auto-memory could escape
/// the reset/backup-managed dataset and survive a hard reset unexpectedly.
pub fn memory_base_dir() -> Option<PathBuf> {
    Some(nomi_config::config::app_data_dir())
}

// ---------------------------------------------------------------------------
// Project-specific memory directory
// ---------------------------------------------------------------------------

/// Returns the auto-memory directory for a specific project.
///
/// Path: `<base>/projects/<sanitized_project_root>/memory/`
///
/// The project root is sanitized to produce a safe directory name:
/// all non-alphanumeric characters become hyphens, and long paths
/// are truncated with a hash suffix for uniqueness.
pub fn auto_memory_dir(project_root: &Path) -> Option<PathBuf> {
    let base = memory_base_dir()?;
    let sanitized = sanitize_path(&project_root.to_string_lossy());
    Some(base.join("projects").join(sanitized).join("memory"))
}

// ---------------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------------

/// Returns the MEMORY.md entrypoint path within a memory directory.
pub fn memory_entrypoint(memory_dir: &Path) -> PathBuf {
    memory_dir.join(ENTRYPOINT_NAME)
}

// ---------------------------------------------------------------------------
// Directory creation
// ---------------------------------------------------------------------------

/// Ensure a memory directory exists, creating it and all parent
/// directories if necessary. Idempotent — safe to call repeatedly.
pub fn ensure_memory_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Path sanitization
// ---------------------------------------------------------------------------

/// Make a string safe for use as a directory name.
///
/// Replaces all non-alphanumeric characters with hyphens. If the result
/// exceeds `MAX_SANITIZED_LENGTH`, truncates and appends a hash suffix
/// to preserve uniqueness.
pub fn sanitize_path(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    if sanitized.len() <= MAX_SANITIZED_LENGTH {
        return sanitized;
    }

    let hash = simple_hash(name);
    format!("{}-{hash}", &sanitized[..MAX_SANITIZED_LENGTH])
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Simple hash function for path truncation suffix.
fn simple_hash(s: &str) -> String {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{hash:x}")
}

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::path::Path;

    // -- sanitize_path --------------------------------------------------------

    #[test]
    fn sanitize_simple_path() {
        assert_eq!(sanitize_path("/home/user/project"), "-home-user-project");
    }

    #[test]
    fn sanitize_preserves_alphanumeric() {
        assert_eq!(sanitize_path("abc123"), "abc123");
    }

    #[test]
    fn sanitize_replaces_special_chars() {
        assert_eq!(sanitize_path("a/b:c d"), "a-b-c-d");
    }

    #[test]
    fn sanitize_long_path_truncates_with_hash() {
        let long_path = "/".to_string() + &"a".repeat(300);
        let result = sanitize_path(&long_path);
        assert!(result.len() > MAX_SANITIZED_LENGTH); // truncated + hash
        assert!(result.len() < MAX_SANITIZED_LENGTH + 20); // hash isn't huge
        assert!(result.contains('-')); // has separator before hash
    }

    #[test]
    fn sanitize_two_long_paths_produce_different_results() {
        let path_a = "/".to_string() + &"a".repeat(300);
        let path_b = "/".to_string() + &"b".repeat(300);
        assert_ne!(sanitize_path(&path_a), sanitize_path(&path_b));
    }

    // -- memory_entrypoint ----------------------------------------------------

    #[test]
    fn entrypoint_appends_memory_md() {
        let dir = Path::new("/base/memory");
        assert_eq!(
            memory_entrypoint(dir),
            PathBuf::from("/base/memory/MEMORY.md")
        );
    }

    // -- ensure_memory_dir ----------------------------------------------------

    #[test]
    fn ensure_creates_nested_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a").join("b").join("c");
        assert!(!deep.exists());
        ensure_memory_dir(&deep).unwrap();
        assert!(deep.is_dir());
    }

    #[test]
    fn ensure_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("memory");
        ensure_memory_dir(&dir).unwrap();
        // Second call should not error
        ensure_memory_dir(&dir).unwrap();
        assert!(dir.is_dir());
    }

    // -- memory_base_dir ------------------------------------------------------

    #[test]
    #[serial(env)]
    fn base_dir_uses_host_data_dir() {
        const KEY: &str = "NOMIFUN_DATA_DIR";
        let original = std::env::var(KEY).ok();

        // SAFETY: #[serial(env)] ensures no concurrent env mutation.
        unsafe { std::env::set_var(KEY, "/custom/nomifun-data") };
        let result = memory_base_dir();
        assert_eq!(result, Some(PathBuf::from("/custom/nomifun-data")));

        restore_env(KEY, original);
    }

    // -- auto_memory_dir ------------------------------------------------------

    #[test]
    #[serial(env)]
    fn auto_memory_dir_structure() {
        let key = "NOMIFUN_DATA_DIR";
        let original = std::env::var(key).ok();

        // SAFETY: #[serial(env)] ensures no concurrent env mutation.
        unsafe { std::env::set_var(key, "/base") };
        let dir = auto_memory_dir(Path::new("/home/user/project")).unwrap();
        assert_eq!(
            dir,
            PathBuf::from("/base/projects/-home-user-project/memory")
        );

        restore_env(key, original);
    }

    fn restore_env(key: &str, saved: Option<String>) {
        // SAFETY: only called from #[serial(env)] tests.
        unsafe {
            match saved {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}
