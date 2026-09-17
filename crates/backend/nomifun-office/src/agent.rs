//! Typed Office product actions used by the Agent Capability host.
//!
//! This module owns the document/sheet/slides payload contract and bounded
//! preview behavior. Persistence remains with the asset-library owner; model
//! providers, credentials, filesystem paths, and preview-process ports never
//! enter these Agent-facing inputs.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const DEFAULT_PREVIEW_CHARS: usize = 12_000;
pub const MAX_PREVIEW_CHARS: usize = 65_536;
pub const MAX_OFFICE_CONTENT_CHARS: usize = 1_048_576;
pub const MAX_SHEET_COLUMNS: usize = 256;
pub const MAX_SHEET_ROWS: usize = 10_000;
pub const MAX_SLIDES: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficeAssetFormat {
    Document,
    Sheet,
    Slides,
    Text,
}

impl OfficeAssetFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Sheet => "sheet",
            Self::Slides => "slides",
            Self::Text => "text",
        }
    }

    pub fn from_tags(tags: &[String]) -> Self {
        if tags.iter().any(|tag| tag == "office:sheet") {
            Self::Sheet
        } else if tags.iter().any(|tag| tag == "office:slides") {
            Self::Slides
        } else if tags.iter().any(|tag| tag == "office:document") {
            Self::Document
        } else {
            Self::Text
        }
    }

    pub fn asset_tags(self) -> Vec<String> {
        vec!["office".to_owned(), format!("office:{}", self.as_str())]
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfficePreviewRequest {
    pub asset_id: String,
    #[serde(default = "default_preview_chars")]
    pub max_chars: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfficeDocumentEditRequest {
    pub source_asset_id: Option<String>,
    pub title: String,
    pub content: String,
    pub collection: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OfficeSheetEditRequest {
    pub source_asset_id: Option<String>,
    pub title: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub collection: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfficeSlideInput {
    pub title: String,
    pub body: String,
    pub speaker_notes: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfficeSlidesEditRequest {
    pub source_asset_id: Option<String>,
    pub title: String,
    pub slides: Vec<OfficeSlideInput>,
    pub collection: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfficeRevisionDraft {
    pub source_asset_id: Option<String>,
    pub title: String,
    pub collection: Option<String>,
    pub format: OfficeAssetFormat,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OfficePreview {
    pub asset_id: String,
    pub title: String,
    pub format: OfficeAssetFormat,
    pub content: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum OfficeAgentError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("Office content exceeds the {MAX_OFFICE_CONTENT_CHARS}-character limit")]
    ContentTooLarge,
}

const fn default_preview_chars() -> usize {
    DEFAULT_PREVIEW_CHARS
}

pub fn build_document_revision(
    request: OfficeDocumentEditRequest,
) -> Result<OfficeRevisionDraft, OfficeAgentError> {
    let title = validate_title(request.title)?;
    validate_source_asset_id(request.source_asset_id.as_deref())?;
    validate_collection(request.collection.as_deref())?;
    validate_non_empty_content("content", &request.content)?;
    Ok(OfficeRevisionDraft {
        source_asset_id: request.source_asset_id,
        title,
        collection: normalize_optional(request.collection),
        format: OfficeAssetFormat::Document,
        content: request.content,
    })
}

pub fn build_sheet_revision(
    request: OfficeSheetEditRequest,
) -> Result<OfficeRevisionDraft, OfficeAgentError> {
    let title = validate_title(request.title)?;
    validate_source_asset_id(request.source_asset_id.as_deref())?;
    validate_collection(request.collection.as_deref())?;
    if request.columns.is_empty() || request.columns.len() > MAX_SHEET_COLUMNS {
        return Err(OfficeAgentError::InvalidInput(format!(
            "columns must contain 1 to {MAX_SHEET_COLUMNS} entries"
        )));
    }
    if request.rows.len() > MAX_SHEET_ROWS {
        return Err(OfficeAgentError::InvalidInput(format!(
            "rows must contain at most {MAX_SHEET_ROWS} entries"
        )));
    }
    let columns = request
        .columns
        .iter()
        .map(|column| validate_cell_text("column", column, 256))
        .collect::<Result<Vec<_>, _>>()?;
    let mut content = String::new();
    append_csv_row(&mut content, columns.iter().map(String::as_str));
    for (index, row) in request.rows.iter().enumerate() {
        if row.len() != columns.len() {
            return Err(OfficeAgentError::InvalidInput(format!(
                "row {index} has {} cells; expected {}",
                row.len(),
                columns.len()
            )));
        }
        let cells = row
            .iter()
            .map(sheet_cell_text)
            .collect::<Result<Vec<_>, _>>()?;
        append_csv_row(&mut content, cells.iter().map(String::as_str));
        if content.chars().count() > MAX_OFFICE_CONTENT_CHARS {
            return Err(OfficeAgentError::ContentTooLarge);
        }
    }
    Ok(OfficeRevisionDraft {
        source_asset_id: request.source_asset_id,
        title,
        collection: normalize_optional(request.collection),
        format: OfficeAssetFormat::Sheet,
        content,
    })
}

pub fn build_slides_revision(
    request: OfficeSlidesEditRequest,
) -> Result<OfficeRevisionDraft, OfficeAgentError> {
    let title = validate_title(request.title)?;
    validate_source_asset_id(request.source_asset_id.as_deref())?;
    validate_collection(request.collection.as_deref())?;
    if request.slides.is_empty() || request.slides.len() > MAX_SLIDES {
        return Err(OfficeAgentError::InvalidInput(format!(
            "slides must contain 1 to {MAX_SLIDES} entries"
        )));
    }
    let mut content = String::new();
    for (index, slide) in request.slides.iter().enumerate() {
        let slide_title = validate_cell_text("slide title", &slide.title, 1_000)?;
        validate_non_empty_content("slide body", &slide.body)?;
        if let Some(notes) = slide.speaker_notes.as_deref() {
            validate_non_empty_content("speaker notes", notes)?;
        }
        if index > 0 {
            content.push_str("\n\n---\n\n");
        }
        content.push_str("# ");
        content.push_str(&slide_title);
        content.push_str("\n\n");
        content.push_str(&slide.body);
        if let Some(notes) = slide
            .speaker_notes
            .as_deref()
            .filter(|notes| !notes.trim().is_empty())
        {
            content.push_str("\n\n<!-- speaker-notes\n");
            content.push_str(notes);
            content.push_str("\n-->");
        }
        if content.chars().count() > MAX_OFFICE_CONTENT_CHARS {
            return Err(OfficeAgentError::ContentTooLarge);
        }
    }
    Ok(OfficeRevisionDraft {
        source_asset_id: request.source_asset_id,
        title,
        collection: normalize_optional(request.collection),
        format: OfficeAssetFormat::Slides,
        content,
    })
}

pub fn bounded_preview(
    request: OfficePreviewRequest,
    title: String,
    format: OfficeAssetFormat,
    content: &str,
) -> Result<OfficePreview, OfficeAgentError> {
    validate_source_asset_id(Some(&request.asset_id))?;
    if request.max_chars == 0 || request.max_chars > MAX_PREVIEW_CHARS {
        return Err(OfficeAgentError::InvalidInput(format!(
            "max_chars must be between 1 and {MAX_PREVIEW_CHARS}"
        )));
    }
    let mut chars = content.chars();
    let bounded = chars.by_ref().take(request.max_chars).collect::<String>();
    let truncated = chars.next().is_some();
    Ok(OfficePreview {
        asset_id: request.asset_id,
        title: validate_title(title)?,
        format,
        content: bounded,
        truncated,
    })
}

fn validate_title(value: String) -> Result<String, OfficeAgentError> {
    let title = value.trim();
    if title.is_empty() || title.chars().count() > 1_000 {
        return Err(OfficeAgentError::InvalidInput(
            "title must contain 1 to 1000 characters".to_owned(),
        ));
    }
    Ok(title.to_owned())
}

fn validate_collection(value: Option<&str>) -> Result<(), OfficeAgentError> {
    if value.is_some_and(|value| value.trim().is_empty() || value.chars().count() > 1_000) {
        return Err(OfficeAgentError::InvalidInput(
            "collection must contain at most 1000 characters".to_owned(),
        ));
    }
    Ok(())
}

fn validate_content(value: &str) -> Result<(), OfficeAgentError> {
    if value.chars().count() > MAX_OFFICE_CONTENT_CHARS {
        Err(OfficeAgentError::ContentTooLarge)
    } else {
        Ok(())
    }
}

fn validate_non_empty_content(label: &str, value: &str) -> Result<(), OfficeAgentError> {
    if value.trim().is_empty() {
        return Err(OfficeAgentError::InvalidInput(format!(
            "{label} must not be empty"
        )));
    }
    validate_content(value)
}

fn validate_source_asset_id(value: Option<&str>) -> Result<(), OfficeAgentError> {
    if let Some(value) = value
        && !is_canonical_uuidv7(value)
    {
        return Err(OfficeAgentError::InvalidInput(
            "asset identity must be a canonical lowercase UUIDv7".to_owned(),
        ));
    }
    Ok(())
}

fn is_canonical_uuidv7(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[14] == b'7'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        && bytes.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index)
                || byte.is_ascii_digit()
                || (b'a'..=b'f').contains(byte)
        })
}

fn validate_cell_text(
    label: &str,
    value: &str,
    max_chars: usize,
) -> Result<String, OfficeAgentError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max_chars {
        return Err(OfficeAgentError::InvalidInput(format!(
            "{label} must contain 1 to {max_chars} characters"
        )));
    }
    Ok(value.to_owned())
}

fn sheet_cell_text(value: &Value) -> Result<String, OfficeAgentError> {
    match value {
        Value::Null => Ok(String::new()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::String(value) if value.chars().count() <= 65_536 => Ok(value.clone()),
        Value::String(_) => Err(OfficeAgentError::InvalidInput(
            "sheet cell text exceeds 65536 characters".to_owned(),
        )),
        _ => Err(OfficeAgentError::InvalidInput(
            "sheet cells must be strings, numbers, booleans, or null".to_owned(),
        )),
    }
}

fn append_csv_row<'a>(output: &mut String, cells: impl Iterator<Item = &'a str>) {
    for (index, cell) in cells.enumerate() {
        if index > 0 {
            output.push(',');
        }
        if cell.contains(&[',', '"', '\r', '\n'][..]) {
            output.push('"');
            output.push_str(&cell.replace('"', "\"\""));
            output.push('"');
        } else {
            output.push_str(cell);
        }
    }
    output.push('\n');
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const ASSET_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";

    #[test]
    fn sheet_action_builds_bounded_csv_without_provider_fields() {
        let draft = build_sheet_revision(OfficeSheetEditRequest {
            source_asset_id: Some(ASSET_ID.into()),
            title: "Budget".into(),
            columns: vec!["Item".into(), "Cost".into()],
            rows: vec![vec![json!("Coffee, tea"), json!(12.5)]],
            collection: Some("Reports".into()),
        })
        .unwrap();
        assert_eq!(draft.format, OfficeAssetFormat::Sheet);
        assert_eq!(draft.content, "Item,Cost\n\"Coffee, tea\",12.5\n");
        assert_eq!(
            draft.format.asset_tags(),
            vec!["office".to_owned(), "office:sheet".to_owned()]
        );
    }

    #[test]
    fn preview_is_unicode_bounded_and_reports_truncation() {
        let preview = bounded_preview(
            OfficePreviewRequest {
                asset_id: ASSET_ID.into(),
                max_chars: 3,
            },
            "Brief".into(),
            OfficeAssetFormat::Document,
            "你好世界",
        )
        .unwrap();
        assert_eq!(preview.content, "你好世");
        assert!(preview.truncated);
    }

    #[test]
    fn action_payloads_fail_closed_on_shape_and_bounds() {
        assert!(serde_json::from_value::<OfficeDocumentEditRequest>(json!({
            "title": "Brief", "content": "text", "provider_id": "forbidden"
        })).is_err());
        assert!(build_sheet_revision(OfficeSheetEditRequest {
            source_asset_id: None,
            title: "Bad".into(),
            columns: vec!["One".into(), "Two".into()],
            rows: vec![vec![json!(1)]],
            collection: None,
        }).is_err());
        assert!(bounded_preview(
            OfficePreviewRequest { asset_id: ASSET_ID.into(), max_chars: 0 },
            "Brief".into(),
            OfficeAssetFormat::Text,
            "text",
        ).is_err());
    }
}
