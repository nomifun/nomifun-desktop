//! Channel presentation receipts use the Session's dedicated canonical writer.
//! A provider reply resolves its delivered question, never the latest question.
use super::*;
use nomifun_channel::ChannelQuestionDelivery;
use nomifun_agent_contracts::ChannelInputQuestionDeliveredV1;

pub(super) async fn record_question_delivery(owner:&NomiCoreSessionOwner,owner_id:&str,session_id:&str,delivery:ChannelQuestionDelivery)
    ->Result<(),AppError> {
    let session=parse_agent_session_id(session_id).map_err(|error|AppError::BadRequest(error.message))?;
    let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.into()};
    owner.require_ordinary_owner_input(&principal,&session).await?;
    let operation=owner.operation_for_accepted_input(owner_id,&session,&delivery.target.turn_id).await?;
    owner.canonical().store().record_channel_input_question_delivery(&principal,ChannelInputQuestionDeliveredV1 {
        agent_session_id:session,owner_ref:principal.clone(),operation_id:operation,source_message_id:delivery.target.turn_id.into(),
        question_seq:delivery.target.question_seq,request_id:delivery.request_id,platform:delivery.platform,
        channel_plugin_id:delivery.channel_plugin_id,chat_id:delivery.chat_id,provider_message_id:delivery.provider_message_id,
    }).await.map_err(agent_session_store_error)?;
    Ok(())
}

pub(super) async fn resolve_question_delivery(owner:&NomiCoreSessionOwner,owner_id:&str,session_id:&str,
    platform:&str,plugin_id:&str,chat_id:&str,provider_message_id:&str)
    ->Result<Option<nomifun_api_types::AgentQuestionReplyTargetDto>,AppError> {
    let session=parse_agent_session_id(session_id).map_err(|error|AppError::BadRequest(error.message))?;
    let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.into()};
    owner.require_ordinary_owner_input(&principal,&session).await?;
    Ok(owner.canonical().store().resolve_channel_input_question_delivery(&principal,&session,platform,plugin_id,chat_id,provider_message_id)
        .await.map_err(agent_session_store_error)?.map(|delivery|nomifun_api_types::AgentQuestionReplyTargetDto {
            turn_id:delivery.source_message_id.as_ref().into(),question_seq:delivery.question_seq,
        }))
}
