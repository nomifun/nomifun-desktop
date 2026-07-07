//! Built-in integration of the upstream [`obra/superpowers`](https://github.com/obra/superpowers)
//! skills library.
//!
//! superpowers is a collection of methodology skills (`SKILL.md` files: TDD,
//! systematic-debugging, brainstorming, …). We embed the upstream corpus at
//! build time (the offline *baseline*) and optionally refresh it at runtime
//! from GitHub releases (the *overlay*). The "effective" directory prefers the
//! overlay and falls back to the baseline, so the feature works offline out of
//! the box and never degrades when a download fails.
//!
//! Design: `docs/superpowers/specs/2026-07-07-superpowers-integration-design.md`.

use include_dir::{Dir, include_dir};
use sha2::{Digest, Sha256};

/// Embedded upstream superpowers corpus — the offline baseline. Contains the
/// 14 skill directories plus `LICENSE` and `VERSION`.
static SUPERPOWERS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/assets/superpowers");

/// Upstream release tag of the embedded baseline corpus (e.g. `6.0.3`).
pub const SUPERPOWERS_BUNDLED_VERSION: &str = include_str!("../../assets/superpowers/VERSION");

/// Expose the embedded superpowers corpus for startup materialization.
/// Consumers outside this crate should not depend on `include_dir` directly.
pub fn superpowers_corpus() -> &'static Dir<'static> {
    &SUPERPOWERS
}

/// Deterministic SHA-256 fingerprint of the embedded superpowers corpus.
/// Same scheme as [`crate::skill_service::builtin_skills_corpus_fingerprint`]:
/// sorted `(relative_path, contents)` pairs, NUL-separated, lowercase hex.
pub fn superpowers_corpus_fingerprint() -> String {
    let mut files: Vec<(String, &'static [u8])> = Vec::new();
    collect_corpus_files(&SUPERPOWERS, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (path, contents) in files {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(contents);
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn collect_corpus_files(dir: &'static Dir<'static>, out: &mut Vec<(String, &'static [u8])>) {
    for file in dir.files() {
        out.push((file.path().to_string_lossy().into_owned(), file.contents()));
    }
    for subdir in dir.dirs() {
        collect_corpus_files(subdir, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_contains_core_skills() {
        let mut files = Vec::new();
        collect_corpus_files(&SUPERPOWERS, &mut files);
        let paths: Vec<String> = files.iter().map(|(p, _)| p.replace('\\', "/")).collect();

        assert!(
            paths.iter().any(|p| p.ends_with("using-superpowers/SKILL.md")),
            "using-superpowers bootstrap must be embedded; got {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("test-driven-development/SKILL.md")),
            "tdd skill must be embedded"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("systematic-debugging/SKILL.md")),
            "systematic-debugging skill must be embedded"
        );
        assert_eq!(
            paths.iter().filter(|p| p.ends_with("SKILL.md")).count(),
            14,
            "all 14 upstream skills must be embedded"
        );
    }

    #[test]
    fn fingerprint_is_stable_hex() {
        let fp = superpowers_corpus_fingerprint();
        assert_eq!(fp.len(), 64, "sha-256 hex length");
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(fp, superpowers_corpus_fingerprint(), "fingerprint deterministic");
    }

    #[test]
    fn bundled_version_nonempty() {
        assert!(!SUPERPOWERS_BUNDLED_VERSION.trim().is_empty());
    }
}
