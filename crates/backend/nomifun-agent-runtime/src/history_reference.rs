//! Explicit user turn addresses resolve through the existing scoped reader.
//! No store, owner invocation, inferred target, or fresh-evidence path lives here.
use std::{collections::BTreeSet, time::Duration};
use nomifun_chat_model_broker::{ChatCausality, ChatContentPart, ChatMessage, ChatRole};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use crate::{AgentEngineError, AgentHistoryPort, EngineBinding};

const MAX_REFERENCES: usize = 4;
const MAX_PAGES: usize = 8;
const LOOKUP_BUDGET: Duration = Duration::from_secs(5);

pub(crate) fn references(input:&ChatMessage, causality:&ChatCausality)->Vec<String> {
    addressed(input,causality.agent_session_id.as_ref(),causality.turn_operation_id.as_ref())
}

pub(crate) fn addressed(input:&ChatMessage,session:&str,current:&str)->Vec<String> {
    if input.role!=ChatRole::User {return Vec::new();}
    let mut seen=BTreeSet::new();let mut result=Vec::new();
    for part in &input.content {
        let ChatContentPart::Text {text}=part else {continue;};
        for (start,_) in text.match_indices("turn:user:") {
            let suffix=&text[start..];
            let end=suffix.find(|ch:char|!ch.is_ascii_alphanumeric()&&!matches!(ch,':'|'-'|'_')).unwrap_or(suffix.len());
            let address=&suffix[..end];let fields=address.split(':').collect::<Vec<_>>();
            if address.len()>256 || fields.len()!=5 || fields.iter().any(|field|field.is_empty())
                || fields[3]!=session || address==current {continue;}
            if seen.insert(address.to_owned()) {result.push(address.to_owned());}
            if result.len()==MAX_REFERENCES {return result;}
        }
    }
    result
}

pub(crate) async fn load(
    archive:&mut crate::tool_archive::ToolArchive, references:&[String], port:&dyn AgentHistoryPort,
    causality:&ChatCausality,binding:&EngineBinding,cancellation:&CancellationToken,
)->Result<Vec<Value>,AgentEngineError> {
    let deadline=tokio::time::Instant::now()+LOOKUP_BUDGET;
    let mut reports=Vec::new();let mut remaining=MAX_PAGES;
    for target in references.iter().take(MAX_REFERENCES) {
        if remaining==0 || tokio::time::Instant::now()>=deadline {break;}
        let mut cursor=None;let mut seen=BTreeSet::new();
        let mut report=json!({"source_turn":target,"status":"reference_not_loaded","current_evidence":false});
        // Optional exact capability never falls back after an authoritative
        // outcome/error. Unsupported defaults perform no page read and retain
        // the original eight-page cursor budget. A direct page consumes one.
        let direct=tokio::select! {
            biased;
            _=cancellation.cancelled()=>return Err(AgentEngineError::Cancelled),
            result=tokio::time::timeout_at(deadline,port.read_exact(causality,target))=>match result {
                Ok(Ok(page))=>Ok(page),
                Ok(Err(AgentEngineError::Cancelled))=>return Err(AgentEngineError::Cancelled),
                Ok(Err(_))|Err(_)=>Err(()),
            }
        };
        let fallback=match direct {
            Ok(Some(page))=>{
                remaining-=1;
                if page.turn.as_ref().is_some_and(|turn| turn.operation_id==*target)
                    && tokio::time::Instant::now()<deadline {
                    report=admit(archive,page,target,port,causality,binding,cancellation,deadline).await?;
                }
                false
            }
            Ok(None)=>true,
            Err(())=>false,
        };
        for _ in 0..(if fallback {MAX_PAGES} else {0}) {
            if remaining==0 || tokio::time::Instant::now()>=deadline {break;}
            remaining-=1;
            let page=tokio::select! {
                biased;
                _=cancellation.cancelled()=>return Err(AgentEngineError::Cancelled),
                result=tokio::time::timeout_at(deadline,port.read_previous(causality,cursor.as_deref()))=>match result {
                    Ok(Ok(page))=>page,
                    Ok(Err(AgentEngineError::Cancelled))=>return Err(AgentEngineError::Cancelled),
                    Ok(Err(_))|Err(_)=>break,
                }
            };
            if tokio::time::Instant::now()>=deadline {break;}
            let Some(turn)=page.turn else {break;};
            if turn.operation_id.is_empty() || turn.operation_id.len()>256
                || turn.operation_id.chars().any(char::is_control)
                || turn.operation_id==causality.turn_operation_id.as_ref()
                || !seen.insert(turn.operation_id.clone()) {break;}
            if crate::stream_limits::serialized_size(&turn.events,8*1024*1024).is_err() {break;}
            let next=turn.operation_id.clone();
            if &next==target {
                report=admit(archive,crate::AgentHistoryPage {turn:Some(turn),has_older:page.has_older},target,port,causality,binding,cancellation,deadline).await?;
                break;
            }
            if !page.has_older {break;}
            cursor=Some(next);
        }
        // Keep only bounded lookup metadata. Result bodies and notices remain
        // in the validated archive, never promoted into system instructions.
        if let Some(fields)=report.as_object_mut() {
            fields.retain(|key,_|matches!(key.as_str(),"source_turn"|"status"|"receipt_status"|"imported_records"|"retained_records"|"evicted_records"|"current_evidence"));
            fields.insert("current_evidence".into(),json!(false));
        }
        reports.push(report);
    }
    Ok(reports)
}

#[allow(clippy::too_many_arguments)]
async fn admit(
    archive:&mut crate::tool_archive::ToolArchive, page:crate::AgentHistoryPage,target:&str,
    port:&dyn AgentHistoryPort,causality:&ChatCausality,binding:&EngineBinding,
    cancellation:&CancellationToken,deadline:tokio::time::Instant,
)->Result<Value,AgentEngineError> {
    if page.turn.as_ref().is_none_or(|turn| turn.operation_id!=target
        || crate::stream_limits::serialized_size(&turn.events,8*1024*1024).is_err())
    {
        return Ok(json!({"source_turn":target,"status":"reference_not_admitted","current_evidence":false}));
    }
    let admitted=tokio::select! {
        biased;
        _=cancellation.cancelled()=>return Err(AgentEngineError::Cancelled),
        result=tokio::time::timeout_at(deadline,archive.import_scoped_reference(page,binding,port,causality))=>
            result.unwrap_or_else(|_|Err(AgentEngineError::ContextAssembly("reference lookup budget expired".into()))),
    };
    match admitted {
        Err(AgentEngineError::Cancelled)=>Err(AgentEngineError::Cancelled),
        Ok(report)=>Ok(report),
        Err(_)=>Ok(json!({"source_turn":target,"status":"reference_not_admitted","current_evidence":false})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize,Ordering};
    use nomifun_agent_contracts::{AgentSessionId,RuntimeBindingId,ResolvedSnapshotRef,ResolvedSnapshotId,DigestHex,EventId,OperationId,ModelRouteId,ChatRouteIdentity};
    fn fixture()->(EngineBinding,ChatCausality) {
        let binding=crate::AgentEngine::new(crate::AgentEngineBuild {build_id:"coding-dev".into(),build_digest:DigestHex::from("a".repeat(64))}).unwrap()
            .bind(AgentSessionId::from("session"),RuntimeBindingId::from("binding"),ResolvedSnapshotRef {
                snapshot_id:ResolvedSnapshotId::from("snapshot"),snapshot_digest:DigestHex::from("b".repeat(64))}).unwrap();
        let cause=ChatCausality {agent_session_id:AgentSessionId::from("session"),turn_operation_id:OperationId::from("turn"),
            causation_event_id:EventId::from("input"),resolved_snapshot_ref:binding.resolved_snapshot_ref().clone(),
            route_identity:ChatRouteIdentity::new("preset@1","agent_chat",ModelRouteId::from("route"),1),operation_id:OperationId::from("model")};
        (binding,cause)
    }
    #[test]
    fn references_are_exact_current_user_addresses_not_old_or_foreign_text() {
        let mut input=crate::context_lifecycle::text_message(ChatRole::User,
            "operation_id：`turn:user:msg:session:old`，turn:user:msg:session:old; turn:user:msg:foreign:old turn:user:msg:session:current turn:user:msg:session:old:extra".into());
        assert_eq!(addressed(&input,"session","turn:user:msg:session:current"),["turn:user:msg:session:old"]);
        input.role=ChatRole::Assistant;assert!(addressed(&input,"session","turn:user:msg:session:current").is_empty());
    }

    #[tokio::test]
    async fn cyclic_and_long_history_cursors_are_bounded_without_importing_other_turns() {
        #[derive(Debug)] struct Pages {calls:AtomicUsize,cycle:bool}
        #[async_trait::async_trait] impl AgentHistoryPort for Pages {
            async fn read_previous(&self,_:&ChatCausality,_:Option<&str>)->Result<crate::AgentHistoryPage,AgentEngineError> {
                let n=self.calls.fetch_add(1,Ordering::SeqCst);
                Ok(crate::AgentHistoryPage {has_older:true,turn:Some(crate::AgentRecordedTurn {
                    operation_id:if self.cycle {"turn:user:page:session:other".into()} else {format!("turn:user:page{n}:session:other")},
                    receipt_status:"failed".into(),requirement:crate::context_lifecycle::text_message(ChatRole::User,"old".into()),events:vec![]})})
            }
        }
        let (binding,cause)=fixture();
        for (cycle,expected) in [(true,2),(false,MAX_PAGES)] {
            let port=Pages {calls:AtomicUsize::new(0),cycle};let mut archive=crate::tool_archive::ToolArchive::new("scope".into());
            let before=archive.context();
            let report=load(&mut archive,&["turn:user:msg:session:target".into()],&port,&cause,&binding,&CancellationToken::new()).await.unwrap();
            assert_eq!(port.calls.load(Ordering::SeqCst),expected);assert_eq!(archive.context(),before);
            assert_eq!(report[0]["status"],"reference_not_loaded");assert_eq!(report[0]["current_evidence"],false);
        }
    }

    #[tokio::test]
    async fn lookup_timeout_and_cancellation_are_not_absence_or_cleanup_proof() {
        #[derive(Debug,Default)] struct Pending(AtomicUsize);
        #[async_trait::async_trait] impl AgentHistoryPort for Pending {
            async fn read_previous(&self,_:&ChatCausality,_:Option<&str>)->Result<crate::AgentHistoryPage,AgentEngineError> {
                self.0.fetch_add(1,Ordering::SeqCst);std::future::pending().await
            }
        }
        let (binding,cause)=fixture();let port=Pending::default();let mut archive=crate::tool_archive::ToolArchive::new("scope".into());
        let report=load(&mut archive,&["turn:user:msg:session:target".into()],&port,&cause,&binding,&CancellationToken::new()).await.unwrap();
        assert_eq!(port.0.load(Ordering::SeqCst),1);assert_eq!(report[0]["status"],"reference_not_loaded");
        let cancel=CancellationToken::new();cancel.cancel();
        assert!(matches!(load(&mut archive,&["turn:user:msg:session:target".into()],&port,&cause,&binding,&cancel).await,Err(AgentEngineError::Cancelled)));
        assert_eq!(port.0.load(Ordering::SeqCst),1);
    }

    #[tokio::test]
    async fn exact_reference_beyond_cursor_window_imports_only_addressed_validated_turn() {
        #[derive(Debug)] struct Exact { calls:AtomicUsize, binding:EngineBinding }
        #[async_trait::async_trait] impl AgentHistoryPort for Exact {
            async fn read_exact(&self,_:&ChatCausality,target:&str)->Result<Option<crate::AgentHistoryPage>,AgentEngineError> {
                self.calls.fetch_add(1,Ordering::SeqCst);
                assert_eq!(target,"turn:user:msg:session:older-than-eight");
                Ok(Some(crate::AgentHistoryPage {has_older:false,turn:Some(crate::AgentRecordedTurn {
                    operation_id:target.into(),receipt_status:"failed".into(),
                    requirement:crate::context_lifecycle::text_message(ChatRole::User,"original file task".into()),
                    events:vec![
                        crate::AgentEngineEvent::TurnStarted {binding:self.binding.clone(),turn_operation_id:target.into()},
                        crate::AgentEngineEvent::ModelStepStarted {step:1,operation_id:"old:model:1".into()},
                        crate::AgentEngineEvent::ToolCallCompleted {step:1,call:nomifun_chat_model_broker::ChatToolCall {
                            call_id:"original-read".into(),name:"read_file".into(),
                            arguments:nomifun_agent_contracts::StrictJsonValue(json!({"path":"result.txt"})),provider_metadata:None}},
                        crate::AgentEngineEvent::ToolCompleted {step:1,result:crate::AgentToolResult::text(
                            "original-read".into(),"第一行 MAC-B\n第二行 after\n",false)},
                        crate::AgentEngineEvent::TurnFailed {model_steps:1,message:"old report failure".into()},
                    ],
                })}))
            }
            async fn read_previous(&self,_:&ChatCausality,_:Option<&str>)->Result<crate::AgentHistoryPage,AgentEngineError> {
                panic!("direct addressing must not walk or import unrelated intervening turns")
            }
        }
        let (binding,cause)=fixture();let port=Exact {calls:AtomicUsize::new(0),binding:binding.clone()};
        let mut archive=crate::tool_archive::ToolArchive::new("scope".into());
        let report=load(&mut archive,&["turn:user:msg:session:older-than-eight".into()],&port,&cause,&binding,&CancellationToken::new()).await.unwrap();
        assert_eq!(port.calls.load(Ordering::SeqCst),1);
        assert_eq!(report[0]["status"],"loaded");assert_eq!(report[0]["imported_records"],1);
        assert_eq!(report[0]["source_turn"],"turn:user:msg:session:older-than-eight");
        assert_eq!(report[0]["current_evidence"],false);
    }

    #[tokio::test]
    async fn direct_unavailable_or_wrong_target_does_not_fall_back_or_import() {
        #[derive(Debug)] struct Exact { wrong:bool }
        #[async_trait::async_trait] impl AgentHistoryPort for Exact {
            async fn read_exact(&self,_:&ChatCausality,_:&str)->Result<Option<crate::AgentHistoryPage>,AgentEngineError> {
                Ok(Some(crate::AgentHistoryPage {has_older:false,turn:if self.wrong {Some(crate::AgentRecordedTurn {
                    operation_id:"turn:user:msg:session:other".into(),receipt_status:"failed".into(),
                    requirement:crate::context_lifecycle::text_message(ChatRole::User,"other".into()),events:vec![],
                })} else {None}}))
            }
            async fn read_previous(&self,_:&ChatCausality,_:Option<&str>)->Result<crate::AgentHistoryPage,AgentEngineError> {
                panic!("an authoritative direct outcome must not bypass its history floor with a cursor fallback")
            }
        }
        let (binding,cause)=fixture();
        for wrong in [false,true] {
            let mut archive=crate::tool_archive::ToolArchive::new("scope".into());let before=archive.context();
            let report=load(&mut archive,&["turn:user:msg:session:target".into()],&Exact {wrong},&cause,&binding,&CancellationToken::new()).await.unwrap();
            assert_eq!(archive.context(),before);assert_eq!(report[0]["status"],"reference_not_loaded");
            assert_eq!(report[0]["current_evidence"],false);
        }
    }
}
