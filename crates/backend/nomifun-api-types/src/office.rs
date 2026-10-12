use nomifun_common::{ConversationId, PreviewContentType, PreviewSnapshotId};
use serde::{Deserialize, Serialize};

/// Preview capabilities carry 256 bits of OS entropy and are encoded as
/// canonical lowercase hexadecimal for safe use as one URL path segment.
pub const PREVIEW_CAPABILITY_BYTES: usize = 32;
pub const PREVIEW_CAPABILITY_HEX_LEN: usize = PREVIEW_CAPABILITY_BYTES * 2;

pub fn is_preview_capability(value: &str) -> bool {
    value.len() == PREVIEW_CAPABILITY_HEX_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

// ---------------------------------------------------------------------------
// A. Office artifact requests and responses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum OfficePreviewDocType {
    Word,
    Excel,
    Ppt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartOfficePreviewRequest {
    /// Client-owned UUIDv7 lease, created before dispatch so cancellation can win.
    pub request_id: String,
    pub file_path: String,
    #[serde(default)]
    pub workspace: Option<String>,
    pub doc_type: OfficePreviewDocType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopOfficePreviewRequest {
    pub request_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OfficePreviewState {
    Queued,
    Rendering,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OfficePreviewError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OfficePreviewResponse {
    pub request_id: String,
    pub state: OfficePreviewState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OfficePreviewError>,
}

// ---------------------------------------------------------------------------
// C. Preview history target & snapshot info
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreviewHistoryTargetDto {
    pub content_type: PreviewContentType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PreviewSnapshotInfoDto {
    pub snapshot_id: PreviewSnapshotId,
    pub label: String,
    pub created_at: i64,
    pub size: u64,
    pub content_type: PreviewContentType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

// ---------------------------------------------------------------------------
// D. Snapshot requests & responses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveSnapshotRequest {
    pub target: PreviewHistoryTargetDto,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListSnapshotsRequest {
    pub target: PreviewHistoryTargetDto,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetSnapshotContentRequest {
    pub target: PreviewHistoryTargetDto,
    pub snapshot_id: PreviewSnapshotId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotContentResponse {
    pub snapshot: PreviewSnapshotInfoDto,
    pub content: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preview_capability_requires_canonical_256_bit_hex() {
        assert!(is_preview_capability(&"a".repeat(64)));
        assert!(!is_preview_capability(&"A".repeat(64)));
        assert!(!is_preview_capability(&"a".repeat(63)));
        assert!(!is_preview_capability("12345"));
    }

    #[test]
    fn artifact_request_requires_identity_type_and_path() {
        let value = json!({"request_id":"0190f5fe-7c00-7a00-8000-000000000001", "file_path":"/a.pptx", "doc_type":"ppt"});
        let request: StartOfficePreviewRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(request.doc_type, OfficePreviewDocType::Ppt);
        assert!(request.workspace.is_none());
        for field in ["request_id", "file_path", "doc_type"] {
            let mut incomplete = value.clone();
            incomplete.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<StartOfficePreviewRequest>(incomplete).is_err());
        }
        assert!(
            serde_json::from_value::<StopOfficePreviewRequest>(json!({"capability":"old"}))
                .is_err()
        );
    }

    #[test]
    fn artifact_error_and_ready_contract_roundtrip() {
        for state in [
            OfficePreviewState::Queued,
            OfficePreviewState::Rendering,
            OfficePreviewState::Ready,
            OfficePreviewState::Error,
        ] {
            let response = OfficePreviewResponse {
                request_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
                state,
                url: None,
                capability: None,
                error: None,
            };
            let encoded = serde_json::to_value(&response).unwrap();
            assert!(encoded.get("url").is_none());
            assert_eq!(
                serde_json::from_value::<OfficePreviewResponse>(encoded).unwrap(),
                response
            );
        }
        let error = OfficePreviewError {
            code: "ENGINE_UNAVAILABLE".into(),
            message: "Preview component unavailable".into(),
            retryable: false,
        };
        assert_eq!(
            serde_json::from_value::<OfficePreviewError>(serde_json::to_value(&error).unwrap())
                .unwrap(),
            error
        );
    }

    // -- C. PreviewHistoryTargetDto -------------------------------------------

    #[test]
    fn target_dto_full_fields() {
        let conversation_id = "0190f5fe-7c00-7a00-8abc-012345678901";
        let raw = json!({
            "content_type": "markdown",
            "file_path": "/a.md",
            "workspace": "/ws",
            "file_name": "a.md",
            "title": "My Doc",
            "language": "rust",
            "conversation_id": conversation_id
        });
        let t: PreviewHistoryTargetDto = serde_json::from_value(raw).unwrap();
        assert_eq!(t.content_type, PreviewContentType::Markdown);
        assert_eq!(t.file_path.as_deref(), Some("/a.md"));
        assert_eq!(t.workspace.as_deref(), Some("/ws"));
        assert_eq!(t.file_name.as_deref(), Some("a.md"));
        assert_eq!(t.title.as_deref(), Some("My Doc"));
        assert_eq!(t.language.as_deref(), Some("rust"));
        assert_eq!(
            t.conversation_id.as_ref().map(|id| id.as_str()),
            Some(conversation_id)
        );
    }

    #[test]
    fn target_dto_minimal() {
        let raw = json!({"content_type": "code"});
        let t: PreviewHistoryTargetDto = serde_json::from_value(raw).unwrap();
        assert_eq!(t.content_type, PreviewContentType::Code);
        assert!(t.file_path.is_none());
        assert!(t.workspace.is_none());
        assert!(t.file_name.is_none());
        assert!(t.title.is_none());
        assert!(t.language.is_none());
        assert!(t.conversation_id.is_none());
    }

    #[test]
    fn target_dto_missing_content_type() {
        let raw = json!({"file_path": "/a.md"});
        assert!(serde_json::from_value::<PreviewHistoryTargetDto>(raw).is_err());
    }

    #[test]
    fn target_dto_serialize_omits_none() {
        let t = PreviewHistoryTargetDto {
            content_type: PreviewContentType::Html,
            file_path: Some("/b.html".into()),
            workspace: None,
            file_name: None,
            title: None,
            language: None,
            conversation_id: None,
        };
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["content_type"], "html");
        assert_eq!(json["file_path"], "/b.html");
        assert!(json.get("workspace").is_none());
        assert!(json.get("file_name").is_none());
        assert!(json.get("title").is_none());
        assert!(json.get("language").is_none());
        assert!(json.get("conversation_id").is_none());
    }

    #[test]
    fn target_dto_roundtrip() {
        let conversation_id =
            ConversationId::try_from("0190f5fe-7c00-7a00-8abc-012345678904").unwrap();
        let t = PreviewHistoryTargetDto {
            content_type: PreviewContentType::Excel,
            file_path: Some("/sheet.xlsx".into()),
            workspace: Some("/ws".into()),
            file_name: Some("sheet.xlsx".into()),
            title: Some("Budget".into()),
            language: None,
            conversation_id: Some(conversation_id),
        };
        let json = serde_json::to_string(&t).unwrap();
        let parsed: PreviewHistoryTargetDto = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, t);
    }

    #[test]
    fn target_dto_all_content_types() {
        let types = [
            ("markdown", PreviewContentType::Markdown),
            ("diff", PreviewContentType::Diff),
            ("code", PreviewContentType::Code),
            ("html", PreviewContentType::Html),
            ("pdf", PreviewContentType::Pdf),
            ("ppt", PreviewContentType::Ppt),
            ("word", PreviewContentType::Word),
            ("excel", PreviewContentType::Excel),
            ("image", PreviewContentType::Image),
        ];
        for (name, expected) in types {
            let raw = json!({"content_type": name});
            let t: PreviewHistoryTargetDto = serde_json::from_value(raw).unwrap();
            assert_eq!(t.content_type, expected);
        }
        assert!(
            serde_json::from_value::<PreviewHistoryTargetDto>(json!({"content_type":"url"}))
                .is_err(),
            "web pages belong to Browser Workspace, not document preview"
        );
    }

    // -- C2. PreviewSnapshotInfoDto -------------------------------------------

    #[test]
    fn snapshot_info_serialize() {
        let info = PreviewSnapshotInfoDto {
            snapshot_id: PreviewSnapshotId::try_from("0190f5fe-7c00-7a00-8000-000000000001")
                .unwrap(),
            label: "2023-11-14 12:00".into(),
            created_at: 1700000000000,
            size: 1024,
            content_type: PreviewContentType::Markdown,
            file_name: Some("doc.md".into()),
            file_path: Some("/a/doc.md".into()),
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["snapshot_id"], "0190f5fe-7c00-7a00-8000-000000000001");
        assert!(json.get("id").is_none());
        assert_eq!(json["label"], "2023-11-14 12:00");
        assert_eq!(json["created_at"], 1700000000000_i64);
        assert_eq!(json["size"], 1024);
        assert_eq!(json["content_type"], "markdown");
        assert_eq!(json["file_name"], "doc.md");
        assert_eq!(json["file_path"], "/a/doc.md");
    }

    #[test]
    fn snapshot_info_without_file_info() {
        let info = PreviewSnapshotInfoDto {
            snapshot_id: PreviewSnapshotId::try_from("0190f5fe-7c00-7a00-8000-000000000002")
                .unwrap(),
            label: "Snapshot 1".into(),
            created_at: 1000,
            size: 256,
            content_type: PreviewContentType::Code,
            file_name: None,
            file_path: None,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert!(json.get("file_name").is_none());
        assert!(json.get("file_path").is_none());
    }

    #[test]
    fn snapshot_info_roundtrip() {
        let info = PreviewSnapshotInfoDto {
            snapshot_id: PreviewSnapshotId::try_from("0190f5fe-7c00-7a00-8000-000000000003")
                .unwrap(),
            label: "Label".into(),
            created_at: 2000,
            size: 512,
            content_type: PreviewContentType::Ppt,
            file_name: Some("slides.pptx".into()),
            file_path: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: PreviewSnapshotInfoDto = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, info);
    }

    #[test]
    fn snapshot_info_rejects_removed_generic_id() {
        let raw = json!({
            "id": "0190f5fe-7c00-7a00-8000-000000000003",
            "label": "Legacy",
            "created_at": 2000,
            "size": 512,
            "content_type": "ppt"
        });
        assert!(serde_json::from_value::<PreviewSnapshotInfoDto>(raw).is_err());
    }

    // -- D. Snapshot requests & responses -------------------------------------

    #[test]
    fn save_snapshot_request_deserialize() {
        let raw = json!({
            "target": {"content_type": "markdown", "file_path": "/a.md"},
            "content": "# Hello World"
        });
        let req: SaveSnapshotRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.target.content_type, PreviewContentType::Markdown);
        assert_eq!(req.content, "# Hello World");
    }

    #[test]
    fn save_snapshot_request_missing_content() {
        let raw = json!({"target": {"content_type": "markdown"}});
        assert!(serde_json::from_value::<SaveSnapshotRequest>(raw).is_err());
    }

    #[test]
    fn save_snapshot_request_missing_target() {
        let raw = json!({"content": "hello"});
        assert!(serde_json::from_value::<SaveSnapshotRequest>(raw).is_err());
    }

    #[test]
    fn list_snapshots_request_deserialize() {
        let raw = json!({"target": {"content_type": "html"}});
        let req: ListSnapshotsRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.target.content_type, PreviewContentType::Html);
    }

    #[test]
    fn get_snapshot_content_request_deserialize() {
        let raw = json!({
            "target": {"content_type": "code", "language": "rust"},
            "snapshot_id": "0190f5fe-7c00-7a00-8000-000000000006"
        });
        let req: GetSnapshotContentRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.target.language.as_deref(), Some("rust"));
        assert_eq!(
            req.snapshot_id.as_str(),
            "0190f5fe-7c00-7a00-8000-000000000006"
        );
    }

    #[test]
    fn get_snapshot_content_request_missing_snapshot_id() {
        let raw = json!({"target": {"content_type": "markdown"}});
        assert!(serde_json::from_value::<GetSnapshotContentRequest>(raw).is_err());
    }

    #[test]
    fn snapshot_content_response_serialize() {
        let resp = SnapshotContentResponse {
            snapshot: PreviewSnapshotInfoDto {
                snapshot_id: PreviewSnapshotId::try_from("0190f5fe-7c00-7a00-8000-000000000004")
                    .unwrap(),
                label: "L".into(),
                created_at: 1000,
                size: 5,
                content_type: PreviewContentType::Markdown,
                file_name: None,
                file_path: None,
            },
            content: "hello".into(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(
            json["snapshot"]["snapshot_id"],
            "0190f5fe-7c00-7a00-8000-000000000004"
        );
        assert!(json["snapshot"].get("id").is_none());
        assert_eq!(json["content"], "hello");
    }

    #[test]
    fn snapshot_content_response_roundtrip() {
        let resp = SnapshotContentResponse {
            snapshot: PreviewSnapshotInfoDto {
                snapshot_id: PreviewSnapshotId::try_from("0190f5fe-7c00-7a00-8000-000000000005")
                    .unwrap(),
                label: "Lab".into(),
                created_at: 2000,
                size: 10,
                content_type: PreviewContentType::Word,
                file_name: Some("doc.docx".into()),
                file_path: Some("/path/doc.docx".into()),
            },
            content: "content here".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: SnapshotContentResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, resp);
    }
}
