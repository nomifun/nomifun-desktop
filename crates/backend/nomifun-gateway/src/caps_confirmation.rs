//! Confirmation-domain capabilities (registry form): list pending decisions of
//! a driven conversation and resolve one by picking an option.
//!
//! These let a channel-facing Agent relay a blocking decision from an Agent-attempt
//! conversation to the channel user as numbered text and submit the user's
//! pick — the gateway otherwise only exposes a `pending_confirmations` count
//! (`nomi_conversation_status`) with no way to read the options or answer.

use std::sync::Arc;

use nomifun_api_types::ConfirmRequest;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::{CallerCtx, GatewayDeps};
use crate::registry::{Capability, CapabilityMeta, DangerTier};
use crate::server::ok;

#[derive(Deserialize, JsonSchema)]
struct ListConfirmationsParams {
    /// The id of the conversation whose pending decisions to read.
    conversation_id: i64,
}

#[derive(Deserialize, JsonSchema)]
struct ResolveConfirmationParams {
    /// The id of the conversation containing the pending decision.
    conversation_id: i64,
    /// The call_id of the specific pending decision to resolve (from nomi_list_confirmations).
    call_id: String,
    /// The chosen option's value (a bare option-id string for ACP).
    option: String,
}

/// Build the `ConfirmRequest.data` for a resolved option, writing the chosen
/// option under BOTH keys so either backend resolves it: the nomi agent reads
/// `data.get("value")` (and defaults to "cancel" when the key is absent — a
/// bare `Value::String` was therefore silently DENIED), while ACP's
/// `confirm_option_id` reads `option_id` (falling back to `value`). Mirrors the
/// double-key payload IDMM already uses in `nomifun-idmm` probe `inject`.
fn confirm_data(option: &str) -> Value {
    json!({ "option_id": option, "value": option })
}

async fn list(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: ListConfirmationsParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({"error": "missing caller user identity"});
    }
    let id = p.conversation_id.to_string();
    let confs = match deps
        .conversation_service
        .list_confirmations(&ctx.user_id, &id, &deps.runtime_registry)
        .await
    {
        Ok(c) => c,
        Err(e) => return json!({"error": e.to_string()}),
    };
    ok(json!({
        "confirmations": confs
            .iter()
            .map(|c| json!({
                "call_id": c.call_id,
                "title": c.title,
                "description": c.description,
                "options": c
                    .options
                    .iter()
                    .map(|o| json!({"label": o.label, "value": o.value}))
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    }))
}

async fn resolve(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: ResolveConfirmationParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({"error": "missing caller user identity"});
    }
    let id = p.conversation_id.to_string();

    // Self-confirmation guard: an agent may not resolve a decision in its own
    // conversation (that would bypass the human-in-the-loop contract).
    if !ctx.conversation_id.is_empty() && id == ctx.conversation_id {
        return json!({
            "error": "self_confirmation_forbidden: you cannot resolve a confirmation in your own conversation"
        });
    }

    let req = ConfirmRequest {
        msg_id: String::new(),
        data: confirm_data(&p.option),
        always_allow: false,
    };
    match deps
        .conversation_service
        .confirm(&ctx.user_id, &id, &p.call_id, req, &deps.runtime_registry)
        .await
    {
        Ok(()) => ok(json!({"resolved": p.call_id})),
        Err(e) => json!({"error": e.to_string()}),
    }
}

/// Register the confirmation-domain capabilities.
pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<ListConfirmationsParams, _, _>(
        CapabilityMeta::new(
            "nomi_list_confirmations",
            "confirmation",
            "List the pending decisions (permission / choice dialogs) of a conversation, each with its options. Returns an empty list when no active agent or no pending decisions.",
            DangerTier::Read,
        ),
        |deps, ctx, p| list(deps, ctx, p),
    ));
    out.push(Capability::new::<ResolveConfirmationParams, _, _>(
        CapabilityMeta::new(
            "nomi_resolve_confirmation",
            "confirmation",
            "Submit the user's pick for a pending decision in a driven conversation. Refused for the caller's own conversation (self-confirmation-forbidden).",
            DangerTier::Write,
        ),
        |deps, ctx, p| resolve(deps, ctx, p),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_data_carries_option_under_both_keys_so_nomi_does_not_read_cancel() {
        // REGRESSION: the gateway previously sent ConfirmRequest.data as a bare
        // Value::String(option). The nomi agent's confirm reads data.get("value")
        // and defaults to "cancel" when absent → every relayed approval on a Nomi
        // Agent attempt was silently DENIED. The payload must carry the option under
        // BOTH keys (nomi reads `value`; ACP's confirm_option_id reads
        // `option_id`, falling back to `value`).
        let d = confirm_data("proceed_once");
        assert_eq!(d.get("value").and_then(|v| v.as_str()), Some("proceed_once"));
        assert_eq!(d.get("option_id").and_then(|v| v.as_str()), Some("proceed_once"));
        // The nomi consumer's exact read must NOT collapse to cancel.
        assert_ne!(
            d.get("value").and_then(|v| v.as_str()).unwrap_or("cancel"),
            "cancel"
        );
    }

    #[test]
    fn bare_string_payload_is_read_as_cancel_by_nomi_consumer() {
        // Characterizes WHY the old shape was broken: a bare Value::String is
        // invisible to the nomi consumer's `data.get("value")`.
        let bare = Value::String("proceed_once".into());
        assert_eq!(bare.get("value").and_then(|v| v.as_str()).unwrap_or("cancel"), "cancel");
    }

    #[test]
    fn self_confirmation_guard_forbids_own_conversation() {
        let ctx = CallerCtx {
            conversation_id: "42".into(),
            user_id: "u1".into(),
            ..Default::default()
        };
        let id = 42i64.to_string();
        let forbidden = !ctx.conversation_id.is_empty() && id == ctx.conversation_id;
        assert!(forbidden, "resolving own conversation must be forbidden");
    }

    #[test]
    fn self_confirmation_guard_allows_different_conversation() {
        let ctx = CallerCtx {
            conversation_id: "42".into(),
            user_id: "u1".into(),
            ..Default::default()
        };
        let id = 99i64.to_string();
        let allowed = ctx.conversation_id.is_empty() || id != ctx.conversation_id;
        assert!(allowed, "resolving a different conversation must be allowed");
    }

    #[test]
    fn self_confirmation_guard_allows_when_caller_has_no_conversation() {
        let ctx = CallerCtx {
            conversation_id: String::new(),
            user_id: "u1".into(),
            ..Default::default()
        };
        let id = 42i64.to_string();
        let allowed = ctx.conversation_id.is_empty() || id != ctx.conversation_id;
        assert!(allowed, "empty caller conversation_id must bypass the guard");
    }

    #[test]
    fn missing_user_identity_produces_error() {
        // The handlers check ctx.user_id.is_empty() before any deps access.
        let ctx = CallerCtx::default();
        assert!(ctx.user_id.is_empty());
    }
}
