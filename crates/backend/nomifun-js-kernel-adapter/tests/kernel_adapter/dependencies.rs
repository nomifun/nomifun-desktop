use super::*;

const GRANDCHILD: &str = "fixture.grandchild";

fn dependency_artifact(main: &[u8], mapped: bool) -> PluginPackageArtifactV1 {
    dependency_artifact_for(main, mapped, false)
}

fn dependency_artifact_for(main: &[u8], mapped: bool, context: bool) -> PluginPackageArtifactV1 {
    let base = artifact(main);
    let mut manifest = base.manifest.payload;
    let capabilities = &mut manifest.package.contributions.capabilities;
    let parent = capabilities
        .iter_mut()
        .find(|c| c.id.as_ref() == if context { CONTEXT_ID } else { TOOL_ID })
        .unwrap();
    if context {
        parent.contributions.context_phase =
            nomifun_agent_contracts::ContextContributionPhase::BeforeTurn;
    }
    parent.requires = vec![CapabilityRef {
        id: UI_TOOL_ID.into(),
        version: VERSION.into(),
    }];
    let child = capabilities
        .iter_mut()
        .find(|c| c.id.as_ref() == UI_TOOL_ID)
        .unwrap();
    child.supported_surfaces =
        capability_surface_declarations(["desktop"], [CapabilityConsumer::Agent]);
    let mut grandchild = child.clone();
    grandchild.id = GRANDCHILD.into();
    grandchild.contribution_id = "fixture.grandchild.contribution".into();
    child.requires = vec![CapabilityRef {
        id: GRANDCHILD.into(),
        version: VERSION.into(),
    }];
    capabilities.push(grandchild);
    let base = PluginPackageArtifactV1::new(base.artifact_id, manifest, base.files).unwrap();
    if !mapped {
        return base;
    }
    let (contract, _) = contract(&base);
    let mut manifest = base.manifest.payload.clone();
    manifest.package.contributions.role_providers = vec![provider(&contract, true)];
    PluginPackageArtifactV1::new(base.artifact_id, manifest, base.files).unwrap()
}

fn call(target: &str, action: &str, key: &str, input: serde_json::Value) -> serde_json::Value {
    json!({"capabilityId":target,"actionId":action,"callKey":key,"input":input})
}

#[tokio::test]
async fn dependency_contexts_cannot_start_js_through_direct_or_role_public_entries() {
    let main = std::fs::read(fixture("main.mjs")).unwrap();
    let temp = TempDir::new().unwrap();
    let base = dependency_artifact(&main, true);
    let mut manifest = base.manifest.payload.clone();
    let root = manifest
        .package
        .contributions
        .capabilities
        .iter_mut()
        .find(|value| value.id.as_ref() == TOOL_ID)
        .unwrap();
    root.requires.extend(
        [CONTEXT_ID, MEMBERS[1]]
            .into_iter()
            .map(|id| CapabilityRef {
                id: id.into(),
                version: VERSION.into(),
            }),
    );
    let artifact = PluginPackageArtifactV1::new(base.artifact_id, manifest, base.files).unwrap();
    let adapter = adapter(artifact, &temp);
    let host = host().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = registry();
    let materialized = registry
        .replace_all(vec![
            builtin(&adapter, host.clone(), calls.clone()),
            adapter.registration(host.clone()).unwrap(),
        ])
        .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "fixture-owner".into(),
    };
    let mut revision = revision(&owner, &materialized);
    revision.payload.enabled_capabilities.truncate(1);
    revision.payload.enabled_capabilities[0].capability.id = MEMBERS[0].into();
    revision.contribution_locks = vec![
        materialized
            .capability(&MEMBERS[0].into())
            .unwrap()
            .contribution_lock
            .clone(),
    ];
    revision
        .payload
        .system_role_provider_overrides
        .insert(ROLE.into(), selection(&materialized, USER_MOUNT));
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let snapshot = AgentPresetCompiler::compile(
        &materialized,
        &environment(materialized.registry_digest.clone()),
        CompileRequest {
            revision,
            plugin_product_capabilities: Vec::new(),
            principal: owner.clone(),
            scene: "fixture".into(),
            surface: "desktop".into(),
            audience: "test".into(),
            created_at_ms: 2,
            resolver_run_id: "dependency-context".into(),
        },
    )
    .unwrap();
    let active = SessionCapabilityState::new(&snapshot).snapshot().unwrap();
    assert_eq!(snapshot.content().contributions().count(), 1);
    for id in [CONTEXT_ID, MEMBERS[1]] {
        assert!(matches!(
            registry
                .contribute_context(
                    &snapshot,
                    &active,
                    access(&snapshot, active.generation, &owner, id)
                )
                .await,
            Err(KernelError::CapabilityNotInPreset { .. })
        ));
    }
    let member = RoleMemberInvocationRequest {
        principal: owner.clone(),
        session_owner: owner.clone(),
        turn_id: Some("private-role-turn".into()),
        operation_id: "private-role-context".into(),
        correlation_id: "private-role-context".into(),
        capability_id: MEMBERS[1].into(),
        resource_binding_ids: BTreeSet::new(),
        state_scope_key: "session:fixture-session".into(),
        admission: RoleMemberAdmission::Agent {
            agent_session_id: "fixture-session".into(),
            resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
            active_set_generation: active.generation,
        },
    };
    assert!(matches!(
        registry
            .contribute_role_context(&snapshot, &active, member)
            .await,
        Err(KernelError::CapabilityNotInPreset { .. })
    ));
    assert_eq!(
        host.process_count(),
        0,
        "denial precedes JS activation and resource effects"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn real_js_direct_and_role_tools_call_only_declared_dependencies_through_kernel() {
    run_dependency_cases(false).await;
}

#[tokio::test]
async fn real_js_direct_and_role_contexts_call_only_declared_dependencies_through_kernel() {
    run_dependency_cases(true).await;
}

async fn invoke_parent(
    registry: &KernelRegistry,
    snapshot: &CompiledSnapshot,
    active: &nomifun_agent_kernel::ActiveCapabilitySetSnapshot,
    request: CapabilityInvocationRequest,
    context: bool,
) -> Result<StrictJsonValue, KernelError> {
    if !context {
        return registry.invoke(snapshot, active, request).await;
    }
    let input = nomifun_agent_contracts::ContextContributionInput::BeforeTurn {
        turn: nomifun_agent_contracts::ContextTurnInput {
            source_message_id: "context-dependency-message".into(),
            text: request.input.0.to_string(),
            image_media_types: Vec::new(),
        },
    };
    registry
        .contribute_context_with_input(
            snapshot,
            active,
            access(
                snapshot,
                active.generation,
                &request.principal,
                request.capability_id.as_ref(),
            ),
            input,
        )
        .await
        .map(|result| result.value.expect("fixture Context returns a value"))
}

async fn run_dependency_cases(context: bool) {
    for (mapped, cross_package) in [(false, false), (true, false), (false, true), (true, true)] {
        let main = std::fs::read(fixture("main.mjs")).unwrap();
        let temp = TempDir::new().unwrap();
        let mut artifact = dependency_artifact_for(&main, mapped, context);
        let child = if cross_package {
            let mut root = artifact.manifest.payload.clone();
            let mut child = root.clone();
            child.package.package_id = "fixture.dependencies".into();
            child.package.contributions.role_providers.clear();
            child
                .package
                .contributions
                .capabilities
                .retain(|c| matches!(c.id.as_ref(), UI_TOOL_ID | GRANDCHILD));
            for capability in &mut child.package.contributions.capabilities {
                capability.package.id = child.package.package_id.clone();
            }
            // This package contains only the dependency Tools. Its schema
            // registry must describe those exports, not the removed Context
            // and root-only actions inherited from the original fixture.
            let child_schemas = child
                .package
                .contributions
                .capabilities
                .iter()
                .flat_map(|capability| &capability.contributions.actions)
                .flat_map(|action| [action.input_schema.clone(), action.output_schema.clone()])
                .collect::<BTreeSet<_>>();
            child
                .schemas
                .retain(|schema, _| child_schemas.contains(schema));
            root.package
                .contributions
                .capabilities
                .retain(|c| !matches!(c.id.as_ref(), UI_TOOL_ID | GRANDCHILD));
            let child = PluginPackageArtifactV1::new(
                "fixture-dependencies-artifact".into(),
                child,
                artifact.files.clone(),
            )
            .unwrap();
            artifact =
                PluginPackageArtifactV1::new(artifact.artifact_id, root, artifact.files).unwrap();
            Some(
                JsKernelPluginAdapter::new(PluginPackageInput {
                    config: ValidatedPluginConfig {
                        schema_digest: digest_payload(
                            &child.manifest.payload.package.config_schema,
                        )
                        .unwrap(),
                        config_revision: 1,
                        value: StrictJsonValue(json!({})),
                    },
                    artifact: child,
                    mount_id: "fixture-dependency-mount".into(),
                    package_root: fixture("main.mjs")
                        .canonicalize()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .to_path_buf(),
                    credential_bindings: Vec::new(),
                    data_dir: temp.path().join("child"),
                })
                .unwrap(),
            )
        } else {
            None
        };
        let adapter = adapter(artifact, &temp);
        let host = host().await;
        let builtin_calls = Arc::new(AtomicUsize::new(0));
        let mut registrations = vec![adapter.registration(host.clone()).unwrap()];
        if let Some(child) = child {
            registrations.push(child.registration(host.clone()).unwrap());
        }
        if mapped {
            registrations.push(builtin(&adapter, host.clone(), builtin_calls.clone()));
        }
        let registry = registry();
        let materialized = registry.replace_all(registrations.clone()).unwrap();
        let owner = PrincipalRef {
            principal_kind: "user".into(),
            principal_id: "fixture-owner".into(),
        };
        let snapshot = if mapped {
            compile(&materialized, &owner, true)
        } else {
            compile_snapshot(&materialized, &owner)
        };
        let active = SessionCapabilityState::new(&snapshot).snapshot().unwrap();
        let mut request = invoke_request(&snapshot, &owner);
        let parent_id = if context {
            if mapped { MEMBERS[1] } else { CONTEXT_ID }
        } else if mapped {
            MEMBERS[0]
        } else {
            TOOL_ID
        };
        request.capability_id = parent_id.into();
        // Real Node -> Rust Kernel -> Node -> Rust Kernel -> Node, one frozen
        // plan and one Host. The facade has no private implementation deps.
        for id in [UI_TOOL_ID, GRANDCHILD] {
            let dependency = snapshot.resolved_capability(&id.into()).unwrap();
            assert_eq!(
                dependency.consumption,
                nomifun_agent_contracts::CapabilityConsumption::Dependency
            );
            assert!(
                !snapshot
                    .content()
                    .contributions()
                    .any(|value| value.capability.id.as_ref() == id)
            );
            let mut direct = request.clone();
            direct.capability_id = id.into();
            direct.action_id = UI_TOOL_ACTION.into();
            assert!(matches!(
                registry.invoke(&snapshot, &active, direct).await,
                Err(KernelError::CapabilityNotInPreset { .. })
            ));
        }
        if mapped {
            assert!(
                materialized
                    .capability(&parent_id.into())
                    .unwrap()
                    .manifest
                    .requires
                    .is_empty()
            );
            assert_eq!(
                snapshot
                    .resolved_capability(&parent_id.into())
                    .unwrap()
                    .dependency_refs[0]
                    .id
                    .as_ref(),
                UI_TOOL_ID
            );
        }
        request.input = StrictJsonValue(
            json!({"dependency":call(UI_TOOL_ID, UI_TOOL_ACTION, "child",
            json!({"dependency":call(GRANDCHILD, UI_TOOL_ACTION, "grandchild", json!({"value":17}))}))}),
        );
        let value = invoke_parent(&registry, &snapshot, &active, request.clone(), context)
            .await
            .unwrap();
        assert_eq!(
            value.0["contributionId"],
            "fixture.grandchild.contribution",
            "unexpected dependency response: {:?}",
            value.0
        );
        assert_eq!(value.0["input"]["value"], 17);
        assert_eq!(builtin_calls.load(Ordering::SeqCst), 0);

        for (target, action, code) in [
            (GRANDCHILD, UI_TOOL_ACTION, "DEPENDENCY_NOT_DECLARED"),
            (
                if context { TOOL_ID } else { CONTEXT_ID },
                TOOL_ACTION,
                "DEPENDENCY_NOT_DECLARED",
            ),
            (parent_id, TOOL_ACTION, "DEPENDENCY_CALL_CYCLE"),
        ] {
            request.input =
                StrictJsonValue(json!({"dependency":call(target, action, "denied", json!({}))}));
            let result = invoke_parent(&registry, &snapshot, &active, request.clone(), context)
                .await
                .unwrap();
            assert!(
                result.0["dependencyError"].as_str().unwrap().contains(code),
                "{result:?}"
            );
        }
        request.input = StrictJsonValue(
            json!({"dependency":call(UI_TOOL_ID, "not-authorized", "bad-action", json!({}))}),
        );
        let result = invoke_parent(&registry, &snapshot, &active, request.clone(), context)
            .await
            .unwrap();
        assert!(result.0["dependencyError"].is_string());

        if cross_package {
            // Keep the parent artifact unchanged: a stale dependency must be
            // rejected by the child dispatch, not merely by root admission.
            let mut changed = registrations.clone();
            changed[1].metadata.source.source_digest = Some(DigestHex::from("e".repeat(64)));
            registry.replace_all(changed).unwrap();
            request.input = StrictJsonValue(
                json!({"dependency":call(UI_TOOL_ID, UI_TOOL_ACTION, "stale-child", json!({}))}),
            );
            let result = invoke_parent(&registry, &snapshot, &active, request.clone(), context)
                .await
                .unwrap();
            assert!(
                result.0["dependencyError"]
                    .as_str()
                    .unwrap()
                    .contains("CAPABILITY_NOT_MATERIALIZED"),
                "{result:?}"
            );
            assert_eq!(builtin_calls.load(Ordering::SeqCst), 0);
            registry.replace_all(registrations.clone()).unwrap();
        }

        // Source drift must reject this old frozen invocation, not fall back.
        let child_id = CapabilityId::from(UI_TOOL_ID);
        let mut changed = registrations;
        // Change the root package in both layouts. Separate child/ancestor
        // revocation is covered by the Kernel dependency tests.
        changed[0].metadata.source.source_digest = Some(DigestHex::from("f".repeat(64)));
        registry.replace_all(changed).unwrap();
        request.input = StrictJsonValue(
            json!({"dependency":call(child_id.as_ref(), UI_TOOL_ACTION, "stale", json!({}))}),
        );
        assert!(
            invoke_parent(&registry, &snapshot, &active, request, context)
                .await
                .is_err()
        );
        assert_eq!(builtin_calls.load(Ordering::SeqCst), 0);
        let generation = match host.state() {
            JavaScriptHostState::Running { generation, .. } => generation,
            _ => panic!(),
        };
        host.stop_generation(generation).await.unwrap();
    }
}
