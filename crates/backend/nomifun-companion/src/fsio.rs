//! Shared file-IO helpers for the companion domain's small JSON config files:
//! atomic temp+rename writes and lenient reads that fall back to `Default`
//! (a corrupt config must never brick boot).

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Process-wide sequence shared by every atomic save in this crate, so two
/// concurrent saves (even of different config types into the same dir) can
/// never collide on a temp name and rename each other's half-written temp
/// into place.
static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Atomically persist `value` as pretty JSON to `{dir}/{file}`: write a
/// uniquely-named temp file, then rename it over the target. On any error
/// the temp file is removed.
pub(crate) fn save_json_atomic(dir: &Path, file: &str, value: &impl Serialize) -> std::io::Result<()> {
    let raw = serde_json::to_string_pretty(value).expect("companion config types serialize");
    save_bytes_atomic(dir, file, raw.as_bytes())
}

/// Atomically persist raw `bytes` to `{dir}/{file}` (the same unique-temp +
/// rename pattern as [`save_json_atomic`] — both draw temp names from
/// [`SAVE_SEQ`] so concurrent saves into one dir can never collide).
pub(crate) fn save_bytes_atomic(dir: &Path, file: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(file);
    let seq = SAVE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{file}.tmp.{}.{seq}", std::process::id()));
    let result = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Load JSON from `path`, falling back to `T::default()` when the file is
/// missing or unreadable.
pub(crate) fn load_json_or_default<T: DeserializeOwned + Default>(path: &Path) -> T {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
            tracing::warn!(error = %e, path = %path.display(), "companion json unreadable; using defaults");
            T::default()
        }),
        Err(_) => T::default(),
    }
}
