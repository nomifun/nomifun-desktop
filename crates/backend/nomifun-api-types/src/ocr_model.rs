use serde::{Deserialize, Serialize};

use crate::{LocalModelErrorKind, LocalModelInstallPhase};

/// Immutable metadata for one curated OCR bundle.
///
/// Download URLs, checksums, revisions and local paths intentionally remain
/// server-side so callers cannot turn the managed downloader into an
/// arbitrary fetch or file-system primitive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelCatalogEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub format: String,
    pub download_size_bytes: u64,
    pub required_memory_bytes: u64,
    pub license: String,
    pub source: String,
    pub components: Vec<OcrModelComponent>,
    pub recommended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrModelComponent {
    Detector,
    DetectorConfig,
    Recognizer,
    RecognizerConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelTransferProgress {
    pub component: OcrModelComponent,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub overall_downloaded_bytes: u64,
    pub overall_total_bytes: u64,
    pub bytes_per_second: u64,
}

/// Mutable installation state for an OCR bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelState {
    pub model_id: String,
    pub install_phase: LocalModelInstallPhase,
    pub progress: Option<OcrModelTransferProgress>,
    pub installed_bytes: u64,
    pub error_kind: Option<LocalModelErrorKind>,
    /// Sanitized user-safe detail. It must not contain paths, URLs or response
    /// bodies from the download origin.
    pub message: Option<String>,
}

/// Complete OCR artifact status returned by status and mutation endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelServiceStatus {
    pub protocol_version: String,
    pub artifacts_ready: bool,
    /// Kept explicit while inference wiring is developed. Installing weights
    /// alone must never be represented to clients as a usable OCR runtime.
    pub inference_ready: bool,
    pub models: Vec<OcrModelState>,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_status_has_stable_safe_wire_contract() {
        let status = OcrModelServiceStatus {
            protocol_version: "1".into(),
            artifacts_ready: false,
            inference_ready: false,
            models: vec![OcrModelState {
                model_id: "pp-ocrv6-small-onnx".into(),
                install_phase: LocalModelInstallPhase::Downloading,
                progress: Some(OcrModelTransferProgress {
                    component: OcrModelComponent::Detector,
                    downloaded_bytes: 5,
                    total_bytes: 10,
                    overall_downloaded_bytes: 5,
                    overall_total_bytes: 30,
                    bytes_per_second: 2,
                }),
                installed_bytes: 5,
                error_kind: None,
                message: None,
            }],
            last_error: None,
        };

        let json = serde_json::to_value(status).unwrap();
        assert_eq!(json["protocolVersion"], "1");
        assert_eq!(json["artifactsReady"], false);
        assert_eq!(json["inferenceReady"], false);
        assert_eq!(json["models"][0]["progress"]["component"], "detector");
        assert!(json.get("downloadUrl").is_none());
        assert!(json.get("localPath").is_none());
    }
}
