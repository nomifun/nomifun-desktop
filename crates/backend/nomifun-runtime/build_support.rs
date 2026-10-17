//! The version-controlled component supply contract shared by build and runtime.
#![allow(dead_code)] // One schema is compiled in both build and runtime contexts.
use serde::{Deserialize, Serialize};

pub const MANIFEST: &str = include_str!("components.json");
pub const NOTICE_MANIFEST: &str = include_str!("notices/manifest.json");

#[derive(Debug, Deserialize)]
pub struct NoticeManifest {
    pub schema_version: u32,
    pub components: Vec<ComponentNotices>,
}

#[derive(Debug, Deserialize)]
pub struct ComponentNotices {
    pub id: String,
    pub version: String,
    pub files: Vec<NoticeFile>,
}

#[derive(Debug, Deserialize)]
pub struct NoticeFile {
    pub filename: String,
    pub source: String,
    pub sha256: String,
}

pub fn notice_manifest() -> NoticeManifest {
    let manifest: NoticeManifest =
        serde_json::from_str(NOTICE_MANIFEST).expect("valid component notice manifest");
    assert_eq!(
        manifest.schema_version, 1,
        "unsupported notice manifest schema"
    );
    manifest
}

pub fn validate_notices(spec: &ComponentSpec, notices: &ComponentNotices) -> Result<(), String> {
    if notices.id != spec.id || notices.version != spec.version || notices.files.is_empty() {
        return Err(format!(
            "{} notices do not match the approved component version",
            spec.id
        ));
    }
    let prefix = match spec.id.as_str() {
        "bun" => format!(
            "https://raw.githubusercontent.com/oven-sh/bun/bun-v{}/",
            spec.version
        ),
        "node" => format!("https://nodejs.org/dist/v{}/", spec.version),
        "officecli" => format!(
            "https://raw.githubusercontent.com/iOfficeAI/OfficeCLI/v{}/",
            spec.version
        ),
        _ => return Err("unrecognized notice owner".into()),
    };
    for file in &notices.files {
        if file.filename.is_empty()
            || file.filename.contains(['/', '\\'])
            || file.filename == "."
            || file.filename == ".."
            || !file.source.starts_with(&prefix)
            || file.sha256.len() != 64
            || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!("invalid {} notice provenance", spec.id));
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct ComponentManifest {
    pub schema_version: u32,
    pub components: Vec<ComponentSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentSpec {
    pub id: String,
    pub version: String,
    pub source: String,
    pub package: String,
    pub development_version_requirement: String,
    pub hash_source: String,
    pub assets: Vec<ComponentAsset>,
    pub delivery: ComponentDelivery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentDelivery {
    Embedded,
    Download,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentAsset {
    pub target: String,
    pub variant: String,
    pub url: String,
    pub sha256: String,
    pub format: String,
    pub executable: String,
}

impl ComponentSpec {
    pub fn asset(&self, target: &str, variant: &str) -> Option<&ComponentAsset> {
        self.assets
            .iter()
            .find(|asset| asset.target == target && asset.variant == variant)
            .or_else(|| {
                self.assets
                    .iter()
                    .find(|asset| asset.target == target && asset.variant == "default")
            })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.delivery
            != if self.id == "officecli" {
                ComponentDelivery::Embedded
            } else {
                ComponentDelivery::Download
            }
        {
            return Err(format!("unapproved delivery policy for {}", self.id));
        }
        semver::Version::parse(&self.version)
            .map_err(|_| format!("invalid approved version for {}", self.id))?;
        semver::VersionReq::parse(&self.development_version_requirement)
            .map_err(|_| format!("invalid development version range for {}", self.id))?;
        let prefix = match self.id.as_str() {
            "bun" if self.source == "https://github.com/oven-sh/bun" && self.package == "bun" => {
                format!("{}/releases/download/bun-v{}/", self.source, self.version)
            }
            "node" if self.source == "https://nodejs.org" && self.package == "node" => {
                format!("{}/dist/v{}/", self.source, self.version)
            }
            "officecli"
                if self.source == "https://github.com/iOfficeAI/OfficeCLI"
                    && self.package == "@officecli/officecli" =>
            {
                format!("{}/releases/download/v{}/", self.source, self.version)
            }
            _ => {
                return Err(format!(
                    "unsupported component source or package: {}",
                    self.id
                ));
            }
        };
        let checksum_source = match self.id.as_str() {
            "bun" => format!("{prefix}SHASUMS256.txt"),
            "node" => format!("{prefix}SHASUMS256.txt"),
            "officecli" => format!("{}/releases/expanded_assets/v{}", self.source, self.version),
            _ => unreachable!(),
        };
        if self.hash_source != checksum_source {
            return Err(format!("untrusted checksum source: {}", self.id));
        }
        let mut keys = std::collections::BTreeSet::new();
        for asset in &self.assets {
            let expected_executable = format!(
                "{}{}",
                self.id,
                if asset.target.contains("windows") {
                    ".exe"
                } else {
                    ""
                }
            );
            if !asset.url.starts_with(&prefix)
                || asset.url.contains("..")
                || asset.sha256.len() != 64
                || !asset.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                || !matches!(asset.format.as_str(), "raw" | "zip" | "tar.gz")
                || (self.id == "node" && asset.format == "raw")
                || asset.executable != expected_executable
                || !matches!(asset.variant.as_str(), "default" | "baseline")
                || !keys.insert((&asset.target, &asset.variant))
            {
                return Err(format!(
                    "invalid approved asset for {}: {}",
                    self.id, asset.target
                ));
            }
        }
        Ok(())
    }
}

pub fn manifest() -> ComponentManifest {
    let manifest: ComponentManifest =
        serde_json::from_str(MANIFEST).expect("valid component manifest");
    assert_eq!(
        manifest.schema_version, 1,
        "unsupported component manifest schema"
    );
    let ids: std::collections::BTreeSet<_> = manifest
        .components
        .iter()
        .map(|spec| spec.id.as_str())
        .collect();
    assert_eq!(
        manifest.components.len(),
        3,
        "all product components must be declared exactly once"
    );
    assert_eq!(
        ids,
        std::collections::BTreeSet::from(["bun", "node", "officecli"]),
        "each product component must have one supply declaration"
    );
    for spec in &manifest.components {
        spec.validate().expect("trusted component manifest");
    }
    manifest
}

pub fn write_meta_stub(path: &std::path::Path, id: &str) {
    std::fs::write(path, format!(
        "// @generated\npub const BLOB: &[u8] = b\"\";\npub const SHA256: &str = \"\";\npub const VERSION: &str = \"\";\npub const EXECUTABLE: &str = \"{id}\";\npub const HAS_EMBEDDED: bool = false;\n"
    )).expect("write component metadata");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_covers_every_supported_desktop_target_with_trusted_assets() {
        for spec in manifest().components {
            for target in [
                "x86_64-apple-darwin",
                "aarch64-apple-darwin",
                "x86_64-unknown-linux-gnu",
                "aarch64-unknown-linux-gnu",
                "x86_64-pc-windows-msvc",
                "aarch64-pc-windows-msvc",
            ] {
                assert!(
                    spec.asset(target, "default").is_some(),
                    "{} missing {target}",
                    spec.id
                );
            }
        }
    }

    #[test]
    fn notices_match_approved_versions_and_preserve_upstream_bytes() {
        use sha2::{Digest, Sha256};
        let notices = notice_manifest();
        for spec in manifest().components {
            let component = notices
                .components
                .iter()
                .find(|notice| notice.id == spec.id)
                .unwrap();
            validate_notices(&spec, component).unwrap();
            for file in &component.files {
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("notices")
                    .join(&file.filename);
                assert_eq!(
                    hex::encode(Sha256::digest(std::fs::read(path).unwrap())),
                    file.sha256
                );
            }
        }
        let spec = manifest().components.remove(0);
        let mut component = notice_manifest().components.remove(0);
        component.version = "0.0.0".into();
        assert!(validate_notices(&spec, &component).is_err());
    }

    #[test]
    fn wrong_same_name_source_and_changed_archive_are_rejected() {
        let mut spec = manifest().components.remove(2);
        spec.package = "officecli".into();
        assert!(spec.validate().is_err());
        spec.package = "@officecli/officecli".into();
        spec.assets[0].url = "https://example.org/officecli".into();
        assert!(spec.validate().is_err());
    }

    #[test]
    fn stub_metadata_never_references_a_stale_blob() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("officecli_meta.rs");
        write_meta_stub(&path, "bun");
        let body = std::fs::read_to_string(path).unwrap();
        assert!(!body.contains("include_bytes!"));
        assert!(body.contains("HAS_EMBEDDED: bool = false"));
    }
}
