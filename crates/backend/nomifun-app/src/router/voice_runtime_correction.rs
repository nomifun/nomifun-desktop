//! Only a frozen voice-started Immediate-policy Turn installs this port.
//! It owns model opening and producer lifetimes, never tool/effect authority.
use std::collections::BTreeMap;
use std::sync::{Arc,Mutex,Weak,atomic::{AtomicBool,Ordering}};

use async_trait::async_trait;
use futures_util::future::BoxFuture;
use nomifun_agent_contracts::OperationId;
use nomifun_agent_runtime::{AgentEngineError,AgentImmediateCorrectionPort};
use nomifun_chat_model_broker::{ChatCausality,ChatModelError,ChatModelErrorCode,ChatModelRequest,ChatRetryDirective,EngineModelPort,EngineModelStream,OwnedModelCleanupReceipt,OwnedModelCleanupStage,OwnedModelCleanupOutcome};
use tokio::sync::{Mutex as AsyncMutex,Notify,oneshot};
use tokio_util::sync::CancellationToken;

use super::ConversationRuntimeHost;

type ProducerClose=Arc<dyn Fn(tokio::time::Instant)->BoxFuture<'static,Result<OwnedModelCleanupReceipt,ChatModelError>>+Send+Sync>;
struct Opening {
    task:AsyncMutex<Option<tokio::task::JoinHandle<()>>>,
    id:String,cancellation:CancellationToken,aborted:AtomicBool,
    outcome:Mutex<Option<OwnedModelCleanupOutcome>>,
    producer:Arc<Mutex<Option<ProducerClose>>>,
    closed:AsyncMutex<Option<OwnedModelCleanupReceipt>>,
}
impl Drop for Opening {
    fn drop(&mut self){self.cancellation.cancel();if let Some(task)=self.task.get_mut().as_ref(){task.abort();}}
}
#[derive(Clone)]
struct OwnedModels {inner:Arc<dyn EngineModelPort>,attempts:Arc<Mutex<BTreeMap<OperationId,Arc<Opening>>>>}
fn model_error(reason:&str)->ChatModelError {ChatModelError::new(ChatModelErrorCode::AdapterUnavailable,reason,ChatRetryDirective::Never)}
fn engine_error(reason:impl std::fmt::Display)->AgentEngineError {AgentEngineError::InvalidContract(format!("voice owned model attempt: {reason}"))}

#[async_trait]
impl EngineModelPort for OwnedModels {
    async fn open_stream(&self,request:ChatModelRequest,cancellation:CancellationToken)->Result<EngineModelStream,ChatModelError> {
        let operation=request.causality.operation_id.clone();let child=cancellation.child_token();
        let producer=Arc::new(Mutex::new(None));let producer_writer=producer.clone();
        let inner=self.inner.clone();let attempt_child=child.clone();let(sender,receiver)=oneshot::channel();
        {
            let mut registry=self.attempts.lock().unwrap_or_else(|error|error.into_inner());
            if registry.contains_key(&operation){return Err(model_error("the exact model operation already has an owned attempt"));}
            // No await between spawning, registering the actual task and
            // exposing this opening future. Cancellation cannot lose ownership.
            let task=tokio::spawn(async move {
                match inner.open_owned_attempt(request,attempt_child).await {
                    Ok(attempt)=>{
                        let shutdown=attempt.shutdown.clone();
                        let close:ProducerClose=Arc::new(move|deadline|{let shutdown=shutdown.clone();Box::pin(async move{shutdown.shutdown(deadline).await})});
                        *producer_writer.lock().unwrap_or_else(|error|error.into_inner())=Some(close);
                        let _=sender.send(Ok(attempt.stream));
                    }
                    Err(error)=>{let _=sender.send(Err(error));}
                }
            });
            let id=task.id().to_string();registry.insert(operation,Arc::new(Opening {task:AsyncMutex::new(Some(task)),id,cancellation:child,aborted:AtomicBool::new(false),outcome:Mutex::new(None),producer,closed:AsyncMutex::new(None)}));
        }
        receiver.await.map_err(|_|model_error("owned model opening stopped before returning its stream"))?
    }
}
impl OwnedModels {
    async fn close(&self,operation:&OperationId,deadline:tokio::time::Instant)->Result<OwnedModelCleanupReceipt,AgentEngineError> {
        let attempt=self.attempts.lock().unwrap_or_else(|error|error.into_inner()).get(operation).cloned().ok_or_else(||engine_error("unknown attempt; no cleanup proof is available"))?;
        let mut closed=attempt.closed.lock().await;if let Some(receipt)=&*closed{return Ok(receipt.clone());}
        attempt.cancellation.cancel();
        if attempt.outcome.lock().unwrap_or_else(|error|error.into_inner()).is_none() {
            let mut opening=attempt.task.lock().await;
            let task=opening.as_mut().ok_or_else(||engine_error("opening task has no join proof"))?;
            let result=match tokio::time::timeout_at(deadline,&mut *task).await {
                Ok(result)=>result,
                Err(_)=>{attempt.aborted.store(true,Ordering::Release);task.abort();
                    tokio::time::timeout(std::time::Duration::from_secs(1),&mut *task).await.map_err(|_|engine_error("opening abort has not joined; the attempt remains unresolved"))?}
            };
            match result {
                Ok(())=>{},Err(error) if error.is_cancelled()&&attempt.aborted.load(Ordering::Acquire)=>{},
                Err(_)=>{*opening=None;return Err(engine_error("opening exited without conclusive producer registration"));},
            }
            let outcome=if attempt.aborted.load(Ordering::Acquire){OwnedModelCleanupOutcome::AbortedJoined}else{OwnedModelCleanupOutcome::Joined};
            *attempt.outcome.lock().unwrap_or_else(|error|error.into_inner())=Some(outcome);*opening=None;
        }
        let producer=attempt.producer.lock().unwrap_or_else(|error|error.into_inner()).clone();
        let receipt=if let Some(close)=producer {close(deadline).await.map_err(|error|engine_error(format!("producer cleanup {:?}",error.code)))?}
            else {OwnedModelCleanupReceipt {operation_id:operation.clone(),task_id:attempt.id.clone(),stage:OwnedModelCleanupStage::OpeningOnly,
                outcome:attempt.outcome.lock().unwrap_or_else(|error|error.into_inner()).expect("opening joined above")}};
        if receipt.operation_id!=*operation||receipt.task_id.is_empty(){return Err(engine_error("producer proof differs from its owned operation"));}
        *closed=Some(receipt.clone());Ok(receipt)
    }
    async fn close_all(&self,deadline:tokio::time::Instant)->Result<Vec<OwnedModelCleanupReceipt>,AgentEngineError> {
        let keys=self.attempts.lock().unwrap_or_else(|error|error.into_inner()).keys().cloned().collect::<Vec<_>>();let mut receipts=Vec::new();
        for key in keys {receipts.push(self.close(&key,deadline).await?);}Ok(receipts)
    }
}

pub(super) struct VoiceCorrectionPort {host:Weak<ConversationRuntimeHost>,models:Arc<OwnedModels>,wake:Notify}
pub(super) struct VoiceGate(pub(super) Weak<ConversationRuntimeHost>);
#[async_trait]
impl nomifun_chat_model_broker::ChatCausalityGate for VoiceGate {
    async fn authorize(&self,causality:&ChatCausality)->Result<(),ChatModelError> {
        let host=self.0.upgrade().ok_or_else(||model_error("original canonical host closed"))?;
        nomifun_chat_model_broker::ChatCausalityGate::authorize(host.as_ref(),causality).await
    }
}
impl std::fmt::Debug for VoiceCorrectionPort {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str("VoiceOwnedCorrectionPort")}}
impl VoiceCorrectionPort {
    pub(super) fn new(host:Weak<ConversationRuntimeHost>,model:Arc<dyn EngineModelPort>)->Self {
        Self {host,models:Arc::new(OwnedModels {inner:model,attempts:Default::default()}),wake:Notify::new()}
    }
    pub(super) fn notify(&self){self.wake.notify_waiters();}
    pub(super) async fn confirms(&self,receipt:&OwnedModelCleanupReceipt)->bool {
        let attempt=self.models.attempts.lock().unwrap_or_else(|error|error.into_inner()).get(&receipt.operation_id).cloned();
        match attempt {Some(attempt)=>attempt.closed.lock().await.as_ref()==Some(receipt),None=>false}
    }
}
#[async_trait]
impl AgentImmediateCorrectionPort for VoiceCorrectionPort {
    fn model_port(&self)->Arc<dyn EngineModelPort>{self.models.clone()}
    async fn wait(&self,causality:&ChatCausality)->Result<Vec<String>,AgentEngineError> {
        loop {
            let notified=self.wake.notified();tokio::pin!(notified);notified.as_mut().enable();
            let host=self.host.upgrade().ok_or_else(||engine_error("canonical host closed"))?;
            let cancellation={
                let active=host.active.lock().await;let turn=active.as_ref().ok_or_else(||engine_error("no opted-in active Turn"))?;
                super::steering::admitted(turn,&host,causality)?;
                let ids=turn.steering.immediate_receipt_ids();if !ids.is_empty(){return Ok(ids);}turn.cancellation.clone()
            };
            tokio::select!{biased;_=cancellation.cancelled()=>return Err(engine_error("the original Turn closed before applying the correction")),_=notified=>{}}
        }
    }
    async fn pending(&self,causality:&ChatCausality)->Result<Vec<String>,AgentEngineError> {
        let host=self.host.upgrade().ok_or_else(||engine_error("canonical host closed"))?;let active=host.active.lock().await;
        let turn=active.as_ref().ok_or_else(||engine_error("no opted-in active Turn"))?;super::steering::admitted(turn,&host,causality)?;
        Ok(turn.steering.immediate_receipt_ids())
    }
    async fn has_admitted_tools(&self,causality:&ChatCausality,step:u16)->Result<bool,AgentEngineError> {
        let host=self.host.upgrade().ok_or_else(||engine_error("canonical host closed"))?;let active=host.active.lock().await;
        let turn=active.as_ref().ok_or_else(||engine_error("no opted-in active Turn"))?;super::steering::admitted(turn,&host,causality)?;
        Ok(turn.steering.has_admitted_model_tools(step))
    }
    async fn quiesce_model(&self,operation:&OperationId,deadline:tokio::time::Instant)->Result<OwnedModelCleanupReceipt,AgentEngineError>{self.models.close(operation,deadline).await}
    async fn quiesce_all(&self,deadline:tokio::time::Instant)->Result<Vec<OwnedModelCleanupReceipt>,AgentEngineError>{self.models.close_all(deadline).await}
}

#[cfg(test)]
#[path="voice_runtime_correction_tests.rs"]
mod tests;
