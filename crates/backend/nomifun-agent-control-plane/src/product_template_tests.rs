#[tokio::test]
async fn released_template_and_saved_official_agent_have_independent_authority() {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let owner: UserId = "0190f5fe-7c00-7a00-8000-000000000001".into();
    let control = general_template_control_plane(store.clone(), false);
    let authored = publish_official_fixture(&control, &owner, OfficialPresetKey::AssistantGeneral, None).await;
    let mut draft = authored.draft;
    draft.document.instructions = "This is my explicitly saved Agent".into();
    draft.document.enabled_capabilities.retain(|selection| selection.capability.id != "computer");
    let saved = control.save_revision(&owner, &authored.preset.preset_id, SaveAgentPresetRevisionRequest {
        expected_current_revision: draft.current_revision.clone(), draft, reason: None,
    }).await.unwrap();
    let reference: PresetRevisionRef = wire_cast(&saved.revision.reference).unwrap();
    let saved_revision = store.get_revision(&reference).await.unwrap().unwrap();
    let saved_snapshot = store.get_snapshot(&reference).await.unwrap().unwrap();

    control.validate_product_selection(&owner, Some("assistant.general"), None, None).await.unwrap();
    let released = control.create_from_template(&owner, "assistant.general", official_launch_request(true)).await.unwrap();
    assert_ne!(released.preset.preset_id, authored.preset.preset_id);
    assert!(released.draft.document.instructions.is_empty());
    assert!(released.draft.document.enabled_capabilities.iter().any(|selection| selection.capability.id == "computer"));
    let selected = control.prepare_preset_session_binding(&owner, &authored.preset.preset_id, None).await.unwrap();
    assert_eq!(selected.preset_revision_ref, saved.revision.reference);
    assert_eq!(control.saved_binding_artifacts(&owner, &selected).await.unwrap().1, saved_revision);
    assert_eq!(store.get_revision(&reference).await.unwrap().unwrap(), saved_revision);
    assert_eq!(store.get_snapshot(&reference).await.unwrap().unwrap(), saved_snapshot);
    assert_eq!(control.editor(&owner, &authored.preset.preset_id, None).await.unwrap().preset.current_stable_revision,
        Some(saved.revision.reference));
}

#[tokio::test]
async fn every_released_template_prepares_an_executable_current_definition() {
    let owner: UserId = "0190f5fe-7c00-7a00-8000-000000000001".into();
    for key in OfficialPresetKey::ALL {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let route = media_chat_route("released-template-model");
        let model = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route.primary.provider_id.clone(), model: route.primary.model.clone(),
        };
        let control = template_control_plane(store, key, false)
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route])));
        control.validate_product_selection(&owner, Some(key.as_str()), None, Some(&model)).await.unwrap();
        let binding = control.prepare_template_base_binding(&owner, key.as_str(), Some(&model)).await.unwrap();
        let (_, revision, snapshot) = control.saved_binding_artifacts(&owner, &binding).await.unwrap();
        assert_eq!(revision.payload.enabled_capabilities, control.templates.seed(key).unwrap().enabled_capabilities,
            "{} must use the released action grants", key.as_str());
        assert_eq!(snapshot.content.preset_revision_ref, revision.reference);
        assert_eq!(control.internal_official_template(&owner, &binding.preset_revision_ref.preset_id).await.unwrap(), Some(key));
    }
}

#[tokio::test]
async fn stale_readable_template_cache_does_not_become_the_next_definition() {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let owner: UserId = "0190f5fe-7c00-7a00-8000-000000000001".into();
    let first = general_template_control_plane(store.clone(), false);
    let cached = first.create_from_template(&owner, "assistant.general", official_launch_request(true)).await.unwrap();
    let reference: PresetRevisionRef = wire_cast(cached.preset.current_stable_revision.as_ref().unwrap()).unwrap();
    let revision = store.get_revision(&reference).await.unwrap().unwrap();
    let snapshot = store.get_snapshot(&reference).await.unwrap().unwrap();
    let current = general_template_control_plane(store.clone(), true);
    let exact = current.prepare_preset_base_binding(&owner, &cached.preset.preset_id, None).await.unwrap();
    assert_eq!(exact.preset_revision_ref, cached.preset.current_stable_revision.clone().unwrap(),
        "reading the prepared cache must not move its pointer under a changed catalog");
    assert_eq!(current.saved_binding_artifacts(&owner, &exact).await.unwrap().2, snapshot);
    let prepared = current.create_from_template(&owner, "assistant.general", official_launch_request(true)).await.unwrap();
    assert_ne!(prepared.preset.preset_id, cached.preset.preset_id);
    let repeated = current.create_from_template(&owner, "assistant.general", official_launch_request(true)).await.unwrap();
    assert_eq!(repeated.preset.current_stable_revision, prepared.preset.current_stable_revision);
    assert_eq!(store.get_revision(&reference).await.unwrap().unwrap(), revision);
    assert_eq!(store.get_snapshot(&reference).await.unwrap().unwrap(), snapshot);
}

#[tokio::test]
async fn template_model_selection_does_not_inherit_the_saved_official_default() {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let owner: UserId = "0190f5fe-7c00-7a00-8000-000000000001".into();
    let route_a = media_chat_route("saved-agent-model");
    let mut route_b = media_chat_route("session-selected-model");
    route_b.primary.model_route_id = "0190f5fe-7c00-7a00-8000-000000000012".into();
    let control = test_control_plane(store).with_default_chat_route_resolver(
        Arc::new(MediaChatRoutes(vec![route_a.clone(), route_b.clone()])));
    let selection_a = nomifun_api_types::AgentChatModelSelectionDto {
        provider_id: route_a.primary.provider_id.clone(), model: route_a.primary.model.clone(),
    };
    let authored = publish_official_fixture(&control, &owner, OfficialPresetKey::ChatMinimal, Some(&selection_a)).await;
    let request = CreateAgentPresetFromTemplateRequest {
        model: Some(nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route_b.primary.provider_id.clone(), model: route_b.primary.model.clone(),
        }), ..official_launch_request(true)
    };
    let selected = control.create_from_template(&owner, "chat.minimal", request).await.unwrap();
    assert_eq!(selected.draft.document.chat_route_records[CHAT_MODEL_TASK], serde_json::to_value(route_b).unwrap());
    assert_eq!(control.editor(&owner, &authored.preset.preset_id, None).await.unwrap().draft.document.chat_route_records[CHAT_MODEL_TASK],
        serde_json::to_value(route_a).unwrap());
}

#[tokio::test]
async fn invalid_released_definition_is_not_hidden_by_a_matching_cache() {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let owner: UserId = "0190f5fe-7c00-7a00-8000-000000000001".into();
    let mut control = general_template_control_plane(store.clone(), false);
    let cached = control.create_from_template(&owner, "assistant.general", official_launch_request(true)).await.unwrap();
    let reference: PresetRevisionRef = wire_cast(cached.preset.current_stable_revision.as_ref().unwrap()).unwrap();
    let revision = store.get_revision(&reference).await.unwrap().unwrap();
    let mut catalog = (*control.catalog.snapshot().unwrap()).clone();
    catalog.capabilities.retain(|capability| capability.manifest.id.as_ref() != "computer");
    catalog.formal_capability_entries.retain(|reference, _| reference.id.as_ref() != "computer");
    control.catalog = Arc::new(StaticCatalogProvider::new(catalog));
    let error = control.create_from_template(&owner, "assistant.general", official_launch_request(true)).await.unwrap_err();
    assert_eq!(error.code().as_ref(), "CAPABILITY_NOT_MATERIALIZED");
    assert_eq!(store.get_revision(&reference).await.unwrap().unwrap(), revision);
}

#[tokio::test]
async fn public_authoring_cannot_replace_a_template_cache_during_reuse() {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let owner: UserId = "0190f5fe-7c00-7a00-8000-000000000001".into();
    let control = test_control_plane(store.clone());
    let cached = control.create_from_template(&owner, "chat.minimal", official_launch_request(true)).await.unwrap();
    let reference: PresetRevisionRef = wire_cast(cached.preset.current_stable_revision.as_ref().unwrap()).unwrap();
    let revision = store.get_revision(&reference).await.unwrap().unwrap();
    let snapshot = store.get_snapshot(&reference).await.unwrap().unwrap();
    let mut draft = cached.draft;
    draft.document.instructions = "A caller must not author a host compilation cache".into();
    draft.document.enabled_capabilities.clear();
    let (saved, reused) = tokio::join!(
        control.save_revision(&owner, &cached.preset.preset_id, SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(), draft, reason: None,
        }),
        control.prepare_template_base_binding(&owner, "chat.minimal", None),
    );
    let rejected = saved.unwrap_err();
    assert_eq!(rejected.code().as_ref(), "PRESET_REVISION_SAVE_FAILED");
    assert_eq!(rejected.status(), axum::http::StatusCode::FORBIDDEN);
    let reused = reused.unwrap();
    assert_eq!(reused.preset_revision_ref, cached.preset.current_stable_revision.unwrap());
    assert_eq!(store.get_revision(&reference).await.unwrap().unwrap(), revision);
    assert_eq!(store.get_snapshot(&reference).await.unwrap().unwrap(), snapshot);
    assert_eq!(control.saved_binding_artifacts(&owner, &reused).await.unwrap().1, revision);
}

#[tokio::test]
async fn explicit_ssh_preparation_adds_only_exec_and_preserves_the_released_template() {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let owner: UserId = "0190f5fe-7c00-7a00-8000-000000000001".into();
    let templates = OfficialTemplateCatalog::load().unwrap();
    let ssh = templates.seed(OfficialPresetKey::AssistantGeneral).unwrap().enabled_capabilities.iter()
        .find(|selection| selection.capability.id.as_ref() == "ssh").unwrap().clone();
    let route = media_chat_route("ssh-selected-model");
    let model = nomifun_api_types::AgentChatModelSelectionDto {
        provider_id: route.primary.provider_id.clone(), model: route.primary.model.clone(),
    };
    let control = template_control_plane_with_extra_capabilities(store.clone(), OfficialPresetKey::ChatMinimal,
        false, false, &[ssh]).with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route.clone()])));
    let ordinary = control.prepare_template_base_binding(&owner, "chat.minimal", Some(&model)).await.unwrap();
    let prepared = control.prepare_ssh_session_base_binding(&owner, Some(&model)).await.unwrap();
    assert_ne!(prepared.preset_revision_ref.preset_id, ordinary.preset_revision_ref.preset_id);
    let (_, revision, snapshot) = control.saved_binding_artifacts(&owner, &prepared).await.unwrap();
    let selection = revision.payload.enabled_capabilities.iter().find(|selection| selection.capability.id.as_ref() == "ssh").unwrap();
    assert_eq!(selection.action_allowlist, BTreeSet::from(["ssh/exec".into()]));
    assert_eq!(revision.payload.enabled_capabilities.len(),
        templates.seed(OfficialPresetKey::ChatMinimal).unwrap().enabled_capabilities.len() + 1);
    assert_eq!(revision.payload.chat_route_records[CHAT_MODEL_TASK], route);
    assert_eq!(snapshot.content.preset_revision_ref, revision.reference);
    assert_eq!(control.prepare_ssh_session_base_binding(&owner, Some(&model)).await.unwrap(), prepared);
    assert_eq!(control.prepare_template_base_binding(&owner, "chat.minimal", Some(&model)).await.unwrap(), ordinary,
        "the SSH launch must not grant SSH to ordinary template Sessions or replace their cache");
    assert!(control.library(&owner).await.unwrap().user_presets.is_empty());
    let stored = store.get_preset(&revision.reference.preset_id).await.unwrap().unwrap();
    assert!(stored.session_only);
    assert_eq!(stored.preset.source, AgentPresetSource::User);
}
