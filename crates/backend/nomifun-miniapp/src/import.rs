//! Import: adopt an app the user already wrote as a mini-app.
//!
//! Two entry points, one code path: `validate` reports and writes nothing,
//! `import` reports and — when nothing fatal was found — creates the row and
//! materializes the working copy so the app is immediately iterable. Sharing the
//! path is deliberate: a user who was told "this will import" must not then be
//! refused, and vice versa.
//!
//! **Where the bytes come from.** Either inline text (paste, or a renderer that
//! already holds the file) or an absolute path the user picked — a single
//! document, or a directory whose entry document we locate. The path form is not
//! a new capability: these routes are instance-owner-only, and the owner can
//! already read local files through the `/api/fs/*` surface. What the checks
//! below buy is that a *mistake* fails cheaply — an extension allowlist so a
//! stray binary is rejected before it is read, a size cap so a huge file cannot
//! be pulled into memory, and a bounded walk so a deep tree cannot be traversed
//! forever.
//!
//! **What import does NOT do.** It never copies sibling files. `/serve` returns
//! exactly one stored document, so a page that needs its own `style.css` cannot
//! work here no matter where the file sits — [`crate::validation`] reports that
//! as fatal and the UI offers to rewrite the app in a conversation instead. The
//! honest boundary is "single self-contained document", the same contract every
//! generated mini-app already meets.

use std::path::Path;

use nomifun_common::MiniAppId;

use crate::dto::{MiniAppImportRequest, MiniAppImportResponse, MiniAppResponse};
use crate::service::{MINI_APP_HTML_MAX_BYTES, MiniAppService, MiniAppServiceError};
use crate::validation::{
    ImportFinding, ImportReport, apply_fixes, document_title, find_root_document, validate_import,
};

/// Extensions a document may arrive as. Anything else is refused before a read,
/// so pointing the importer at a video is a fast no rather than a 4 MiB read.
const DOCUMENT_EXTENSIONS: &[&str] = &["html", "htm"];

/// Ceilings on the directory walk. A bundle is inspected only to find the entry
/// document and to tell the user which references cannot be served, so there is
/// no reason to read a large tree.
const MAX_BUNDLE_ENTRIES: usize = 500;
const MAX_BUNDLE_DEPTH: usize = 6;

impl MiniAppService {
    /// Report on a candidate without writing anything.
    pub async fn validate_candidate(
        &self,
        req: &MiniAppImportRequest,
    ) -> Result<MiniAppImportResponse, MiniAppServiceError> {
        let (document, siblings) = self.load_candidate(req).await?;
        let report = validate_import(&document, &siblings);
        Ok(MiniAppImportResponse { report, applied_fixes: Vec::new(), app: None })
    }

    /// Validate, then adopt when nothing fatal was found.
    ///
    /// A blocked candidate returns `Ok` with `app: None` and `blocked: true` — the
    /// route turns that into a 4xx carrying the report, because the caller needs
    /// the findings, not just a status.
    pub async fn import_candidate(
        &self,
        user_id: &str,
        req: MiniAppImportRequest,
    ) -> Result<MiniAppImportResponse, MiniAppServiceError> {
        let (document, siblings) = self.load_candidate(&req).await?;
        let report = validate_import(&document, &siblings);
        if report.blocked {
            return Ok(MiniAppImportResponse { report, applied_fixes: Vec::new(), app: None });
        }

        let (document, applied_fixes) = apply_fixes(&document, &report);
        // The fix can only grow the document (it wraps), so re-check the one bound
        // that growth can break rather than trusting the pre-fix measurement.
        if document.len() > MINI_APP_HTML_MAX_BYTES {
            return Ok(MiniAppImportResponse {
                report: ImportReport {
                    findings: vec![ImportFinding {
                        rule_id: "size_over_limit",
                        severity: crate::validation::ImportSeverity::Fatal,
                        detail: Some(document.len().to_string()),
                    }],
                    blocked: true,
                },
                applied_fixes: Vec::new(),
                app: None,
            });
        }

        let name = req
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| document_title(&document))
            // A page's own file name beats a hardcoded English string in a
            // Chinese library. Only reached when the document has no `<title>`.
            .or_else(|| candidate_stem(&req))
            .unwrap_or_else(|| "Imported mini-app".to_string());
        // Clamped, not validated: `<title>` and a file name are as long as their
        // author felt like, and `create` refuses a name over the cap. Failing there
        // would break this route's whole promise — a user who was told "this will
        // import" must not then be refused, least of all over a field they never
        // filled in. Sliced by code point so a multi-byte title cannot be cut in
        // half.
        let name = clamp_name(&name);

        let app = self
            .create(
                user_id,
                crate::dto::CreateMiniAppRequest {
                    name,
                    description: req.description,
                    icon: req.icon,
                    html: document,
                    // Provenance is for apps born in a conversation. An imported
                    // app has none, and inventing one would make the library lie
                    // about where it came from.
                    source_conversation_id: None,
                },
            )
            .await?;

        // Materialize immediately: the whole point of importing is that 「继续迭代」
        // works on the very next click, and that needs a working copy on disk.
        let id = MiniAppId::try_from(app.miniapp_id.clone())
            .map_err(|e| MiniAppServiceError::Internal(format!("imported id is not canonical: {e}")))?;
        self.ensure_workspace(user_id, &id).await?;
        // Re-read so the response carries the post-materialization publish state
        // rather than the pre-materialization one.
        let app: MiniAppResponse = self.get(user_id, &id).await?;

        Ok(MiniAppImportResponse { report, applied_fixes, app: Some(app) })
    }

    /// Resolve the request into a document plus the payload-relative paths that
    /// travelled with it.
    async fn load_candidate(
        &self,
        req: &MiniAppImportRequest,
    ) -> Result<(String, Vec<String>), MiniAppServiceError> {
        match (req.html.as_deref(), req.path.as_deref()) {
            (Some(_), Some(_)) => Err(MiniAppServiceError::BadRequest(
                "supply either inline html or a path, not both".into(),
            )),
            (None, None) => Err(MiniAppServiceError::BadRequest(
                "supply either inline html or a path".into(),
            )),
            (Some(html), None) => {
                if html.len() > MINI_APP_HTML_MAX_BYTES {
                    // Reported as a finding by the scan too, but rejecting here
                    // keeps an oversized body from being copied around first.
                    return Err(MiniAppServiceError::BadRequest("document is too large".into()));
                }
                Ok((html.to_string(), Vec::new()))
            }
            (None, Some(path)) => self.load_from_path(path).await,
        }
    }

    async fn load_from_path(&self, raw: &str) -> Result<(String, Vec<String>), MiniAppServiceError> {
        let path = Path::new(raw.trim());
        if !path.is_absolute() {
            return Err(MiniAppServiceError::BadRequest(
                "the import path must be absolute".into(),
            ));
        }
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|_| MiniAppServiceError::BadRequest("that path does not exist".into()))?;

        if meta.is_file() {
            require_document_extension(path)?;
            let document = read_document(path, meta.len()).await?;
            // A lone document has no siblings, which is exactly what makes every
            // relative reference in it unserveable.
            return Ok((document, Vec::new()));
        }

        if !meta.is_dir() {
            return Err(MiniAppServiceError::BadRequest(
                "the import path is neither a file nor a directory".into(),
            ));
        }

        let siblings = walk_bundle(path).await?;
        let root = find_root_document(&siblings)
            .map_err(|finding| MiniAppServiceError::BadRequest(finding.rule_id.to_string()))?;
        let root_path = path.join(&root);
        let root_meta = tokio::fs::metadata(&root_path)
            .await
            .map_err(|e| MiniAppServiceError::Internal(format!("stat entry document: {e}")))?;
        let document = read_document(&root_path, root_meta.len()).await?;
        Ok((document, siblings))
    }
}

fn require_document_extension(path: &Path) -> Result<(), MiniAppServiceError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if DOCUMENT_EXTENSIONS.contains(&ext.as_str()) {
        return Ok(());
    }
    Err(MiniAppServiceError::BadRequest(
        "pick an .html file, or the folder that contains its index.html".into(),
    ))
}

/// The picked file's or folder's own name, as a last-resort app name.
///
/// Only the desktop path intake has one; a pasted document does not, which is why
/// this is a fallback and not the primary source.
fn candidate_stem(req: &MiniAppImportRequest) -> Option<String> {
    let path = Path::new(req.path.as_deref()?.trim());
    let stem = path
        .file_stem()
        .or_else(|| path.file_name())
        .and_then(|s| s.to_str())?
        .trim();
    (!stem.is_empty()).then(|| stem.to_string())
}

/// Trim to the name cap by code point.
///
/// `create` refuses a longer name, and every source this route derives one from
/// (`<title>`, a file name) is unbounded. Slicing by code point rather than byte
/// keeps a CJK or emoji title from being cut into invalid UTF-8.
fn clamp_name(name: &str) -> String {
    let trimmed = name.trim();
    let clamped: String = trimmed
        .chars()
        .take(crate::service::MINI_APP_NAME_MAX_CHARS)
        .collect();
    if clamped.trim().is_empty() {
        // `create` also refuses a blank name, and a document whose title is all
        // whitespace must not turn this route's promise into a rejection.
        return "Imported mini-app".to_string();
    }
    clamped.trim().to_string()
}

async fn read_document(path: &Path, len: u64) -> Result<String, MiniAppServiceError> {
    if len as usize > MINI_APP_HTML_MAX_BYTES {
        return Err(MiniAppServiceError::BadRequest("document is too large".into()));
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| MiniAppServiceError::BadRequest(format!("cannot read that file: {e}")))?;
    // Invalid UTF-8 is reported as a bad request rather than a finding: the scan
    // works on text, so there is nothing to report on until it decodes.
    String::from_utf8(bytes)
        .map_err(|_| MiniAppServiceError::BadRequest("the document is not valid UTF-8 text".into()))
}

/// Collect payload-relative paths, breadth-first, with hard ceilings.
///
/// Symlinks are listed but never followed: a link out of the bundle would let a
/// walk wander the filesystem, and the entry document is resolved by name inside
/// the chosen directory anyway.
async fn walk_bundle(root: &Path) -> Result<Vec<String>, MiniAppServiceError> {
    let mut out: Vec<String> = Vec::new();
    let mut queue: Vec<(std::path::PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, depth)) = queue.pop() {
        let mut entries = tokio::fs::read_dir(&dir)
            .await
            .map_err(|e| MiniAppServiceError::BadRequest(format!("cannot read that folder: {e}")))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| MiniAppServiceError::Internal(format!("read folder entry: {e}")))?
        {
            if out.len() >= MAX_BUNDLE_ENTRIES {
                return Err(MiniAppServiceError::BadRequest(
                    "that folder holds too many files for an import".into(),
                ));
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| MiniAppServiceError::Internal("bundle entry escaped its root".into()))?
                .to_string_lossy()
                .replace('\\', "/");
            let file_type = entry
                .file_type()
                .await
                .map_err(|e| MiniAppServiceError::Internal(format!("stat folder entry: {e}")))?;
            if file_type.is_dir() {
                if depth + 1 <= MAX_BUNDLE_DEPTH {
                    queue.push((path, depth + 1));
                }
                continue;
            }
            out.push(relative);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<!DOCTYPE html><html><head><title>Imported</title></head><body><p>hi</p></body></html>";

    #[test]
    fn only_html_extensions_are_accepted_before_a_read() {
        assert!(require_document_extension(Path::new("/tmp/a.html")).is_ok());
        assert!(require_document_extension(Path::new("/tmp/a.HTM")).is_ok());
        for bad in ["/tmp/a.mp4", "/tmp/a.js", "/tmp/a", "/tmp/a.html.txt"] {
            assert!(require_document_extension(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn a_bundle_walk_is_relative_bounded_and_slash_separated() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("index.html"), PAGE).await.unwrap();
        tokio::fs::create_dir(dir.path().join("assets")).await.unwrap();
        tokio::fs::write(dir.path().join("assets/app.css"), "body{}").await.unwrap();

        let mut found = walk_bundle(dir.path()).await.unwrap();
        found.sort();
        assert_eq!(found, vec!["assets/app.css".to_string(), "index.html".to_string()]);
    }

    #[tokio::test]
    async fn an_oversized_file_is_refused_without_being_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.html");
        tokio::fs::write(&path, "x").await.unwrap();
        // The length is supplied by the caller's `stat`, so the guard is provable
        // without writing four megabytes.
        let err = read_document(&path, (MINI_APP_HTML_MAX_BYTES + 1) as u64).await.unwrap_err();
        assert!(matches!(err, MiniAppServiceError::BadRequest(_)), "{err:?}");
    }

    #[tokio::test]
    async fn non_utf8_bytes_are_a_bad_request_not_a_finding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.html");
        tokio::fs::write(&path, [0xff, 0xfe, 0x00]).await.unwrap();
        let err = read_document(&path, 3).await.unwrap_err();
        assert!(matches!(err, MiniAppServiceError::BadRequest(_)), "{err:?}");
    }
}
