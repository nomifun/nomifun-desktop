use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    CapabilityId, ResourceBindingId, ResourceId, ResourceKind, TypedResourceBinding,
};

use super::*;

fn knowledge_binding(
    root: &std::path::Path,
    owner_id: &str,
    operations: &[&str],
) -> TypedResourceBinding {
    let resource_id = nomifun_common::KnowledgeBaseId::new();
    TypedResourceBinding {
        binding_id: ResourceBindingId::from("knowledge-binding"),
        resource_kind: ResourceKind::from(
            nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND,
        ),
        resource_id: ResourceId::from(resource_id.as_str()),
        owner_id: owner_id.to_owned(),
        operations: operations.iter().map(|operation| (*operation).to_owned()).collect(),
        connection_config_ref: None,
        typed_parameters: BTreeMap::from([
            (
                nomifun_agent_domain_wave1::KNOWLEDGE_ROOT_PARAMETER.to_owned(),
                root.to_string_lossy().into_owned(),
            ),
            (
                nomifun_agent_domain_wave1::KNOWLEDGE_NAME_PARAMETER.to_owned(),
                "Private Knowledge".to_owned(),
            ),
        ]),
    }
}

#[test]
fn application_registration_uses_only_target_modules() {
    let registrations = nomifun_agent_domain_wave1::registrations().unwrap();
    let ids = registrations
        .iter()
        .flat_map(|registration| {
            registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities
                .iter()
                .map(|capability| capability.id.clone())
        })
        .collect::<BTreeSet<_>>();
    assert!(ids.contains(&CapabilityId::from(
        nomifun_agent_domain_wave1::WEB_RESEARCH_MODULE_ID,
    )));
    assert!(ids.contains(&CapabilityId::from(
        nomifun_agent_domain_wave1::KNOWLEDGE_MODULE_ID,
    )));
    assert!(ids.contains(&CapabilityId::from(
        nomifun_agent_domain_wave1::PROJECT_MEMORY_MODULE_ID,
    )));
    assert!(ids.contains(&CapabilityId::from(
        nomifun_agent_domain_wave1::COMPANION_MEMORY_MODULE_ID,
    )));
    for retired in [
        "web.search",
        "citation.render",
        "nomi_local_websearch",
        "knowledge.embedding",
        "memory.project.distill",
        "memory.companion.merge",
    ] {
        assert!(!ids.contains(&CapabilityId::from(retired)));
    }
}

#[test]
fn knowledge_adapter_accepts_only_product_resource_operations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("knowledge");
    std::fs::create_dir(&root).unwrap();
    let binding = knowledge_binding(&root, "owner-a", &["read", "search", "write"]);
    let resource = agent_knowledge_resource(&binding).unwrap();
    let authority = nomifun_knowledge::AgentKnowledgeAuthority::new("owner-a", [resource])
        .unwrap();
    assert_eq!(
        authority
            .resource_ids_for(nomifun_knowledge::KnowledgeAction::Search)
            .len(),
        1
    );

    let invalid = knowledge_binding(&root, "owner-a", &["mount"]);
    assert!(agent_knowledge_resource(&invalid).is_err());
}

#[test]
fn project_memory_state_identity_is_module_action_exact() {
    let operation = Wave1MemoryOperation::ProjectWrite;
    assert_eq!(
        operation.capability_id(),
        nomifun_agent_domain_wave1::PROJECT_MEMORY_MODULE_ID
    );
    assert_eq!(
        operation.action_id(),
        nomifun_agent_domain_wave1::PROJECT_MEMORY_WRITE_ACTION_ID
    );
    assert_eq!(operation.package_id(), nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID);
    assert_eq!(operation.mount_id(), nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID);
}
