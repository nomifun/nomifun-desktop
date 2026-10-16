//! Channel guests receive an immutable model-only ceiling. Editable Agents and
//! installation extensions are not sources for this product boundary.
use super::*;

const CHANNEL_GUEST_CONFIGURATION: &str = "Channel guest: model-only execution ceiling";

fn model_only_ceiling(
    revision: &AgentPresetRevision,
    snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
) -> bool {
    let payload = &revision.payload;
    // Compare the complete formal document outside its selected Chat route.
    // This also excludes non-default execution/supervision policy and future
    // authoring fields instead of maintaining a lossy list of prompt mirrors.
    let Ok(empty) = document_payload(&empty_document()) else {
        return false;
    };
    let mut non_model = payload.clone();
    non_model.model_route_refs.clear();
    non_model.chat_route_records.clear();
    non_model == empty
        && payload.model_route_refs.len() == 1
        && payload.model_route_refs.contains_key(CHAT_MODEL_TASK)
        && payload.chat_route_records.len() == 1
        && payload.chat_route_records.contains_key(CHAT_MODEL_TASK)
        && revision.contribution_locks.is_empty()
        && snapshot.content.model_route_refs == payload.model_route_refs
        && snapshot.content.chat_route_identity.is_some()
        && snapshot.content.enabled_capabilities.is_empty()
        && snapshot.content.capability_allowlist.is_empty()
        && snapshot.content.context_order.is_empty()
        && snapshot.content.middleware_order.is_empty()
        && snapshot.content.required_resource_kinds.is_empty()
        && snapshot.content.required_runtime_features.is_empty()
        && snapshot.content.skill_locks.is_empty()
        && snapshot.content.mcp_tool_locks.is_empty()
        && snapshot.content.resolved_role_providers.is_empty()
}

impl AgentControlPlane {
    /// Read-only admission/recovery proof over the saved immutable contract.
    /// Cache labels and authored/session-only presentation grant no authority.
    pub async fn validate_channel_guest_model_only_binding(
        &self,
        owner: &UserId,
        binding: &AgentBindingValueDto,
    ) -> Result<(), ControlPlaneError> {
        let (binding, revision, snapshot) = self
            .saved_binding_artifacts(owner, binding)
            .await
            .map_err(|error| {
                ControlPlaneError::with_details(
                    "CHANNEL_GUEST_CEILING_INVALID",
                    axum::http::StatusCode::CONFLICT,
                    "the channel guest has no valid owned immutable contract",
                    json!({"control_plane_code":error.code().as_ref()}),
                )
            })?;
        if !binding.typed_resource_bindings.is_empty() || !model_only_ceiling(&revision, &snapshot)
        {
            return Err(ControlPlaneError::canonical(
                "CHANNEL_GUEST_CEILING_INVALID",
                axum::http::StatusCode::CONFLICT,
                "the channel guest exceeds its model-only execution ceiling",
            ));
        }
        Ok(())
    }

    /// A host-owned guest is a requested model with the formal empty Agent
    /// contract. It never resolves the owner's editable chat.minimal document
    /// or the normal Session extension/default selection path.
    pub async fn resolve_channel_guest_model_only_binding(
        &self,
        owner: &UserId,
        model: &nomifun_api_types::AgentChatModelSelectionDto,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let _guard = self.template_launch_lock.lock().await;
        let route = self
            .resolve_selected_chat_route(owner, model, &BTreeSet::new())
            .await?;
        let mut payload = document_payload(&empty_document())?;
        payload
            .model_route_refs
            .insert(CHAT_MODEL_TASK.into(), route.primary.model_route_id.clone());
        payload
            .chat_route_records
            .insert(CHAT_MODEL_TASK.into(), route);

        for candidate in self.store.list_presets(owner).await? {
            if !candidate.session_only
                || candidate.preset.source != AgentPresetSource::Official
                || owned_official_key(&candidate) != Some(OfficialPresetKey::ChatMinimal)
                || candidate.preset.description.as_deref() != Some(CHANNEL_GUEST_CONFIGURATION)
            {
                continue;
            }
            let Some((revision, snapshot)) = self
                .reusable_session_configuration(&candidate, &payload, true)
                .await
            else {
                continue;
            };
            if !model_only_ceiling(&revision, &snapshot) {
                continue;
            }
            let Ok(binding) = session_binding_from_stable_artifacts(
                revision.reference.clone(),
                Some(revision),
                Some(snapshot),
            ) else {
                continue;
            };
            // Optional internal caches cannot mutate or lend authority to an
            // authored configuration, including an unreadable older candidate.
            if self.validate_agent_binding(owner, &binding).await.is_err() {
                continue;
            }
            return wire_cast(&binding);
        }

        let created = self
            .create_configuration_with_payload(
                owner,
                AgentPresetId::from(Uuid::now_v7().to_string()),
                OfficialPresetKey::ChatMinimal.as_str().into(),
                Some(CHANNEL_GUEST_CONFIGURATION.into()),
                payload,
                Some(OfficialPresetKey::ChatMinimal),
                true,
                None,
            )
            .await?;
        let stored = self.owned_preset(owner, &created.preset.preset_id).await?;
        let revision = self
            .current_revision(&stored)
            .await?
            .ok_or_else(|| not_found("AgentPresetRevision"))?;
        let snapshot = self
            .current_snapshot(Some(&revision))
            .await?
            .ok_or_else(|| not_found("ResolvedSnapshot"))?;
        if !model_only_ceiling(&revision, &snapshot) {
            return Err(ControlPlaneError::canonical(
                "CHANNEL_GUEST_CEILING_INVALID",
                axum::http::StatusCode::CONFLICT,
                "the channel guest compiler produced authority outside its model-only ceiling",
            ));
        }
        let binding = session_binding_from_stable_artifacts(
            revision.reference.clone(),
            Some(revision),
            Some(snapshot),
        )?;
        self.validate_agent_binding(owner, &binding).await?;
        wire_cast(&binding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InMemoryControlPlaneStore, StaticCatalogProvider};
    use nomifun_agent_contracts::{
        ChatRouteCandidate, ChatRouteRecord, ChatRouteRecordSchema, ChatRouteTask,
        ConnectionConfigRef, DigestHex, FrozenLibrarySkill, LibrarySkillSource, ModelRouteId,
        RuntimeProfileKind, RuntimeTarget, VersionString,
    };
    use nomifun_agent_kernel::{CompilerEnvironment, MaterializedRegistry};
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

    const PROVIDER: &str = "0190f5fe-7c00-7a00-8000-000000000010";

    struct Routes(Arc<AtomicU8>);

    fn route(model: &str, revision: u8) -> ChatRouteRecord {
        ChatRouteRecord {
            schema: ChatRouteRecordSchema::V1,
            task: ChatRouteTask::AgentChat,
            primary: ChatRouteCandidate {
                model_route_id: ModelRouteId::from(Uuid::now_v7().to_string()),
                model_route_revision: 1,
                provider_id: PROVIDER.into(),
                model: model.into(),
                protocol: ChatRouteProtocol::OpenaiChat,
                connection_config_ref: ConnectionConfigRef::from("default"),
                config_revision_digest: DigestHex::from(format!("{revision:064x}")),
                credential_ref: "test-credential-alias".into(),
                features: BTreeSet::from([
                    ChatRouteFeature::TextInput,
                    ChatRouteFeature::TextOutput,
                ]),
                activation_features: BTreeSet::new(),
            },
            failovers: Vec::new(),
        }
    }

    #[async_trait::async_trait]
    impl DefaultChatRouteResolver for Routes {
        async fn resolve_default_chat_route(
            &self,
            _: &UserId,
        ) -> Result<Option<ChatRouteRecord>, ControlPlaneError> {
            Ok(Some(route(
                "private-authored-model",
                self.0.load(Ordering::SeqCst),
            )))
        }

        async fn resolve_selected_chat_route(
            &self,
            _: &UserId,
            model: &nomifun_api_types::AgentChatModelSelectionDto,
        ) -> Result<Option<ChatRouteRecord>, ControlPlaneError> {
            Ok((model.provider_id == PROVIDER
                && matches!(model.model.as_str(), "guest-model" | "other-guest-model"))
            .then(|| route(&model.model, self.0.load(Ordering::SeqCst))))
        }
    }

    struct PrivateDefaults(Arc<AtomicUsize>);

    #[async_trait::async_trait]
    impl SessionCapabilitiesResolver for PrivateDefaults {
        async fn resolve(
            &self,
            _: &UserId,
            _: Option<&nomifun_api_types::SessionCapabilitySelectionDto>,
        ) -> Result<ResolvedSessionCapabilities, ControlPlaneError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ResolvedSessionCapabilities {
                skills: vec![
                    FrozenLibrarySkill::new(
                        "private-guide".into(),
                        "Owner private guide".into(),
                        LibrarySkillSource::Custom,
                        "PRIVATE_GLOBAL_SKILL_CONTENT".into(),
                        BTreeMap::new(),
                    )
                    .unwrap(),
                ],
                selected_skill_names: BTreeSet::from(["private-guide".into()]),
                mcp_server_ids: BTreeSet::from(["private-mcp-server".into()]),
            })
        }
    }

    fn control(
        store: Arc<InMemoryControlPlaneStore>,
        revision: Arc<AtomicU8>,
    ) -> AgentControlPlane {
        let compiler = PresetRevisionCompiler::new().with_materialized_registry(
            Arc::new(MaterializedRegistry::empty()),
            CompilerEnvironment {
                resolver_version: VersionString::from("1.0.0"),
                required_runtime_protocol_version: VersionString::from("1.0.0"),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: DigestHex::from("1".repeat(64)),
                available_runtime_features: BTreeSet::new(),
                installation_role_bindings: BTreeMap::new(),
                canonical_schema_manifest_digest: DigestHex::from("2".repeat(64)),
                target_contribution_manifest_digest: DigestHex::from("3".repeat(64)),
                host_target: RuntimeTarget::from("test"),
                host_surface: "desktop".into(),
                availability_evidence_revision: "channel-guest-fixture".into(),
            },
        );
        AgentControlPlane::new(
            store,
            Arc::new(StaticCatalogProvider::new(Default::default())),
            OfficialTemplateCatalog::load().unwrap(),
            compiler,
        )
        .with_default_chat_route_resolver(Arc::new(Routes(revision)))
    }

    fn model(name: &str) -> nomifun_api_types::AgentChatModelSelectionDto {
        nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: PROVIDER.into(),
            model: name.into(),
        }
    }

    #[tokio::test]
    async fn channel_guest_ignores_authored_minimal_and_global_extensions_and_reuses_the_exact_model_cache()
     {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let control = control(store.clone(), Arc::new(AtomicU8::new(1)));
        let library = control.library(&owner).await.unwrap();
        let authored_id = &library.official_presets
            [&nomifun_api_types::OfficialPresetKeyDto::ChatMinimal]
            .preset_id;
        let mut draft = control
            .editor(&owner, authored_id, None)
            .await
            .unwrap()
            .draft;
        draft.document.persona = "PRIVATE_OWNER_PERSONA".into();
        draft.document.instructions = "PRIVATE_OWNER_INSTRUCTIONS".into();
        draft.document.starter_prompts = vec!["PRIVATE_OWNER_STARTER".into()];
        let authored_route = route("private-authored-model", 1);
        draft.document.model_route_refs.insert(
            CHAT_MODEL_TASK.into(),
            authored_route.primary.model_route_id.as_ref().into(),
        );
        draft.document.chat_route_records.insert(
            CHAT_MODEL_TASK.into(),
            serde_json::to_value(authored_route).unwrap(),
        );
        let saved = control
            .save_revision(
                &owner,
                authored_id,
                SaveAgentPresetRevisionRequest {
                    expected_current_revision: None,
                    draft,
                    reason: Some("Owner customization must remain private".into()),
                },
            )
            .await
            .unwrap();
        let authored_ref: PresetRevisionRef = wire_cast(&saved.revision.reference).unwrap();
        let authored_before = store.get_revision(&authored_ref).await.unwrap().unwrap();
        let preset_before = store
            .get_preset(&AgentPresetId::from(authored_id.clone()))
            .await
            .unwrap()
            .unwrap()
            .preset;
        let extension_calls = Arc::new(AtomicUsize::new(0));
        let control = control
            .with_session_capabilities_resolver(Arc::new(PrivateDefaults(extension_calls.clone())));
        let requested = model("guest-model");
        let (first, concurrent) = tokio::join!(
            control.resolve_channel_guest_model_only_binding(&owner, &requested),
            control.resolve_channel_guest_model_only_binding(&owner, &requested),
        );
        let first = first.unwrap();
        assert_eq!(
            concurrent.unwrap(),
            first,
            "concurrent guest requests reuse the same immutable configuration"
        );
        let repeated = control
            .resolve_channel_guest_model_only_binding(&owner, &model("guest-model"))
            .await
            .unwrap();
        assert_eq!(
            repeated, first,
            "fresh route aliases do not duplicate the same effective requested model"
        );
        assert!(first.typed_resource_bindings.is_empty());
        assert_ne!(first.preset_revision_ref.preset_id, *authored_id);
        let (_, revision, snapshot) = control
            .saved_binding_artifacts(&owner, &first)
            .await
            .unwrap();
        assert!(model_only_ceiling(&revision, &snapshot));
        control
            .validate_channel_guest_model_only_binding(&owner, &first)
            .await
            .unwrap();
        let mut formal = revision.payload.clone();
        formal.model_route_refs.clear();
        formal.chat_route_records.clear();
        assert_eq!(
            formal,
            document_payload(&empty_document()).unwrap(),
            "the real saved non-model contract equals the formal empty document"
        );
        let mut knowledge = first.clone();
        knowledge
            .typed_resource_bindings
            .push(nomifun_api_types::TypedResourceBindingDto {
                binding_id: "private-knowledge".into(),
                resource_kind: "knowledge_base".into(),
                resource_id: "owner-private-knowledge".into(),
                owner_id: owner.as_ref().into(),
                operations: BTreeSet::from(["read".into()]),
                connection_config_ref: None,
                typed_parameters: BTreeMap::new(),
            });
        let error = control
            .validate_channel_guest_model_only_binding(&owner, &knowledge)
            .await
            .unwrap_err();
        assert_eq!(error.code().as_ref(), "CHANNEL_GUEST_CEILING_INVALID");
        let authored_snapshot = store.get_snapshot(&authored_ref).await.unwrap();
        let authored_binding: AgentBindingValueDto = wire_cast(
            &session_binding_from_stable_artifacts(
                authored_ref.clone(),
                Some(authored_before.clone()),
                authored_snapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let error = control
            .validate_channel_guest_model_only_binding(&owner, &authored_binding)
            .await
            .unwrap_err();
        assert_eq!(
            error.code().as_ref(),
            "CHANNEL_GUEST_CEILING_INVALID",
            "a familiar official label does not authorize private instructions"
        );
        assert!(revision.payload.persona.is_empty() && revision.payload.instructions.is_empty());
        assert_eq!(
            revision.payload.chat_route_records[CHAT_MODEL_TASK]
                .primary
                .provider_id,
            PROVIDER
        );
        assert_eq!(
            revision.payload.chat_route_records[CHAT_MODEL_TASK]
                .primary
                .model,
            "guest-model"
        );
        assert_eq!(
            extension_calls.load(Ordering::SeqCst),
            0,
            "guest creation never reads global Skill/MCP defaults"
        );
        assert_eq!(
            store.get_revision(&authored_ref).await.unwrap().unwrap(),
            authored_before
        );
        assert_eq!(
            store
                .get_preset(&AgentPresetId::from(authored_id.clone()))
                .await
                .unwrap()
                .unwrap()
                .preset,
            preset_before
        );
        assert!(
            control
                .library(&owner)
                .await
                .unwrap()
                .user_presets
                .is_empty(),
            "no authored guest Agent enters the user's library"
        );
        let cached = store
            .list_presets(&owner)
            .await
            .unwrap()
            .into_iter()
            .filter(|preset| {
                preset.preset.description.as_deref() == Some(CHANNEL_GUEST_CONFIGURATION)
            })
            .collect::<Vec<_>>();
        assert_eq!(cached.len(), 1);
        assert!(cached[0].session_only);
        let other_model = control
            .resolve_channel_guest_model_only_binding(&owner, &model("other-guest-model"))
            .await
            .unwrap();
        assert_ne!(other_model.preset_revision_ref, first.preset_revision_ref);
        let other_owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let other = control
            .resolve_channel_guest_model_only_binding(&other_owner, &model("guest-model"))
            .await
            .unwrap();
        assert_ne!(
            other.preset_revision_ref, first.preset_revision_ref,
            "guest caches remain owner-scoped"
        );
    }

    #[tokio::test]
    async fn channel_guest_route_changes_create_new_artifacts_and_invalid_models_fail_closed() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let route_revision = Arc::new(AtomicU8::new(1));
        let control = control(store.clone(), route_revision.clone());
        let first = control
            .resolve_channel_guest_model_only_binding(&owner, &model("guest-model"))
            .await
            .unwrap();
        let before = control
            .saved_binding_artifacts(&owner, &first)
            .await
            .unwrap();
        route_revision.store(2, Ordering::SeqCst);
        let next = control
            .resolve_channel_guest_model_only_binding(&owner, &model("guest-model"))
            .await
            .unwrap();
        assert_ne!(
            next.preset_revision_ref, first.preset_revision_ref,
            "effective route configuration changes cannot reuse the old guest snapshot"
        );
        assert_eq!(
            control
                .saved_binding_artifacts(&owner, &first)
                .await
                .unwrap(),
            before,
            "historical guest artifacts remain immutable"
        );
        let count = store.list_presets(&owner).await.unwrap().len();
        let error = control
            .resolve_channel_guest_model_only_binding(&owner, &model("missing-model"))
            .await
            .unwrap_err();
        assert_eq!(error.code().as_ref(), "MODEL_ROUTE_NOT_FOUND");
        assert_eq!(
            store.list_presets(&owner).await.unwrap().len(),
            count,
            "invalid models create no guest artifacts"
        );
        assert!(model_only_ceiling(
            &control
                .saved_binding_artifacts(&owner, &next)
                .await
                .unwrap()
                .1,
            &control
                .saved_binding_artifacts(&owner, &next)
                .await
                .unwrap()
                .2
        ));
    }

    #[tokio::test]
    async fn channel_guest_validation_uses_saved_contracts_and_rejects_nondefault_execution_policy()
    {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let control = control(store.clone(), Arc::new(AtomicU8::new(1)));
        for nondefault_policy in [false, true] {
            let mut document = empty_document();
            let selected = route("guest-model", 1);
            document.model_route_refs.insert(
                CHAT_MODEL_TASK.into(),
                selected.primary.model_route_id.as_ref().into(),
            );
            document.chat_route_records.insert(
                CHAT_MODEL_TASK.into(),
                serde_json::to_value(selected).unwrap(),
            );
            if nondefault_policy {
                document.runtime_policy.idmm.recover_provider_failures =
                    !document.runtime_policy.idmm.recover_provider_failures;
            }
            let created = control
                .create_preset(
                    &owner,
                    CreateAgentPresetRequest {
                        display_name: "Authored model-only contract validation fixture".into(),
                        description: None,
                        document: Some(document),
                        fork_from_revision: None,
                    },
                )
                .await
                .unwrap();
            let reference: PresetRevisionRef =
                wire_cast(created.preset.current_stable_revision.as_ref().unwrap()).unwrap();
            let revision = store.get_revision(&reference).await.unwrap().unwrap();
            let snapshot = store.get_snapshot(&reference).await.unwrap().unwrap();
            let canonical = session_binding_from_stable_artifacts(
                reference.clone(),
                Some(revision.clone()),
                Some(snapshot.clone()),
            )
            .unwrap();
            let binding: AgentBindingValueDto = wire_cast(&canonical).unwrap();
            let before_count = store.list_presets(&owner).await.unwrap().len();
            let validated = control
                .validate_channel_guest_model_only_binding(&owner, &binding)
                .await;
            if nondefault_policy {
                assert_eq!(
                    validated.unwrap_err().code().as_ref(),
                    "CHANNEL_GUEST_CEILING_INVALID"
                );
            } else {
                validated.unwrap();
                assert!(
                    !store
                        .get_preset(&reference.preset_id)
                        .await
                        .unwrap()
                        .unwrap()
                        .session_only,
                    "a formal model-only contract is valid without trusting a cache label"
                );
            }
            assert_eq!(
                store.list_presets(&owner).await.unwrap().len(),
                before_count,
                "validation never creates cache artifacts"
            );
            assert_eq!(
                store.get_revision(&reference).await.unwrap().unwrap(),
                revision
            );
            assert_eq!(
                store.get_snapshot(&reference).await.unwrap().unwrap(),
                snapshot
            );
        }
    }
}
