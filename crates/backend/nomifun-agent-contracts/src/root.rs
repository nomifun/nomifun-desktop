//! Canonical Fresh-v4 data-root and initialization marker contracts.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    DigestHex, OperationId, FRESH_V4_DATA_GENERATION, FRESH_V4_MIGRATION_HEAD,
    FRESH_V4_PROJECTION_SCHEMA_VERSION,
};

pub const FRESH_V4_PARENT_MARKER_FILE: &str = ".nomifun-v4-operation.json";
pub const FRESH_V4_READY_MARKER_FILE: &str = ".nomifun-v4-ready.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FreshV4OperationKind {
    Fresh,
    Cutover,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FreshV4ParentOperationMarker {
    pub operation_id: OperationId,
    pub operation_kind: FreshV4OperationKind,
    pub canonical_normalized_relative_basename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cutover_archive_sibling_relative_basename: Option<String>,
    pub target_data_generation: u32,
    pub canonical_schema_manifest_digest: DigestHex,
}

impl FreshV4ParentOperationMarker {
    pub fn validate(&self) -> Result<(), String> {
        if self.operation_id.as_ref().trim().is_empty() {
            return Err("operation_id must not be empty".into());
        }
        validate_single_relative_basename(&self.canonical_normalized_relative_basename)?;
        match self.operation_kind {
            FreshV4OperationKind::Fresh => {
                if self.cutover_archive_sibling_relative_basename.is_some() {
                    return Err("fresh operation must not carry an archive basename".into());
                }
            }
            FreshV4OperationKind::Cutover => {
                let archive = self
                    .cutover_archive_sibling_relative_basename
                    .as_deref()
                    .ok_or("cutover operation requires an archive basename")?;
                validate_single_relative_basename(archive)?;
                let prefix = format!(
                    "{}.pre-v4-archive-",
                    self.canonical_normalized_relative_basename
                );
                if !archive.starts_with(&prefix)
                    || archive.len() <= prefix.len()
                    || !archive[prefix.len()..]
                        .bytes()
                        .all(|byte| byte.is_ascii_digit())
                {
                    return Err(
                        "cutover archive basename must be <canonical>.pre-v4-archive-<UTC timestamp>"
                            .into(),
                    );
                }
            }
        }
        if self.target_data_generation != FRESH_V4_DATA_GENERATION {
            return Err("target_data_generation must be Fresh-v4 generation 4".into());
        }
        validate_digest(&self.canonical_schema_manifest_digest)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FreshV4SchemaMetadata {
    pub singleton_key: String,
    pub data_generation: u32,
    pub root_instance_id: String,
    pub migration_head: u32,
    pub seed_manifest_digest: DigestHex,
    pub canonical_schema_manifest_digest: DigestHex,
    pub projection_schema_version: u32,
}

impl FreshV4SchemaMetadata {
    pub fn validate(&self) -> Result<(), String> {
        if self.singleton_key != "canonical" {
            return Err("schema metadata singleton_key must be canonical".into());
        }
        if self.data_generation != FRESH_V4_DATA_GENERATION {
            return Err("schema metadata data_generation must be 4".into());
        }
        if self.root_instance_id.trim().is_empty()
            || self.root_instance_id.contains(['/', '\\'])
        {
            return Err("schema metadata root_instance_id must be a non-path value".into());
        }
        if self.migration_head < FRESH_V4_MIGRATION_HEAD {
            return Err("schema metadata migration_head is below the baseline".into());
        }
        if self.projection_schema_version < FRESH_V4_PROJECTION_SCHEMA_VERSION {
            return Err("schema metadata projection_schema_version is below the baseline".into());
        }
        validate_digest(&self.seed_manifest_digest)?;
        validate_digest(&self.canonical_schema_manifest_digest)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FreshV4ReadyMarker {
    pub data_generation: u32,
    pub root_instance_id: String,
    pub migration_head: u32,
    pub seed_manifest_digest: DigestHex,
    pub canonical_schema_manifest_digest: DigestHex,
    pub projection_schema_version: u32,
    pub application_build_digest: DigestHex,
}

impl FreshV4ReadyMarker {
    pub fn validate(&self) -> Result<(), String> {
        FreshV4SchemaMetadata {
            singleton_key: "canonical".into(),
            data_generation: self.data_generation,
            root_instance_id: self.root_instance_id.clone(),
            migration_head: self.migration_head,
            seed_manifest_digest: self.seed_manifest_digest.clone(),
            canonical_schema_manifest_digest: self.canonical_schema_manifest_digest.clone(),
            projection_schema_version: self.projection_schema_version,
        }
        .validate()?;
        validate_digest(&self.application_build_digest)
    }

    pub fn matches_schema_metadata(&self, metadata: &FreshV4SchemaMetadata) -> bool {
        self.data_generation == metadata.data_generation
            && self.root_instance_id == metadata.root_instance_id
            && self.migration_head == metadata.migration_head
            && self.seed_manifest_digest == metadata.seed_manifest_digest
            && self.canonical_schema_manifest_digest
                == metadata.canonical_schema_manifest_digest
            && self.projection_schema_version == metadata.projection_schema_version
    }
}

fn validate_single_relative_basename(value: &str) -> Result<(), String> {
    if value.trim().is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\'])
        || value.contains(':')
    {
        return Err(format!(
            "value must be one normalized relative basename: {value:?}"
        ));
    }
    Ok(())
}

fn validate_digest(value: &DigestHex) -> Result<(), String> {
    if value.as_ref().len() == 64
        && value
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err("digest must be a 64-character hexadecimal value".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn marker(kind: FreshV4OperationKind) -> FreshV4ParentOperationMarker {
        FreshV4ParentOperationMarker {
            operation_id: "operation-1".into(),
            operation_kind: kind,
            canonical_normalized_relative_basename: "NomiFun-v4".into(),
            cutover_archive_sibling_relative_basename: match kind {
                FreshV4OperationKind::Fresh => None,
                FreshV4OperationKind::Cutover => {
                    Some("NomiFun-v4.pre-v4-archive-20260829123456".into())
                }
            },
            target_data_generation: 4,
            canonical_schema_manifest_digest: DIGEST.into(),
        }
    }

    #[test]
    fn fresh_marker_has_no_archive() {
        assert!(marker(FreshV4OperationKind::Fresh).validate().is_ok());
    }

    #[test]
    fn cutover_marker_requires_timestamped_archive_sibling() {
        assert!(marker(FreshV4OperationKind::Cutover).validate().is_ok());
        let mut invalid = marker(FreshV4OperationKind::Cutover);
        invalid.cutover_archive_sibling_relative_basename =
            Some("NomiFun-v4.pre-v4-archive-not-a-timestamp".into());
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn marker_rejects_path_traversal_and_cross_volume_values() {
        for value in ["", ".", "..", "nested/root", r"nested\root", "C:root"] {
            let mut invalid = marker(FreshV4OperationKind::Fresh);
            invalid.canonical_normalized_relative_basename = value.into();
            assert!(invalid.validate().is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn schema_metadata_validates_exact_generation_and_digests() {
        let metadata = FreshV4SchemaMetadata {
            singleton_key: "canonical".into(),
            data_generation: 4,
            root_instance_id: "root-1".into(),
            migration_head: 1,
            seed_manifest_digest: DIGEST.into(),
            canonical_schema_manifest_digest: DIGEST.into(),
            projection_schema_version: 1,
        };
        assert!(metadata.validate().is_ok());
    }

    #[test]
    fn ready_marker_matches_schema_metadata_and_has_a_build_digest() {
        let metadata = FreshV4SchemaMetadata {
            singleton_key: "canonical".into(),
            data_generation: 4,
            root_instance_id: "root-1".into(),
            migration_head: 1,
            seed_manifest_digest: DIGEST.into(),
            canonical_schema_manifest_digest: DIGEST.into(),
            projection_schema_version: 1,
        };
        let ready = FreshV4ReadyMarker {
            data_generation: metadata.data_generation,
            root_instance_id: metadata.root_instance_id.clone(),
            migration_head: metadata.migration_head,
            seed_manifest_digest: metadata.seed_manifest_digest.clone(),
            canonical_schema_manifest_digest: metadata
                .canonical_schema_manifest_digest
                .clone(),
            projection_schema_version: metadata.projection_schema_version,
            application_build_digest: DIGEST.into(),
        };
        assert!(ready.validate().is_ok());
        assert!(ready.matches_schema_metadata(&metadata));
    }
}
