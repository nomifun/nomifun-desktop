use crate::manager::acp::AcpAgentManager;

use crate::manager::acp::mode_normalize::normalize_requested_mode;
use crate::protocol::error::AcpError;
use crate::session::{ConfigKey, ConfigValue, ModeId, ModelId};
use agent_client_protocol::schema::{
    SessionId, SetSessionConfigOptionRequest, SetSessionModeRequest, SetSessionModelRequest,
};
use nomifun_common::AppError;
use tracing::{error, info, warn};

/// Actions the session driver must execute to align CLI state with user intent.
///
/// Produced by `AcpSession::plan_reconcile` — a pure function that compares
/// desired vs observed and returns a list of idempotent, order-independent ops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileAction {
    SetMode { mode: ModeId },
    SetModel { model: ModelId },
    SetConfigOption { key: ConfigKey, value: ConfigValue },
}

impl AcpAgentManager {
    /// Execute reconcile actions produced by `AcpSession::plan_reconcile`.
    ///
    /// Compares the aggregate's desired state against what the CLI has
    /// reported as current, then issues the minimal set of SDK calls
    /// (set_mode, set_model, set_config_option) to bring the CLI into
    /// alignment.
    ///
    /// Failure handling:
    /// - `SessionNotFound`: returned as `AppError::NotFound` so callers
    ///   (e.g. `open_session_resume`) can drop the stale sid and rebuild
    ///   the session. ELECTRON-1HQ regressed because we silently swallowed
    ///   this case during warmup, leaving downstream `session/prompt` to
    ///   surface the same error to the user every turn.
    /// - Any other error: logged and skipped (best-effort), so a failed
    ///   `set_config_option` doesn't block a successful `set_mode`.
    pub(super) async fn reconcile_session(&self, session_id: &str) -> Result<(), AppError> {
        use crate::manager::acp::ReconcileAction;

        let (invalid_model, actions) = {
            let mut session = self.session.write().await;
            let invalid_model = session.clear_invalid_desired_model();
            let actions = session.plan_reconcile();
            (invalid_model, actions)
        };
        if let Some(model) = invalid_model {
            warn!(
                conversation_id = %self.params.conversation_id,
                model_id = %model,
                "reconcile_session: dropped unavailable desired model"
            );
        }
        for action in actions {
            match action {
                ReconcileAction::SetMode { mode } => {
                    let normalized = normalize_requested_mode(&self.params.metadata, mode.as_str());
                    if normalized.is_empty() {
                        continue;
                    }
                    if let Err(e) = self
                        .protocol
                        .set_mode(SetSessionModeRequest::new(
                            SessionId::new(session_id),
                            normalized.clone(),
                        ))
                        .await
                    {
                        if matches!(e, AcpError::SessionNotFound { .. }) {
                            warn!(
                                conversation_id = %self.params.conversation_id,
                                mode_id = %normalized,
                                error = %e,
                                "reconcile_session: set_mode hit SessionNotFound; aborting reconcile"
                            );
                            return Err(AppError::from(e));
                        }
                        error!(
                            conversation_id = %self.params.conversation_id,
                            mode_id = %normalized,
                            error = %e,
                            "reconcile_session: set_mode failed"
                        );
                        continue;
                    }
                    // SDK does not push a notification after a successful
                    // set_mode — sync observed/advertised ourselves so the
                    // next plan_reconcile is a no-op.
                    let mut session = self.session.write().await;
                    session.apply_observed_mode(ModeId::new(normalized));
                    self.commit_session_changes(&mut session).await;
                }

                ReconcileAction::SetModel { model } => {
                    if let Err(e) = self
                        .protocol
                        .set_model(SetSessionModelRequest::new(
                            SessionId::new(session_id),
                            model.as_str().to_owned(),
                        ))
                        .await
                    {
                        if matches!(e, AcpError::SessionNotFound { .. }) {
                            warn!(
                                conversation_id = %self.params.conversation_id,
                                model_id = %model,
                                error = %e,
                                "reconcile_session: set_model hit SessionNotFound; aborting reconcile"
                            );
                            return Err(AppError::from(e));
                        }
                        error!(
                            conversation_id = %self.params.conversation_id,
                            model_id = %model,
                            error = %e,
                            "reconcile_session: set_model failed"
                        );
                        continue;
                    }
                    // SDK does not push a CurrentModelUpdate notification —
                    // sync observed/advertised ourselves.
                    let mut session = self.session.write().await;
                    let model_for_notice = model.clone();
                    session.apply_observed_model(model);
                    if self.params.metadata.behavior_policy.self_identity_sticky {
                        session.set_pending_model_notice(model_for_notice);
                    }
                    self.commit_session_changes(&mut session).await;
                }

                ReconcileAction::SetConfigOption { key, value } => {
                    if let Err(err) = self
                        .protocol
                        .set_config_option(SetSessionConfigOptionRequest::new(
                            SessionId::new(session_id),
                            key.as_str().to_owned(),
                            value.as_str().to_owned(),
                        ))
                        .await
                    {
                        if matches!(err, AcpError::SessionNotFound { .. }) {
                            warn!(
                                conversation_id = %self.params.conversation_id,
                                config_id = %key,
                                desired = %value,
                                error = %err,
                                "reconcile_session: set_config_option hit SessionNotFound; aborting reconcile"
                            );
                            return Err(AppError::from(err));
                        }
                        info!(
                            conversation_id = %self.params.conversation_id,
                            config_id = %key,
                            desired = %value,
                            error = %err,
                            "reconcile_session: set_config_option failed; skipping"
                        );
                        continue;
                    }
                    // Sync observed ourselves so the next plan_reconcile
                    // does not replay this action. CLI does not push a
                    // config-update notification after set_config_option.
                    let mut session = self.session.write().await;
                    session.apply_observed_config(key, value);
                    self.commit_session_changes(&mut session).await;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_action_equality() {
        let a = ReconcileAction::SetMode {
            mode: ModeId::new("plan"),
        };
        let b = ReconcileAction::SetMode {
            mode: ModeId::new("plan"),
        };
        assert_eq!(a, b);
    }
}
