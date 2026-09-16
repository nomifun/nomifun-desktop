//! Keep the frozen target inventory aligned with the actual bundled registrations.
//! Run this before `agent-v2-contract write` after a Wave 1/2 contract change.
use nomifun_agent_contracts::{
    CapabilityRef, TargetCapabilityContribution, TargetPackageInventoryPayload,
};
use std::{error::Error, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "check".into());
    if mode != "check" && mode != "write" {
        return Err("usage: target_inventory [check|write]".into());
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../nomifun-agent-contracts/contracts/target-packages/target-first-party-contributions.v1.json");
    let original: TargetPackageInventoryPayload = serde_json::from_slice(&std::fs::read(&path)?)?;
    let mut updated = original.clone();
    for registration in nomifun_agent_domain_wave1::registrations()?.into_iter()
        .chain(nomifun_agent_domain_wave2::registrations()?) {
        let manifest = registration.metadata.manifest.payload;
        let target = updated
            .packages
            .iter_mut()
            .find(|target| target.package.id == manifest.package_id)
            .ok_or("registered package missing from target inventory")?;
        target.package.version = manifest.package_version;
        target.capabilities = manifest
            .contributions
            .capabilities
            .iter()
            .map(|capability| TargetCapabilityContribution {
                capability: CapabilityRef {
                    id: capability.id.clone(),
                    version: capability.version.clone(),
                },
                kind: capability.kind,
            })
            .collect();
        target.role_contracts = manifest.contributions.role_contracts;
        target.role_providers = manifest.contributions.role_providers;
    }
    if mode == "write" {
        std::fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&updated)?),
        )?;
    } else if updated != original {
        return Err(
            "Wave 1/2 target inventory differs from actual registrations; run target_inventory write"
                .into(),
        );
    }
    Ok(())
}
