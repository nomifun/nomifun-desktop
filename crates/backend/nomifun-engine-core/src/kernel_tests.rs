#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;

    use super::super::*;
    use nomifun_agent_contracts::{
        AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload, AgentSessionId,
        CapabilityRef, CapabilitySelection, CorrelationId, DigestHex, IdempotencyKey, OperationId,
        PresetRevisionRef, ResourceBindingId, ResourceId, ResourceKind, RuntimeProfileKind,
        RuntimeTarget, UserId, VersionString, digest_payload,
    };
    use nomifun_agent_domain_wave2::{
        CONTRACT_VERSION, Wave2HostPort, Wave2HostPortError, Wave2HostRequest,
        registrations_with_host_port,
    };
    use nomifun_agent_kernel::{
        AgentPresetCompiler, CompileRequest, CompilerEnvironment, InMemoryPluginStatePersistence,
        MaterializationPolicy,
    };
    use nomifun_chat_model_broker::{ChatToolCall, ToolCallId};
    use serde_json::json;

    type SeenInvocation = (String, String, String, Vec<String>);

    struct RecordingHost {
        seen: Arc<Mutex<Vec<SeenInvocation>>>,
    }

    impl Wave2HostPort for RecordingHost {
        fn invoke<'a>(
            &'a self,
            request: Wave2HostRequest,
        ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>
        {
            let seen = Arc::clone(&self.seen);
            Box::pin(async move {
                seen.lock().unwrap().push((
                    request.context.capability_id.as_ref().to_owned(),
                    request.context.action_id.as_ref().to_owned(),
                    request.context.agent_session_id.as_ref().to_owned(),
                    request
                        .context
                        .resource_bindings
                        .iter()
                        .map(|binding| binding.binding_id.as_ref().to_owned())
                        .collect(),
                ));
                Ok(StrictJsonValue(json!({
                    "path": request.operation_input().0["path"],
                    "content": "owner-result"
                })))
            })
        }
    }

    trait Wave2RequestInput {
        fn operation_input(&self) -> &StrictJsonValue;
    }

    impl Wave2RequestInput for Wave2HostRequest {
        fn operation_input(&self) -> &StrictJsonValue {
            match &self.operation {
                nomifun_agent_domain_wave2::Wave2CapabilityOperation::WorkspaceExecution {
                    input,
                }
                | nomifun_agent_domain_wave2::Wave2CapabilityOperation::Ssh { input }
                | nomifun_agent_domain_wave2::Wave2CapabilityOperation::Browser { input }
                | nomifun_agent_domain_wave2::Wave2CapabilityOperation::ComputerA11y { input } => {
                    input
                }
            }
        }
    }

    struct KernelFixture {
        registry: Arc<KernelRegistry>,
        materialized: Arc<MaterializedRegistry>,
        snapshot: Arc<CompiledSnapshot>,
        active: Arc<SessionCapabilityState>,
        principal: PrincipalRef,
        action_id: ActionId,
        binding_id: ResourceBindingId,
        seen: Arc<Mutex<Vec<SeenInvocation>>>,
    }

    fn kernel_fixture() -> KernelFixture {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let registry = Arc::new(
            KernelRegistry::new(
                MaterializationPolicy::stable(CONTRACT_VERSION),
                Arc::new(InMemoryPluginStatePersistence::new()),
            )
            .unwrap(),
        );
        let materialized = registry
            .replace_all(
                registrations_with_host_port(Arc::new(RecordingHost {
                    seen: Arc::clone(&seen),
                }))
                .unwrap(),
            )
            .unwrap();
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "coding-owner".to_owned(),
        };
        let capability_id = CapabilityId::from("workspace.files");
        let action_id = ActionId::from("workspace.files/read");
        let binding_id = ResourceBindingId::from("workspace-binding");
        let binding = nomifun_agent_contracts::TypedResourceBinding {
            binding_id: binding_id.clone(),
            resource_kind: ResourceKind::from("workspace"),
            resource_id: ResourceId::from("workspace-resource"),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from(["read".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let payload = AgentPresetRevisionPayload {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: VersionString::from(CONTRACT_VERSION),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![CapabilitySelection {
                capability: CapabilityRef {
                    id: capability_id,
                },
                action_allowlist: BTreeSet::from([action_id.clone()]),
            }],
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "Coding Engine Kernel test".to_owned(),
            instructions: "Read one file.".to_owned(),
            starter_prompts: Vec::new(),
            runtime_policy: Default::default(),
        };
        let mut revision = AgentPresetRevision {
            reference: PresetRevisionRef {
                preset_id: AgentPresetId::from("coding-kernel-test"),
                revision: 1,
                revision_digest: digest_payload(&payload).unwrap(),
            },
            payload,
            contribution_locks: vec![
                materialized
                    .capability(&CapabilityId::from("workspace.files"))
                    .unwrap()
                    .contribution_lock
                    .clone(),
            ],
            created_by: UserId::from(principal.principal_id.clone()),
            created_at_ms: 1,
            reason: None,
        };
        revision.reference.revision_digest = revision.revision_digest().unwrap();
        let snapshot = Arc::new(
            AgentPresetCompiler::compile(
                &materialized,
                &CompilerEnvironment {
                    resolver_version: VersionString::from(CONTRACT_VERSION),
                    required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
                    required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                    runtime_feature_inventory_digest: DigestHex::from("runtime"),
                    available_runtime_features: BTreeSet::new(),
                    installation_role_bindings: BTreeMap::new(),
                    canonical_schema_manifest_digest: DigestHex::from("schema"),
                    target_contribution_manifest_digest: DigestHex::from("target"),
                    host_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
                    host_surface: "desktop".to_owned(),
                    availability_evidence_revision: "coding-kernel-test".to_owned(),
                },
                CompileRequest {
                    revision,
                    plugin_product_capabilities: Vec::new(),
                    principal: principal.clone(),
                    scene: "coding-kernel-test".to_owned(),
                    surface: "desktop".to_owned(),
                    audience: "test".to_owned(),
                    created_at_ms: 2,
                    resolver_run_id: OperationId::from("resolve"),
                },
            )
            .unwrap()
            .with_target_resource_bindings(&principal, vec![binding])
            .unwrap(),
        );
        let active = Arc::new(SessionCapabilityState::new(&snapshot));
        KernelFixture {
            registry,
            materialized,
            snapshot,
            active,
            principal,
            action_id,
            binding_id,
            seen,
        }
    }

    fn read_exposure(action_id: ActionId) -> EngineToolExposure {
        EngineToolExposure {
            definition: ChatToolDefinition {
                name: "read_file".to_owned(),
                description: "Read a UTF-8 file from the bound workspace.".to_owned(),
                input_schema: StrictJsonValue(json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "path": {"type": "string"}
                    },
                    "required": ["path"]
                })),
                deferred: false,
            },
            capability_id: CapabilityId::from("workspace.files"),
            action_id,
        }
    }

    #[test]
    fn adapter_type_keeps_session_scope_explicit() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<KernelEngineToolInvoker>();
    }

    #[test]
    fn workspace_process_terminal_failure_is_model_visible_without_legacy_identity() {
        assert!(is_workspace_process_action(
            &CapabilityId::from("workspace.process"),
            &ActionId::from("workspace.process/exec"),
        ));
        assert!(!is_workspace_process_action(
            &CapabilityId::from("workspace.process/exec"),
            &ActionId::from("process.exec.invoke"),
        ));
        assert!(process_result_is_error(
            true,
            &json!({"state":"exited", "success":false, "exit_code":7}),
        ));
        assert!(!process_result_is_error(
            true,
            &json!({"state":"running", "success":null}),
        ));
        assert!(!process_result_is_error(
            true,
            &json!({"state":"exited", "success":true, "exit_code":0}),
        ));
    }

    #[tokio::test]
    async fn compiled_plan_invokes_the_kernel_with_exact_snapshot_authority() {
        let fixture = kernel_fixture();
        let active = fixture.active.snapshot().unwrap();
        let plan = compile_engine_tool_plan(
            &fixture.snapshot,
            &active,
            &fixture.materialized,
            [read_exposure(fixture.action_id.clone())],
        )
        .unwrap();
        let binding = plan.binding("read_file").unwrap().clone();
        assert_eq!(
            binding.resource_binding_ids,
            BTreeSet::from([fixture.binding_id.clone()])
        );
        assert_eq!(binding.effect_class, EngineEffectClass::ReadOnly);
        assert!(binding.parallel_safe);

        let invoker = KernelEngineToolInvoker::new(
            Arc::clone(&fixture.registry),
            Arc::clone(&fixture.snapshot),
            Arc::clone(&fixture.active),
            fixture.principal.clone(),
            ScopeKey::from("session:coding-kernel-test"),
        );
        let result = invoker
            .invoke(
                EngineToolInvocation {
                    agent_session_id: AgentSessionId::from("coding-session"),
                    principal: fixture.principal,
                    resolved_snapshot_ref: fixture.snapshot.snapshot_ref().clone(),
                    active_set_generation: active.generation,
                    turn_operation_id: OperationId::from("turn"),
                    operation_id: OperationId::from("turn:tool:call-1"),
                    idempotency_key: IdempotencyKey::from("coding-tool:call-1"),
                    correlation_id: CorrelationId::from("coding-tool:call-1"),
                    call: ChatToolCall {
                        call_id: ToolCallId::from("call-1"),
                        name: "read_file".to_owned(),
                        arguments: StrictJsonValue(json!({"path": "README.md"})),
                        provider_metadata: None,
                    },
                    binding,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert!(result.output_text().contains("owner-result"));
        assert_eq!(
            *fixture.seen.lock().unwrap(),
            vec![(
                "workspace.files".to_owned(),
                "workspace.files/read".to_owned(),
                "coding-session".to_owned(),
                vec!["workspace-binding".to_owned()]
            )]
        );
    }

    #[test]
    fn plan_compilation_rejects_unselected_capabilities() {
        let fixture = kernel_fixture();
        let active = fixture.active.snapshot().unwrap();
        let error = compile_engine_tool_plan(
            &fixture.snapshot,
            &active,
            &fixture.materialized,
            [EngineToolExposure {
                definition: ChatToolDefinition {
                    name: "write_file".to_owned(),
                    description: "Write a file.".to_owned(),
                    input_schema: StrictJsonValue(json!({"type": "object"})),
                    deferred: false,
                },
                capability_id: CapabilityId::from("workspace.vcs"),
                action_id: ActionId::from("workspace.vcs/status"),
            }],
        )
        .unwrap_err();
        assert!(matches!(error, EngineToolError::ToolPlan(_)));
    }

    #[test]
    fn active_execution_error_exposes_only_the_safe_parallel_recovery() {
        let error = kernel_error(KernelError::capability_execution_failed(
            "AGENT_EXECUTION_ALREADY_ACTIVE",
            "private sqlite path and api_key=secret",
        ));
        let EngineToolError::CapabilityKernel { code, message } = error else {
            panic!("typed capability failure changed error class");
        };
        assert_eq!(code, "AGENT_EXECUTION_ALREADY_ACTIVE");
        assert!(message.contains("strategy=parallel"));
        assert!(message.contains("synthesize=true"));
        assert!(!message.contains("sqlite"));
        assert!(!message.contains("api_key"));
    }

    #[test]
    fn committed_session_generation_can_narrow_but_never_expand_the_snapshot_ceiling() {
        let fixture = kernel_fixture();
        let original = fixture.active.snapshot().unwrap();
        assert_eq!(original.generation, 0);
        assert_eq!(original.active, fixture.snapshot.content().capability_allowlist);
        let mut forged = original.clone();
        forged.active.insert(CapabilityId::from("workspace.vcs"));
        assert_eq!(fixture.active.snapshot().unwrap(), original);
        assert!(compile_engine_tool_plan(
            &fixture.snapshot,
            &forged,
            &fixture.materialized,
            [read_exposure(fixture.action_id.clone())],
        ).is_err());
        forged = original.clone();
        forged.generation = 1;
        assert!(compile_engine_tool_plan(
            &fixture.snapshot,
            &forged,
            &fixture.materialized,
            [read_exposure(fixture.action_id.clone())],
        )
        .is_ok());
        forged = original;
        forged.active.clear();
        assert!(compile_engine_tool_plan(
            &fixture.snapshot,
            &forged,
            &fixture.materialized,
            std::iter::empty(),
        )
        .is_ok());
    }

    #[test]
    fn selected_capability_does_not_override_its_action_allowlist() {
        let fixture = kernel_fixture();
        let mut snapshot = (*fixture.snapshot).clone();
        snapshot.authority_policies.get_mut(&CapabilityId::from("workspace.files"))
            .unwrap().allowed_actions.clear();
        assert!(compile_engine_tool_plan(
            &snapshot,
            &fixture.active.snapshot().unwrap(),
            &fixture.materialized,
            [read_exposure(fixture.action_id)],
        ).is_err());
    }
}
