use serde::{Deserialize, Serialize};

/// Response for `GET /api/system/info`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
pub struct SystemInfoResponse {
    pub cache_dir: String,
    /// Effective persistent dataset root; distinct from project workspaces.
    pub data_dir: String,
    pub work_dir: String,
    pub log_dir: String,
    /// Opaque identity of the current on-disk dataset. Browser caches must
    /// include it in their namespace so a reset/restore cannot attach stale
    /// UI state to a different entity graph.
    pub storage_generation: String,
    /// Canonical Agent data generation, independent from preserved domain data.
    pub agent_data_generation: u32,
    /// Operating system: `darwin`, `win32`, or `linux`.
    pub platform: String,
    /// CPU architecture: `x64` or `arm64`.
    pub arch: String,
}

/// Request body for `POST /api/system/check-update`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateCheckRequest {
    #[serde(default)]
    pub include_prerelease: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

/// Prepare a relocation plan without changing the current live dataset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct PrepareStorageRelocationRequest {
    pub target_dir: String,
}

/// Start or cancel the exact persisted plan the user inspected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct StorageRelocationOperationRequest {
    pub operation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
pub struct StorageRelocationBlocker {
    pub code: String,
    pub message: String,
}

/// Durable relocation state. Preparing or scheduling a move does not change
/// `current_dir`, the dataset identity, or the canonical Agent generation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
pub struct StorageRelocationResponse {
    pub operation_id: Option<String>,
    pub phase: String,
    pub current_dir: String,
    pub target_dir: Option<String>,
    #[ts(type = "number")]
    pub total_bytes: u64,
    #[ts(type = "number")]
    pub file_count: u64,
    pub blockers: Vec<StorageRelocationBlocker>,
    pub warnings: Vec<String>,
    pub restart_required: bool,
    pub cancellable: bool,
    pub error: Option<String>,
}

/// Response for `POST /api/system/check-update`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub update_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<UpdateReleaseInfo>,
}

/// A single GitHub Release entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateReleaseInfo {
    pub tag_name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub html_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub draft: bool,
    pub assets: Vec<GitHubReleaseAsset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_asset: Option<GitHubReleaseAsset>,
}

/// A downloadable asset attached to a GitHub Release.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GitHubReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- SystemInfoResponse --

    #[test]
    fn test_system_info_response_serialization() {
        let resp = SystemInfoResponse {
            cache_dir: "/home/user/.cache/nomifun".into(),
            data_dir: "/home/user/.local/share/nomifun".into(),
            work_dir: "/home/user/.local/share/nomifun".into(),
            log_dir: "/home/user/.local/state/nomifun/logs".into(),
            storage_generation: "01900000-0000-7000-8000-000000000000".into(),
            agent_data_generation: 42,
            platform: "linux".into(),
            arch: "x64".into(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["cache_dir"], "/home/user/.cache/nomifun");
        assert_eq!(json["data_dir"], "/home/user/.local/share/nomifun");
        assert_eq!(json["work_dir"], "/home/user/.local/share/nomifun");
        assert_eq!(json["log_dir"], "/home/user/.local/state/nomifun/logs");
        assert_eq!(
            json["storage_generation"],
            "01900000-0000-7000-8000-000000000000"
        );
        assert_eq!(json["platform"], "linux");
        assert_eq!(json["agent_data_generation"], 42);
        assert_eq!(json["arch"], "x64");
        assert!(json.get("experimental_agent_ui_available").is_none());
        // Verify snake_case
        assert!(json.get("cacheDir").is_none());
    }

    #[test]
    fn test_system_info_response_roundtrip() {
        let original = SystemInfoResponse {
            cache_dir: "/tmp/cache".into(),
            data_dir: "/tmp/data".into(),
            work_dir: "/tmp/work".into(),
            log_dir: "/tmp/logs".into(),
            storage_generation: "01900000-0000-7000-8000-000000000001".into(),
            agent_data_generation: 42,
            platform: "darwin".into(),
            arch: "arm64".into(),
        };
        let serialized = serde_json::to_string(&original).unwrap();
        let parsed: SystemInfoResponse = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed, original);
    }

    // -- UpdateCheckRequest --

    #[test]
    fn test_update_check_request_default() {
        let raw = json!({});
        let req: UpdateCheckRequest = serde_json::from_value(raw).unwrap();
        assert!(!req.include_prerelease);
        assert!(req.repo.is_none());
    }

    #[test]
    fn test_update_check_request_with_options() {
        let raw = json!({
            "include_prerelease": true,
            "repo": "nomifun/nomifun-app"
        });
        let req: UpdateCheckRequest = serde_json::from_value(raw).unwrap();
        assert!(req.include_prerelease);
        assert_eq!(req.repo.as_deref(), Some("nomifun/nomifun-app"));
    }

    #[test]
    fn relocation_requests_reject_retired_or_unreviewed_fields() {
        let request: PrepareStorageRelocationRequest = serde_json::from_value(json!({
            "target_dir": "/Users/me/NomiFun"
        })).unwrap();
        assert_eq!(request.target_dir, "/Users/me/NomiFun");
        assert!(serde_json::from_value::<PrepareStorageRelocationRequest>(json!({
            "work_dir": "/Users/me/NomiFun"
        })).is_err());
        assert!(serde_json::from_value::<StorageRelocationOperationRequest>(json!({
            "operation_id": "reviewed-plan", "target_dir": "/other"
        })).is_err());
    }

    #[test]
    fn relocation_response_retains_nullable_state_and_blocker_codes() {
        let response = StorageRelocationResponse {
            operation_id: None, phase: "idle".into(), current_dir: "/data".into(),
            target_dir: None, total_bytes: 0, file_count: 0,
            blockers: vec![StorageRelocationBlocker {
                code: "TARGET_OWNED_BY_OTHER_INSTALLATION".into(), message: "Select another folder".into(),
            }],
            warnings: vec![], restart_required: false, cancellable: false, error: None,
        };
        let wire = serde_json::to_value(&response).unwrap();
        assert!(wire["operation_id"].is_null());
        assert!(wire["target_dir"].is_null());
        assert!(wire["error"].is_null());
        assert_eq!(wire["blockers"][0]["code"], "TARGET_OWNED_BY_OTHER_INSTALLATION");
        assert_eq!(serde_json::from_value::<StorageRelocationResponse>(wire).unwrap(), response);
    }

    // -- UpdateCheckResult --

    #[test]
    fn test_update_check_result_no_update() {
        let result = UpdateCheckResult {
            current_version: "1.5.0".into(),
            update_available: false,
            latest: None,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["current_version"], "1.5.0");
        assert_eq!(json["update_available"], false);
        assert!(json.get("latest").is_none());
    }

    #[test]
    fn test_update_check_result_with_update() {
        let result = UpdateCheckResult {
            current_version: "1.0.0".into(),
            update_available: true,
            latest: Some(UpdateReleaseInfo {
                tag_name: "v2.0.0".into(),
                version: "2.0.0".into(),
                name: Some("Version 2.0.0".into()),
                body: Some("Major release".into()),
                html_url: "https://github.com/nomifun/nomifun-app/releases/tag/v2.0.0".into(),
                published_at: Some("2026-04-01T00:00:00Z".into()),
                prerelease: false,
                draft: false,
                assets: vec![GitHubReleaseAsset {
                    name: "nomifun-2.0.0-darwin-arm64.dmg".into(),
                    url: "https://github.com/download/nomifun.dmg".into(),
                    size: 85_000_000,
                    content_type: Some("application/x-apple-diskimage".into()),
                }],
                recommended_asset: None,
            }),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["update_available"], true);
        assert_eq!(json["latest"]["tag_name"], "v2.0.0");
        assert_eq!(json["latest"]["version"], "2.0.0");
        assert!(!json["latest"]["html_url"].as_str().unwrap().is_empty());
        assert_eq!(json["latest"]["assets"].as_array().unwrap().len(), 1);
        assert_eq!(
            json["latest"]["assets"][0]["name"],
            "nomifun-2.0.0-darwin-arm64.dmg"
        );
        assert_eq!(json["latest"]["assets"][0]["size"], 85_000_000_u64);
    }

    // -- GitHubReleaseAsset --

    #[test]
    fn test_github_release_asset_serialization() {
        let asset = GitHubReleaseAsset {
            name: "app.zip".into(),
            url: "https://example.com/app.zip".into(),
            size: 1024,
            content_type: None,
        };
        let json = serde_json::to_value(&asset).unwrap();
        assert_eq!(json["name"], "app.zip");
        assert_eq!(json["url"], "https://example.com/app.zip");
        assert_eq!(json["size"], 1024);
        assert!(json.get("content_type").is_none());
    }

    #[test]
    fn test_github_release_asset_with_content_type() {
        let asset = GitHubReleaseAsset {
            name: "app.exe".into(),
            url: "https://example.com/app.exe".into(),
            size: 50_000_000,
            content_type: Some("application/octet-stream".into()),
        };
        let json = serde_json::to_value(&asset).unwrap();
        assert_eq!(json["content_type"], "application/octet-stream");
    }

    // -- UpdateReleaseInfo --

    #[test]
    fn test_update_release_info_minimal() {
        let info = UpdateReleaseInfo {
            tag_name: "v1.0.0".into(),
            version: "1.0.0".into(),
            name: None,
            body: None,
            html_url: "https://github.com/org/repo/releases/tag/v1.0.0".into(),
            published_at: None,
            prerelease: false,
            draft: false,
            assets: vec![],
            recommended_asset: None,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["tag_name"], "v1.0.0");
        assert!(json.get("name").is_none());
        assert!(json.get("body").is_none());
        assert!(json.get("published_at").is_none());
        assert!(json.get("recommended_asset").is_none());
    }

    #[test]
    fn test_update_release_info_prerelease() {
        let info = UpdateReleaseInfo {
            tag_name: "v2.0.0-beta.1".into(),
            version: "2.0.0-beta.1".into(),
            name: Some("Beta 1".into()),
            body: None,
            html_url: "https://github.com/org/repo/releases/tag/v2.0.0-beta.1".into(),
            published_at: None,
            prerelease: true,
            draft: false,
            assets: vec![],
            recommended_asset: None,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["prerelease"], true);
        assert_eq!(json["draft"], false);
    }
}
