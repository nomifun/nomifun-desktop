use crate::{AgentChildrenWait, AgentCollaborationPort, AgentCollaborationSelection, AgentCollaborationSnapshot, AgentEngineError};
use nomifun_chat_model_broker::ChatCausality;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Keep an acknowledged child wait parked through temporary read admission
/// failures. This observes the same owned work; it cannot submit inputs,
/// replay effects, reset a budget or grant execution capacity. The caller
/// retains the original checkpoint and emits a single resolved event.
pub(crate) async fn wait_for_children(
    port: &dyn AgentCollaborationPort,
    causality: &ChatCausality,
    wait: &AgentChildrenWait,
    progress:&crate::AgentCollaborationProgress,
    cancellation: CancellationToken,
) -> Result<AgentCollaborationSnapshot, AgentEngineError> {
    let mut request=wait.clone();
    loop {
        let snapshot=observe(cancellation.clone(), || port.wait(causality,&request,cancellation.clone())).await?;
        crate::collaboration::validate_snapshot(&snapshot)?;
        request.validate_wake(&snapshot.cursor)?;
        // Aggregate hashes can also change when an old failure leaves the
        // current ready set after a retry. That is no new child decision.
        // Advance only this observation subscription; a crash may repeat a
        // read, never a model call or a business effect.
        if snapshot.decision_blocked()? || snapshot.parent_input_pending() || progress.has_new_ready_source(&snapshot)? {
            return Ok(snapshot);
        }
        if request.after_cursor.as_ref()==Some(&snapshot.cursor) {
            return Err(AgentEngineError::InvalidContract("child wait repeatedly returned an unchanged non-ready observation".into()));
        }
        request.after_cursor=Some(snapshot.cursor);
    }
}

/// Ownership validation before accepting a wait and canonical result reads
/// after it wakes are pure observations too. Sending messages is excluded.
pub(crate) async fn read_status(
    port: &dyn AgentCollaborationPort,
    causality: &ChatCausality,
    selection: &AgentCollaborationSelection,
    cancellation: CancellationToken,
) -> Result<AgentCollaborationSnapshot, AgentEngineError> {
    observe(cancellation.clone(), || port.status(causality, selection, cancellation.clone())).await
}

async fn observe<F, Fut>(cancellation: CancellationToken, mut read: F)
    -> Result<AgentCollaborationSnapshot, AgentEngineError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<AgentCollaborationSnapshot, AgentEngineError>>,
{
    let mut backoff = Duration::from_millis(100);
    loop {
        let result = tokio::select! { biased;
            _ = cancellation.cancelled() => return Err(AgentEngineError::Cancelled),
            result = read() => result,
        };
        match result {
            Err(AgentEngineError::ObservationUnavailable { .. }) => {
                tokio::select! { biased;
                    _ = cancellation.cancelled() => return Err(AgentEngineError::Cancelled),
                    _ = tokio::time::sleep(backoff) => {},
                }
                backoff = (backoff * 2).min(Duration::from_secs(5));
            }
            result => return result,
        }
    }
}
