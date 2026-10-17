//! Bounded, version-checked text pages through the existing workspace owner.
//! A digest identifies bytes observed by this read, not a filesystem snapshot
//! or a write lease. Windows reads pin the authorized directory and verify
//! the opened file identity before returning a page.
use std::path::Path;

use nomifun_common::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::path_safety::{validate_path_authority, validate_path_for_write_authority};
use crate::{AgentSessionWorkspaceBinding, FileService, WORKSPACE_READ_OPERATION};

const MAX_FILE_BYTES: usize = nomi_native_files::MAX_SOURCE_BYTES;
const MAX_PAGE_BYTES: usize = 16 * 1024;
const MAX_JSON_BYTES: usize = 24 * 1024;
const MAX_TOOL_TEXT_BYTES: usize = 24 * 1024;

/// Results become JSON text inside a serialized tool-result text part. Count
/// both escape layers and leave 8 KiB for the host's <=1024-byte call identity
/// (including worst-case JSON escaping)
/// and envelope, otherwise a valid page can still be cut by the 32-KiB journal.
pub(crate) fn fits_text_result(value: &impl Serialize, reserve: usize) -> Result<bool, AppError> {
    let json = serde_json::to_string(value).map_err(|e| AppError::Internal(e.to_string()))?;
    let wire = serde_json::to_vec(&json).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(json.len().saturating_add(reserve) <= MAX_JSON_BYTES
        && wire.len().saturating_add(reserve) <= MAX_TOOL_TEXT_BYTES)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTextReadRequest {
    pub path: String,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    /// Independent source inspection by lines; byte continuation is separate.
    #[serde(default)]
    pub start_line: Option<usize>,
    #[serde(default)]
    pub line_count: Option<usize>,
}

fn default_limit() -> usize {
    MAX_PAGE_BYTES
}

#[derive(Debug, Serialize)]
pub struct AgentTextPage {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<crate::WorkspacePathObservation>,
    pub content: String,
    pub sha256: String,
    pub total_bytes: usize,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub eof: bool,
    /// One-based line containing the first byte; content is never numbered or
    /// newline-normalized, so offsets remain exact source-byte positions.
    pub start_line: usize,
    pub start_column_bytes: usize,
    /// False means an independent read of the current file version. The hash
    /// still identifies the entire source; callers can pin later reads to it.
    pub source_version_pinned: bool,
}

/// Validate before acquiring bytes, including when the acquisition owner needs
/// a native administrator context. Projection repeats this check so every
/// caller retains the same paging contract.
pub fn validate_agent_text_read_request(request: &AgentTextReadRequest) -> Result<(), AppError> {
    let by_lines = request.start_line.is_some() || request.line_count.is_some();
    if request.path.is_empty()
        || request.path.len() > 4096
        || request.path.contains('\0')
        || !(4..=MAX_PAGE_BYTES).contains(&request.limit)
        || request.offset > MAX_FILE_BYTES
        || request.start_line.is_some_and(|line| line == 0 || line > MAX_FILE_BYTES + 1)
        || request.line_count.is_some_and(|lines| !(1..=2000).contains(&lines))
        || (by_lines && (request.offset != 0 || request.limit != MAX_PAGE_BYTES))
        || request.expected_sha256.as_ref().is_some_and(|hash| {
            hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err(AppError::BadRequest("Invalid text read: use a native absolute or Session-relative path and either byte offset/limit 4..16384 or start_line/line_count. expected_sha256 optionally pins the current source version.".into()));
    }
    Ok(())
}

/// Project an owner's bounded, versioned native file acquisition through the
/// canonical text paging rules. The owner supplies bytes, their SHA-256, and
/// the actual canonical target; this function never opens the source. The
/// workspace root is used only for optional project-path presentation.
pub fn project_agent_text_page(
    mut request: AgentTextReadRequest,
    bytes: Vec<u8>,
    sha256: String,
    workspace_root: &Path,
    canonical_path: &Path,
) -> Result<AgentTextPage, AppError> {
    validate_agent_text_read_request(&request)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(AppError::BadRequest("Source is not a bounded regular file".into()));
    }
    if sha256 != format!("{:x}", Sha256::digest(&bytes)) {
        return Err(AppError::Internal("Acquired text source digest does not match its bytes".into()));
    }
    let text = decode_source_text(bytes)?;
    if request.expected_sha256.as_ref().is_some_and(|expected| expected != &sha256) {
        return Err(AppError::Conflict("FILE_CONTENT_CHANGED: discard prior pages and restart at offset 0 without expected_sha256".into()));
    }
    if request.start_line.is_some() || request.line_count.is_some() {
        let line = request.start_line.unwrap_or(1);
        let start = if line == 1 { 0 } else {
            text.match_indices('\n').nth(line - 2).map_or(text.len(), |(index, _)| index + 1)
        };
        let end = text[start..].match_indices('\n').nth(request.line_count.unwrap_or(200) - 1)
            .map_or(text.len(), |(index, _)| start + index + 1);
        request.offset = start;
        request.limit = (end - start).min(MAX_PAGE_BYTES);
    }
    page(request, text, sha256, crate::WorkspacePathObservation::from_canonical(workspace_root, canonical_path))
}

impl FileService {
    /// Raw bytes for platform-owned decoders, not an engine filesystem bypass.
    /// Authority/path checks are identical to text reads. No persistence or
    /// format inference occurs here; the caller owns safe media preparation.
    pub async fn read_bytes_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        relative_path: &str,
        max_bytes: usize,
    ) -> Result<Option<(Vec<u8>, String)>, AppError> {
        scope.require_operation(WORKSPACE_READ_OPERATION)?;
        if relative_path.is_empty()
            || relative_path.len() > 4096
            || relative_path.contains('\0')
            || max_bytes > MAX_FILE_BYTES
        {
            return Err(AppError::BadRequest(
                "Invalid bounded workspace byte read".into(),
            ));
        }
        let path = scope.resolve_path(relative_path)?;
        let authority = scope.authority();
        tokio::task::spawn_blocking(move || {
            let mut charged_bytes = 0;
            read_source_bytes(&path, &authority, max_bytes, &mut charged_bytes)
                .map(|source| source.map(|(bytes, digest, _)| (bytes, digest)))
        })
        .await
        .map_err(|error| AppError::Internal(format!("workspace byte read task failed: {error}")))?
    }

    pub async fn read_text_page_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        request: AgentTextReadRequest,
    ) -> Result<Option<AgentTextPage>, AppError> {
        scope.require_operation(WORKSPACE_READ_OPERATION)?;
        validate_agent_text_read_request(&request)?;
        let path = scope.resolve_path(&request.path)?;
        let authority = scope.authority();
        let root = scope.workspace_root().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut charged_bytes = 0;
            let Some((bytes, digest, canonical)) = read_source_bytes(&path, &authority, MAX_FILE_BYTES, &mut charged_bytes)? else {
                return Ok(None);
            };
            project_agent_text_page(request, bytes, digest, &root, &canonical).map(Some)
        }).await.map_err(|error| AppError::Internal(format!("text page task failed: {error}")))?
    }
}

/// Shared native bounded read. Charge actual bytes even on a partial IO
/// failure so a search cannot escape its aggregate read budget via errors.
pub(crate) fn read_source(
    path: &std::path::Path,
    authority: &crate::PathAuthority,
    max_bytes: usize,
    charged_bytes: &mut usize,
) -> Result<Option<(String, String)>, AppError> {
    read_source_with_path(path, authority, max_bytes, charged_bytes)
        .map(|source| source.map(|(text, digest, _)| (text, digest)))
}

fn read_source_with_path(
    path: &std::path::Path,
    authority: &crate::PathAuthority,
    max_bytes: usize,
    charged_bytes: &mut usize,
) -> Result<Option<(String, String, std::path::PathBuf)>, AppError> {
    let Some((bytes, digest, canonical)) = read_source_bytes(path, authority, max_bytes, charged_bytes)?
    else {
        return Ok(None);
    };
    let text = decode_source_text(bytes)?;
    Ok(Some((text, digest, canonical)))
}

fn decode_source_text(bytes: Vec<u8>) -> Result<String, AppError> {
    String::from_utf8(bytes).map_err(|_| {
        AppError::BadRequest(
            "Text source is not UTF-8; no lossy binary conversion was performed".into(),
        )
    })
}

fn read_source_bytes(
    path: &std::path::Path,
    authority: &crate::PathAuthority,
    max_bytes: usize,
    charged_bytes: &mut usize,
) -> Result<Option<(Vec<u8>, String, std::path::PathBuf)>, AppError> {
    read_source_bytes_with_hooks(path, authority, max_bytes, charged_bytes, || {}, || {})
}

pub(crate) fn read_source_bytes_with_hooks(
    path: &std::path::Path,
    authority: &crate::PathAuthority,
    max_bytes: usize,
    charged_bytes: &mut usize,
    before_open: impl FnOnce(),
    after_read: impl FnOnce(),
) -> Result<Option<(Vec<u8>, String, std::path::PathBuf)>, AppError> {
    if max_bytes > MAX_FILE_BYTES {
        return Err(AppError::BadRequest(
            "Source byte budget exceeds the owner limit".into(),
        ));
    }
    let canonical = match validate_path_authority(&path.to_string_lossy(), authority) {
        Ok(path) => path,
        Err(error) => {
            // Never turn an authority rejection or permission error
            // into absence. A missing target still needs confinement.
            if std::fs::symlink_metadata(path)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                && missing_path_is_confined(path, authority)
            {
                return Ok(None);
            }
            return Err(error);
        }
    };
    #[cfg(windows)]
    let root = crate::windows_read::ReadRoot::for_target(&canonical,authority)?;
    let open = || {
        #[cfg(windows)]
        if let Some(root) = &root { return root.open(&canonical); }
        // Keep ambient special-file rejection on the existing fallback path;
        // opening a FIFO before checking its type could block indefinitely.
        let before = std::fs::symlink_metadata(&canonical).map_err(io_error)?;
        if !before.is_file() || before.len() > max_bytes as u64 {
            return Err(AppError::BadRequest("Source is not a bounded regular file".into()));
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
        }
        options.open(&canonical).map_err(io_error)
    };
    before_open();
    let (bytes, sha256) = nomi_native_files::acquire_source_with_adapter(
        max_bytes, charged_bytes, open, after_read,
        || Ok(validate_path_authority(&path.to_string_lossy(), authority)? == canonical),
        crate::native_file_acquisition_error,
    )?;
    Ok(Some((bytes, sha256, canonical)))
}

fn io_error(error: std::io::Error) -> AppError {
    crate::path_safety::native_io_error("reading native file", error)
}

fn missing_path_is_confined(path: &std::path::Path, authority: &crate::PathAuthority) -> bool {
    // Ascend only over genuinely absent components. A dangling symlink is an
    // existing component and must not be mistaken for an absent directory.
    let mut candidate = path;
    loop {
        if validate_path_for_write_authority(&candidate.to_string_lossy(), authority).is_ok() {
            return true;
        }
        if !std::fs::symlink_metadata(candidate)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            return false;
        }
        let Some(parent) = candidate.parent() else {
            return false;
        };
        candidate = parent;
    }
}

fn page(
    request: AgentTextReadRequest,
    text: String,
    sha256: String,
    workspace_path: Option<crate::WorkspacePathObservation>,
) -> Result<AgentTextPage, AppError> {
    let start = request.offset;
    if start > text.len() || !text.is_char_boundary(start) {
        return Err(AppError::BadRequest(
            "Text offset must be a UTF-8 boundary within total_bytes; use next_offset".into(),
        ));
    }
    let mut end = (start + request.limit).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let (start_line, start_column_bytes) = crate::agent_patch_lines::line_position(&text, start);
    let mut result = AgentTextPage {
        path: request.path,
        workspace_path,
        content: String::new(),
        sha256,
        total_bytes: text.len(),
        offset: start,
        next_offset: None,
        eof: false,
        start_line,
        start_column_bytes,
        source_version_pinned: request.expected_sha256.is_some(),
    };
    loop {
        result.content = text[start..end].to_owned();
        result.eof = end == text.len();
        result.next_offset = (!result.eof).then_some(end);
        if fits_text_result(&result, 0)? {
            return Ok(result);
        }
        // Include metadata and JSON escaping in the page envelope. No lossy
        // downstream truncation, and every non-EOF page makes progress.
        if end == start {
            return Err(AppError::BadRequest(
                "Text page metadata exceeds its envelope".into(),
            ));
        }
        end = start + (end - start) / 2;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            return Err(AppError::BadRequest(
                "Text page cannot fit one UTF-8 character".into(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_request(value: serde_json::Value) -> AgentTextReadRequest {
        serde_json::from_value(value).unwrap()
    }

    fn project_bytes(request: AgentTextReadRequest, bytes: &[u8]) -> Result<AgentTextPage, AppError> {
        let root = tempfile::tempdir().unwrap();
        let canonical = root.path().canonicalize().unwrap().join("not-opened.txt");
        assert!(!canonical.exists());
        project_agent_text_page(request, bytes.to_vec(), format!("{:x}", Sha256::digest(bytes)), root.path(), &canonical)
    }

    #[test]
    fn shared_request_validation_preserves_native_paths_and_rejects_invalid_paging() {
        for value in [
            serde_json::json!({"path":""}),
            serde_json::json!({"path":"bad\0path"}),
            serde_json::json!({"path":"x".repeat(4097)}),
            serde_json::json!({"path":"text", "limit":3}),
            serde_json::json!({"path":"text", "limit":MAX_PAGE_BYTES + 1}),
            serde_json::json!({"path":"text", "offset":MAX_FILE_BYTES + 1}),
            serde_json::json!({"path":"text", "start_line":0}),
            serde_json::json!({"path":"text", "start_line":MAX_FILE_BYTES + 2}),
            serde_json::json!({"path":"text", "line_count":0}),
            serde_json::json!({"path":"text", "line_count":2001}),
            serde_json::json!({"path":"text", "start_line":1, "offset":1}),
            serde_json::json!({"path":"text", "line_count":1, "limit":4}),
            serde_json::json!({"path":"text", "expected_sha256":"a".repeat(63)}),
            serde_json::json!({"path":"text", "expected_sha256":"A".repeat(64)}),
            serde_json::json!({"path":"text", "expected_sha256":"g".repeat(64)}),
        ] {
            let request = text_request(value);
            assert!(matches!(validate_agent_text_read_request(&request), Err(AppError::BadRequest(_))), "{request:?}");
            assert!(matches!(project_bytes(request, b"content"), Err(AppError::BadRequest(_))));
        }
        for path in ["../native path/文件", " /native\nname "] {
            let result = project_bytes(text_request(serde_json::json!({"path":path})), b"content").unwrap();
            assert_eq!(result.path, path);
        }
        assert!(serde_json::from_value::<AgentTextReadRequest>(serde_json::json!({"path":"text", "unknown":true})).is_err());
    }

    #[test]
    fn shared_projection_checks_source_budget_digest_utf8_and_version_before_paging() {
        let request = || text_request(serde_json::json!({"path":"text"}));
        assert!(matches!(project_bytes(request(), &vec![b'x'; MAX_FILE_BYTES + 1]), Err(AppError::BadRequest(_))));
        let root = tempfile::tempdir().unwrap();
        let inconsistent = project_agent_text_page(request(), b"text".to_vec(), "0".repeat(64), root.path(), &root.path().join("text"));
        assert!(matches!(inconsistent, Err(AppError::Internal(message)) if message.contains("digest")));
        assert!(matches!(project_bytes(request(), &[0xff, 0xfe]), Err(AppError::BadRequest(message)) if message.contains("not UTF-8")));
        let stale = text_request(serde_json::json!({"path":"text", "offset":99, "expected_sha256":"0".repeat(64)}));
        assert!(matches!(project_bytes(stale, b"text"), Err(AppError::Conflict(message)) if message.contains("FILE_CONTENT_CHANGED")));
        let digest = format!("{:x}", Sha256::digest(b"text"));
        let pinned = project_bytes(text_request(serde_json::json!({"path":"text", "expected_sha256":digest})), b"text").unwrap();
        assert!(pinned.source_version_pinned);
        assert_eq!(pinned.sha256, digest);
        assert_eq!(pinned.total_bytes, 4);
    }

    #[test]
    fn shared_projection_keeps_utf8_boundaries_empty_eof_and_exact_line_selection() {
        let text = "é🌟tail\r\n第二行\nlast";
        let result = project_bytes(text_request(serde_json::json!({"path":"text", "offset":2, "limit":5})), text.as_bytes()).unwrap();
        assert_eq!(result.content, "🌟t");
        assert_eq!((result.start_line, result.start_column_bytes), (1, 3));
        assert_eq!(result.next_offset, Some(7));
        assert!(!result.eof);
        for offset in [1, text.len() + 1] {
            assert!(matches!(project_bytes(text_request(serde_json::json!({"path":"text", "offset":offset})), text.as_bytes()), Err(AppError::BadRequest(_))));
        }
        let lines = project_bytes(text_request(serde_json::json!({"path":"text", "start_line":2, "line_count":1})), text.as_bytes()).unwrap();
        assert_eq!(lines.content, "第二行\n");
        assert_eq!((lines.start_line, lines.start_column_bytes), (2, 1));
        assert_eq!(lines.next_offset, Some("é🌟tail\r\n第二行\n".len()));
        assert!(!lines.eof, "line_count ending does not imply source EOF");
        for bytes in [b"".as_slice(), b"one\n".as_slice()] {
            let eof = project_bytes(text_request(serde_json::json!({"path":"text", "start_line":99, "line_count":1})), bytes).unwrap();
            assert!(eof.eof);
            assert_eq!(eof.offset, bytes.len());
            assert!(eof.content.is_empty());
            assert_eq!(eof.next_offset, None);
        }
        let long = vec![b'x'; MAX_PAGE_BYTES + 1];
        let first_line = project_bytes(text_request(serde_json::json!({"path":"text", "line_count":1})), &long).unwrap();
        assert_eq!(first_line.content.len(), MAX_PAGE_BYTES);
        assert_eq!(first_line.next_offset, Some(MAX_PAGE_BYTES));
    }

    #[test]
    fn shared_projection_bounds_both_json_escape_layers_and_preserves_progress() {
        let text = "\0\"\\\n💫".repeat(4000);
        let path = "\\\"".repeat(1500);
        let mut offset = 0;
        let mut content = String::new();
        loop {
            let page = project_bytes(text_request(serde_json::json!({"path":path, "offset":offset})), text.as_bytes()).unwrap();
            let json = serde_json::to_string(&page).unwrap();
            assert!(json.len() <= MAX_JSON_BYTES);
            assert!(serde_json::to_vec(&json).unwrap().len() <= MAX_TOOL_TEXT_BYTES);
            content.push_str(&page.content);
            if page.eof { break; }
            let next = page.next_offset.unwrap();
            assert!(next > offset);
            assert!(text.is_char_boundary(next));
            offset = next;
        }
        assert_eq!(content, text);
    }

    #[tokio::test]
    async fn native_and_acquired_byte_pages_have_identical_metadata_without_reopening_source() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let source = root.join("text.txt");
        let text = "\u{feff}first\r\n中文🌟\nlast";
        std::fs::write(&source, text).unwrap();
        let canonical = source.canonicalize().unwrap();
        let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
        let scope = crate::resource::workspace_binding(nomifun_common::generate_id(), "binding", "workspace", "owner",
            [WORKSPACE_READ_OPERATION], &root).unwrap();
        let service = FileService::new(std::sync::Arc::new(NoEvents), vec![]);
        for value in [
            serde_json::json!({"path":"text.txt"}),
            serde_json::json!({"path":"text.txt", "offset":10, "limit":7, "expected_sha256":digest}),
            serde_json::json!({"path":"text.txt", "start_line":2, "line_count":1}),
        ] {
            let normal = service.read_text_page_for_agent_session(&scope, text_request(value.clone())).await.unwrap().unwrap();
            let projected = project_agent_text_page(text_request(value), text.as_bytes().to_vec(), digest.clone(), &root, &canonical).unwrap();
            assert_eq!(serde_json::to_value(normal).unwrap(), serde_json::to_value(projected).unwrap());
        }
        std::fs::remove_file(&source).unwrap();
        let retained = project_agent_text_page(text_request(serde_json::json!({"path":"text.txt"})), text.as_bytes().to_vec(), digest.clone(), &root, &canonical).unwrap();
        assert_eq!(retained.content, text);
        assert_eq!(retained.workspace_path.unwrap().path, "text.txt");
        let outside = fixture.path().canonicalize().unwrap().join("outside.txt");
        let external = project_agent_text_page(text_request(serde_json::json!({"path":"requested-alias"})), text.as_bytes().to_vec(), digest, &root, &outside).unwrap();
        assert_eq!(external.path, "requested-alias");
        assert!(external.workspace_path.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn shared_source_adapter_preserves_post_read_authority_errors_and_byte_charges() {
        use std::{fs, os::unix::fs::symlink};
        for confined in [false, true] {
            let fixture = tempfile::tempdir().unwrap();
            let root = fixture.path().join("workspace");
            fs::create_dir(&root).unwrap();
            let source = root.join("source.txt");
            let outside = fixture.path().join("outside.txt");
            let alias = root.join("alias.txt");
            fs::write(&source, b"inside").unwrap();
            fs::write(&outside, b"outside").unwrap();
            symlink(&source, &alias).unwrap();
            let authority = if confined { crate::PathAuthority::Workspace(root) } else { crate::PathAuthority::Unrestricted };
            let mut charged = 0;
            let result = read_source_bytes_with_hooks(&alias, &authority, 64, &mut charged, || {}, || {
                fs::remove_file(&alias).unwrap();
                symlink(&outside, &alias).unwrap();
            });
            if confined {
                assert!(matches!(result, Err(AppError::Forbidden(_))), "{result:?}");
            } else {
                assert!(matches!(result, Err(AppError::Conflict(message)) if message.contains("FILE_CHANGED_DURING_READ")));
            }
            assert_eq!(charged, 6, "bytes are charged even when final authority verification fails");
            assert_eq!(fs::read(&source).unwrap(), b"inside");
            assert_eq!(fs::read(&outside).unwrap(), b"outside");
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn native_utf8_alias_can_read_a_non_utf8_target_without_a_lossy_project_path() {
        use std::{ffi::OsString, fs, os::unix::{ffi::OsStringExt as _, fs::symlink}};
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join(OsString::from_vec(b"native-\xff.txt".to_vec()));
        fs::write(&source, b"native bytes").unwrap();
        symlink(&source, root.path().join("alias.txt")).unwrap();
        let scope = crate::resource::workspace_binding(nomifun_common::generate_id(), "binding", "workspace", "owner",
            [WORKSPACE_READ_OPERATION], root.path()).unwrap();
        let service = FileService::new(std::sync::Arc::new(NoEvents), vec![]);
        let page = service.read_text_page_for_agent_session(&scope, text_request(serde_json::json!({"path":"alias.txt"}))).await.unwrap().unwrap();
        assert_eq!(page.path, "alias.txt");
        assert_eq!(page.content, "native bytes");
        assert!(page.workspace_path.is_none(), "an unrepresentable project path is never replaced with lossy text");
        let (bytes, digest) = service.read_bytes_for_agent_session(&scope, "alias.txt", 64).await.unwrap().unwrap();
        assert_eq!(bytes, b"native bytes");
        assert_eq!(digest, page.sha256);
    }

    #[cfg(windows)]
    fn race_fixture() -> tempfile::TempDir {
        let mut builder = tempfile::Builder::new();
        builder.prefix("read-race-");
        match std::env::var_os("NOMIFUN_RELIABILITY_FIXTURE_PARENT") {
            Some(parent) => builder.tempdir_in(parent).unwrap(),
            None => builder.tempdir().unwrap(),
        }
    }

    #[cfg(windows)]
    #[test]
    fn transient_parent_junction_never_reads_outside_bytes() {
        use std::{cell::Cell, fs};
        let fixture = race_fixture();
        let root = fixture.path().join("workspace");
        let inside = root.join("inside");
        let saved = root.join("saved");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&inside).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(inside.join("value.txt"),b"inside").unwrap();
        fs::write(outside.join("value.txt"),b"secret").unwrap();
        let redirected = Cell::new(false);
        let restore = || {
            if redirected.replace(false) {
                junction::delete(&inside).unwrap();
                fs::rename(&saved,&inside).unwrap();
            }
        };
        let mut charged = 0;
        let result = read_source_bytes_with_hooks(&inside.join("value.txt"),
            &crate::PathAuthority::Workspace(root),64,&mut charged,
            || {
                fs::rename(&inside,&saved).unwrap();
                junction::create(&outside,&inside).unwrap();
                redirected.set(true);
            }, restore);
        restore();
        assert_eq!(fs::read(inside.join("value.txt")).unwrap(),b"inside");
        assert_eq!(fs::read(outside.join("value.txt")).unwrap(),b"secret");
        if result.is_ok() || charged != 0 {
            let observation = serde_json::json!({"charged_bytes":charged,"result":format!("{result:?}")});
            fs::write(fixture.path().join("observation.json"),observation.to_string()).unwrap();
            let retained = fixture.keep();
            panic!("outside read was not prevented; retained fixture: {}",retained.display());
        }
    }

    #[cfg(windows)]
    #[test]
    fn replacement_at_the_same_name_during_read_is_a_conflict() {
        use std::fs;
        let fixture = race_fixture();
        let path = fixture.path().join("value.txt");
        let retained = fixture.path().join("original.txt");
        fs::write(&path,b"inside").unwrap();
        let mut charged = 0;
        let authority = crate::PathAuthority::Workspace(fixture.path().to_path_buf());
        let result = read_source_bytes_with_hooks(&path,&authority,64,&mut charged,|| {},|| {
            fs::rename(&path,&retained).unwrap();
            fs::write(&path,b"newest").unwrap();
        });
        assert_eq!(fs::read(&path).unwrap(),b"newest");
        assert_eq!(fs::read(&retained).unwrap(),b"inside");
        if !matches!(&result,Err(AppError::Conflict(message)) if message.contains("FILE_CHANGED_DURING_READ")) {
            fs::write(fixture.path().join("observation.json"),format!("{result:?}")).unwrap();
            let location = fixture.keep();
            panic!("same-name replacement was not rejected; retained fixture: {}",location.display());
        }
        assert_eq!(charged,6);
        assert_eq!(read_source_bytes(&path,&authority,64,&mut 0).unwrap().unwrap().0,b"newest");
    }

    #[cfg(windows)]
    #[test]
    fn transient_workspace_root_junction_is_rejected_before_reading() {
        use std::{cell::Cell, fs};
        let fixture = race_fixture();
        let root = fixture.path().join("workspace");
        let saved = fixture.path().join("saved");
        let outside = fixture.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(root.join("value.txt"),b"inside").unwrap();
        fs::write(outside.join("value.txt"),b"secret").unwrap();
        let redirected = Cell::new(false);
        let restore = || {
            if redirected.replace(false) {
                junction::delete(&root).unwrap();
                fs::rename(&saved,&root).unwrap();
            }
        };
        let mut charged = 0;
        let result = read_source_bytes_with_hooks(&root.join("value.txt"),
            &crate::PathAuthority::Workspace(root.clone()),64,&mut charged,
            || {
                fs::rename(&root,&saved).unwrap();
                junction::create(&outside,&root).unwrap();
                redirected.set(true);
            },restore);
        restore();
        assert!(result.is_err(),"{result:?}");
        assert_eq!(charged,0);
        assert_eq!(fs::read(root.join("value.txt")).unwrap(),b"inside");
        assert_eq!(fs::read(outside.join("value.txt")).unwrap(),b"secret");
    }

    #[cfg(windows)]
    #[test]
    fn transient_alias_into_owner_namespace_never_reads_its_bytes() {
        use std::{cell::Cell, fs};
        let fixture = race_fixture();
        let inside = fixture.path().join("inside");
        let saved = fixture.path().join("saved");
        let protected = fixture.path().join(".nomifun");
        fs::create_dir(&inside).unwrap();
        fs::create_dir(&protected).unwrap();
        fs::write(inside.join("value.txt"),b"inside").unwrap();
        fs::write(protected.join("value.txt"),b"secret").unwrap();
        let redirected = Cell::new(false);
        let restore = || {
            if redirected.replace(false) {
                junction::delete(&inside).unwrap();
                fs::rename(&saved,&inside).unwrap();
            }
        };
        let mut charged = 0;
        let result = read_source_bytes_with_hooks(&inside.join("value.txt"),
            &crate::PathAuthority::Workspace(fixture.path().to_path_buf()),64,&mut charged,
            || {
                fs::rename(&inside,&saved).unwrap();
                junction::create(&protected,&inside).unwrap();
                redirected.set(true);
            },restore);
        restore();
        assert!(result.is_err(),"{result:?}");
        assert_eq!(charged,0);
        assert_eq!(fs::read(inside.join("value.txt")).unwrap(),b"inside");
        assert_eq!(fs::read(protected.join("value.txt")).unwrap(),b"secret");
    }
    struct NoEvents;
    impl nomifun_realtime::UserEventSink for NoEvents {
        fn send_to_user(&self, _: &str, _: nomifun_api_types::WebSocketMessage<serde_json::Value>) {}
    }

    #[tokio::test]
    async fn whole_text_agent_entry_preserves_content_and_uses_the_agent_read_budget() {
        let root=tempfile::tempdir().unwrap();
        let scope=crate::resource::workspace_binding(nomifun_common::generate_id(),"binding","workspace","owner",
            [WORKSPACE_READ_OPERATION],root.path()).unwrap();
        let service=FileService::new(std::sync::Arc::new(NoEvents),vec![]);
        let text="\u{feff}first\r\n中文\nlast";
        std::fs::write(root.path().join("text.txt"),text).unwrap();
        assert_eq!(service.read_file_for_agent_session(&scope,"text.txt").await.unwrap().as_deref(),Some(text));
        std::fs::write(root.path().join("large.txt"),vec![b'x';MAX_FILE_BYTES]).unwrap();
        assert_eq!(service.read_file_for_agent_session(&scope,"large.txt").await.unwrap().unwrap().len(),MAX_FILE_BYTES);
        std::fs::write(root.path().join("large.txt"),vec![b'x';MAX_FILE_BYTES+1]).unwrap();
        assert!(service.read_file_for_agent_session(&scope,"large.txt").await.is_err());
        std::fs::write(root.path().join("binary.bin"),[0xff,0xfe]).unwrap();
        assert!(service.read_file_for_agent_session(&scope,"binary.bin").await.is_err());
        assert!(service.read_file_for_agent_session(&scope,"missing.txt").await.unwrap().is_none());
        assert!(service.read_file_for_agent_session(&scope,"bad\0path").await.is_err());
    }

    #[tokio::test]
    async fn line_and_random_byte_reads_are_independent_but_explicit_version_pins_still_hold() {
        let root=tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source.js"),"first\r\n第二行\r\nlast\n").unwrap();
        let scope=crate::resource::workspace_binding(nomifun_common::generate_id(),"binding","workspace","owner",
            [WORKSPACE_READ_OPERATION],root.path()).unwrap();
        let service=FileService::new(std::sync::Arc::new(NoEvents),vec![root.path().to_path_buf()]);
        let request=|value| serde_json::from_value::<AgentTextReadRequest>(value).unwrap();
        let line=service.read_text_page_for_agent_session(&scope,request(serde_json::json!({
            "path":"source.js","start_line":2,"line_count":1
        }))).await.unwrap().unwrap();
        assert_eq!(line.content,"第二行\r\n");
        assert_eq!(line.offset,7);
        assert!(!line.source_version_pinned);
        let bytes=service.read_text_page_for_agent_session(&scope,request(serde_json::json!({
            "path":"source.js","offset":7,"limit":6
        }))).await.unwrap().unwrap();
        assert_eq!(bytes.content,"第二");
        assert_eq!(bytes.sha256,line.sha256);
        std::fs::write(root.path().join("source.js"),"first\r\n变化\n").unwrap();
        let stale=service.read_text_page_for_agent_session(&scope,request(serde_json::json!({
            "path":"source.js","offset":7,"limit":6,"expected_sha256":line.sha256
        }))).await.unwrap_err();
        assert!(stale.to_string().contains("FILE_CONTENT_CHANGED"));
        let fresh=service.read_text_page_for_agent_session(&scope,request(serde_json::json!({
            "path":"source.js","offset":7,"limit":6
        }))).await.unwrap().unwrap();
        assert_eq!(fresh.content,"变化");
        assert_ne!(fresh.sha256,bytes.sha256);
        assert!(service.read_text_page_for_agent_session(&scope,request(serde_json::json!({
            "path":"bad\0path","offset":7,"limit":6
        }))).await.is_err());
    }
}
