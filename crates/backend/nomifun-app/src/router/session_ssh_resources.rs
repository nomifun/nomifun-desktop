//! User resource close belongs to the original SSH pool capability and the
//! canonical Session. Activity discovery does not own a replacement connection.
use std::sync::Arc;
use nomifun_agent_contracts::{SessionSshLinkCloseV1,SessionSshLinkClosedV1,SessionSshLinkCloseResult};
use nomifun_agent_session::AgentSessionStore;
use nomifun_conversation::BackgroundTaskRegistrar;
use nomifun_ssh::{SshConnectionPool,SshLinkKey,SshTeardown};
use nomifun_common::AppError;

pub(super) struct SshResourceCloseReceipt {pub accepted:bool,pub execution_stopped:bool}

pub(super) async fn request_close(store:AgentSessionStore,pool:SshConnectionPool,tasks:Arc<dyn BackgroundTaskRegistrar>,
    close:SessionSshLinkCloseV1)->Result<SshResourceCloseReceipt,AppError> {
    close.validate().map_err(AppError::BadRequest)?;
    let recorded=store.session_ssh_link_closes(&close.owner_ref,&close.agent_session_id).await.map_err(|error|AppError::Conflict(error.to_string()))?
        .into_iter().find(|(original,_)|original==&close);
    if let Some((_,Some(result)))=&recorded {
        return Ok(SshResourceCloseReceipt {accepted:true,execution_stopped:matches!(result.result,SessionSshLinkCloseResult::Reaped {..})});
    }
    let host=nomifun_common::SshHostId::parse(&close.ssh_host_id).map_err(|error|AppError::BadRequest(error.to_string()))?;
    let control=match pool.prepare_close_instance(&close.owner_ref.principal_id,&SshLinkKey::new(close.agent_session_id.as_ref(),host),
        &close.pool_instance_id,close.instance_id) {
        Ok(control)=>control,
        Err(_) if recorded.is_some()=>return Ok(SshResourceCloseReceipt {accepted:true,execution_stopped:false}),
        Err(error)=>return Err(AppError::Conflict(error)),
    };
    store.record_session_ssh_link_close_requested(&close.owner_ref,close.clone()).await
        .map_err(|error|AppError::Conflict(error.to_string()))?;
    // Request first becomes durable, then the exact original capability closes
    // its own admission. No live pool lookup occurs after this point.
    control.request();
    let mut shutdown=tasks.shutdown_signal();
    let _=tasks.spawn(Box::pin(async move {
        let native=tokio::select! {biased;
            _=&mut shutdown=>return,
            native=control.settled()=>native,
        };
        let diagnostic=|detail:String| {
            let detail=nomi_redact::redact_secrets_owned(detail);
            let mut end=detail.len().min(4096);while !detail.is_char_boundary(end){end-=1;}
            detail[..end].chars().filter(|character|!character.is_control()||matches!(character,'\n'|'\r'|'\t')).collect::<String>()
        };
        let result=match native {SshTeardown::Reaped {detail}=>SessionSshLinkCloseResult::Reaped {detail:diagnostic(detail)},
            SshTeardown::Lost {detail}=>SessionSshLinkCloseResult::Lost {detail:diagnostic(detail)},
            SshTeardown::AlreadyDown {detail}=>SessionSshLinkCloseResult::AlreadyDown {detail:diagnostic(detail)}};
        let receipt=SessionSshLinkClosedV1 {close,result};
        let mut delay=std::time::Duration::from_secs(1);
        loop {
            match store.record_session_ssh_link_closed(&receipt.close.owner_ref,receipt.clone()).await {
                Ok(_)=>return,
                Err(error)=>tracing::warn!(agent_session_id=%receipt.close.agent_session_id.as_ref(),ssh_host_id=%receipt.close.ssh_host_id,%error,
                    "original SSH close receipt remains unconfirmed"),
            }
            tokio::select! {_=&mut shutdown=>return,_=tokio::time::sleep(delay)=>{}}
            delay=(delay*2).min(std::time::Duration::from_secs(30));
        }
    }));
    Ok(SshResourceCloseReceipt {accepted:true,execution_stopped:false})
}

pub(super) async fn observe_resources(store:&AgentSessionStore,pool:&SshConnectionPool,owner:&nomifun_agent_contracts::PrincipalRef,
    session:&nomifun_agent_contracts::AgentSessionId)->Result<Vec<nomifun_ssh::SshResourceObservation>,AppError> {
    let mut resources=pool.resource_instances(&owner.principal_id).into_iter().filter(|resource|resource.agent_session_id==session.as_ref()).collect::<Vec<_>>();
    for (close,result) in store.session_ssh_link_closes(owner,session).await.map_err(|error|AppError::Conflict(error.to_string()))? {
        if resources.iter().any(|resource|resource.pool_instance_id==close.pool_instance_id&&resource.ssh_host_id==close.ssh_host_id&&resource.instance_id==close.instance_id) {continue;}
        let reaped=result.as_ref().is_some_and(|result|matches!(&result.result,SessionSshLinkCloseResult::Reaped {..}));
        resources.push(nomifun_ssh::SshResourceObservation {pool_instance_id:close.pool_instance_id,agent_session_id:session.as_ref().into(),
            ssh_host_id:close.ssh_host_id,instance_id:close.instance_id,registered:false,phase:Some(nomifun_ssh::SshLinkPhase::Closed),
            closing:true,close_in_flight:false,close_confirmed:result.map(|_|reaped),cleanup_unconfirmed:!reaped,display_name:None});
    }
    Ok(resources)
}
