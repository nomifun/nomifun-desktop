//! Product roles freeze the selected reference data without exposing the
//! installation-wide Skill directory. Ordinary Sessions retain discovery.
use super::tests::{official_launch_request, template_control_plane};
use super::*;
use crate::InMemoryControlPlaneStore;
use nomifun_agent_contracts::{
    FrozenLibrarySkill, FrozenSkillContent, FrozenSkillResource, LibrarySkillSource,
    ResolvedSkillLock, ResolvedSnapshotEnvelope,
};
use nomifun_api_types::SessionCapabilitySelectionDto;

const ROBOT_GUIDE: &str = "selected-robot-guide";
const DESKTOP_SECRET: &str = "UNSELECTED_DESKTOP_SKILL_BODY";
const SUPPORT_TEXT: &str = "SELECTED_ROBOT_SUPPORTING_REFERENCE";

fn owner() -> UserId {
    "0190f5fe-7c00-7a00-8000-000000000001".into()
}

fn inventory() -> Vec<FrozenLibrarySkill> {
    vec![
        FrozenLibrarySkill::new(
            ROBOT_GUIDE.into(),
            "Selected physical interaction reference".into(),
            LibrarySkillSource::Custom,
            "SELECTED_ROBOT_SKILL_BODY".into(),
            BTreeMap::from([(
                "references/device.md".into(),
                FrozenSkillResource::new(FrozenSkillContent::Text {
                    text: SUPPORT_TEXT.into(),
                })
                .unwrap(),
            )]),
        )
        .unwrap(),
        FrozenLibrarySkill::new(
            "desktop-code-guide".into(),
            "Unselected desktop coding reference".into(),
            LibrarySkillSource::Builtin,
            DESKTOP_SECRET.repeat(2400),
            BTreeMap::new(),
        )
        .unwrap(),
        FrozenLibrarySkill::new(
            "desktop-office-guide".into(),
            "Unselected desktop office reference".into(),
            LibrarySkillSource::BuiltinAuto,
            DESKTOP_SECRET.repeat(2400),
            BTreeMap::new(),
        )
        .unwrap(),
    ]
}

struct FullLibrary;

#[async_trait::async_trait]
impl SessionCapabilitiesResolver for FullLibrary {
    async fn resolve(
        &self,
        _: &UserId,
        selection: Option<&SessionCapabilitySelectionDto>,
    ) -> Result<ResolvedSessionCapabilities, ControlPlaneError> {
        let selected_skill_names = selection.map_or_else(
            || BTreeSet::from([ROBOT_GUIDE.into()]),
            |selection| selection.skill_names.iter().cloned().collect(),
        );
        Ok(ResolvedSessionCapabilities {
            skills: inventory(),
            selected_skill_names,
            mcp_server_ids: BTreeSet::new(),
        })
    }
}

async fn fixture(
    key: OfficialPresetKey,
) -> (
    AgentControlPlane,
    Arc<InMemoryControlPlaneStore>,
    AgentBindingValueDto,
) {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let control = template_control_plane(store.clone(), key, false)
        .with_session_capabilities_resolver(Arc::new(FullLibrary));
    let created = control
        .create_from_template(&owner(), key.as_str(), official_launch_request(false))
        .await
        .unwrap();
    let current = control
        .resolve_agent_session_binding(&owner(), &created.preset.preset_id)
        .await
        .unwrap();
    let (_, _, snapshot) = control
        .saved_binding_artifacts(&owner(), &current)
        .await
        .unwrap();
    assert_eq!(
        snapshot.content.skill_locks.len(),
        3,
        "the base binding must actually contain the global directory"
    );
    assert!(
        skill_bytes(&snapshot) > 100 * 1024,
        "the regression includes substantial unselected readable bodies"
    );
    (control, store, current)
}

fn skill_bytes(snapshot: &ResolvedSnapshotEnvelope) -> usize {
    serde_json::to_vec(&snapshot.content.skill_locks)
        .unwrap()
        .len()
}

#[tokio::test]
async fn selected_skills_only_empty_role_retires_prior_global_locks_and_bodies() {
    let (control, store, current) = fixture(OfficialPresetKey::CompanionDefault).await;
    let (_, original_revision, original_snapshot) = control
        .saved_binding_artifacts(&owner(), &current)
        .await
        .unwrap();
    let selection = SessionCapabilitySelectionDto::default();
    let restricted = control
        .resolve_agent_session_capabilities_binding_selected_skills(&owner(), &current, &selection)
        .await
        .unwrap();
    let (_, revision, snapshot) = control
        .saved_binding_artifacts(&owner(), &restricted)
        .await
        .unwrap();

    assert!(revision.payload.skill_bindings.is_empty());
    assert!(snapshot.content.skill_locks.is_empty());
    assert_eq!(
        skill_bytes(&snapshot),
        2,
        "an empty role has no Skill directory, body or supporting resource bytes"
    );
    assert_eq!(
        snapshot.content.enabled_capabilities,
        original_snapshot.content.enabled_capabilities
    );
    assert_eq!(
        revision.payload.instructions,
        original_revision.payload.instructions
    );
    assert_eq!(
        store
            .get_revision(&original_revision.reference)
            .await
            .unwrap()
            .unwrap(),
        original_revision
    );
    assert_eq!(
        store
            .get_snapshot(&original_revision.reference)
            .await
            .unwrap()
            .unwrap(),
        original_snapshot
    );
}

#[tokio::test]
async fn selected_skills_only_role_freezes_exact_choice_and_supporting_resources_with_bounded_bytes()
 {
    let (control, _, current) = fixture(OfficialPresetKey::CompanionDefault).await;
    let selection = SessionCapabilitySelectionDto {
        skill_names: vec![ROBOT_GUIDE.into()],
        mcp_server_ids: Vec::new(),
    };
    let restricted = control
        .resolve_agent_session_capabilities_binding_selected_skills(&owner(), &current, &selection)
        .await
        .unwrap();
    let (_, revision, snapshot) = control
        .saved_binding_artifacts(&owner(), &restricted)
        .await
        .unwrap();
    assert_eq!(revision.payload.skill_bindings.len(), 1);
    let [
        ResolvedSkillLock::Library {
            skill, selected, ..
        },
    ] = snapshot.content.skill_locks.as_slice()
    else {
        panic!("the role must freeze exactly its one selected library Skill");
    };
    assert!(*selected);
    assert_eq!(
        skill,
        &inventory()[0],
        "preserve the selected body's exact content, digest and support resources"
    );
    let data = serde_json::to_string(&snapshot.content.skill_locks).unwrap();
    assert!(data.contains(SUPPORT_TEXT));
    assert!(!data.contains(DESKTOP_SECRET));
    assert!(!data.contains("desktop-code-guide"));
    assert!(!data.contains("desktop-office-guide"));
    assert!(
        skill_bytes(&snapshot) < 2048,
        "selected reference data must not carry the 100 KiB global inventory"
    );
    let repeated = control
        .resolve_agent_session_capabilities_binding_selected_skills(
            &owner(),
            &restricted,
            &selection,
        )
        .await
        .unwrap();
    assert_eq!(
        repeated, restricted,
        "an exact selected-only binding is reused without another Revision"
    );
}

#[tokio::test]
async fn ordinary_session_empty_selection_keeps_global_discovery_without_default_application() {
    let (control, _, current) = fixture(OfficialPresetKey::CompanionDefault).await;
    let selection = SessionCapabilitySelectionDto::default();
    let ordinary = control
        .resolve_agent_session_capabilities_binding(&owner(), &current, Some(&selection))
        .await
        .unwrap();
    let (_, revision, snapshot) = control
        .saved_binding_artifacts(&owner(), &ordinary)
        .await
        .unwrap();
    assert_eq!(revision.payload.skill_bindings.len(), 3);
    assert_eq!(snapshot.content.skill_locks.len(), 3);
    assert!(snapshot.content.skill_locks.iter().all(|lock| matches!(
        lock,
        ResolvedSkillLock::Library {
            selected: false,
            ..
        }
    )));
    assert!(
        serde_json::to_string(&snapshot.content.skill_locks)
            .unwrap()
            .contains(DESKTOP_SECRET)
    );
    assert!(
        skill_bytes(&snapshot) > 100 * 1024,
        "ordinary Agents keep their existing full-library discovery contract"
    );
}

#[tokio::test]
async fn selected_skills_only_rejects_a_resolver_that_adds_or_loses_the_explicit_choice() {
    struct WrongSelection {
        missing: bool,
    }
    #[async_trait::async_trait]
    impl SessionCapabilitiesResolver for WrongSelection {
        async fn resolve(
            &self,
            _: &UserId,
            _: Option<&SessionCapabilitySelectionDto>,
        ) -> Result<ResolvedSessionCapabilities, ControlPlaneError> {
            Ok(ResolvedSessionCapabilities {
                skills: if self.missing {
                    Vec::new()
                } else {
                    inventory()
                },
                selected_skill_names: if self.missing {
                    BTreeSet::from([ROBOT_GUIDE.into()])
                } else {
                    BTreeSet::from([ROBOT_GUIDE.into(), "desktop-code-guide".into()])
                },
                mcp_server_ids: BTreeSet::new(),
            })
        }
    }
    for missing in [false, true] {
        let (mut control, store, current) = fixture(OfficialPresetKey::CompanionDefault).await;
        control.session_capabilities_resolver = Some(Arc::new(WrongSelection { missing }));
        let before = store.list_presets(&owner()).await.unwrap().len();
        let selection = SessionCapabilitySelectionDto {
            skill_names: vec![ROBOT_GUIDE.into()],
            mcp_server_ids: Vec::new(),
        };
        let error = control
            .resolve_agent_session_capabilities_binding_selected_skills(
                &owner(),
                &current,
                &selection,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code().as_ref(), "SESSION_CAPABILITIES_INVALID");
        assert_eq!(
            store.list_presets(&owner()).await.unwrap().len(),
            before,
            "inconsistent resolved data must not persist a candidate"
        );
    }
}

#[tokio::test]
async fn companion_materialization_keeps_saved_explicit_skill_before_global_defaults_can_replace_it()
 {
    let (control, store, current) = fixture(OfficialPresetKey::CompanionDefault).await;
    let authored_choice = SessionCapabilitySelectionDto {
        skill_names: vec!["desktop-code-guide".into()],
        mcp_server_ids: Vec::new(),
    };
    let authored = control
        .resolve_agent_session_capabilities_binding(&owner(), &current, Some(&authored_choice))
        .await
        .unwrap();
    let (_, source_revision, source_snapshot) = control
        .saved_binding_artifacts(&owner(), &authored)
        .await
        .unwrap();
    assert_eq!(
        source_snapshot.content.skill_locks.len(),
        3,
        "the saved source has one authored choice and unselected discoverable library locks"
    );
    assert!(source_snapshot.content.skill_locks.iter().any(|lock|
        matches!(lock, ResolvedSkillLock::Library { skill, selected: true, .. } if skill.name == "desktop-code-guide")));

    let prepared = control.prepare_preset_base_binding(
        &owner(), &authored.preset_revision_ref.preset_id, None,
    ).await.unwrap();
    let robot = control
        .resolve_companion_session_capabilities_binding(&owner(), &prepared)
        .await
        .unwrap();
    let (_, _, snapshot) = control
        .saved_binding_artifacts(&owner(), &robot)
        .await
        .unwrap();
    let [
        ResolvedSkillLock::Library {
            skill,
            selected: true,
            ..
        },
    ] = snapshot.content.skill_locks.as_slice()
    else {
        panic!("Companion materialization must preserve exactly the saved explicit choice");
    };
    assert_eq!(skill.name, "desktop-code-guide");
    assert!(
        !serde_json::to_string(&snapshot.content.skill_locks)
            .unwrap()
            .contains(ROBOT_GUIDE)
    );

    let ordinary = control
        .resolve_agent_session_binding_with_model(
            &owner(),
            &authored.preset_revision_ref.preset_id,
            None,
        )
        .await
        .unwrap();
    let (_, _, ordinary_snapshot) = control
        .saved_binding_artifacts(&owner(), &ordinary)
        .await
        .unwrap();
    assert_eq!(ordinary_snapshot.content.skill_locks.len(), 3);
    assert!(ordinary_snapshot.content.skill_locks.iter().any(|lock|
        matches!(lock, ResolvedSkillLock::Library { skill, selected: true, .. } if skill.name == ROBOT_GUIDE)),
        "the ordinary None path retains its installation-default contract");
    assert_eq!(
        store
            .get_revision(&source_revision.reference)
            .await
            .unwrap()
            .unwrap(),
        source_revision
    );
    assert_eq!(
        store
            .get_snapshot(&source_revision.reference)
            .await
            .unwrap()
            .unwrap(),
        source_snapshot
    );
}

#[tokio::test]
async fn prepared_product_bases_do_not_promote_global_default_skills_into_companion_authority() {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let control = template_control_plane(store, OfficialPresetKey::CompanionDefault, false)
        .with_session_capabilities_resolver(Arc::new(FullLibrary));
    let personal = control.create_from_template(
        &owner(), "companion.default", official_launch_request(false),
    ).await.unwrap();
    let untouched = control.library(&owner()).await.unwrap().official_presets
        [&nomifun_api_types::OfficialPresetKeyDto::CompanionDefault].preset_id.clone();
    let bases = [
        control.prepare_template_base_binding(&owner(), "companion.default", None).await.unwrap(),
        control.prepare_preset_base_binding(&owner(), &untouched, None).await.unwrap(),
        control.prepare_preset_base_binding(&owner(), &personal.preset.preset_id, None).await.unwrap(),
    ];
    for base in bases {
        let restricted = control.resolve_companion_session_capabilities_binding(&owner(), &base).await.unwrap();
        let (_, _, snapshot) = control.saved_binding_artifacts(&owner(), &restricted).await.unwrap();
        assert!(snapshot.content.skill_locks.is_empty(),
            "a nonempty installation default does not become the product author's selected data");
        assert!(!serde_json::to_string(&snapshot.content).unwrap().contains(ROBOT_GUIDE));
        assert!(!serde_json::to_string(&snapshot.content).unwrap().contains(DESKTOP_SECRET));
    }
    let ordinary = control.prepare_preset_session_binding(&owner(), &personal.preset.preset_id, None).await.unwrap();
    let (_, _, snapshot) = control.saved_binding_artifacts(&owner(), &ordinary).await.unwrap();
    assert_eq!(snapshot.content.skill_locks.len(), 3,
        "the same nonempty global directory remains available to ordinary Sessions");
    assert!(snapshot.content.skill_locks.iter().any(|lock|
        matches!(lock, ResolvedSkillLock::Library { skill, selected: true, .. } if skill.name == ROBOT_GUIDE)));
    assert!(skill_bytes(&snapshot) > 100 * 1024);
}
