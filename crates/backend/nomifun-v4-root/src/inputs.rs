use nomifun_agent_contracts::{
    ArtifactEnvelope, CanonicalV4SchemaManifest, DigestHex, OfficialPresetSeedManifestPayload,
    TargetPackageInventoryPayload, digest_bytes, digest_payload,
    official_preset_seed_manifest_payload, FRESH_V4_BASELINE_SQL,
};

use crate::FreshV4RootError;

const CANONICAL_SCHEMA_MANIFEST_JSON: &str = include_str!(
    "../../nomifun-agent-contracts/contracts/generated/canonical-v4-schema-manifest.envelope.json"
);
const TARGET_FIRST_PARTY_INVENTORY_JSON: &str = include_str!(
    "../../nomifun-agent-contracts/contracts/generated/target-first-party-contributions.envelope.json"
);

pub(crate) struct FrozenRootInputs {
    pub canonical_manifest: CanonicalV4SchemaManifest,
    pub target_inventory: TargetPackageInventoryPayload,
    pub seed_manifest: OfficialPresetSeedManifestPayload,
    pub seed_manifest_digest: DigestHex,
}

impl FrozenRootInputs {
    pub fn load() -> Result<Self, FreshV4RootError> {
        let canonical_manifest =
            serde_json::from_str::<CanonicalV4SchemaManifest>(CANONICAL_SCHEMA_MANIFEST_JSON)?;
        if !canonical_manifest.verify()? {
            return Err(FreshV4RootError::Contract(
                "canonical v4 schema manifest envelope failed digest verification".into(),
            ));
        }

        let target_inventory =
            serde_json::from_str::<ArtifactEnvelope<TargetPackageInventoryPayload>>(
                TARGET_FIRST_PARTY_INVENTORY_JSON,
            )?;
        if !target_inventory.verify()? {
            return Err(FreshV4RootError::Contract(
                "target first-party inventory envelope failed digest verification".into(),
            ));
        }

        let seed_manifest = official_preset_seed_manifest_payload();
        seed_manifest
            .validate()
            .map_err(|error| FreshV4RootError::Contract(error.message))?;
        seed_manifest
            .validate_against_target_inventory(&target_inventory.payload)
            .map_err(|error| FreshV4RootError::Contract(error.message))?;
        let seed_manifest_digest = digest_payload(&seed_manifest)?;

        let database_schema_digest = digest_bytes(FRESH_V4_BASELINE_SQL.as_bytes());
        if database_schema_digest != canonical_manifest.payload.database_schema_digest {
            return Err(FreshV4RootError::Contract(
                "fresh-v4 baseline digest does not match the canonical schema manifest".into(),
            ));
        }
        if seed_manifest_digest
            != canonical_manifest
                .payload
                .official_preset_seed_manifest_digest
        {
            return Err(FreshV4RootError::Contract(
                "official preset seed digest does not match the canonical schema manifest".into(),
            ));
        }
        if target_inventory.payload_digest
            != seed_manifest.target_first_party_contribution_digest
        {
            return Err(FreshV4RootError::Contract(
                "target first-party inventory digest does not match the frozen seed manifest"
                    .into(),
            ));
        }

        Ok(Self {
            canonical_manifest,
            target_inventory: target_inventory.payload,
            seed_manifest,
            seed_manifest_digest,
        })
    }
}

pub fn application_build_digest(build_identity: &str) -> Result<DigestHex, FreshV4RootError> {
    let build_identity = build_identity.trim();
    if build_identity.is_empty() {
        return Err(FreshV4RootError::Contract(
            "application build identity must not be empty".into(),
        ));
    }
    Ok(digest_bytes(
        format!("nomifun-application-build-v1:{build_identity}").as_bytes(),
    ))
}

pub fn canonical_schema_manifest_digest() -> Result<DigestHex, FreshV4RootError> {
    Ok(FrozenRootInputs::load()?
        .canonical_manifest
        .payload_digest)
}

pub fn official_seed_manifest_digest() -> Result<DigestHex, FreshV4RootError> {
    Ok(FrozenRootInputs::load()?.seed_manifest_digest)
}
