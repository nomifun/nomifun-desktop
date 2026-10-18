//! Transport observation of accepted canonical inputs. This registry deduplicates
//! live relays only; it never stores input bodies, claims work or runs a model.
use std::{collections::{HashMap,HashSet},sync::{Arc,Mutex},time::Duration};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use crate::{message_service::ChannelMessageService,session_port::ChannelInputObservation,
    stream_relay::{ChannelSender,ChannelStreamRelay,RelayConfig}};
use nomifun_agent_contracts::ChannelInputDeliverySubject;

/// Observe the original sender's actual ACKs. The relay remains the only
/// producer of platform messages, edits and media uploads.
struct ReceiptSender {
    inner:Arc<dyn ChannelSender>,ids:Mutex<Vec<String>>,failed:std::sync::atomic::AtomicBool,
}
impl ReceiptSender {
    fn new(inner:Arc<dyn ChannelSender>)->Arc<Self> {Arc::new(Self {inner,ids:Default::default(),failed:Default::default()})}
    fn record(&self,result:&Result<String,crate::error::ChannelError>) {
        match result {Ok(id) if !id.is_empty()=>{
            let mut ids=self.ids.lock().unwrap_or_else(|e|e.into_inner());if !ids.contains(id) {ids.push(id.clone());}
        },Ok(_)=>self.failed.store(true,std::sync::atomic::Ordering::Release),
            Err(_)=>self.failed.store(true,std::sync::atomic::Ordering::Release)}
    }
    fn outcome(&self,delivered:bool)->(bool,Vec<String>) {
        (delivered&&!self.failed.load(std::sync::atomic::Ordering::Acquire),self.ids.lock().unwrap_or_else(|e|e.into_inner()).clone())
    }
}
#[async_trait::async_trait]
impl ChannelSender for ReceiptSender {
    async fn send_message(&self,plugin:&str,chat:&str,message:crate::types::UnifiedOutgoingMessage)->Result<String,crate::error::ChannelError> {
        let result=self.inner.send_message(plugin,chat,message).await;self.record(&result);result
    }
    async fn edit_message(&self,plugin:&str,chat:&str,id:&str,message:crate::types::UnifiedOutgoingMessage)->Result<(),crate::error::ChannelError> {
        let result=self.inner.edit_message(plugin,chat,id,message).await;
        match &result {Ok(())=>self.record(&Ok(id.to_owned())),Err(_)=>self.failed.store(true,std::sync::atomic::Ordering::Release)}
        result
    }
    async fn send_media(&self,plugin:&str,chat:&str,media:crate::types::OutgoingMedia,caption:Option<&str>)->Result<String,crate::error::ChannelError> {
        let result=self.inner.send_media(plugin,chat,media,caption).await;self.record(&result);result
    }
}

pub(crate) async fn recover_input_observers(service:Arc<ChannelMessageService>,sender:Arc<dyn ChannelSender>,shutdown:CancellationToken)
    ->Vec<tokio::task::JoinHandle<()>> {
    let inputs=match service.recover_inputs().await {Ok(inputs)=>inputs,Err(error)=>{warn!(%error,"canonical Channel input recovery is unavailable");return Vec::new();}};
    let mut tasks=Vec::new();
    for input in inputs {
        let Some(platform)=crate::types::PluginType::from_str_opt(&input.source.platform) else {continue;};
        let config=RelayConfig {platform,plugin_id:input.source.channel_plugin_id,chat_id:input.source.chat_id,
            conversation_id:input.session_id,throttle_ms:500};
        if let Some(task)=register_input_observer(service.clone(),config,sender.clone(),input.input_event_id,shutdown.clone()) {tasks.push(task);}
    }
    tasks
}

type Scope=(String,String,String);
#[derive(Default)]
pub(crate) struct InputObserverRegistry {
    entries:Mutex<HashMap<Scope,mpsc::UnboundedSender<String>>>,
}

struct Registration {
    registry:Arc<InputObserverRegistry>,scope:Scope,sender:mpsc::UnboundedSender<String>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        let mut entries=self.registry.entries.lock().unwrap_or_else(|e|e.into_inner());
        if entries.get(&self.scope).is_some_and(|sender|sender.same_channel(&self.sender)) {entries.remove(&self.scope);}
    }
}

pub(crate) fn register_input_observer(service:Arc<ChannelMessageService>,config:RelayConfig,
    sender:Arc<dyn ChannelSender>,key:String,shutdown:CancellationToken)->Option<tokio::task::JoinHandle<()>> {
    let registry=service.input_observers.clone();
    let scope=(config.conversation_id.clone(),config.plugin_id.clone(),config.chat_id.clone());
    let mut entries=registry.entries.lock().unwrap_or_else(|e|e.into_inner());
    if let Some(existing)=entries.get(&scope) && existing.send(key.clone()).is_ok() {return None;}
    let(tx,rx)=mpsc::unbounded_channel();
    tx.send(key).expect("new observer receiver is live");
    entries.insert(scope.clone(),tx.clone());
    drop(entries);
    let registration=Registration {registry,scope,sender:tx};
    Some(tokio::spawn(async move {
        tokio::select! {_=shutdown.cancelled()=>{},_=observe(service,config,sender,rx,&registration)=>{}}
    }))
}

async fn observe(service:Arc<ChannelMessageService>,config:RelayConfig,sender:Arc<dyn ChannelSender>,
    mut inputs:mpsc::UnboundedReceiver<String>,registration:&Registration) {
    let mut seen_inputs=HashSet::new();let mut seen_turns=HashSet::new();
    let question_receipts=Arc::new(Mutex::new(HashSet::new()));
    loop {
        let key=match tokio::time::timeout(Duration::from_secs(30),inputs.recv()).await {
            Ok(Some(key))=>key,Ok(None)=>return,
            Err(_)=>{
                // Registration and expiry share the same lock: an input either
                // reaches this receiver or starts a replacement observer.
                let mut entries=registration.registry.entries.lock().unwrap_or_else(|e|e.into_inner());
                match inputs.try_recv() {
                    Ok(key)=>key,
                    Err(_)=>{
                        if entries.get(&registration.scope).is_some_and(|entry|entry.same_channel(&registration.sender)) {
                            entries.remove(&registration.scope);
                        }
                        return;
                    }
                }
            }
        };
        if seen_inputs.contains(&key) {continue;}
        let started=std::time::Instant::now();let mut pending_announced=false;
        loop {
            match service.observe_input(&config.conversation_id,&key).await {
                Ok(ChannelInputObservation::Pending)=>{
                    if !pending_announced && started.elapsed()>=Duration::from_secs(1) {
                        let _=sender.send_message(&config.plugin_id,&config.chat_id,
                            ChannelMessageService::build_final_message("消息已保存，等待处理。")).await;
                        pending_announced=true;
                    }
                }
                Ok(ChannelInputObservation::Blocked {reason})=>{
                    let detail=match reason.as_str() {"ATTACHMENT_PREPARATION_FAILED"=>"请检查附件是否可用。",
                        "SKILL_UNAVAILABLE"=>"请检查所选技能是否可用。","INPUT_BUDGET_EXCEEDED"=>"请缩短内容或减少附件。",_=>""};
                    let _=sender.send_message(&config.plugin_id,&config.chat_id,ChannelMessageService::build_final_message(
                        &format!("此输入已保存但未处理，检查问题后重新发送。{detail}"))).await;
                    break;
                }
                Ok(ChannelInputObservation::Discarded {..})=>{
                    let _=sender.send_message(&config.plugin_id,&config.chat_id,
                        ChannelMessageService::build_final_message("此输入已随上下文清空取消处理。")).await;
                    seen_inputs.insert(key.clone());break;
                }
                Ok(ChannelInputObservation::Claimed {turn_id,delivery,question,terminal_events})=>{
                    if seen_turns.contains(&turn_id) {seen_inputs.insert(key.clone());break;}
                    let receipt_sender=ReceiptSender::new(sender.clone());
                    let relay=ChannelStreamRelay::new(config.clone(),receipt_sender.clone(),service.stop_confirmations(),service.asset_resolver())
                        .with_input_question_delivery(service.clone()).with_question_receipts(question_receipts.clone());
                    let mut question_unknown=false;
                    let mut initial_question=None;
                    if let Some(question)=question {
                        initial_question=Some(question.request.clone());
                        let subject=ChannelInputDeliverySubject::Question {question_seq:question.target.question_seq};
                        match service.begin_input_delivery(&config.conversation_id,&key,&turn_id,subject.clone()).await {
                            Ok(crate::ChannelInputDeliveryDisposition::Ready)=>{
                                let delivered=relay.send_input_question(&question).await;
                                let(confirmed,ids)=receipt_sender.outcome(delivered);
                                if let Err(error)=service.finish_input_delivery(&config.conversation_id,&key,&turn_id,subject,confirmed,ids).await {
                                    warn!(%error,"original Channel question delivery acknowledgement is unknown");break;
                                }
                                question_unknown=!confirmed;
                            },
                            Ok(crate::ChannelInputDeliveryDisposition::Confirmed)=>{},
                            Ok(crate::ChannelInputDeliveryDisposition::Unknown)=>question_unknown=true,
                            Err(error)=>{warn!(%error,"original Channel question delivery is fenced");break;},
                        }
                    }
                    if delivery.delivery.completed {
                        match service.begin_input_delivery(&config.conversation_id,&key,&turn_id,ChannelInputDeliverySubject::Terminal).await {
                            Ok(crate::ChannelInputDeliveryDisposition::Ready)=>{},
                            Ok(crate::ChannelInputDeliveryDisposition::Confirmed|crate::ChannelInputDeliveryDisposition::Unknown)=>{
                                seen_inputs.insert(key.clone());seen_turns.insert(turn_id);break;
                            },Err(error)=>{warn!(%error,"original Channel terminal delivery is fenced");break;},
                        }
                        let terminal_sender=ReceiptSender::new(sender.clone());
                        let question_only=initial_question.is_some() && terminal_events.is_empty()
                            && matches!(delivery.delivery.turn_outcome,Some(nomifun_agent_contracts::AgentTurnOutcome::InputRequired {..}));
                        let mut terminal_relay=ChannelStreamRelay::new(config.clone(),terminal_sender.clone(),service.stop_confirmations(),service.asset_resolver())
                            .with_input_question_delivery(service.clone()).with_question_receipts(question_receipts.clone());
                        if let Some(request)=initial_question {
                            terminal_relay=terminal_relay.with_initial_question(request);
                        }
                        let delivered=if question_unknown {false} else if question_only {true} else {
                        if delivery.delivery.result_ok==Some(false) {
                            terminal_sender.send_message(&config.plugin_id,&config.chat_id,ChannelMessageService::build_final_message("本次处理未完成，请在会话中查看具体原因。")).await.is_ok()
                        } else {
                            // Tool receipts upload/verify before the complete
                            // canonical text reaches the existing relay.
                            use nomifun_ai_agent::{AgentStreamEvent,protocol::events::{TextEventData,FinishEventData,TurnStopReason}};
                            let input_required=matches!(delivery.delivery.turn_outcome,Some(nomifun_agent_contracts::AgentTurnOutcome::InputRequired {..}));
                            let mut replay=terminal_events;
                            if !input_required && let Some(text)=delivery.delivery.result_text.filter(|text|!text.trim().is_empty()) {
                                replay.push(AgentStreamEvent::Text(TextEventData {content:text,..Default::default()}));
                            }
                            replay.push(AgentStreamEvent::Finish(FinishEventData {turn_outcome:delivery.delivery.turn_outcome,
                                stop_reason:Some(if input_required {TurnStopReason::InputRequired}else{TurnStopReason::EndTurn}),..Default::default()}));
                            let(tx,rx)=tokio::sync::broadcast::channel(replay.len().max(1));
                            for event in replay {let _=tx.send(event);}drop(tx);
                            terminal_relay.run(rx).await
                        }
                        };
                        let(confirmed,ids)=terminal_sender.outcome(delivered);
                        if let Err(error)=service.finish_input_delivery(&config.conversation_id,&key,&turn_id,
                            ChannelInputDeliverySubject::Terminal,confirmed,ids).await {warn!(%error,"original Channel terminal acknowledgement is unknown");}
                        seen_inputs.insert(key.clone());seen_turns.insert(turn_id);break;
                    }
                    // Input may be claimed after this Turn has already emitted
                    // text or tools. A late live subscription cannot prove its
                    // prefix; the original canonical terminal supplies the
                    // complete reply through the same artifact-validating relay.
                }
                Err(crate::error::ChannelError::SessionNotFound(_)|crate::error::ChannelError::UserNotAuthorized(_))=>{
                    seen_inputs.insert(key.clone());break;
                }
                Err(error)=>{warn!(%error,"could not observe the original accepted Channel input");}
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use async_trait::async_trait;
    use nomifun_api_types::{ConversationResponse,CreateConversationRequest,ListMessagesQuery,MessageListResponse,SendMessageRequest};
    use nomifun_common::AppError;
    use crate::{session_port::{ChannelSessionPort,ChannelTurnDelivery,ChannelTurnDeliveryReceipt,ChannelTurnReceiptState,ChannelQuestionDelivery},
        stream_relay::MessageRecorder,types::PluginType};
    struct Port {
        values:Mutex<HashMap<String,VecDeque<ChannelInputObservation>>>,
        deliveries:Mutex<Vec<ChannelQuestionDelivery>>,
        reads:std::sync::atomic::AtomicUsize,
    }
    #[async_trait]
    impl ChannelSessionPort for Port {
        async fn begin_input_delivery(&self,_:&str,_:&str,_:&str,_:&str,_:ChannelInputDeliverySubject)->Result<crate::ChannelInputDeliveryDisposition,AppError> {
            Ok(crate::ChannelInputDeliveryDisposition::Ready)
        }
        async fn finish_input_delivery(&self,_:&str,_:&str,_:&str,_:&str,_:ChannelInputDeliverySubject,_:bool,_:Vec<String>)->Result<(),AppError> {Ok(())}
        async fn is_busy(&self,_:&str)->bool {false}
        async fn turn_outcome(&self,_:&str,_:&str,_:&str)->Result<ChannelTurnReceiptState,AppError> {panic!("observer must use the original input claim")}
        async fn cancel(&self,_:&str,_:&str)->Result<(),AppError> {panic!("observation cannot cancel")}
        async fn list_messages(&self,_:&str,_:&str,_:ListMessagesQuery)->Result<MessageListResponse,AppError> {panic!("no transcript inference")}
        async fn send_turn(&self,_:&str,_:&str,_:&str,_:SendMessageRequest)->Result<ChannelTurnDelivery,AppError> {panic!("observation cannot send")}
        async fn get(&self,_:&str,_:&str)->Result<ConversationResponse,AppError> {panic!("do not substitute the latest Turn")}
        async fn create_idempotent(&self,_:&str,_:CreateConversationRequest,_:&str)->Result<ConversationResponse,AppError> {panic!("observation cannot create")}
        async fn observe_input(&self,_:&str,_:&str,key:&str)->Result<ChannelInputObservation,AppError> {
            self.reads.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            Ok(self.values.lock().unwrap().get_mut(key).and_then(VecDeque::pop_front).unwrap_or(ChannelInputObservation::Pending))
        }
        async fn record_question_delivery(&self,_:&str,_:&str,delivery:ChannelQuestionDelivery)->Result<(),AppError> {
            self.deliveries.lock().unwrap().push(delivery);Ok(())
        }
    }
    fn receipt(text:&str,input_required:bool)->ChannelTurnDelivery {
        ChannelTurnDelivery {delivery:ChannelTurnDeliveryReceipt {message_id:"0190f5fe-7c00-7a00-8000-000000000905".into(),replayed:false,completed:true,
            turn_outcome:Some(if input_required {nomifun_agent_contracts::AgentTurnOutcome::InputRequired {request_id:"original-question".into()}}
                else {nomifun_agent_contracts::AgentTurnOutcome::Answered {message_ids:vec!["0190f5fe-7c00-7a00-8000-000000000906".into()]}}),
            result_ok:Some(true),result_text:Some(text.into()),result_error:None,result_error_code:None,result_error_retryable:None},events:None}
    }
    fn question()->nomifun_ai_agent::protocol::events::InputQuestionEventData {
        nomifun_ai_agent::protocol::events::InputQuestionEventData {
            request:nomifun_agent_contracts::AgentUserInputRequest {request_id:"original-question".into(),question:"Complete authorization.".into(),remaining_work:"Verify the result.".into()},
            target:nomifun_api_types::AgentQuestionReplyTargetDto {turn_id:"0190f5fe-7c00-7a00-8000-000000000902".into(),question_seq:41},
        }
    }
    async fn service(values:HashMap<String,VecDeque<ChannelInputObservation>>)->(Arc<ChannelMessageService>,Arc<Port>) {
        service_with_resolver(values,None).await
    }
    async fn service_with_resolver(values:HashMap<String,VecDeque<ChannelInputObservation>>,
        resolver:Option<Arc<dyn crate::message_service::AssetResolver>>)->(Arc<ChannelMessageService>,Arc<Port>) {
        let db=nomifun_db::init_database_memory().await.unwrap();
        let port=Arc::new(Port {values:Mutex::new(values),deliveries:Default::default(),reads:Default::default()});
        let preferences:Arc<dyn nomifun_db::IClientPreferenceRepository>=Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(db.pool().clone()));
        let mut service=ChannelMessageService::new(port.clone(),Arc::new(crate::channel_settings::ChannelSettingsService::new(preferences)),
            Arc::new(nomifun_db::SqliteChannelRepository::new(db.pool().clone())),"owner".into());
        if let Some(resolver)=resolver {service=service.with_asset_resolver(resolver);}
        (Arc::new(service),port)
    }
    fn config()->RelayConfig {RelayConfig {platform:PluginType::Weixin,plugin_id:"plugin".into(),chat_id:"chat".into(),
        throttle_ms:0,conversation_id:"0190f5fe-7c00-7a00-8000-000000000901".into()}}
    async fn wait_until(mut check:impl FnMut()->bool) {
        tokio::time::timeout(Duration::from_secs(3),async {while !check() {tokio::task::yield_now().await;}}).await.unwrap();
    }
    #[tokio::test]
    async fn one_observer_recovers_finite_questions_and_next_turn_without_duplicate_claim_delivery() {
        let closed=||ChannelInputObservation::Claimed {turn_id:"turn-a".into(),delivery:receipt("Complete authorization.",true),question:Some(question()),terminal_events:Vec::new()};
        let values=HashMap::from([("input-a".into(),VecDeque::from([closed()])),("same-claim".into(),VecDeque::from([closed()])),
            ("input-b".into(),VecDeque::from([ChannelInputObservation::Claimed {turn_id:"turn-b".into(),delivery:receipt("Authorization verified.",false),question:None,terminal_events:Vec::new()}]))]);
        let(service,port)=service(values).await;let recorder=Arc::new(MessageRecorder::new());let shutdown=CancellationToken::new();
        let task=register_input_observer(service.clone(),config(),recorder.clone(),"input-a".into(),shutdown.clone()).unwrap();
        assert!(register_input_observer(service.clone(),config(),recorder.clone(),"same-claim".into(),shutdown.clone()).is_none());
        assert!(register_input_observer(service,config(),recorder.clone(),"input-b".into(),shutdown.clone()).is_none());
        wait_until(||port.reads.load(std::sync::atomic::Ordering::SeqCst)>=3 && recorder.send_count()==2).await;
        let texts=recorder.take_sends().into_iter().filter_map(|message|message.text).collect::<Vec<_>>().join("\n");
        assert_eq!(texts.matches("Complete authorization.").count(),1);assert_eq!(texts.matches("Authorization verified.").count(),1);
        assert_eq!(port.deliveries.lock().unwrap().len(),1);assert_eq!(port.deliveries.lock().unwrap()[0].target,question().target);
        shutdown.cancel();task.await.unwrap();
    }
    #[tokio::test]
    async fn an_already_delivered_question_does_not_create_an_empty_completion_card() {
        let values=HashMap::from([("input".into(),VecDeque::from([ChannelInputObservation::Claimed {turn_id:"question-turn".into(),
            delivery:receipt("Complete authorization.",true),question:Some(question()),terminal_events:Vec::new()}]))]);
        let(service,port)=service(values).await;let recorder=Arc::new(MessageRecorder::new());let shutdown=CancellationToken::new();
        let mut editable=config();editable.platform=PluginType::Lark;
        let task=register_input_observer(service,editable,recorder.clone(),"input".into(),shutdown.clone()).unwrap();
        wait_until(||!port.deliveries.lock().unwrap().is_empty()).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let messages=recorder.take_sends();assert_eq!(messages.len(),1);
        assert!(messages[0].text.as_ref().unwrap().contains("Complete authorization."));
        shutdown.cancel();task.await.unwrap();
    }
    #[tokio::test]
    async fn a_late_stream_suffix_never_replaces_the_complete_canonical_reply() {
        use nomifun_ai_agent::{AgentStreamEvent,protocol::events::{TextEventData,FinishEventData}};
        // The original prefix was emitted before subscription. A receiver
        // containing only the suffix and Finish is insufficient delivery data.
        let(tx,rx)=tokio::sync::broadcast::channel(8);
        tx.send(AgentStreamEvent::Text(TextEventData {content:"suffix.".into(),..Default::default()})).unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();drop(tx);
        let mut live=receipt("prefix suffix.",false);live.delivery.completed=false;live.events=Some(rx);
        let values=HashMap::from([("input".into(),VecDeque::from([
            ChannelInputObservation::Pending,
            ChannelInputObservation::Claimed {turn_id:"same-turn".into(),delivery:live,question:None,terminal_events:Vec::new()},
            ChannelInputObservation::Claimed {turn_id:"same-turn".into(),delivery:receipt("prefix suffix.",false),question:None,terminal_events:Vec::new()}]))]);
        let(service,port)=service(values).await;let recorder=Arc::new(MessageRecorder::new());let shutdown=CancellationToken::new();
        let task=register_input_observer(service,config(),recorder.clone(),"input".into(),shutdown.clone()).unwrap();
        wait_until(||port.reads.load(std::sync::atomic::Ordering::SeqCst)>=3&&recorder.send_count()>0).await;
        let texts=recorder.take_sends().into_iter().filter_map(|message|message.text).collect::<Vec<_>>().join("\n");
        assert_eq!(texts.matches("prefix suffix.").count(),1);
        assert_eq!(texts.matches("suffix.").count(),1);
        shutdown.cancel();task.await.unwrap();
    }

    struct MissingAcknowledgement;
    #[async_trait]
    impl ChannelSender for MissingAcknowledgement {
        async fn send_message(&self,_:&str,_:&str,_:crate::types::UnifiedOutgoingMessage)->Result<String,crate::error::ChannelError> {Ok(String::new())}
        async fn edit_message(&self,_:&str,_:&str,_:&str,_:crate::types::UnifiedOutgoingMessage)->Result<(),crate::error::ChannelError> {Ok(())}
        async fn send_media(&self,_:&str,_:&str,_:crate::types::OutgoingMedia,_:Option<&str>)->Result<String,crate::error::ChannelError> {Ok(String::new())}
    }
    #[tokio::test]
    async fn transport_receipt_observer_never_fabricates_a_missing_provider_identity() {
        let sender=ReceiptSender::new(Arc::new(MissingAcknowledgement));
        assert_eq!(sender.send_message("plugin","chat",ChannelMessageService::build_final_message("Original result")).await.unwrap(),"");
        let(confirmed,ids)=sender.outcome(true);
        assert!(!confirmed);assert!(ids.is_empty());
    }
    #[tokio::test]
    async fn transport_receipt_observer_retains_actual_message_and_edit_acknowledgements_once() {
        let inner=Arc::new(MessageRecorder::new());let sender=ReceiptSender::new(inner.clone());
        let id=sender.send_message("plugin","chat",ChannelMessageService::build_final_message("Original result")).await.unwrap();
        sender.edit_message("plugin","chat",&id,ChannelMessageService::build_final_message("Complete result")).await.unwrap();
        assert_eq!(sender.outcome(true),(true,vec![id]));
        assert_eq!(inner.send_count(),1);
    }
    #[tokio::test]
    async fn blocked_accepted_input_has_an_actionable_saved_notice_without_fake_turn_failure() {
        let values=HashMap::from([("input".into(),VecDeque::from([ChannelInputObservation::Pending,
            ChannelInputObservation::Blocked {reason:"ATTACHMENT_PREPARATION_FAILED".into()}]))]);
        let(service,_)=service(values).await;let recorder=Arc::new(MessageRecorder::new());let shutdown=CancellationToken::new();
        let task=register_input_observer(service,config(),recorder.clone(),"input".into(),shutdown.clone()).unwrap();
        wait_until(||recorder.send_count()>0).await;
        let text=recorder.take_sends().into_iter().filter_map(|message|message.text).collect::<Vec<_>>().join("\n");
        assert!(text.contains("已保存但未处理"));assert!(text.contains("检查附件"));assert!(!text.contains("任务失败"));
        shutdown.cancel();task.await.unwrap();
    }
    #[tokio::test]
    async fn context_cleared_input_stops_observation_without_failure_or_repeated_delivery() {
        let values = HashMap::from([("input".into(), VecDeque::from([
            ChannelInputObservation::Discarded { reason: "CONTEXT_CLEARED".into() },
        ]))]);
        let (service, port) = service(values).await;
        let recorder = Arc::new(MessageRecorder::new());
        let shutdown = CancellationToken::new();
        let task = register_input_observer(service.clone(), config(), recorder.clone(), "input".into(), shutdown.clone()).unwrap();
        wait_until(|| recorder.send_count() > 0).await;
        assert!(register_input_observer(service, config(), recorder.clone(), "input".into(), shutdown.clone()).is_none());
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(port.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        let text = recorder.take_sends().into_iter().filter_map(|message| message.text).collect::<Vec<_>>().join("\n");
        assert!(text.contains("随上下文清空取消处理"));
        assert!(!text.contains("失败"));
        shutdown.cancel();task.await.unwrap();
    }

    struct Resolver(bool);
    #[async_trait]
    impl crate::message_service::AssetResolver for Resolver {
        async fn resolve(&self,id:&str)->Option<crate::types::OutgoingMedia> {
            self.0.then(||crate::types::OutgoingMedia {bytes:vec![1,2,3],mime:"image/png".into(),filename:format!("{id}.png"),kind:crate::types::MediaKind::Image})
        }
    }
    #[tokio::test]
    async fn missed_terminal_media_uses_the_live_relay_upload_and_verification_before_a_green_final() {
        use nomifun_ai_agent::{AgentStreamEvent,protocol::events::{ToolCallEventData,ToolCallStatus}};
        for available in [true,false] {
            let mut tool=ToolCallEventData {identity:Default::default(),call_id:"image".into(),name:"generate_image".into(),
                description:None,status:ToolCallStatus::Completed,artifacts:Vec::new()};
            tool.identity.display.result_asset_ids=vec!["0190f5fe-7c00-7a00-8000-000000000081".into()];
            let values=HashMap::from([("input".into(),VecDeque::from([ChannelInputObservation::Claimed {turn_id:"original-turn".into(),
                delivery:receipt("Created the image.",false),question:None,terminal_events:vec![AgentStreamEvent::ToolCall(tool)]}]))]);
            let(service,port)=service_with_resolver(values,Some(Arc::new(Resolver(available)))).await;
            let recorder=Arc::new(MessageRecorder::new());let shutdown=CancellationToken::new();
            let task=register_input_observer(service,config(),recorder.clone(),"input".into(),shutdown.clone()).unwrap();
            wait_until(||port.reads.load(std::sync::atomic::Ordering::SeqCst)>0 && recorder.send_count()>0).await;
            shutdown.cancel();task.await.unwrap();
            let sends=recorder.take_sends();
            assert_eq!(recorder.take_media().len(),usize::from(available));
            assert_eq!(sends.iter().any(|message|message.text.as_deref()==Some("Created the image.")),available,
                "failed media verification cannot fall through to a plain successful body");
            if !available {assert!(sends.iter().any(|message|message.text.as_deref().is_some_and(|text|text.contains("failed") || text.contains("could not"))));}
        }
    }

}
