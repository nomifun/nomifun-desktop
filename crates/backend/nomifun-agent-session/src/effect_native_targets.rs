//! Native file footprints live only in the original Effect start fact. The
//! existing Effect lifecycle index selects unsettled facts; no target ledger
//! or filesystem lookup is used when admitting or settling another operation.
use super::*;
use nomifun_agent_contracts::NativeMutationTargets;
#[cfg(windows)]
use nomifun_agent_contracts::NativeMutationTarget;

const FIELD: &str = "native_mutation_targets";

fn explicit_file_mutation(payload: &Value) -> bool {
    payload.get("capability_module").and_then(Value::as_str) == Some("workspace.files")
        && matches!(payload.get("action_id").and_then(Value::as_str),
            Some("workspace.files/write" | "workspace.files/patch" | "workspace.files/delete" | "workspace.files/rename"))
}

pub(super) fn from_payload(payload: &Value) -> Result<Option<NativeMutationTargets>, SessionStoreError> {
    let Some(value) = payload.get(FIELD) else { return Ok(None); };
    let targets: NativeMutationTargets = serde_json::from_value(value.clone())
        .map_err(|error| SessionStoreError::InvalidEvent(format!("invalid native mutation target facts: {error}")))?;
    if !explicit_file_mutation(payload) || targets.version != 1 || targets.targets.is_empty() || targets.targets.len() > 256 {
        return Err(SessionStoreError::InvalidEvent("native mutation target facts require a versioned explicit file footprint".into()));
    }
    let action = payload.get("action_id").and_then(Value::as_str);
    if matches!(action, Some("workspace.files/write" | "workspace.files/delete")) && targets.targets.len() != 1
        || action == Some("workspace.files/rename") && targets.targets.len() != 2 {
        return Err(SessionStoreError::InvalidEvent("native mutation footprint does not cover the action's complete target set".into()));
    }
    let publishes_object = matches!(action,
        Some("workspace.files/write" | "workspace.files/patch"));
    for target in &targets.targets {
        let valid_id = |id: &str| !id.is_empty() && id.len() <= 4096 && !id.contains('\0');
        if (publishes_object && !target.volume_wide)
            || !valid_id(&target.volume_id) || !valid_id(&target.namespace.parent_id)
            || target.object_id.as_deref().is_some_and(|id| !valid_id(id))
            || target.ancestor_ids.is_empty() || target.ancestor_ids.len() > 512
            || !target.ancestor_ids.contains(&target.namespace.parent_id)
            || target.ancestor_ids.iter().any(|id| !valid_id(id))
            || target.namespace.components.is_empty() || target.namespace.components.len() > 512
            || target.namespace.components.iter().any(|key| key.is_empty() || key.len() > 4096 || key.contains(&0)) {
            return Err(SessionStoreError::InvalidEvent("native mutation target identity or pinned ancestry is incomplete".into()));
        }
    }
    Ok(Some(targets))
}

#[cfg(windows)]
fn prefix(left: &[Vec<u16>], right: &[Vec<u16>]) -> bool {
    left.len() <= right.len() && right.starts_with(left)
}

#[cfg(windows)]
fn overlaps(left: &NativeMutationTarget, right: &NativeMutationTarget) -> bool {
    // IDs from different native platforms are incomparable, not proof that
    // a restored database cannot still refer to the same network filesystem.
    if left.platform != right.platform { return true; }
    if left.volume_id == right.volume_id && (left.volume_wide || right.volume_wide || left.subtree || right.subtree) { return true; }
    if left.object_id.is_some() && left.object_id == right.object_id { return true; }
    if left.namespace.parent_id == right.namespace.parent_id {
        let l = &left.namespace.components;
        let r = &right.namespace.components;
        // A missing ancestor must be created before its child can be changed,
        // so namespace prefix conflicts even for a non-recursive file action.
        if prefix(l, r) || prefix(r, l) { return true; }
    }
    // Deleting/renaming a directory entry and writing below it conflict even
    // when the first action did not request recursive traversal.
    if left.object_id.as_ref().is_some_and(|id| right.ancestor_ids.contains(id)) { return true; }
    if right.object_id.as_ref().is_some_and(|id| left.ancestor_ids.contains(id)) { return true; }
    false
}

#[cfg(windows)]
pub(super) async fn validate_admission_tx(
    tx: &mut Transaction<'_, Sqlite>, append: &SessionEventAppend,
) -> Result<(), SessionStoreError> {
    let payload = effect_payload_from_append(append)?;
    let incoming = from_payload(payload)?;
    if !explicit_file_mutation(payload) { return Ok(()); }
    // BEGIN IMMEDIATE is acquired before this read and remains held through
    // Event append and Effect projection. Separate Store instances sharing a
    // database cannot both observe a clear footprint and then commit it.
    let rows = sqlx::query_as::<_, StoredEffectRow>(
        "SELECT effect_id, session_id, turn_id, operation_id, owner_domain, \
                capability_module, action_id, resource_binding_id, resource_key, \
                input_digest, strategy, state, started_event_id, terminal_event_id, created_at, settled_at \
         FROM agent_effects WHERE state IN ('pending','unknown') AND capability_module='workspace.files' \
         AND action_id IN ('workspace.files/write','workspace.files/patch','workspace.files/delete','workspace.files/rename')")
        .fetch_all(&mut **tx).await?;
    for row in rows {
        // Validate the original canonical event and lifecycle against its
        // index before using its targets. Damaged facts fail closed.
        terminal_content::effect_observation(tx, &row).await?;
        let event = event_by_event_id_tx(tx, &row.started_event_id).await?
            .map(event_from_row).transpose()?
            .ok_or_else(|| SessionStoreError::InvalidEvent("native Effect start fact is missing".into()))?;
        let prior = from_payload(effect_payload_from_event(&event)?)?;
        let conflict = match (&incoming, &prior) {
            (Some(incoming), Some(prior)) => incoming.targets.iter().any(|left| prior.targets.iter().any(|right| overlaps(left, right))),
            // A digest, project root or lexical path cannot establish past
            // native identity. In particular, never inspect today's disk to
            // infer the footprint of an old unknown operation.
            _ => true,
        };
        if conflict {
            return Err(SessionStoreError::NativeMutationTargetUnsettled {
                effect_id: row.effect_id, state: row.state,
            });
        }
    }
    Ok(())
}

pub(super) fn validate_settlement(start: &Value, next: &Value) -> Result<(), SessionStoreError> {
    // Target facts remain solely at the original start. A caller may echo
    // them but cannot replace, augment or backfill a historical footprint.
    from_payload(start)?;
    if next.get(FIELD).is_some() && next.get(FIELD) != start.get(FIELD) {
        return Err(SessionStoreError::InvalidEvent("Effect settlement changed its original native mutation target facts".into()));
    }
    Ok(())
}

impl AgentSessionStore {
    #[cfg(windows)]
    pub async fn record_effect_started_with_native_targets(
        &self, mut request: EffectEventRequest, targets: NativeMutationTargets,
    ) -> Result<SessionEventAppendResult, SessionStoreError> {
        let SessionEventPayloadRef::InlineJson(payload) = &mut request.payload else {
            return Err(SessionStoreError::InvalidEvent("native mutation Effect requires inline canonical facts".into()));
        };
        let object = payload.0.as_object_mut().ok_or_else(|| SessionStoreError::InvalidEvent("native mutation Effect requires an object".into()))?;
        let value = serde_json::to_value(targets)?;
        if object.get(FIELD).is_some_and(|old| old != &value) {
            return Err(SessionStoreError::InvalidEvent("native target facts differ from the supplied Effect facts".into()));
        }
        object.insert(FIELD.into(), value);
        self.record_effect_started(request).await
    }
}
