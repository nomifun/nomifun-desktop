//! Explicit ownership for opt-in model-step correction. Ordinary streams keep
//! their existing cancellation API and never construct this owner.
use super::*;
use nomifun_agent_contracts::OperationId;
use tokio::task::JoinHandle;
use tokio::sync::Mutex;
use tokio::time::{Instant,Duration};

#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum OwnedModelCleanupStage{OpeningOnly,Producer}
#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum OwnedModelCleanupOutcome{Joined,AbortedJoined}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedModelCleanupReceipt{
    pub operation_id:OperationId,pub task_id:String,
    pub stage:OwnedModelCleanupStage,pub outcome:OwnedModelCleanupOutcome,
}
#[cfg(test)]mod tests{
    use super::*;
    use std::sync::atomic::{AtomicBool,Ordering};
    fn producer_owner(worker:JoinHandle<()>,cancellation:CancellationToken)->OwnedProducer{OwnedProducer{operation_id:"owned-step".into(),task_id:worker.id().to_string(),cancellation,state:Mutex::new(ProducerState{worker:Some(worker),receipt:None})}}
    struct Dropped(Arc<AtomicBool>);impl Drop for Dropped{fn drop(&mut self){self.0.store(true,Ordering::SeqCst);}}
    #[tokio::test]async fn owned_cleanup_returns_exact_receipt_only_after_real_producer_future_is_dropped(){
        let cancellation=CancellationToken::new();let token=cancellation.clone();let dropped=Arc::new(AtomicBool::new(false));let witness=dropped.clone();let entered=Arc::new(tokio::sync::Notify::new());let signal=entered.clone();
        let worker=tokio::spawn(async move{let _drop=Dropped(witness);signal.notify_one();token.cancelled().await;});let task_id=worker.id().to_string();let owner=producer_owner(worker,cancellation);entered.notified().await;
        let receipt=owner.shutdown(Instant::now()+Duration::from_secs(1)).await.unwrap();assert!(dropped.load(Ordering::SeqCst));assert_eq!(receipt.task_id,task_id);assert_eq!(receipt.operation_id.as_ref(),"owned-step");assert_eq!(receipt.stage,OwnedModelCleanupStage::Producer);
        assert_eq!(owner.shutdown(Instant::now()+Duration::from_secs(1)).await.unwrap(),receipt);
    }
    #[tokio::test]async fn owned_cleanup_abort_must_join_and_a_panicking_producer_never_supplies_success_proof(){
        let dropped=Arc::new(AtomicBool::new(false));let witness=dropped.clone();let entered=Arc::new(tokio::sync::Notify::new());let signal=entered.clone();let worker=tokio::spawn(async move{let _drop=Dropped(witness);signal.notify_one();std::future::pending::<()>().await;});let owner=producer_owner(worker,CancellationToken::new());entered.notified().await;
        let receipt=owner.shutdown(Instant::now()+Duration::from_millis(5)).await.unwrap();assert_eq!(receipt.outcome,OwnedModelCleanupOutcome::AbortedJoined);assert!(dropped.load(Ordering::SeqCst));
        let failed=producer_owner(tokio::spawn(async{panic!("controlled producer failure")}),CancellationToken::new());tokio::task::yield_now().await;assert!(failed.shutdown(Instant::now()+Duration::from_secs(1)).await.is_err());assert!(failed.shutdown(Instant::now()+Duration::from_secs(1)).await.is_err());
    }
}
#[async_trait]
pub trait OwnedModelShutdown:std::fmt::Debug+Send+Sync{
    async fn shutdown(&self,deadline:Instant)->Result<OwnedModelCleanupReceipt,ChatModelError>;
}
pub struct OwnedChatAttempt{pub stream:ChatModelStream,pub shutdown:Arc<dyn OwnedModelShutdown>}

#[derive(Debug)]
struct ProducerState{worker:Option<JoinHandle<()>>,receipt:Option<OwnedModelCleanupReceipt>}
#[derive(Debug)]
struct OwnedProducer{
    operation_id:OperationId,task_id:String,cancellation:CancellationToken,state:Mutex<ProducerState>,
}
impl Drop for OwnedProducer{
    fn drop(&mut self){self.cancellation.cancel();if let Some(worker)=self.state.get_mut().worker.as_ref(){worker.abort();}}
}
fn cleanup_error(message:&str)->ChatModelError{ChatModelError::new(ChatModelErrorCode::Cancelled,message,ChatRetryDirective::Never)}
#[async_trait]
impl OwnedModelShutdown for OwnedProducer{
    async fn shutdown(&self,deadline:Instant)->Result<OwnedModelCleanupReceipt,ChatModelError>{
        self.cancellation.cancel();
        let mut state=tokio::time::timeout_at(deadline,self.state.lock()).await.map_err(|_|cleanup_error("owned model cleanup is already pending"))?;
        if let Some(receipt)=&state.receipt{return Ok(receipt.clone());}
        let Some(worker)=state.worker.as_mut() else{return Err(cleanup_error("owned producer handle is unavailable"));};
        let outcome=match tokio::time::timeout_at(deadline,&mut *worker).await{
            Ok(Ok(()))=>OwnedModelCleanupOutcome::Joined,
            Ok(Err(error)) if error.is_cancelled()=>OwnedModelCleanupOutcome::AbortedJoined,
            Ok(Err(_))=>{state.worker=None;return Err(cleanup_error("owned model producer failed during cleanup"));}
            Err(_)=>{
                worker.abort();
                // Abort requests cancellation; only actually joining it is a
                // proof. A non-yielding producer retains its handle on timeout.
                match tokio::time::timeout(Duration::from_millis(500),&mut *worker).await{
                    Ok(Ok(()))=>OwnedModelCleanupOutcome::Joined,
                    Ok(Err(error)) if error.is_cancelled()=>OwnedModelCleanupOutcome::AbortedJoined,
                    Ok(Err(_))=>{state.worker=None;return Err(cleanup_error("aborted model producer failed during cleanup"));}
                    Err(_)=>return Err(cleanup_error("owned producer abort has not joined; correction remains unconfirmed")),
                }
            }
        };
        state.worker=None;
        let receipt=OwnedModelCleanupReceipt{operation_id:self.operation_id.clone(),task_id:self.task_id.clone(),stage:OwnedModelCleanupStage::Producer,outcome};
        state.receipt=Some(receipt.clone());Ok(receipt)
    }
}
impl ChatModelBroker{
    pub async fn open_owned_chat_attempt(&self,request:ChatModelRequest,cancellation:CancellationToken)->Result<OwnedChatAttempt,ChatModelError>{
        let cancellation=cancellation.child_token();
        let routes=tokio::select!{biased;_=cancellation.cancelled()=>return Err(cleanup_error("owned model opening cancelled")),result=self.prepare(&request)=>result?};
        let operation_id=request.causality.operation_id.clone();
        let adapters=self.adapters.clone();let credential_store=self.credential_store.clone();let route_resolver=self.route_resolver.clone();
        let retry_policy=self.retry_policy;let capability_observer=self.capability_observer.clone();
        let(sender,receiver)=mpsc::channel(BROKER_STREAM_CAPACITY);let producer_cancel=cancellation.clone();
        let worker=tokio::spawn(async move{tokio::select!{biased;_=producer_cancel.cancelled()=>{},_=sender.closed()=>{},
            _=run_broker(request,routes,adapters,route_resolver,credential_store,retry_policy,capability_observer,sender.clone())=>{}}});
        let owner=OwnedProducer{operation_id,task_id:worker.id().to_string(),cancellation:cancellation.clone(),state:Mutex::new(ProducerState{worker:Some(worker),receipt:None})};
        // No await occurs between producing the worker and returning its owner.
        Ok(OwnedChatAttempt{stream:ChatModelStream{receiver,cancellation},shutdown:Arc::new(owner)})
    }
}
