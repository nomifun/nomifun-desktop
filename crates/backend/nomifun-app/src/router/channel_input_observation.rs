//! Channel observes exact accepted inputs independently of Runtime dispatch.
use super::*;

impl NomiCoreSessionOwner {
    pub(super) async fn recover_channel_inputs(&self,owner_id:&str)->Result<Vec<nomifun_channel::RecoverableChannelInput>,AppError> {
        let owner=PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.into()};
        Ok(self.canonical.store().recoverable_channel_inputs(&owner).await.map_err(agent_session_store_error)?
            .into_iter().map(|input|nomifun_channel::RecoverableChannelInput {session_id:input.agent_session_id.as_ref().into(),
                input_event_id:input.input_event_id.as_ref().into(),source:input.source}).collect())
    }

    pub(super) async fn begin_channel_input_delivery(&self,owner_id:&str,session_id:&str,input_id:&str,turn_id:&str,
        subject:nomifun_agent_contracts::ChannelInputDeliverySubject)->Result<nomifun_channel::ChannelInputDeliveryDisposition,AppError> {
        let owner=PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.into()};
        let outcome=self.canonical.store().begin_channel_input_delivery(&owner,&session_id.to_owned().into(),&input_id.to_owned().into(),turn_id,subject)
            .await.map_err(agent_session_store_error)?;
        use nomifun_agent_session::ChannelInputDeliveryDisposition as Stored;
        Ok(match outcome {Stored::Ready=>nomifun_channel::ChannelInputDeliveryDisposition::Ready,
            Stored::Confirmed=>nomifun_channel::ChannelInputDeliveryDisposition::Confirmed,
            Stored::Unknown=>nomifun_channel::ChannelInputDeliveryDisposition::Unknown})
    }

    pub(super) async fn finish_channel_input_delivery(&self,owner_id:&str,session_id:&str,input_id:&str,turn_id:&str,
        subject:nomifun_agent_contracts::ChannelInputDeliverySubject,confirmed:bool,provider_message_ids:Vec<String>)->Result<(),AppError> {
        self.canonical.store().finish_channel_input_delivery(&PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.into()},
            &session_id.to_owned().into(),&input_id.to_owned().into(),turn_id,subject,confirmed,provider_message_ids)
            .await.map_err(agent_session_store_error)
    }
}
