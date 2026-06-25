//! Resolve the conversation workspace directory.

use std::path::{Path, PathBuf};

/// Priority: `--work-dir` CLI flag → `NOMIFUN_WORK_DIR` env (when non-empty) →
/// `--data-dir` fallback.
pub(super) fn resolve_work_dir(cli_work_dir: Option<PathBuf>, data_dir: &Path) -> PathBuf {
    cli_work_dir.unwrap_or_else(|| {
        std::env::var("NOMIFUN_WORK_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.to_path_buf())
    })
}
