//! Machine-verifiable retirement routing for the 136 pre-UARC capability IDs.
use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{CapabilityId, DigestHex, TargetPackageInventoryPayload, VersionString, digest_payload};

const RETIREMENT_JSON: &str =
    include_str!("../contracts/closure/capability-retirement.v1.json");
const SOURCE_INVENTORY_JSON: &str =
    include_str!("../contracts/target-packages/target-first-party-contributions.v1.json");

/// Extension-era authoring roots retired by UARC-022. These identities are
/// globally reserved: a different package or Plugin must not republish them
/// after the built-in packages disappear.
pub const RETIRED_EXTENSION_AUTHORING_CAPABILITY_IDS: [&str; 10] = [
    "skill.catalog",
    "skill.describe",
    "skill.invoke",
    "skill.hooks",
    "connector.data.read",
    "connector.data.write",
    "mcp.connect",
    "mcp.oauth",
    "mcp.resource",
    "mcp.tool_proxy",
];

pub fn is_retired_extension_authoring_capability(value: &str) -> bool {
    RETIRED_EXTENSION_AUTHORING_CAPABILITY_IDS.contains(&value)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRetirementManifest {
    pub schema_version: VersionString,
    pub source_inventory_digest: DigestHex,
    pub routes: Vec<CapabilityRetirementRoute>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRetirementRoute {
    pub source_package: String,
    pub source_capability_ids: Vec<CapabilityId>,
    pub disposition: String,
    pub target: String,
    pub owner_tasks: Vec<String>,
}

pub fn capability_retirement_manifest() -> CapabilityRetirementManifest {
    serde_json::from_str(RETIREMENT_JSON).expect("embedded capability retirement manifest")
}

pub fn legacy_first_party_inventory() -> TargetPackageInventoryPayload {
    serde_json::from_str(SOURCE_INVENTORY_JSON).expect("embedded first-party inventory")
}

impl CapabilityRetirementManifest {
    pub fn validate(
        &self,
        inventory: &TargetPackageInventoryPayload,
    ) -> Result<(), String> {
        if self.schema_version.as_ref() != "1.0.0" || self.routes.is_empty() {
            return Err("capability retirement manifest has an unsupported or empty shape".into());
        }
        let digest = digest_payload(inventory).map_err(|error| error.to_string())?;
        if digest != self.source_inventory_digest {
            return Err("capability retirement manifest targets another source inventory".into());
        }
        let packages = inventory
            .packages
            .iter()
            .map(|package| {
                (
                    package.package.id.as_ref().to_owned(),
                    package
                        .capabilities
                        .iter()
                        .map(|capability| capability.capability.id.clone())
                        .collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let expected = packages
            .values()
            .flat_map(|ids| ids.iter().cloned())
            .collect::<BTreeSet<_>>();
        let allowed_dispositions = BTreeSet::from([
            "binding_context",
            "binding_runtime_port",
            "deleted",
            "derived_context",
            "derived_role_actions",
            "dynamic_module_actions",
            "module_actions",
            "platform_service",
            "resource_host_port",
            "resource_provider",
            "runtime_internal",
            "session_input",
        ]);
        let mut routed = BTreeSet::new();
        for route in &self.routes {
            let Some(package_ids) = packages.get(&route.source_package) else {
                return Err(format!(
                    "retirement route names unknown package {}",
                    route.source_package
                ));
            };
            if route.source_capability_ids.is_empty()
                || route.target.trim().is_empty()
                || !allowed_dispositions.contains(route.disposition.as_str())
                || route.owner_tasks.is_empty()
                || route.owner_tasks.iter().any(|task| !task.starts_with("UARC-"))
                || !route.owner_tasks.iter().any(|task| task == "UARC-053")
            {
                return Err(format!(
                    "retirement route for {} has incomplete disposition/ownership",
                    route.source_package
                ));
            }
            for id in &route.source_capability_ids {
                if !package_ids.contains(id) {
                    return Err(format!(
                        "retirement route assigns {} to the wrong source package",
                        id.as_ref()
                    ));
                }
                if !routed.insert(id.clone()) {
                    return Err(format!(
                        "retirement route assigns {} more than once",
                        id.as_ref()
                    ));
                }
            }
        }
        if routed != expected || routed.len() != 136 {
            let missing = expected.difference(&routed).map(AsRef::as_ref).collect::<Vec<_>>();
            let extra = routed.difference(&expected).map(AsRef::as_ref).collect::<Vec<_>>();
            return Err(format!(
                "retirement inventory must cover 136/136 IDs exactly; missing={missing:?}, extra={extra:?}"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retirement_manifest_covers_every_legacy_id_once() {
        let inventory = legacy_first_party_inventory();
        let manifest = capability_retirement_manifest();
        manifest.validate(&inventory).unwrap();
        assert_eq!(
            manifest
                .routes
                .iter()
                .map(|route| route.source_capability_ids.len())
                .sum::<usize>(),
            136
        );
    }

    #[test]
    fn duplicate_or_missing_route_fails_closed() {
        let inventory = legacy_first_party_inventory();
        let mut manifest = capability_retirement_manifest();
        let duplicate = manifest.routes[0].source_capability_ids[0].clone();
        manifest.routes[1]
            .source_capability_ids
            .push(duplicate);
        assert!(manifest.validate(&inventory).is_err());
        let mut manifest = capability_retirement_manifest();
        manifest.routes[0].source_capability_ids.clear();
        assert!(manifest.validate(&inventory).is_err());
    }

    #[test]
    fn retired_extension_authoring_ids_are_exact_and_unique() {
        let ids = RETIRED_EXTENSION_AUTHORING_CAPABILITY_IDS
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), 10);
        for expected in [
            "skill.catalog",
            "skill.describe",
            "skill.invoke",
            "skill.hooks",
            "connector.data.read",
            "connector.data.write",
            "mcp.connect",
            "mcp.oauth",
            "mcp.resource",
            "mcp.tool_proxy",
        ] {
            assert!(is_retired_extension_authoring_capability(expected));
        }
        assert!(!is_retired_extension_authoring_capability("mcp.server"));
        assert!(!is_retired_extension_authoring_capability(
            "mcp.0199a000-0000-7000-8000-000000000001.search"
        ));
    }
}
