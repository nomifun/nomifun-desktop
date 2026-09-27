//! Bounded, version-checked text pages through the existing workspace owner.
//! A digest identifies bytes observed by this read, not a filesystem snapshot
//! or a write lease. Windows reads pin the authorized directory and verify
//! the opened file identity before returning a page.
use std::io::Read;

use nomifun_common::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::path_safety::{validate_path_authority, validate_path_for_write_authority};
use crate::{AgentSessionWorkspaceBinding, FileService, WORKSPACE_READ_OPERATION};

const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
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
        if relative_path.trim().is_empty()
            || relative_path.len() > 4096
            || relative_path.chars().any(char::is_control)
            || max_bytes > MAX_FILE_BYTES
        {
            return Err(AppError::BadRequest(
                "Invalid bounded workspace byte read".into(),
            ));
        }
        let path = scope.resolve_relative_path(relative_path)?;
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
        mut request: AgentTextReadRequest,
    ) -> Result<Option<AgentTextPage>, AppError> {
        scope.require_operation(WORKSPACE_READ_OPERATION)?;
        let by_lines = request.start_line.is_some() || request.line_count.is_some();
        if request.path.trim().is_empty()
            || request.path.len() > 4096
            || request.path.chars().any(char::is_control)
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
            return Err(AppError::BadRequest("Invalid text read: use a relative path and either byte offset/limit 4..16384 or start_line/line_count. expected_sha256 optionally pins the current source version.".into()));
        }
        let path = scope.resolve_relative_path(&request.path)?;
        let authority = scope.authority();
        let root = scope.workspace_root().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut charged_bytes = 0;
            let Some((text, digest, canonical)) = read_source_with_path(&path, &authority, MAX_FILE_BYTES, &mut charged_bytes)? else {
                return Ok(None);
            };
            if request.expected_sha256.as_ref().is_some_and(|expected| expected != &digest) {
                return Err(AppError::Conflict("FILE_CONTENT_CHANGED: discard prior pages and restart at offset 0 without expected_sha256".into()));
            }
            if by_lines {
                let line = request.start_line.unwrap_or(1);
                let start = if line == 1 { 0 } else {
                    text.match_indices('\n').nth(line - 2).map_or(text.len(), |(index, _)| index + 1)
                };
                let end = text[start..].match_indices('\n').nth(request.line_count.unwrap_or(200) - 1)
                    .map_or(text.len(), |(index, _)| start + index + 1);
                request.offset = start;
                request.limit = (end - start).min(MAX_PAGE_BYTES);
            }
            page(request, text, digest, crate::WorkspacePathObservation::from_canonical(&root, &canonical)).map(Some)
        }).await.map_err(|error| AppError::Internal(format!("text page task failed: {error}")))?
    }
}

/// Shared confined bounded read. Charge actual bytes even on a partial IO
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
    let text = String::from_utf8(bytes).map_err(|_| {
        AppError::BadRequest(
            "Text source is not UTF-8; no lossy binary conversion was performed".into(),
        )
    })?;
    Ok(Some((text, digest, canonical)))
}

fn read_source_bytes(
    path: &std::path::Path,
    authority: &crate::PathAuthority,
    max_bytes: usize,
    charged_bytes: &mut usize,
) -> Result<Option<(Vec<u8>, String, std::path::PathBuf)>, AppError> {
    read_source_bytes_with_hooks(path, authority, max_bytes, charged_bytes, || {}, || {})
}

fn read_source_bytes_with_hooks(
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
        std::fs::File::open(&canonical).map_err(io_error)
    };
    before_open();
    let file = open()?;
    let opened = file.metadata().map_err(io_error)?;
    if !opened.is_file() || opened.len() > max_bytes as u64 {
        return Err(AppError::BadRequest(
            "Source is not a bounded regular file".into(),
        ));
    }
    let identity = same_file::Handle::from_file(file.try_clone().map_err(io_error)?).map_err(io_error)?;
    let mut bytes = Vec::with_capacity((opened.len() as usize).min(max_bytes));
    let mut bounded = file.take((max_bytes + 1) as u64);
    let read_result = bounded.read_to_end(&mut bytes);
    *charged_bytes = charged_bytes.saturating_add(bytes.len());
    read_result.map_err(io_error)?;
    after_read();
    let after = bounded.get_ref().metadata().map_err(io_error)?;
    let current = open().map_err(|_| AppError::Conflict("FILE_CHANGED_DURING_READ: named source can no longer be verified".into()))?;
    let current_identity = same_file::Handle::from_file(current).map_err(io_error)?;
    if bytes.len() > max_bytes
        || bytes.len() as u64 != opened.len()
        || opened.len() != after.len()
        || opened.modified().ok() != after.modified().ok()
        || identity != current_identity
        || validate_path_authority(&path.to_string_lossy(), authority)? != canonical
    {
        return Err(AppError::Conflict(
            "FILE_CHANGED_DURING_READ: discard this read and restart".into(),
        ));
    }
    let digest = format!("{:x}", Sha256::digest(&bytes));
    Ok(Some((bytes, digest, canonical)))
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::BadRequest(format!("Workspace file read failed: {error}"))
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
        path: request.path.trim().to_owned(),
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
        assert!(service.read_file_for_agent_session(&scope,"../outside.txt").await.is_err());
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
            "path":"../escape","offset":7,"limit":6
        }))).await.is_err());
    }
}
