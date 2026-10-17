//! Exact-source asynchronous desktop results delivered through the existing
//! Robot connection actor. The Execution report outbox supplies retry/lifetime.

use super::*;
use crate::router::nomi_core_session::DirectedReportDeliveryPort;
use nomifun_common::AppError;
use nomifun_voice_contracts::VoiceSpeechSourceRef;

#[async_trait::async_trait]
impl DirectedReportDeliveryPort for AppRobotBackend {
    async fn hold_input_connection(&self,owner_id:&str,companion_id:&str,robot_id:&str,connection_id:&str)
        ->Result<Option<nomifun_robot::registry::RobotActionLease>,AppError>{
        if owner_id!=self.owner_user_id.as_ref(){return Err(AppError::Forbidden("Robot input belongs to another owner".into()));}
        let Some(lease)=self.registry.hold_action_connection(robot_id,connection_id).await else{return Ok(None);};
        if !self.registry.get(robot_id).await.is_some_and(|record|record.companion_id.as_deref()==Some(companion_id)){return Ok(None);}
        Ok(Some(lease))
    }
    async fn deliver_input_required(&self,owner_id:&str,input:&nomifun_agent_execution::DirectedDelegationInput)->Result<(),AppError>{
        if owner_id!=self.owner_user_id.as_ref(){return Err(AppError::Forbidden("Robot question belongs to another owner".into()));}
        let Some(notice)=self.sessions.prepare_directed_question_notice(owner_id,input).await? else{return Ok(());};
        let Some(_connection)=self.hold_input_connection(owner_id,&notice.source.companion_id,&notice.source.robot_id,&notice.source.connection_id).await?
            else{return Ok(());};
        let Some(current)=self.sessions.prepare_directed_question_notice(owner_id,input).await? else{return Ok(());};
        if current.speech_source!=notice.speech_source{return Ok(());}
        if let Err(error)=self.registry.speak_requested_question_source(&notice.source.robot_id,&notice.source.connection_id,notice.text.clone(),notice.speech_source.clone(),notice.presentation_id.clone()).await{
            if error.can_retry_same_source(){return Err(AppError::Conflict(format!("Robot question playback is pending: {error}")));}
            tracing::debug!(%error,"automatic Robot question playback was suppressed");
        } else {self.sessions.record_directed_question_playback(owner_id,&notice).await?;}
        Ok(())
    }
    async fn deliver_report(&self, owner_id: &str, session_id: &str,
        operation_id: &str, report_message_id: &str) -> Result<(), AppError> {
        if owner_id != self.owner_user_id.as_ref() {
            return Err(AppError::Forbidden("Robot report belongs to another installation owner".into()));
        }
        let Some(prepared)=self.sessions.prepare_directed_report_delivery(owner_id,session_id,
            operation_id,report_message_id).await? else { return Ok(()); };
        // Automatic delivery stays on the original authenticated connection.
        // Offline/reconnected devices keep the report in history, without a
        // second attempt at physical speech on another connection or device.
        if !self.registry.connection_matches(prepared.robot_id(),prepared.connection_id()).await {
            return Ok(());
        }
        if !self.registry.get(prepared.robot_id()).await.is_some_and(|record|
            record.companion_id.as_deref()==Some(prepared.companion_id())) { return Ok(()); }
        // Match normal Robot ingress: retain the original connection while
        // the canonical input is accepted and its local Runtime/tool surface
        // is materialized. send_message only schedules the model task.
        let Some(admission_connection)=self.hold_input_connection(owner_id,prepared.companion_id(),
            prepared.robot_id(),prepared.connection_id()).await? else{return Ok(());};
        let result=self.sessions.run_directed_report_result(&prepared).await;
        drop(admission_connection);
        let Some(speech)=result? else { return Ok(()); };
        for answer in speech {
            // Context clear, rebind and disconnect can occur while the
            // lightweight follow-up model is running. Recheck before playback.
            if self.sessions.prepare_directed_report_delivery(owner_id,session_id,
                operation_id,report_message_id).await?.is_none() { return Ok(()); }
            let Some(_connection)=self.registry.hold_action_connection(prepared.robot_id(),prepared.connection_id()).await
                else { return Ok(()); };
            if !self.registry.get(prepared.robot_id()).await.is_some_and(|current|
                current.companion_id.as_deref()==Some(prepared.companion_id())
                    ) {
                return Ok(());
            }
            let source=VoiceSpeechSourceRef {message_id:answer.message_id,revision:answer.revision,
                through_seq:answer.through_seq};
            if let Err(error)=self.registry.speak_requested_source(prepared.robot_id(),prepared.connection_id(),answer.text,source).await {
                if error.can_retry_same_source() {
                    return Err(AppError::Conflict(format!("Robot result playback was not accepted: {error}")));
                }
                tracing::debug!(%error, "automatic Robot report playback was suppressed");
                return Ok(());
            }
        }
        // Acceptance is all the transport proves. The connection actor handles
        // barge-in, TTS failures and cleanup; none of those reopens the task.
        Ok(())
    }
}
