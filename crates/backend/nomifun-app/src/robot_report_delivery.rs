//! Exact-source task questions delivered through the original Robot connection.
//! Child results are integrated and spoken through the parent Runtime.

use super::*;
use crate::router::nomi_core_session::ExecutionReportDeliveryPort;
use nomifun_common::AppError;

#[async_trait::async_trait]
impl ExecutionReportDeliveryPort for AppRobotBackend {
    async fn hold_input_connection(&self,owner_id:&str,companion_id:&str,robot_id:&str,connection_id:&str)
        ->Result<Option<nomifun_robot::registry::RobotActionLease>,AppError>{
        if owner_id!=self.owner_user_id.as_ref(){return Err(AppError::Forbidden("Robot input belongs to another owner".into()));}
        let Some(lease)=self.registry.hold_action_connection(robot_id,connection_id).await else{return Ok(None);};
        if !self.registry.get(robot_id).await.is_some_and(|record|record.companion_id.as_deref()==Some(companion_id)){return Ok(None);}
        Ok(Some(lease))
    }
    async fn deliver_input_required(&self,owner_id:&str,input:&nomifun_agent_execution::AgentExecutionInput)->Result<(),AppError>{
        if owner_id!=self.owner_user_id.as_ref(){return Err(AppError::Forbidden("Robot question belongs to another owner".into()));}
        let Some(notice)=self.sessions.prepare_execution_question_notice(owner_id,input).await? else{return Ok(());};
        let Some(_connection)=self.hold_input_connection(owner_id,&notice.source.companion_id,&notice.source.robot_id,&notice.source.connection_id).await?
            else{return Ok(());};
        let Some(current)=self.sessions.prepare_execution_question_notice(owner_id,input).await? else{return Ok(());};
        if current.speech_source!=notice.speech_source{return Ok(());}
        if let Err(error)=self.registry.speak_requested_context_source(&notice.source.robot_id,&notice.source.connection_id,notice.text.clone(),notice.speech_source.clone(),Some(notice.presentation_id.clone()),notice.playback_context()).await{
            if error.can_retry_same_source(){return Err(AppError::Conflict(format!("Robot question playback is pending: {error}")));}
            tracing::debug!(%error,"automatic Robot question playback was suppressed");
        }
        // Queue acceptance is not a presentation. The connection actor calls
        // question_presented only after this exact sentence enters its writer.
        Ok(())
    }
}
