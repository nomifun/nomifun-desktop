fn authoring_process_actions(
    document: &nomifun_api_types::AgentPresetDocumentDto,
) -> &BTreeSet<String> {
    &document.enabled_capabilities.iter()
        .find(|selection| selection.capability.id == "workspace.process")
        .expect("the fixture grants the process module")
        .action_allowlist
}

async fn authoring_store_presets(
    store: &InMemoryControlPlaneStore,
    owner: &UserId,
) -> Vec<serde_json::Value> {
    store.list_presets(owner).await.unwrap().into_iter()
        .map(|stored| json!({"session_only": stored.session_only, "preset": stored.preset}))
        .collect()
}

#[tokio::test]
async fn authoring_contract_evolution_repairs_only_the_current_draft_without_writes() {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let (control, owner, revision, snapshot) = retired_official_launch_fixture(store.clone()).await;
    let preset_id = revision.reference.preset_id.as_ref();
    let before = authoring_store_presets(&store, &owner).await;
    let retired = BTreeSet::from(["workspace.process/exec".to_owned()]);
    let current = BTreeSet::from(["workspace.process/start".to_owned()]);

    let editor = control.editor(&owner, preset_id, None).await.unwrap();
    assert_eq!(authoring_process_actions(&editor.draft.document), &current);
    assert_eq!(document_payload(&editor.draft.document).unwrap(),
        nomifun_agent_contracts::bounded_process_launch_payload(&revision.payload),
        "the proved launch replacement preserves every model, policy and other authored selection");
    assert_eq!(authoring_process_actions(&editor.revision.as_ref().unwrap().document), &retired,
        "the saved revision remains the exact authored historical artifact");
    assert_eq!(editor.draft.current_revision, Some(wire_cast(&revision.reference).unwrap()));
    assert_eq!(editor.draft.document.instructions, revision.payload.instructions);
    assert_eq!(editor.draft.display_name, "Saved official Agent");
    assert_eq!(editor.draft.description.as_deref(), Some("Preserve my presentation"));
    let historical = control.editor(&owner, preset_id, Some(revision.reference.revision)).await.unwrap();
    assert_eq!(authoring_process_actions(&historical.draft.document), &retired);
    assert_eq!(authoring_process_actions(&historical.revision.unwrap().document), &retired);

    let selected_model = nomifun_api_types::AgentChatModelSelectionDto {
        provider_id: media_chat_route("model-b").primary.provider_id,
        model: "model-b".into(),
    };
    for (template, preset) in [(Some("chat.minimal"), None), (None, Some(preset_id))] {
        for model in [None, Some(&selected_model)] {
            control.validate_product_selection(&owner, template, preset, model).await.unwrap();
        }
    }
    assert_eq!(authoring_store_presets(&store, &owner).await, before,
        "opening an editor and checking either selector must not create hidden configurations");
    assert_eq!(store.get_revision(&revision.reference).await.unwrap().unwrap(), revision);
    assert_eq!(store.get_snapshot(&revision.reference).await.unwrap().unwrap(), snapshot);
}

#[tokio::test]
async fn authoring_contract_evolution_saves_a_new_revision_and_keeps_stale_cas_fenced() {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let (control, owner, revision, snapshot) = retired_official_launch_fixture(store.clone()).await;
    let preset_id = revision.reference.preset_id.as_ref();
    let before_count = store.list_presets(&owner).await.unwrap().len();
    let mut draft = control.editor(&owner, preset_id, None).await.unwrap().draft;
    assert_eq!(authoring_process_actions(&draft.document),
        &BTreeSet::from(["workspace.process/start".to_owned()]));
    draft.document.persona = "Keep my explicitly edited persona".into();
    let request = SaveAgentPresetRevisionRequest {
        expected_current_revision: draft.current_revision.clone(),
        draft: draft.clone(),
        reason: Some("Save the current process launch contract".into()),
    };
    let saved = control.save_revision(&owner, preset_id, request.clone()).await.unwrap();
    assert_eq!(saved.revision.reference.preset_id, preset_id);
    assert_eq!(saved.revision.reference.revision, revision.reference.revision + 1);
    assert_eq!(saved.revision.document, draft.document);
    assert_eq!(store.list_presets(&owner).await.unwrap().len(), before_count);
    assert_eq!(store.get_revision(&revision.reference).await.unwrap().unwrap(), revision);
    assert_eq!(store.get_snapshot(&revision.reference).await.unwrap().unwrap(), snapshot);
    assert_eq!(control.save_revision(&owner, preset_id, request).await.unwrap_err().code().as_ref(),
        "PRESET_REVISION_DIGEST_MISMATCH");
    assert_eq!(control.editor(&owner, preset_id, None).await.unwrap().revision.unwrap(), saved.revision);
    assert_eq!(authoring_process_actions(&control.editor(&owner, preset_id,
        Some(revision.reference.revision)).await.unwrap().draft.document),
        &BTreeSet::from(["workspace.process/exec".to_owned()]));
}

#[tokio::test]
async fn authoring_contract_evolution_keeps_unproven_publishers_effects_and_resources_repairable() {
    for drift in ["publisher", "effect", "resource"] {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let (mut control, owner, revision, snapshot) = retired_official_launch_fixture(store.clone()).await;
        let preset_id = revision.reference.preset_id.as_ref();
        let mut catalog = (*control.catalog.snapshot().unwrap()).clone();
        let process = catalog.capabilities.iter_mut()
            .find(|value| value.manifest.id.as_ref() == "workspace.process").unwrap();
        match drift {
            "publisher" => {
                process.manifest.package.id = "other.publisher".into();
                process.source.source_identity = "other.publisher".into();
                process.contribution_lock.source_identity = "other.publisher".into();
            }
            "effect" => process.manifest.contributions.actions.iter_mut()
                .find(|action| action.action_id.as_ref() == "workspace.process/start")
                .unwrap().effect_class = EffectClass::Destructive,
            _ => { process.manifest.contributions.resource_kinds.insert("new-resource".into()); }
        }
        let entry = republish_catalog_capability(process);
        catalog.formal_capability_entries.insert(entry.capability.clone(), entry);
        install_bounded_launch_catalog(&mut control, catalog);
        let before = authoring_store_presets(&store, &owner).await;
        let editor = control.editor(&owner, preset_id, None).await.unwrap();
        assert_eq!(authoring_process_actions(&editor.draft.document),
            &BTreeSet::from(["workspace.process/exec".to_owned()]),
            "{drift}: a failed proof must leave the saved grant visible for repair");
        let selected_model = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: media_chat_route("model-b").primary.provider_id,
            model: "model-b".into(),
        };
        for model in [None, Some(&selected_model)] {
            let error = control.validate_product_selection(&owner, None, Some(preset_id), model)
                .await.unwrap_err();
            assert_eq!(error.code().as_ref(), "AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED", "{drift}");
        }
        assert_eq!(authoring_store_presets(&store, &owner).await, before, "{drift}");
        assert_eq!(store.get_revision(&revision.reference).await.unwrap().unwrap(), revision);
        assert_eq!(store.get_snapshot(&revision.reference).await.unwrap().unwrap(), snapshot);
    }
}

#[tokio::test]
async fn authoring_contract_evolution_does_not_drop_an_unknown_saved_action() {
    let store = Arc::new(InMemoryControlPlaneStore::new());
    let (control, owner, mut revision, mut snapshot) = retired_official_launch_fixture(store.clone()).await;
    let previous_ref = revision.reference.clone();
    let unknown = "workspace.process/unknown_saved_action";
    let process = snapshot.content.enabled_capabilities.iter_mut()
        .find(|value| value.capability.id.as_ref() == "workspace.process").unwrap();
    let mut descriptor = process.actions.iter()
        .find(|action| action.action_id.as_ref() == "workspace.process/exec").unwrap().clone();
    descriptor.action_id = unknown.into();
    process.actions.retain(|action| action.action_id.as_ref() != "workspace.process/exec");
    process.actions.push(descriptor);
    process.action_allowlist = BTreeSet::from([unknown.into()]);
    revision.payload.enabled_capabilities.iter_mut()
        .find(|value| value.capability.id.as_ref() == "workspace.process").unwrap()
        .action_allowlist = process.action_allowlist.clone();
    revision.reference.revision += 1;
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    snapshot.content.preset_revision_ref = revision.reference.clone();
    snapshot.content.chat_route_identity = revision.chat_route_identity().unwrap();
    snapshot.snapshot_ref.snapshot_digest = digest_payload(&snapshot.content).unwrap();
    snapshot = nomifun_agent_kernel::AgentPresetCompiler::derive_model_snapshot(&revision, &snapshot,
        nomifun_agent_kernel::CompileRequest {
            revision: revision.clone(), principal: snapshot.actor.clone(), scene: snapshot.scene.clone(),
            surface: snapshot.surface.clone(), audience: snapshot.audience.clone(),
            created_at_ms: snapshot.created_at_ms, resolver_run_id: snapshot.resolver_run_id.clone(),
        }).unwrap();
    store.append_revision(Some(&previous_ref), revision.clone(), snapshot.clone(),
        "Saved official Agent".into(), Some("Preserve my presentation".into())).await.unwrap();
    let preset_id = revision.reference.preset_id.as_ref();
    let before = authoring_store_presets(&store, &owner).await;
    let editor = control.editor(&owner, preset_id, None).await.unwrap();
    assert_eq!(authoring_process_actions(&editor.draft.document), &BTreeSet::from([unknown.to_owned()]),
        "only a proved replacement can update authoring intent; unknown grants stay visible");
    let selected_model = nomifun_api_types::AgentChatModelSelectionDto {
        provider_id: media_chat_route("model-b").primary.provider_id,
        model: "model-b".into(),
    };
    for model in [None, Some(&selected_model)] {
        let error = control.validate_product_selection(&owner, None, Some(preset_id), model)
            .await.unwrap_err();
        assert_eq!(error.code().as_ref(), "CAPABILITY_NOT_MATERIALIZED");
        let details = error.details().expect("unavailable actions include actionable diagnostics");
        let diagnostics = details["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0]["code"], "CAPABILITY_ACTION_NOT_MATERIALIZED");
        assert_eq!(diagnostics[0]["subject"], unknown);
    }
    let mut draft = editor.draft;
    let actions = &mut draft.document.enabled_capabilities.iter_mut()
        .find(|selection| selection.capability.id == "workspace.process").unwrap().action_allowlist;
    actions.insert("workspace.process/start".into());
    assert!(control.save_revision(&owner, preset_id, SaveAgentPresetRevisionRequest {
        expected_current_revision: draft.current_revision.clone(), draft, reason: None,
    }).await.is_err(), "an unknown action must not disappear during save");
    assert_eq!(authoring_store_presets(&store, &owner).await, before);
    assert_eq!(store.get_revision(&revision.reference).await.unwrap().unwrap(), revision);
    assert_eq!(store.get_snapshot(&revision.reference).await.unwrap().unwrap(), snapshot);
}
