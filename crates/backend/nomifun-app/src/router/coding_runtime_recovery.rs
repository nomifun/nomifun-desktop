//! Conservative Coding restart recovery. The boot provider establishes that
//! the exact prior backend died; this module audits the engine's write-ahead
//! events. Unknown command-tree cleanup is never inferred from an empty map.
use async_trait::async_trait;
use nomifun_agent_contracts::CapabilityId;
use nomifun_api_types::RuntimeEngineBinding;
use nomifun_coding_engine::CodingEngineEvent;
use nomifun_conversation::terminal_proof::{
    RegisteredEngineRestartRecovery, TerminalProofDecision,
};
use nomifun_db::{SqlitePool, sqlx};

pub(crate) struct CodingRestartRecovery {
    pool: SqlitePool,
}

impl CodingRestartRecovery {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn recover(
        &self,
        binding: &RuntimeEngineBinding,
        user: &str,
        conversation: &str,
        epoch: i64,
        operation: &str,
    ) -> Result<String, String> {
        if binding.profile != "coding" {
            return Err("unknown Coding profile".into());
        }
        let mut tx = self.pool.begin().await.map_err(|error| error.to_string())?;
        let extra: Option<(String, String)> = sqlx::query_as(
            "SELECT c.extra, r.message_id FROM conversations c JOIN conversation_delivery_receipts r ON r.operation_id = c.active_turn_operation_id \
             WHERE c.conversation_id = ? AND c.user_id = ? AND c.admission_epoch = ? AND c.active_turn_operation_id = ? \
             AND c.status = 'running' AND r.status = 'accepted' AND r.kind = 'turn' AND r.conversation_id = c.conversation_id AND r.user_id = c.user_id")
            .bind(conversation).bind(user).bind(epoch).bind(operation).fetch_optional(&mut *tx).await.map_err(|error| error.to_string())?;
        let (extra, root_message_id) = extra.ok_or("orphan generation or receipt changed")?;
        let extra: serde_json::Value =
            serde_json::from_str(&extra).map_err(|error| error.to_string())?;
        let saved: RuntimeEngineBinding = serde_json::from_value(
            extra
                .get(nomifun_api_types::RUNTIME_ENGINE_BINDING_KEY)
                .cloned()
                .ok_or("missing engine binding")?,
        )
        .map_err(|error| error.to_string())?;
        if saved != *binding {
            return Err("orphan engine identity changed".into());
        }
        let (hosted_pending,): (i64,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM conversation_hosted_effects WHERE user_id = ? AND conversation_id = ? AND state = 'pending')")
            .bind(user).bind(conversation).fetch_one(&mut *tx).await.map_err(|error| error.to_string())?;
        if hosted_pending != 0 {
            return Err(
                "Hosted effect outcome is unknown; never infer settlement from an engine journal"
                    .into(),
            );
        }
        let hosted_rows: Vec<(String, String, String, i64, String, String)> = sqlx::query_as(
            "SELECT operation_id, capability_id, action_name, admission_epoch, state, owner_domain FROM conversation_hosted_effects \
             WHERE user_id = ? AND conversation_id = ? AND turn_operation_id = ? LIMIT 513")
            .bind(user).bind(conversation).bind(operation).fetch_all(&mut *tx).await.map_err(|error| error.to_string())?;
        if hosted_rows.len() > 512 {
            return Err("Hosted receipt bound exceeded".into());
        }
        let mut hosted_receipts = std::collections::BTreeMap::new();
        let mut hosted_actions = std::collections::BTreeSet::new();
        let mut robot_receipts = std::collections::BTreeMap::new();
        let mut git_receipts = std::collections::BTreeSet::new();
        for (tool_operation, capability, action, receipt_epoch, state, domain) in hosted_rows {
            if receipt_epoch != epoch
                || !matches!(domain.as_str(), "miniapp" | "robot" | "git")
                || !matches!(state.as_str(), "returned" | "rejected")
            {
                return Err("Hosted owner receipt has different or uncertain authority".into());
            }
            if domain == "git" {
                if capability != "vcs.push"
                    || action != "vcs.push.invoke"
                    || !git_receipts.insert(tool_operation)
                {
                    return Err("Git receipt has invalid or duplicate frozen identity".into());
                }
                continue;
            }
            if domain == "robot" {
                if !super::nomi_core_robot::tool_capability_ids()
                    .contains(&CapabilityId::from(capability.clone()))
                    || action.is_empty()
                    || action.len() > 64
                    || robot_receipts
                        .insert(tool_operation, (capability, action))
                        .is_some()
                {
                    return Err("Robot receipt has invalid or duplicate frozen identity".into());
                }
                continue;
            }
            let target = (capability, action);
            hosted_actions.insert(target.clone());
            if hosted_receipts.insert(tool_operation, target).is_some() {
                return Err("Duplicate hosted effect operation".into());
            }
        }
        // Check in the recovery transaction as well as the boot provider: this
        // exact build may close settled MCP history, never infer remote cleanup
        // from a dead process or an engine-authored ToolCompleted event.
        let (pending,): (i64,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM conversation_mcp_effects \
            WHERE user_id = ? AND conversation_id = ? AND state = 'pending')",
        )
        .bind(user)
        .bind(conversation)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
        if pending != 0 {
            return Err("MCP remote outcome or cleanup is unknown; never replay".into());
        }
        let mcp_rows: Vec<(String, String, i64, String)> = sqlx::query_as(
            "SELECT operation_id, capability_id, admission_epoch, state FROM conversation_mcp_effects \
             WHERE user_id = ? AND conversation_id = ? AND turn_operation_id = ? LIMIT 513")
            .bind(user).bind(conversation).bind(operation).fetch_all(&mut *tx).await.map_err(|error| error.to_string())?;
        if mcp_rows.len() > 512 {
            return Err("MCP receipt bound exceeded".into());
        }
        let mut mcp_receipts = std::collections::BTreeMap::new();
        let mut resource_receipts = std::collections::BTreeSet::new();
        for (tool_operation, capability, receipt_epoch, state) in mcp_rows {
            if capability == "mcp.resource" {
                if receipt_epoch != epoch
                    || state != "settled"
                    || !resource_receipts.insert(tool_operation)
                {
                    return Err("MCP resource receipt has different or uncertain authority".into());
                }
                continue;
            }
            if receipt_epoch != epoch
                || state != "settled"
                || !super::nomi_core_mcp_catalog::is_product_tool(&capability)
                || mcp_receipts.insert(tool_operation, capability).is_some()
            {
                return Err("MCP owner receipt has different or uncertain authority".into());
            }
        }
        let (count, bytes): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(length(CAST(event_json AS BLOB))), 0) FROM conversation_runtime_events WHERE conversation_id = ? AND turn_operation_id = ?")
            .bind(conversation).bind(operation).fetch_one(&mut *tx).await.map_err(|error| error.to_string())?;
        if !(1..=4096).contains(&count) || bytes > 8 * 1024 * 1024 {
            return Err("missing or oversized engine evidence journal".into());
        }
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT sequence, event_json FROM conversation_runtime_events WHERE conversation_id = ? AND turn_operation_id = ? ORDER BY sequence LIMIT 4097")
            .bind(conversation).bind(operation).fetch_all(&mut *tx).await.map_err(|error| error.to_string())?;
        if rows.is_empty()
            || rows.len() > 4096
            || rows.iter().map(|(_, raw)| raw.len()).sum::<usize>() > 8 * 1024 * 1024
        {
            return Err("missing or oversized engine evidence journal".into());
        }
        let mut sequence = 0;
        let mut process_audit = super::engine_process_recovery::ProcessRecoveryAudit::default();
        let mut cleanup_proven = false;
        let mut terminal = false;
        let mut model_steps = 0;
        let mut input_scope = false;
        let mut input_receipts = std::collections::BTreeSet::new();
        let mut mcp_calls = std::collections::BTreeMap::<String, String>::new();
        let mut mcp_dispatches = std::collections::BTreeSet::new();
        let mut mcp_operations = std::collections::BTreeSet::new();
        let mut hosted_calls = std::collections::BTreeMap::new();
        let mut hosted_dispatches = std::collections::BTreeSet::new();
        let mut hosted_operations = std::collections::BTreeSet::new();
        let mut robot_calls = std::collections::BTreeMap::new();
        let mut robot_dispatches = std::collections::BTreeSet::new();
        let mut robot_operations = std::collections::BTreeSet::new();
        let mut git_calls = std::collections::BTreeSet::new();
        let mut git_dispatches = std::collections::BTreeSet::new();
        let mut git_operations = std::collections::BTreeSet::new();
        let mut resource_dispatches = std::collections::BTreeMap::new();
        let mut resource_settlements = std::collections::BTreeSet::new();
        let mut resource_calls = std::collections::BTreeSet::new();
        let mut model_operations = std::collections::BTreeSet::new();
        let mut turn_snapshot = None;
        for (index, (seq, raw)) in rows.iter().enumerate() {
            if *seq != sequence + 1 {
                return Err("engine journal sequence is incomplete".into());
            }
            sequence = *seq;
            let value: serde_json::Value =
                serde_json::from_str(raw).map_err(|error| error.to_string())?;
            match value.get("event").and_then(|value| value.as_str()) {
                Some("host_process_dispatch" | "host_process_quiescent") if index > 0 && !terminal && !cleanup_proven => {
                    let witness = serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
                    process_audit.observe(witness)?;
                    continue;
                }
                Some("host_resource_dispatch") if index > 0 && !terminal && !cleanup_proven => {
                    let field = |key: &str| -> Result<&str, String> {
                        value
                            .get(key)
                            .and_then(serde_json::Value::as_str)
                            .filter(|value| {
                                !value.trim().is_empty()
                                    && value.len() <= 1024
                                    && !value.chars().any(char::is_control)
                            })
                            .ok_or_else(|| format!("invalid resource dispatch {key}"))
                    };
                    let operation_id = field("operation_id")?;
                    let call_id = field("call_id")?;
                    let model_id = field("model_operation_id")?;
                    let request_digest = field("request_sha256")?;
                    let resource_binding = field("resource_binding_id")?;
                    let causality: nomifun_chat_model_broker::ChatCausality =
                        serde_json::from_value(value["causality"].clone())
                            .map_err(|error| error.to_string())?;
                    let expected_operation = nomifun_agent_contracts::digest_payload(&(
                        "engine-resource-v1",
                        &causality,
                        call_id,
                    ))
                    .map_err(|error| error.to_string())?;
                    if field("capability_id")? != "mcp.resource"
                        || !resource_binding.starts_with("mcp_server:")
                        || call_id.len() > 256
                        || request_digest.len() != 64
                        || !request_digest
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        || operation_id
                            != format!("engine-resource-{}", expected_operation.as_ref())
                        || causality.agent_session_id.as_ref() != conversation
                        || causality.turn_operation_id.as_ref() != operation
                        || causality.causation_event_id.as_ref() != root_message_id
                        || causality.operation_id.as_ref() != model_id
                        || Some(&causality.resolved_snapshot_ref) != turn_snapshot.as_ref()
                        || !model_operations.contains(model_id)
                        || resource_dispatches.len() >= 64
                        || resource_dispatches.contains_key(operation_id)
                        || !resource_calls.insert((model_id.to_owned(), call_id.to_owned()))
                    {
                        return Err(
                            "MCP resource dispatch differs from its admitted model turn".into()
                        );
                    }
                    let (claimed,): (i64,) = sqlx::query_as(
                        "SELECT EXISTS(SELECT 1 FROM conversation_runtime_events WHERE conversation_id = ? AND turn_operation_id = ? AND model_operation_id = ? AND model_claimed = 1 AND sequence < ?)")
                        .bind(conversation).bind(operation).bind(model_id).bind(seq)
                        .fetch_one(&mut *tx).await.map_err(|error| error.to_string())?;
                    if claimed != 1 {
                        return Err("MCP resource has no prior claimed model operation".into());
                    }
                    // The exact compiled owner reserves BEFORE any connection.
                    // Missing receipt means no owner transaction started; a
                    // settled receipt (including a returned resource rejection)
                    // closes history, never proves success or authorizes replay.
                    let owner_returned = resource_receipts.remove(operation_id);
                    resource_dispatches.insert(operation_id.to_owned(), owner_returned);
                    continue;
                }
                Some("host_resource_settled") if index > 0 && !terminal && !cleanup_proven => {
                    let operation_id = value["operation_id"]
                        .as_str()
                        .ok_or("resource settlement has no operation")?;
                    let owner_returned = value["owner_returned"]
                        .as_bool()
                        .ok_or("resource settlement has no outcome")?;
                    if resource_dispatches.get(operation_id).copied() != Some(owner_returned)
                        || !resource_settlements.insert(operation_id.to_owned())
                    {
                        return Err("MCP resource settlement differs from its owner receipt".into());
                    }
                    continue;
                }
                Some("host_tool_dispatch") if index > 0 && !terminal && !cleanup_proven => {
                    let dispatch: super::engine_tool_host::EngineToolDispatchRecord =
                        serde_json::from_value(value["dispatch"].clone())
                            .map_err(|error| error.to_string())?;
                    // Intent may precede Kernel admission. Conservatively audit
                    // it exactly like ToolStarted, never infer that it ran.
                    match dispatch.capability_id.as_str() {
                        "vcs.push" => {
                            if dispatch.action_id != "vcs.push.invoke"
                                || dispatch.model_name != "git_push"
                                || !git_calls.contains(&dispatch.call_id)
                                || !git_dispatches.insert(dispatch.call_id.clone())
                                || !git_operations.insert(dispatch.operation_id.clone())
                            {
                                return Err("Git dispatch differs from its tool admission".into());
                            }
                            // This exact build's Conversation owner durably
                            // reserves BEFORE push. No receipt means admission
                            // stopped before dispatch; pending was rejected above.
                            // Returned closes history, NEVER authorizes replay.
                            git_receipts.remove(&dispatch.operation_id);
                        }
                        "process.exec" => process_audit.dispatch(&dispatch)?,
                        "fs.read" | "fs.search" | "fs.write" | "fs.patch" | "fs.delete"
                        | "fs.snapshot" | "vcs.status" | "vcs.diff" | "vcs.stage"
                        | "vcs.commit" => {}
                        id if super::nomi_core_mcp_catalog::is_product_tool(id) => {
                            if dispatch.action_id != format!("{id}.invoke")
                                || mcp_calls.get(&dispatch.call_id).map(String::as_str) != Some(id)
                                || !mcp_dispatches.insert(dispatch.call_id.clone())
                                || !mcp_operations.insert(dispatch.operation_id.clone())
                            {
                                return Err("MCP dispatch differs from its tool admission".into());
                            }
                            if let Some(recorded) = mcp_receipts.remove(&dispatch.operation_id) {
                                if recorded != id {
                                    return Err(
                                        "MCP receipt differs from dispatched capability".into()
                                    );
                                }
                            }
                            // With this exact compiled owner, no receipt means
                            // no remote transaction started. The write-ahead
                            // intent itself grants neither execution nor replay.
                        }
                        id if super::nomi_core_robot::tool_capability_ids()
                            .contains(&CapabilityId::from(id)) =>
                        {
                            if dispatch.action_id != format!("{id}.invoke")
                                || robot_calls.get(&dispatch.call_id).map(String::as_str)
                                    != Some(id)
                                || !robot_dispatches.insert(dispatch.call_id.clone())
                                || !robot_operations.insert(dispatch.operation_id.clone())
                                || robot_receipts.remove(&dispatch.operation_id)
                                    != Some((id.to_owned(), dispatch.model_name.clone()))
                            {
                                return Err("Robot dispatch lacks its exact settled device-owner receipt; preserve quarantine".into());
                            }
                            // Only closes the host request history. A returned
                            // move/audio command is NOT a physical-stop witness.
                        }
                        id if hosted_actions
                            .contains(&(id.to_owned(), dispatch.action_id.clone())) =>
                        {
                            let target = (id.to_owned(), dispatch.action_id.clone());
                            if hosted_calls.get(&dispatch.call_id) != Some(&target)
                                || !hosted_dispatches.insert(dispatch.call_id.clone())
                                || !hosted_operations.insert(dispatch.operation_id.clone())
                                || hosted_receipts.remove(&dispatch.operation_id) != Some(target)
                            {
                                return Err("Plugin Product dispatch lacks its exact returned/rejected owner receipt".into());
                            }
                        }
                        _ => {
                            return Err("dispatched tool owner has no Coding recovery audit".into());
                        }
                    }
                    continue;
                }
                Some("host_tool_settled") if index > 0 && !terminal => continue,
                Some("host_cleanup_proven") if index > 0 && !terminal => {
                    let recorded: RuntimeEngineBinding =
                        serde_json::from_value(value["binding"].clone())
                            .map_err(|error| error.to_string())?;
                    if recorded != *binding
                        || value["epoch"].as_i64() != Some(epoch)
                        || value["operation"].as_str() != Some(operation)
                    {
                        return Err("cleanup witness has different authority".into());
                    }
                    cleanup_proven = true;
                    continue;
                }
                _ => {}
            }
            let event: CodingEngineEvent =
                serde_json::from_value(value).map_err(|error| error.to_string())?;
            if index == 0 {
                let CodingEngineEvent::TurnStarted {
                    binding: recorded,
                    turn_operation_id,
                } = &event
                else {
                    return Err("missing turn authority event".into());
                };
                if recorded.family_id().as_ref() != binding.family_id
                    || recorded.build_id().as_ref() != binding.build_id
                    || recorded.build_digest().as_ref() != binding.build_digest
                    || recorded.agent_session_id().as_ref() != conversation
                    || turn_operation_id.as_ref() != operation
                {
                    return Err("turn evidence belongs to a different engine/generation".into());
                }
                turn_snapshot = Some(recorded.resolved_snapshot_ref().clone());
            } else if matches!(&event, CodingEngineEvent::TurnStarted { .. }) || terminal {
                return Err("events after terminal or duplicate turn root".into());
            }
            match event {
                CodingEngineEvent::PatchRecoveryUpdated { state } => {
                    state.validate().map_err(|error| error.to_string())?;
                    // Recovery obligations may remain after this interrupted
                    // turn closes. The next turn loads them independently;
                    // boot cleanup is never treated as a file restoration.
                }
                CodingEngineEvent::TurnInputScope { wire_turn_id } => {
                    if index != 1
                        || input_scope
                        || wire_turn_id.trim().is_empty()
                        || wire_turn_id.len() > 256
                    {
                        return Err("invalid or duplicate turn input scope".into());
                    }
                    input_scope = true;
                }
                CodingEngineEvent::SteeringInputs { inputs }
                | CodingEngineEvent::SteeringDeferred { inputs, .. } => {
                    if !input_scope || cleanup_proven || inputs.is_empty() {
                        return Err("steering input outside its admitted scope/boundary".into());
                    }
                    for input in inputs {
                        input.validate().map_err(|error| error.to_string())?;
                        if input_receipts.len() >= 16
                            || !input_receipts.insert(input.receipt_operation_id)
                        {
                            return Err("duplicate or excessive steering receipts".into());
                        }
                    }
                }
                CodingEngineEvent::CapabilitiesActivated { .. } if cleanup_proven => {
                    return Err("capability activation admitted after cleanup witness".into());
                }
                CodingEngineEvent::ToolStarted {
                    capability_id,
                    call_id,
                    action_id,
                    ..
                } => {
                    if cleanup_proven {
                        return Err("tool admitted after cleanup witness".into());
                    }
                    // Only these audited in-process owners belong to this
                    // build. A future capability needs its own recovery audit.
                    match capability_id.as_ref() {
                        "vcs.push" => {
                            if action_id.as_ref() != "vcs.push.invoke"
                                || git_calls.len() >= 512
                                || !git_calls.insert(call_id.as_ref().to_owned())
                            {
                                return Err("Git tool admission is malformed or duplicated".into());
                            }
                        }
                        "process.exec" => process_audit.admit(call_id.as_ref(), action_id.as_ref())?,
                        "fs.read" | "fs.search" | "fs.write" | "fs.patch" | "fs.delete"
                        | "fs.snapshot" | "vcs.status" | "vcs.diff" | "vcs.stage"
                        | "vcs.commit" => {}
                        id if super::nomi_core_mcp_catalog::is_product_tool(id) => {
                            if action_id.as_ref() != format!("{id}.invoke")
                                || mcp_calls.len() >= 512
                                || mcp_calls
                                    .insert(call_id.as_ref().to_owned(), id.to_owned())
                                    .is_some()
                            {
                                return Err("MCP tool admission is malformed or duplicated".into());
                            }
                        }
                        id if super::nomi_core_robot::tool_capability_ids()
                            .contains(&CapabilityId::from(id)) =>
                        {
                            if action_id.as_ref() != format!("{id}.invoke")
                                || robot_calls.len() >= 512
                                || robot_calls
                                    .insert(call_id.as_ref().to_owned(), id.to_owned())
                                    .is_some()
                            {
                                return Err(
                                    "Robot tool admission is malformed or duplicated".into()
                                );
                            }
                        }
                        id if hosted_actions
                            .contains(&(id.to_owned(), action_id.as_ref().to_owned())) =>
                        {
                            if hosted_calls.len() >= 512
                                || hosted_calls
                                    .insert(
                                        call_id.as_ref().to_owned(),
                                        (id.to_owned(), action_id.as_ref().to_owned()),
                                    )
                                    .is_some()
                            {
                                return Err(
                                    "Plugin Product tool admission is malformed or duplicated".into()
                                );
                            }
                        }
                        _ => return Err("tool owner has no Coding recovery audit".into()),
                    }
                }
                CodingEngineEvent::ModelStepStarted { step, operation_id } => {
                    model_steps = step;
                    if !model_operations.insert(operation_id.as_ref().to_owned()) {
                        return Err("duplicate model operation in resource history".into());
                    }
                }
                CodingEngineEvent::TurnCompleted { .. }
                | CodingEngineEvent::TurnCancelled { .. }
                | CodingEngineEvent::TurnFailed { .. } => terminal = true,
                _ => {}
            }
        }
        if !mcp_receipts.is_empty() {
            return Err("MCP owner receipts have no matching durable tool dispatch".into());
        }
        if !resource_receipts.is_empty() {
            return Err(
                "MCP resource owner receipts have no matching durable resource dispatch".into(),
            );
        }
        if !hosted_receipts.is_empty() {
            return Err("Plugin Product owner receipts have no matching durable tool dispatch".into());
        }
        if !robot_receipts.is_empty() {
            return Err("Robot owner receipts have no matching durable device dispatch".into());
        }
        if !git_receipts.is_empty() {
            return Err("Git owner receipts have no matching durable push dispatch".into());
        }
        if !process_audit.is_quiescent() && !cleanup_proven {
            return Err("command process-tree cleanup is unknown; preserve quarantine and never replay effects".into());
        }
        if !terminal {
            let payload = serde_json::to_string(&CodingEngineEvent::TurnFailed { model_steps,
                message: "Interrupted by application restart. Previously admitted effects may have occurred; inspect state before retrying. No tools were replayed.".into() }).map_err(|error| error.to_string())?;
            sqlx::query("INSERT INTO conversation_runtime_events (conversation_id, turn_operation_id, sequence, event_json, created_at) VALUES (?, ?, ?, ?, ?)")
                .bind(conversation).bind(operation).bind(sequence + 1).bind(payload).bind(nomifun_common::now_ms())
                .execute(&mut *tx).await.map_err(|error| error.to_string())?;
        }
        tx.commit().await.map_err(|error| error.to_string())?;
        Ok("exact boot generation + exact compiled Coding build + contiguous write-ahead journal + matched MCP/Plugin Product/Robot/Git owner receipts + no process-owner dispatch or latest exact owner cleanup barrier or durable joined turn-cleanup witness; history closed without replay, no command-success/rollback/physical-service-quiescence claim".into())
    }
}

#[async_trait]
impl RegisteredEngineRestartRecovery for CodingRestartRecovery {
    async fn prepare_interrupted_turn(
        &self,
        binding: &RuntimeEngineBinding,
        user_id: &str,
        conversation_id: &str,
        admission_epoch: i64,
        operation_id: &str,
    ) -> TerminalProofDecision {
        match self
            .recover(
                binding,
                user_id,
                conversation_id,
                admission_epoch,
                operation_id,
            )
            .await
        {
            Ok(evidence) => TerminalProofDecision::Proven { evidence },
            Err(reason) => TerminalProofDecision::Unproven { reason },
        }
    }
}
