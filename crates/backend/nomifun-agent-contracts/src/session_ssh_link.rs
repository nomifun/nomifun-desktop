//! Host-requested close of one original pooled SSH link. These facts do not
//! recreate a socket, cancel a Session, or prove that an unknown shell exited.
use schemars::JsonSchema;
use serde::{Deserialize,Serialize};
use crate::{AgentSessionId,PrincipalRef};

pub const SESSION_SSH_LINK_CLOSE_REQUESTED:&str="session/ssh-link-close-requested";
pub const SESSION_SSH_LINK_CLOSED:&str="session/ssh-link-closed";

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionSshLinkCloseV1 {
    pub agent_session_id:AgentSessionId,
    pub owner_ref:PrincipalRef,
    pub ssh_host_id:String,
    pub pool_instance_id:String,
    pub instance_id:u64,
}

impl SessionSshLinkCloseV1 {
    pub fn validate(&self)->Result<(),String> {
        let uuid=|value:&str|uuid::Uuid::parse_str(value).is_ok_and(|id|id.get_version_num()==7&&id.to_string()==value);
        if !uuid(self.agent_session_id.as_ref())||!uuid(&self.pool_instance_id)||!uuid(&self.ssh_host_id)||self.instance_id==0
            ||self.owner_ref.principal_kind!="user"||self.owner_ref.principal_id.is_empty()||self.owner_ref.principal_id.len()>1024
            ||self.owner_ref.principal_id.chars().any(char::is_control) {return Err("invalid exact SSH link close identity".into());}
        Ok(())
    }
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(tag="state",rename_all="snake_case",deny_unknown_fields)]
pub enum SessionSshLinkCloseResult {
    Reaped {detail:String},
    Lost {detail:String},
    AlreadyDown {detail:String},
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionSshLinkClosedV1 {
    pub close:SessionSshLinkCloseV1,
    pub result:SessionSshLinkCloseResult,
}

impl SessionSshLinkClosedV1 {
    pub fn validate(&self)->Result<(),String> {
        self.close.validate()?;
        let detail=match &self.result {SessionSshLinkCloseResult::Reaped {detail}|SessionSshLinkCloseResult::Lost {detail}
            |SessionSshLinkCloseResult::AlreadyDown {detail}=>detail};
        if detail.len()>4096||detail.chars().any(|character|character.is_control()&&!matches!(character,'\n'|'\r'|'\t')) {
            return Err("invalid SSH close diagnostic".into());
        }
        Ok(())
    }
}
