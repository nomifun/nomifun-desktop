use super::*;
use nomifun_chat_model_broker::*;

struct Live(Arc<AtomicBool>);
impl Drop for Live{fn drop(&mut self){self.0.store(false,Ordering::Release);}}
struct OpeningModel {entered:Notify,live:Arc<AtomicBool>}
#[async_trait]
impl EngineModelPort for OpeningModel {
    async fn open_stream(&self,_:ChatModelRequest,_:CancellationToken)->Result<EngineModelStream,ChatModelError>{panic!("voice owned policy cannot fall back to the ordinary stream")}
    async fn open_owned_attempt(&self,_:ChatModelRequest,_:CancellationToken)->Result<EngineOwnedModelAttempt,ChatModelError>{
        self.live.store(true,Ordering::Release);let _live=Live(self.live.clone());self.entered.notify_one();futures_util::future::pending().await
    }
}
#[tokio::test]
async fn voice_owned_opening_is_aborted_and_joined_before_quiescence_and_unknown_attempt_has_no_proof(){
    let inner=Arc::new(OpeningModel{entered:Notify::new(),live:Arc::new(AtomicBool::new(false))});
    let models=OwnedModels{inner:inner.clone(),attempts:Default::default()};let request=recorded_conformance_fixtures().remove(0).request;let operation=request.causality.operation_id.clone();
    assert!(models.close(&operation,tokio::time::Instant::now()).await.is_err());
    let parent=CancellationToken::new();let token=parent.clone();let task=tokio::spawn({let models=models.clone();async move{models.open_stream(request,token).await}});
    tokio::time::timeout(std::time::Duration::from_secs(1),inner.entered.notified()).await.unwrap();
    let receipt=models.close(&operation,tokio::time::Instant::now()+std::time::Duration::from_millis(5)).await.unwrap();
    assert_eq!(receipt.operation_id,operation);assert_eq!(receipt.stage,OwnedModelCleanupStage::OpeningOnly);assert_eq!(receipt.outcome,OwnedModelCleanupOutcome::AbortedJoined);
    assert!(!receipt.task_id.is_empty());assert!(!inner.live.load(Ordering::Acquire));assert!(!parent.is_cancelled());assert!(task.await.unwrap().is_err());
    assert_eq!(models.close(&operation,tokio::time::Instant::now()).await.unwrap(),receipt,"a retry reuses the actual join proof, never polls a completed JoinHandle again");
}

struct Gate;
#[async_trait]impl ChatCausalityGate for Gate{async fn authorize(&self,_:&ChatCausality)->Result<(),ChatModelError>{Ok(())}}
struct Routes(ResolvedChatRoute);
#[async_trait]impl ChatRouteResolver for Routes{async fn resolve(&self,_:&ChatRouteSelection)->Result<ResolvedChatRouteSet,ChatModelError>{Ok(ResolvedChatRouteSet{primary:self.0.clone(),failovers:vec![]})}}
struct Credentials;
#[async_trait]impl ProviderCredentialStore for Credentials{async fn lease(&self,reference:&ProviderCredentialRef,target:&CredentialTarget)->Result<CredentialLease,ChatModelError>{Ok(CredentialLease::new(reference.clone(),target.clone(),"isolated-fixture-handle"))}}
struct PendingTransport{entered:Notify,live:Arc<AtomicBool>}
#[async_trait]impl ProviderTransport for PendingTransport{
    async fn open_stream(&self,_:ProviderWireRequest,_:CredentialLease)->Result<ProviderWireStream,ChatModelError>{
        self.live.store(true,Ordering::Release);let _live=Live(self.live.clone());self.entered.notify_one();futures_util::future::pending().await
    }
}
#[tokio::test]
async fn voice_owned_registry_joins_the_real_broker_producer_and_drops_its_provider_open_future(){
    let fixture=recorded_conformance_fixtures().remove(0);let transport=Arc::new(PendingTransport{entered:Notify::new(),live:Arc::new(AtomicBool::new(false))});
    let adapters:Vec<Arc<dyn ChatProtocolAdapter>>=vec![Arc::new(OpenAiChatAdapter::new(transport.clone())),Arc::new(OpenAiResponsesAdapter::new(transport.clone())),
        Arc::new(AnthropicAdapter::new(transport.clone())),Arc::new(GeminiAdapter::new(transport.clone())),Arc::new(VertexAdapter::new(transport.clone())),Arc::new(BedrockAdapter::new(transport.clone()))];
    let broker=ChatModelBroker::new(Arc::new(Gate),Arc::new(Routes(fixture.route)),Arc::new(Credentials),adapters,BrokerRetryPolicy::default()).unwrap();
    let models=OwnedModels{inner:Arc::new(BrokerEngineModelPort::new(Arc::new(broker))),attempts:Default::default()};let operation=fixture.request.causality.operation_id.clone();let parent=CancellationToken::new();
    let stream=models.open_stream(fixture.request,parent.clone()).await.unwrap();tokio::time::timeout(std::time::Duration::from_secs(1),transport.entered.notified()).await.unwrap();
    let receipt=models.close(&operation,tokio::time::Instant::now()+std::time::Duration::from_secs(1)).await.unwrap();
    assert_eq!(receipt.operation_id,operation);assert_eq!(receipt.stage,OwnedModelCleanupStage::Producer);assert!(!receipt.task_id.is_empty());
    assert!(!transport.live.load(Ordering::Acquire),"real Broker cleanup must have dropped the actual provider-opening future before acknowledging closure");
    assert!(!parent.is_cancelled());drop(stream);
}
