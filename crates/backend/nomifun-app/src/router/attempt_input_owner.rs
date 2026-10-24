//! The Session owner keeps the original Attempt input through native dispatch.
//! The explicit boxed boundary also keeps the full dispatch graph out of each
//! async-trait caller and registrar wrapper; this adds no new task supervisor.
use super::*;

impl NomiCoreSessionOwner {
    pub(super) fn deliver_execution_input_owned(self:Arc<Self>,user:String,session:AgentSessionId,
        key:String,authority:AgentExecutionTurnAuthority,request:SendMessageRequest)
        ->std::pin::Pin<Box<dyn Future<Output=Result<nomifun_agent_execution::AgentExecutionDelivery,AppError>>+Send>> {
        self.deliver_attempt_input_owned(user,session,key,authority,request,None)
    }

    /// Keep the original prepared request until a real native producer or a
    /// canonical exit acknowledges custody. A duplicate Accepted receipt is
    /// observation only: another send can only follow formal native claim.
    pub(super) fn deliver_attempt_input_owned(self:Arc<Self>,user:String,session:AgentSessionId,
        key:String,authority:AgentExecutionTurnAuthority,request:SendMessageRequest,
        parent_reply:Option<nomifun_agent_contracts::AgentParentQuestionReply>)
        ->std::pin::Pin<Box<dyn Future<Output=Result<nomifun_agent_execution::AgentExecutionDelivery,AppError>>+Send>> {
        Box::pin(async move {
            let operation=Self::turn_operation_id(&user,session.as_ref(),&key);
            let _delivery=self.session_operation_lock(&format!("attempt-input:{}:{}",session.as_ref(),operation.as_ref())).read_owned().await;
            let mut admit=true;
            let mut admission_error=None;
            let mut recovery_rejected=false;
            let mut backoff=Duration::from_millis(100);
            loop {
                if admit {
                    let delivery:std::pin::Pin<Box<dyn Future<Output=Result<nomifun_agent_execution::AgentExecutionDelivery,AppError>>+Send+'_>>=if let Some(source)=parent_reply.as_ref() {
                        Box::pin(self.deliver_parent_question_reply_once(&user,&session,&key,&authority,request.clone(),source))
                    } else {
                        Box::pin(self.deliver_execution_input_once(&user,&session,&key,&authority,request.clone()))
                    };
                    let result=delivery.await;
                    if let Ok(delivery)=&result && delivery.completed {return result;}
                    admission_error=result.err();
                    admit=false;
                }
                let observed=async {
                    #[cfg(test)]
                    self.attempt_input_owner_cutpoint(&session,&request,"observe").await?;
                    let receipt=self.canonical.store().read_turn_receipt(&session,&operation).await.map_err(agent_session_store_error)?;
                    if receipt.status==nomifun_agent_session::TurnReceiptStatus::NotFound {return Ok(None);}
                    // ACK-unknown goes through the Store's full same-key pair
                    // verification. Copy only the original immutable admission;
                    // caller bytes and private parent provenance must match.
                    if !self.confirm_original_attempt_input(&user,&session,&key,&operation,&authority,&request,parent_reply.as_ref()).await? {
                        return Err(AppError::Conflict("Attempt input key belongs to different original prepared bytes".into()));
                    }
                    let suspended=self.canonical.store().native_pause_state(&session,&operation).await.map_err(agent_session_store_error)?.is_some();
                    if receipt.status!=nomifun_agent_session::TurnReceiptStatus::Running || suspended
                        ||self.collaboration_has_native_progress_owner_without_input(session.as_ref(),operation.as_ref()).await? {
                        return <Self as nomifun_agent_execution::AgentExecutionSessionPort>::delivery_result(self.as_ref(),&user,session.as_ref(),&key).await;
                    }
                    let engines=self.native_engines.get().and_then(std::sync::Weak::upgrade)
                        .ok_or_else(||AppError::Conflict("original Attempt input native host is unavailable".into()))?;
                    #[cfg(test)]
                    self.attempt_input_owner_cutpoint(&session,&request,"recovery_facts").await?;
                    let facts=match self.canonical.store().native_recovery_facts(&session,&operation).await {
                        Ok(facts)=>facts,
                        Err(nomifun_agent_session::SessionStoreError::ExecutionFenced)=>{
                            // This loop has already confirmed the complete
                            // original input. Its producer can finish between
                            // that snapshot and this active-only fact read.
                            // Confirm only the same operation's real exit; an
                            // unrelated terminal cannot repair input damage.
                            let latest=self.canonical.store().read_turn_receipt(&session,&operation).await.map_err(agent_session_store_error)?;
                            if !matches!(latest.status,nomifun_agent_session::TurnReceiptStatus::Running|nomifun_agent_session::TurnReceiptStatus::NotFound)
                                ||self.canonical.store().native_pause_state(&session,&operation).await.map_err(agent_session_store_error)?.is_some() {
                                return <Self as nomifun_agent_execution::AgentExecutionSessionPort>::delivery_result(self.as_ref(),&user,session.as_ref(),&key).await;
                            }
                            return Err(agent_session_store_error(nomifun_agent_session::SessionStoreError::ExecutionFenced));
                        }
                        Err(error)=>return Err(agent_session_store_error(error)),
                    };
                    if recovery_rejected || facts.head.status!="running" {
                        if self.canonical.store().quarantine_native_recovery(&facts.session.owner_ref,&session,&operation,facts.execution_fence)
                            .await.map_err(agent_session_store_error)? {
                            if let Some(start)=receipt.started_event.as_ref() && let Some(root)=facts.event_payloads.get(start.event_id.as_ref())
                                .and_then(|payload|payload.get("source_message_id")).and_then(Value::as_str) {
                                self.user_events.send_to_user(&user,Self::canonical_turn_paused_wire_event(&session,root));
                            }
                        }
                    } else {
                        let recovery:std::pin::Pin<Box<dyn Future<Output=Result<(),AppError>>+Send+'_>>=
                            Box::pin(self.recover_native_turn(engines,&session,&operation,Some(facts.execution_generation)));
                        let result=recovery.await;
                        if let Err(error)=&result {recovery_rejected=!attempt_input_unavailable(error);}
                        #[cfg(test)]
                        if matches!(&result,Err(AppError::ExecutionAdmissionPending)) {
                            self.attempt_input_owner_cutpoint(&session,&request,"recovery_pending").await?;
                        }
                        result?;
                    }
                    // Reobserve the exact original claim/terminal. Neither
                    // successful claim nor an idle registry is attachment.
                    Ok(None)
                }.await;
                match observed {
                    Ok(Some(delivery))=>return Ok(delivery),
                    Ok(None)=>{
                        let missing=self.canonical.store().read_turn_receipt(&session,&operation).await.map_err(agent_session_store_error);
                        if matches!(&missing,Ok(receipt) if receipt.status==nomifun_agent_session::TurnReceiptStatus::NotFound) {
                            if let Some(error)=admission_error.take() && !attempt_input_unavailable(&error) {return Err(error);}
                            admit=true;
                        }
                    }
                    Err(error)=>{
                        // Different bytes are never adopted or settled. Other
                        // permanent recovery errors are quarantined by its
                        // exact writer instead of turning into another send.
                        if matches!(&error,AppError::NotFound(_)|AppError::Forbidden(_)) {return Err(error);}
                        if !attempt_input_unavailable(&error) && !recovery_rejected {return Err(error);}
                        tracing::warn!(session_id=session.as_ref(),operation_id=operation.as_ref(),error_code=error.error_code(),
                            "original Attempt input retains admission/attachment responsibility; business input is not resent");
                    }
                }
                // No Session business gate or database transaction crosses
                // this wait. Stop can fence the exact input before next claim.
                tokio::select! {_=self.background_tasks.shutdown_signal()=>return Err(AppError::Conflict("Session owner closed while retaining original Attempt input".into())),
                    _=tokio::time::sleep(backoff)=>{}}
                backoff=(backoff*2).min(Duration::from_secs(1));
            }
        })
    }

    async fn deliver_execution_input_once(&self,user:&str,session:&AgentSessionId,key:&str,
        authority:&AgentExecutionTurnAuthority,request:SendMessageRequest)->Result<nomifun_agent_execution::AgentExecutionDelivery,AppError> {
        let exact=self.canonical.get(&PrincipalRef {principal_kind:"user".into(),principal_id:user.into()},session).await?;
        let _workspace=self.guard_binding_workspace_read(user,&exact.session.agent_binding)?;
        let _gate=self.session_operation_lock(session.as_ref()).write_owned().await;
        self.dispatch_canonical_turn_locked_with_source_ref(user,session,key,request,false,
            None,None,None,false,None,None,None,Some(authority)).await.map(agent_execution_delivery_from_conversation)
    }

    async fn confirm_original_attempt_input(&self,user:&str,session:&AgentSessionId,key:&str,operation:&OperationId,
        authority:&AgentExecutionTurnAuthority,request:&SendMessageRequest,
        parent_reply:Option<&nomifun_agent_contracts::AgentParentQuestionReply>)->Result<bool,AppError> {
        let (_,original)=self.canonical.store().turn_accepted_input(session,operation).await.map_err(agent_session_store_error)?;
        let mut expected=canonical_turn_input(request);
        if let Some(admission)=original.0.get("admission") {expected["admission"]=admission.clone();}
        if let Some(source)=parent_reply {
            expected["parent_question_reply"]=serde_json::to_value(source).map_err(|error|AppError::Internal(error.to_string()))?;
            expected["reply_to"]=original.0["reply_to"].clone();
            super::parent_question_reply::validate_metadata(&expected)?;
        }
        if expected!=original.0 {return Ok(false);}
        self.canonical.start_execution_turn(&PrincipalRef {principal_kind:"user".into(),principal_id:user.into()},session,key,expected,authority).await?;
        Ok(true)
    }
}

pub(super) fn attempt_input_unavailable(error:&AppError)->bool {
    matches!(error,AppError::DatabaseUnavailable{..}|AppError::ExecutionAdmissionPending)
}

#[cfg(test)]
struct InputOwnerFault {owner:std::sync::Weak<NomiCoreSessionOwner>,error:Option<AppError>,entered:Arc<tokio::sync::Notify>,release:Option<Arc<tokio::sync::Notify>>}
#[cfg(test)]
static INPUT_OWNER_FAULTS:std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeMap<(usize,String,String),InputOwnerFault>>>=std::sync::OnceLock::new();
#[cfg(test)]
impl NomiCoreSessionOwner {
    pub(in crate::router) fn inject_attempt_input_owner_fault(self:&Arc<Self>,marker:&str,stage:&str,error:AppError,hold:bool)
        ->(Arc<tokio::sync::Notify>,Option<Arc<tokio::sync::Notify>>) {
        self.register_attempt_input_owner_cutpoint(marker,stage,Some(error),hold)
    }
    pub(in crate::router) fn hold_attempt_input_owner_cutpoint(self:&Arc<Self>,marker:&str,stage:&str)
        ->(Arc<tokio::sync::Notify>,Arc<tokio::sync::Notify>) {
        let (entered,release)=self.register_attempt_input_owner_cutpoint(marker,stage,None,true);
        (entered,release.expect("requested exact cutpoint hold"))
    }
    fn register_attempt_input_owner_cutpoint(self:&Arc<Self>,marker:&str,stage:&str,error:Option<AppError>,hold:bool)
        ->(Arc<tokio::sync::Notify>,Option<Arc<tokio::sync::Notify>>) {
        let entered=Arc::new(tokio::sync::Notify::new());let release=hold.then(||Arc::new(tokio::sync::Notify::new()));
        let identity=Arc::as_ptr(self) as usize;
        INPUT_OWNER_FAULTS.get_or_init(Default::default).lock().unwrap().insert((identity,marker.into(),stage.into()),
            InputOwnerFault {owner:Arc::downgrade(self),error,entered:entered.clone(),release:release.clone()});
        (entered,release)
    }
    pub(super) async fn attempt_input_owner_cutpoint(&self,_session:&AgentSessionId,request:&SendMessageRequest,stage:&str)->Result<(),AppError> {
        let fault={
            let mut faults=INPUT_OWNER_FAULTS.get_or_init(Default::default).lock().unwrap();
            faults.retain(|_,fault|fault.owner.strong_count()>0);
            let identity=std::ptr::from_ref(self) as usize;
            let key=faults.keys().find(|(owner,marker,point)|*owner==identity&&point==stage&&request.content.contains(marker)).cloned();
            key.and_then(|key|faults.remove(&key))
        };
        if let Some(fault)=fault {fault.entered.notify_one();if let Some(release)=fault.release {release.notified().await;}if let Some(error)=fault.error {return Err(error);}}
        Ok(())
    }
}
