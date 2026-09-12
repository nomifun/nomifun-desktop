//! Real owner adapter for the official Customer Service Agent capabilities.
//!
//! The selected Wave 4 `customer` resource is the stable `cs_agent_id`. The
//! adapter is constructed for the installation's authoritative owner and
//! rejects every other principal before reading or mutating customer data.
//! Notes writes persist through `CustomerServiceService`; handoff creates an
//! idempotent durable queue row. No branch returns a synthetic `accepted` flag.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_domain_wave4::{
    CUSTOMER_RESOURCE_KIND, CUSTOMER_SERVICE_HANDOFF, CUSTOMER_SERVICE_NOTES_READ,
    CUSTOMER_SERVICE_NOTES_WRITE, Wave4CapabilityOperation, Wave4HostPort, Wave4HostPortError,
    Wave4HostRequest, Wave4TurnMiddlewareHostPort, Wave4TurnMiddlewareHostRequest,
    typed_resource_binding,
};
use nomifun_agent_contracts::{StrictJsonValue, TypedResourceBinding, digest_payload};
use nomifun_common::AppError;
use nomifun_db::CsAgentRow;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::dialogue::build_agent_dialogue_context;
use crate::service::{
    AgentCsNoteWriteInput, CustomerServiceService, RequestCsHandoffInput,
};

const DEFAULT_NOTE_LIMIT: usize = 50;
const MAX_NOTE_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CustomerServiceDialogueContext {
    pub kind: String,
    pub cs_agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cs_dialogue_id: Option<String>,
    pub system_prompt: String,
    pub knowledge_base_ids: Vec<String>,
}

/// Owner-scoped customer-service action and dialogue-context adapter.
pub struct CustomerServiceAgentCapabilityOwner {
    authoritative_owner_id: Arc<str>,
    service: Arc<CustomerServiceService>,
}

impl CustomerServiceAgentCapabilityOwner {
    pub fn new(
        authoritative_owner_id: impl Into<Arc<str>>,
        service: Arc<CustomerServiceService>,
    ) -> Self {
        Self {
            authoritative_owner_id: authoritative_owner_id.into(),
            service,
        }
    }

    pub fn service(&self) -> &Arc<CustomerServiceService> {
        &self.service
    }

    /// Resolve a user selection into the only server-authored `customer`
    /// binding accepted by this owner. Operations must already have been
    /// derived from the selected capability set by the host resolver; this
    /// method rejects anything outside the domain's read/write vocabulary.
    pub async fn resolve_resource_binding(
        &self,
        principal_id: &str,
        cs_agent_id: &str,
        operations: BTreeSet<String>,
    ) -> Result<TypedResourceBinding, Wave4HostPortError> {
        self.require_owner(principal_id)?;
        if operations.is_empty()
            || operations
                .iter()
                .any(|operation| !matches!(operation.as_str(), "read" | "write"))
        {
            return Err(Wave4HostPortError::resource_binding_invalid(
                "customer resource operations must be a non-empty subset of read/write",
            ));
        }
        let agent = self.resolve_agent(cs_agent_id).await?;
        if !agent.enabled {
            return Err(Wave4HostPortError::new(
                "CUSTOMER_SERVICE_AGENT_DISABLED",
                format!("customer-service Agent {} is disabled", agent.cs_agent_id),
            ));
        }
        Ok(typed_resource_binding(
            format!("customer:{}", agent.cs_agent_id),
            CUSTOMER_RESOURCE_KIND,
            agent.cs_agent_id,
            principal_id.to_owned(),
            operations,
        ))
    }

    /// Resolve the non-Tool `customer_service.dialogue` contribution for the
    /// same selected resource used by notes and handoff actions.
    pub async fn dialogue_context(
        &self,
        principal_id: &str,
        customer_resource_id: &str,
    ) -> Result<CustomerServiceDialogueContext, Wave4HostPortError> {
        self.dialogue_context_for_turn(principal_id, customer_resource_id, None)
            .await
    }

    /// Resolve dialogue middleware for a concrete Channel turn. When a
    /// dialogue id is present, it must belong to the selected customer
    /// resource before it is exposed to the model or accepted by handoff.
    pub async fn dialogue_context_for_turn(
        &self,
        principal_id: &str,
        customer_resource_id: &str,
        cs_dialogue_id: Option<&str>,
    ) -> Result<CustomerServiceDialogueContext, Wave4HostPortError> {
        self.require_owner(principal_id)?;
        let agent = self.resolve_agent(customer_resource_id).await?;
        if !agent.enabled {
            return Err(Wave4HostPortError::new(
                "CUSTOMER_SERVICE_AGENT_DISABLED",
                format!("customer-service Agent {} is disabled", agent.cs_agent_id),
            ));
        }
        let cs_dialogue_id = if let Some(cs_dialogue_id) = cs_dialogue_id {
            let dialogue = self
                .service
                .repo()
                .get_dialogue(cs_dialogue_id)
                .await
                .map_err(|error| {
                    Wave4HostPortError::new(
                        "CUSTOMER_SERVICE_DIALOGUE_LOOKUP_FAILED",
                        error.to_string(),
                    )
                })?
                .ok_or_else(|| {
                    Wave4HostPortError::new(
                        "CUSTOMER_SERVICE_DIALOGUE_NOT_FOUND",
                        format!("customer-service dialogue {cs_dialogue_id} was not found"),
                    )
                })?;
            if dialogue.cs_agent_id != agent.cs_agent_id {
                return Err(Wave4HostPortError::resource_owner_mismatch(
                    "customer-service dialogue belongs to another selected customer resource",
                ));
            }
            Some(dialogue.cs_dialogue_id)
        } else {
            None
        };
        Ok(CustomerServiceDialogueContext {
            kind: "customer_service_dialogue".to_owned(),
            cs_agent_id: agent.cs_agent_id.clone(),
            cs_dialogue_id,
            system_prompt: build_agent_dialogue_context(&agent),
            knowledge_base_ids: agent.knowledge_base_ids_vec(),
        })
    }

    fn require_owner(&self, principal_id: &str) -> Result<(), Wave4HostPortError> {
        if principal_id != self.authoritative_owner_id.as_ref() {
            return Err(Wave4HostPortError::resource_owner_mismatch(
                "customer resource belongs to another installation owner",
            ));
        }
        Ok(())
    }

    async fn resolve_agent(
        &self,
        cs_agent_id: &str,
    ) -> Result<CsAgentRow, Wave4HostPortError> {
        self.service
            .get_agent(cs_agent_id)
            .await
            .map_err(map_service_error)
    }

    async fn invoke_owned(
        &self,
        request: Wave4HostRequest,
    ) -> Result<StrictJsonValue, Wave4HostPortError> {
        request.validate()?;
        if request.context.principal.principal_kind != "user" {
            return Err(Wave4HostPortError::resource_owner_mismatch(
                "customer-service Agent capabilities require a user principal",
            ));
        }
        self.require_owner(&request.context.principal.principal_id)?;
        let customer = request
            .context
            .resource_bindings
            .iter()
            .find(|binding| binding.resource_kind.as_ref() == CUSTOMER_RESOURCE_KIND)
            .ok_or_else(|| {
                Wave4HostPortError::resource_not_bound(
                    "customer-service capability requires one selected customer resource",
                )
            })?;
        let cs_agent_id = customer.resource_id.as_ref().to_owned();
        let agent = self.resolve_agent(&cs_agent_id).await?;
        if !agent.enabled {
            return Err(Wave4HostPortError::new(
                "CUSTOMER_SERVICE_AGENT_DISABLED",
                format!("customer-service Agent {} is disabled", agent.cs_agent_id),
            ));
        }

        let value = match request.operation {
            Wave4CapabilityOperation::CustomerServiceNotesRead { input } => {
                let input: NotesReadInput = parse_input(input.0)?;
                let mut notes = self
                    .service
                    .list_notes(Some(&cs_agent_id))
                    .await
                    .map_err(map_service_error)?;
                if !input.include_disabled {
                    notes.retain(|note| note.enabled);
                }
                if let Some(cs_note_id) = input.cs_note_id {
                    notes.retain(|note| note.cs_note_id == cs_note_id);
                    if notes.is_empty() {
                        return Err(Wave4HostPortError::new(
                            "CUSTOMER_SERVICE_NOTE_NOT_FOUND",
                            "the selected customer resource cannot read that note",
                        ));
                    }
                }
                notes.truncate(input.limit.unwrap_or(DEFAULT_NOTE_LIMIT).clamp(1, MAX_NOTE_LIMIT));
                json!({ "cs_agent_id": cs_agent_id, "notes": notes })
            }
            Wave4CapabilityOperation::CustomerServiceNotesWrite { input } => {
                let request_value = input.0;
                let request_digest = digest_payload(&json!({
                    "cs_agent_id": cs_agent_id.clone(),
                    "input": request_value.clone(),
                }))
                .map_err(|error| {
                    Wave4HostPortError::invalid_request(format!(
                        "notes.write request could not be digested: {error}"
                    ))
                })?;
                let input: NotesWriteInput = parse_input(request_value)?;
                let receipt = self
                    .service
                    .write_note_idempotent(AgentCsNoteWriteInput {
                        owner_user_id: request.context.principal.principal_id,
                        cs_agent_id: cs_agent_id.clone(),
                        capability_id: CUSTOMER_SERVICE_NOTES_WRITE.to_owned(),
                        idempotency_key: request.context.idempotency_key.as_ref().to_owned(),
                        request_digest: request_digest.as_ref().to_owned(),
                        cs_note_id: input.cs_note_id,
                        kind: input.kind,
                        content: input.content,
                        aliases: input.aliases,
                        enabled: input.enabled,
                    })
                    .await
                    .map_err(map_service_error)?;
                json!({
                    "cs_agent_id": cs_agent_id,
                    "receipt_id": receipt.cs_agent_capability_receipt_id,
                    "replayed": receipt.replayed,
                    "note": receipt.note,
                })
            }
            Wave4CapabilityOperation::CustomerServiceHandoff { input } => {
                let input: HandoffInput = parse_input(input.0)?;
                let result = self
                    .service
                    .request_handoff(RequestCsHandoffInput {
                        cs_agent_id: cs_agent_id.clone(),
                        cs_dialogue_id: input.cs_dialogue_id,
                        requested_by: request.context.principal.principal_id,
                        idempotency_key: request.context.idempotency_key.as_ref().to_owned(),
                        reason: input.reason,
                        summary: input.summary.unwrap_or_default(),
                    })
                    .await
                    .map_err(map_service_error)?;
                json!({
                    "cs_agent_id": cs_agent_id,
                    "created": result.created,
                    "handoff": result.handoff,
                })
            }
            _ => {
                return Err(Wave4HostPortError::action_operation_mismatch(
                    "customer-service owner received a non-customer-service operation",
                ));
            }
        };
        Ok(StrictJsonValue(value))
    }
}

impl Wave4HostPort for CustomerServiceAgentCapabilityOwner {
    fn invoke<'a>(
        &'a self,
        request: Wave4HostRequest,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        StrictJsonValue,
                        Wave4HostPortError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move { self.invoke_owned(request).await })
    }
}

impl Wave4TurnMiddlewareHostPort for CustomerServiceAgentCapabilityOwner {
    fn apply<'a>(
        &'a self,
        request: Wave4TurnMiddlewareHostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>> {
        Box::pin(async move {
            request.validate()?;
            if request.capability_id.as_ref()
                != nomifun_agent_domain_wave4::CUSTOMER_SERVICE_DIALOGUE
            {
                return Err(Wave4HostPortError::action_operation_mismatch(
                    "customer-service middleware owner received another capability",
                ));
            }
            if request.principal.principal_kind != "user" {
                return Err(Wave4HostPortError::resource_owner_mismatch(
                    "customer-service dialogue requires a user principal",
                ));
            }
            let customer = request
                .resource_bindings
                .iter()
                .find(|binding| binding.resource_kind.as_ref() == CUSTOMER_RESOURCE_KIND)
                .ok_or_else(|| {
                    Wave4HostPortError::resource_not_bound(
                        "customer-service dialogue requires one selected customer resource",
                    )
                })?;
            let cs_dialogue_id = request
                .turn_input
                .0
                .get("cs_dialogue_id")
                .and_then(Value::as_str);
            let context = self
                .dialogue_context_for_turn(
                    &request.principal.principal_id,
                    customer.resource_id.as_ref(),
                    cs_dialogue_id,
                )
                .await?;
            serde_json::to_value(context)
                .map(StrictJsonValue)
                .map_err(|error| {
                    Wave4HostPortError::new(
                        "CUSTOMER_SERVICE_CONTEXT_SERIALIZATION_FAILED",
                        error.to_string(),
                    )
                })
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NotesReadInput {
    #[serde(default)]
    cs_note_id: Option<String>,
    #[serde(default)]
    include_disabled: bool,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NotesWriteInput {
    #[serde(default)]
    cs_note_id: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    aliases: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffInput {
    cs_dialogue_id: String,
    reason: String,
    #[serde(default)]
    summary: Option<String>,
}

fn parse_input<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, Wave4HostPortError> {
    serde_json::from_value(value)
        .map_err(|error| Wave4HostPortError::invalid_request(error.to_string()))
}

fn map_service_error(error: AppError) -> Wave4HostPortError {
    Wave4HostPortError::new(
        format!("CUSTOMER_SERVICE_{}", error.error_code()),
        error.to_string(),
    )
}

/// Canonical strict input schema bytes for Nomi's bundled Tool bridge.
pub fn customer_service_action_input_schema(capability_id: &str) -> Option<Value> {
    match capability_id {
        CUSTOMER_SERVICE_NOTES_READ | CUSTOMER_SERVICE_NOTES_WRITE | CUSTOMER_SERVICE_HANDOFF => {
            Some(nomifun_agent_domain_wave4::action_input_schema(capability_id).0)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_agent_contracts::{
        ActionId, AgentSessionId, CapabilityId, CorrelationId, DigestHex, IdempotencyKey,
        OperationId, PrincipalRef, ResolvedSnapshotRef, ResourceBindingId, ResourceId,
        ResourceKind, ScopeKey, TypedResourceBinding,
    };
    use nomifun_agent_domain_wave4::{
        CUSTOMER_SERVICE_HANDOFF_ACTION, CUSTOMER_SERVICE_NOTES_READ_ACTION,
        CUSTOMER_SERVICE_NOTES_WRITE_ACTION, WAVE4_RESOURCE_OWNER_MISMATCH, WAVE4_RESOURCE_NOT_BOUND,
    };
    use nomifun_common::{ChannelPluginId, ChannelUserId, UserId};
    use nomifun_db::{CsDialogueKey, ICustomerServiceRepository, SqliteCustomerServiceRepository};

    use super::*;
    use crate::service::CreateCsAgentInput;

    async fn fixture() -> (
        nomifun_db::Database,
        Arc<CustomerServiceService>,
        Arc<dyn ICustomerServiceRepository>,
        String,
        String,
        String,
    ) {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let repo: Arc<dyn ICustomerServiceRepository> = Arc::new(
            SqliteCustomerServiceRepository::new(database.pool().clone()),
        );
        let service = Arc::new(CustomerServiceService::new(Arc::clone(&repo)));
        let agent = service
            .create_agent(CreateCsAgentInput {
                name: "Official Customer Service".into(),
                greeting: "Hello".into(),
                persona: "Calm".into(),
                service_policy: "Use owned facts".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let dialogue = repo
            .get_or_create_dialogue(
                &agent.cs_agent_id,
                &CsDialogueKey {
                    channel_plugin_id: ChannelPluginId::new().into_string(),
                    channel_user_id: ChannelUserId::new().into_string(),
                    chat_id: "customer-chat".into(),
                },
                1,
            )
            .await
            .unwrap();
        (
            database,
            service,
            repo,
            UserId::new().into_string(),
            agent.cs_agent_id,
            dialogue.cs_dialogue_id,
        )
    }

    fn request(
        owner_id: &str,
        cs_agent_id: &str,
        capability_id: &str,
        action_id: &str,
        operation: Wave4CapabilityOperation,
        binding_operation: &str,
        idempotency_key: &str,
    ) -> Wave4HostRequest {
        Wave4HostRequest {
            context: nomifun_agent_domain_wave4::Wave4HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".into(),
                    principal_id: owner_id.into(),
                },
                agent_session_id: AgentSessionId::from(nomifun_common::generate_id()),
                operation_id: OperationId::from(nomifun_common::generate_id()),
                idempotency_key: IdempotencyKey::from(idempotency_key),
                correlation_id: CorrelationId::from(nomifun_common::generate_id()),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: "customer-snapshot".into(),
                    snapshot_digest: "a".repeat(64).into(),
                },
                registry_generation: 1,
                capability_id: CapabilityId::from(capability_id),
                action_id: ActionId::from(action_id),
                state_scope_key: ScopeKey::from("session:customer-service-test"),
                resource_bindings: vec![TypedResourceBinding {
                    binding_id: ResourceBindingId::from("selected-customer"),
                    resource_kind: ResourceKind::from(CUSTOMER_RESOURCE_KIND),
                    resource_id: ResourceId::from(cs_agent_id.to_owned()),
                    owner_id: owner_id.to_owned(),
                    operations: BTreeSet::from([binding_operation.to_owned()]),
                    connection_config_ref: None,
                    typed_parameters: BTreeMap::new(),
                }],
            },
            operation,
        }
    }

    #[tokio::test]
    async fn notes_write_and_read_persist_only_inside_the_selected_customer_resource() {
        let (_database, service, _repo, owner_id, cs_agent_id, _dialogue_id) = fixture().await;
        let owner = CustomerServiceAgentCapabilityOwner::new(
            owner_id.clone(),
            Arc::clone(&service),
        );
        let create = owner
            .invoke(request(
                &owner_id,
                &cs_agent_id,
                CUSTOMER_SERVICE_NOTES_WRITE,
                CUSTOMER_SERVICE_NOTES_WRITE_ACTION,
                Wave4CapabilityOperation::CustomerServiceNotesWrite {
                    input: StrictJsonValue(json!({
                        "kind": "faq",
                        "content": "A persisted customer answer",
                        "aliases": "customer question"
                    })),
                },
                "write",
                "notes-write-1",
            ))
            .await
            .unwrap();
        let created_note_id = create.0["note"]["cs_note_id"]
            .as_str()
            .expect("created note id")
            .to_owned();
        assert_eq!(
            service.list_notes(Some(&cs_agent_id)).await.unwrap()[0].content,
            "A persisted customer answer"
        );

        let read = owner
            .invoke(request(
                &owner_id,
                &cs_agent_id,
                CUSTOMER_SERVICE_NOTES_READ,
                CUSTOMER_SERVICE_NOTES_READ_ACTION,
                Wave4CapabilityOperation::CustomerServiceNotesRead {
                    input: StrictJsonValue(json!({ "cs_note_id": created_note_id })),
                },
                "read",
                "notes-read-1",
            ))
            .await
            .unwrap();
        assert_eq!(read.0["notes"].as_array().unwrap().len(), 1);
        assert_eq!(read.0["notes"][0]["content"], "A persisted customer answer");

        let foreign_owner = UserId::new().into_string();
        let error = owner
            .invoke(request(
                &foreign_owner,
                &cs_agent_id,
                CUSTOMER_SERVICE_NOTES_READ,
                CUSTOMER_SERVICE_NOTES_READ_ACTION,
                Wave4CapabilityOperation::CustomerServiceNotesRead {
                    input: StrictJsonValue(json!({})),
                },
                "read",
                "foreign-read",
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code, WAVE4_RESOURCE_OWNER_MISMATCH);
    }

    #[tokio::test]
    async fn notes_write_receipt_survives_response_loss_restart_and_rejects_key_reuse() {
        let (database, service, _repo, owner_id, cs_agent_id, _dialogue_id) = fixture().await;
        let owner = CustomerServiceAgentCapabilityOwner::new(
            owner_id.clone(),
            Arc::clone(&service),
        );
        let write_request = |content: &str| {
            request(
                &owner_id,
                &cs_agent_id,
                CUSTOMER_SERVICE_NOTES_WRITE,
                CUSTOMER_SERVICE_NOTES_WRITE_ACTION,
                Wave4CapabilityOperation::CustomerServiceNotesWrite {
                    input: StrictJsonValue(json!({
                        "kind": "fact",
                        "content": content,
                    })),
                },
                "write",
                "notes-write-response-lost",
            )
        };

        // The caller loses this response after the transaction commits.
        let committed = owner
            .invoke(write_request("one durable fact"))
            .await
            .unwrap();
        assert_eq!(committed.0["replayed"], false);
        let note_id = committed.0["note"]["cs_note_id"].clone();
        let receipt_id = committed.0["receipt_id"].clone();

        // Reconstruct every service/owner handle over the same database to
        // model a process restart before the caller retries.
        let restarted_repo: Arc<dyn ICustomerServiceRepository> = Arc::new(
            SqliteCustomerServiceRepository::new(database.pool().clone()),
        );
        let restarted_service = Arc::new(CustomerServiceService::new(restarted_repo));
        let restarted = CustomerServiceAgentCapabilityOwner::new(
            owner_id.clone(),
            Arc::clone(&restarted_service),
        );
        let replay = restarted
            .invoke(write_request("one durable fact"))
            .await
            .unwrap();
        assert_eq!(replay.0["replayed"], true);
        assert_eq!(replay.0["note"]["cs_note_id"], note_id);
        assert_eq!(replay.0["receipt_id"], receipt_id);
        assert_eq!(
            restarted_service
                .list_notes(Some(&cs_agent_id))
                .await
                .unwrap()
                .len(),
            1,
        );
        let receipt_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cs_agent_capability_receipts \
             WHERE owner_user_id = ? AND capability_id = ? AND idempotency_key = ?",
        )
        .bind(&owner_id)
        .bind(CUSTOMER_SERVICE_NOTES_WRITE)
        .bind("notes-write-response-lost")
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(receipt_count, 1);

        let conflict = restarted
            .invoke(write_request("different fact under the same key"))
            .await
            .unwrap_err();
        assert_eq!(conflict.code, "CUSTOMER_SERVICE_CONFLICT");
        assert_eq!(
            restarted_service
                .list_notes(Some(&cs_agent_id))
                .await
                .unwrap()
                .len(),
            1,
        );
    }

    #[tokio::test]
    async fn cancelled_notes_write_rolls_back_before_retry_commits_once() {
        let (database, service, _repo, owner_id, cs_agent_id, _dialogue_id) = fixture().await;
        let mut blocker = database.pool().begin().await.unwrap();
        sqlx::query("UPDATE cs_agents SET updated_at = updated_at WHERE cs_agent_id = ?")
            .bind(&cs_agent_id)
            .execute(&mut *blocker)
            .await
            .unwrap();

        let owner = Arc::new(CustomerServiceAgentCapabilityOwner::new(
            owner_id.clone(),
            Arc::clone(&service),
        ));
        let build_request = || {
            request(
                &owner_id,
                &cs_agent_id,
                CUSTOMER_SERVICE_NOTES_WRITE,
                CUSTOMER_SERVICE_NOTES_WRITE_ACTION,
                Wave4CapabilityOperation::CustomerServiceNotesWrite {
                    input: StrictJsonValue(json!({ "content": "cancel-safe fact" })),
                },
                "write",
                "notes-write-cancelled-before-commit",
            )
        };
        let pending_owner = Arc::clone(&owner);
        let pending_request = build_request();
        let pending = tokio::spawn(async move { pending_owner.invoke(pending_request).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!pending.is_finished(), "notes.write should be waiting on the held DB writer");
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        blocker.rollback().await.unwrap();

        let committed = owner.invoke(build_request()).await.unwrap();
        assert_eq!(committed.0["replayed"], false);
        let replay = owner.invoke(build_request()).await.unwrap();
        assert_eq!(replay.0["replayed"], true);
        assert_eq!(
            service.list_notes(Some(&cs_agent_id)).await.unwrap().len(),
            1,
        );
    }

    #[tokio::test]
    async fn handoff_action_creates_one_real_queue_row_and_replay_returns_it() {
        let (_database, service, _repo, owner_id, cs_agent_id, dialogue_id) = fixture().await;
        let owner = CustomerServiceAgentCapabilityOwner::new(
            owner_id.clone(),
            Arc::clone(&service),
        );
        let invoke = || {
            owner.invoke(request(
                &owner_id,
                &cs_agent_id,
                CUSTOMER_SERVICE_HANDOFF,
                CUSTOMER_SERVICE_HANDOFF_ACTION,
                Wave4CapabilityOperation::CustomerServiceHandoff {
                    input: StrictJsonValue(json!({
                        "cs_dialogue_id": dialogue_id,
                        "reason": "visitor requested a human",
                        "summary": "billing"
                    })),
                },
                "write",
                "handoff-idempotency-1",
            ))
        };
        let first = invoke().await.unwrap();
        assert_eq!(first.0["created"], true);
        assert_eq!(first.0["handoff"]["status"], "pending");
        let second = invoke().await.unwrap();
        assert_eq!(second.0["created"], false);
        assert_eq!(
            first.0["handoff"]["cs_handoff_id"],
            second.0["handoff"]["cs_handoff_id"]
        );
        assert_eq!(
            service
                .list_handoffs(&cs_agent_id, Some("pending"), 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn dialogue_context_requires_the_authoritative_owner_and_live_resource() {
        let (_database, service, _repo, owner_id, cs_agent_id, dialogue_id) = fixture().await;
        let owner = CustomerServiceAgentCapabilityOwner::new(owner_id.clone(), service);
        let context = owner
            .dialogue_context(&owner_id, &cs_agent_id)
            .await
            .unwrap();
        assert_eq!(context.cs_agent_id, cs_agent_id);
        assert!(context.system_prompt.contains("Official Customer Service"));
        let binding = owner
            .resolve_resource_binding(
                &owner_id,
                &cs_agent_id,
                BTreeSet::from(["read".to_owned(), "write".to_owned()]),
            )
            .await
            .unwrap();
        assert_eq!(binding.resource_kind.as_ref(), CUSTOMER_RESOURCE_KIND);
        assert_eq!(binding.resource_id.as_ref(), cs_agent_id);
        assert_eq!(binding.owner_id, owner_id);
        assert!(binding.typed_parameters.is_empty());
        assert!(binding.connection_config_ref.is_none());
        let registration =
            nomifun_agent_domain_wave4::customer_service_registration().unwrap();
        let schema_ref = registration
            .metadata
            .manifest
            .payload
            .contributions
            .capabilities
            .iter()
            .find(|capability| {
                capability.id.as_ref()
                    == nomifun_agent_domain_wave4::CUSTOMER_SERVICE_DIALOGUE
            })
            .and_then(|capability| capability.contributions.context_schema_refs.first())
            .cloned()
            .unwrap();
        let middleware = owner
            .apply(Wave4TurnMiddlewareHostRequest {
                principal: PrincipalRef {
                    principal_kind: "user".into(),
                    principal_id: owner_id.clone(),
                },
                agent_session_id: AgentSessionId::from(nomifun_common::generate_id()),
                operation_id: OperationId::from(nomifun_common::generate_id()),
                correlation_id: CorrelationId::from(nomifun_common::generate_id()),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: "customer-middleware-snapshot".into(),
                    snapshot_digest: "b".repeat(64).into(),
                },
                registry_generation: 1,
                registry_digest: DigestHex::from("c".repeat(64)),
                capability_id: CapabilityId::from(
                    nomifun_agent_domain_wave4::CUSTOMER_SERVICE_DIALOGUE,
                ),
                state_scope_key: ScopeKey::from("session:customer-middleware"),
                resource_bindings: vec![binding],
                schema_ref,
                turn_input: StrictJsonValue(json!({
                    "text": "hello",
                    "cs_dialogue_id": dialogue_id
                })),
            })
            .await
            .unwrap();
        assert_eq!(middleware.0["kind"], "customer_service_dialogue");
        assert_eq!(middleware.0["cs_agent_id"], cs_agent_id);
        assert_eq!(middleware.0["cs_dialogue_id"], dialogue_id);

        let error = owner
            .dialogue_context(&UserId::new().into_string(), &cs_agent_id)
            .await
            .unwrap_err();
        assert_eq!(error.code, WAVE4_RESOURCE_OWNER_MISMATCH);

        let unbound = Wave4HostPortError::resource_not_bound("customer required");
        assert_eq!(unbound.code, WAVE4_RESOURCE_NOT_BOUND);
    }
}
