//! A Channel question's actual presentation identity. This is neither a user
//! answer nor authorization to execute, resume, or select a newer question.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use crate::{AgentSessionId, EventId, OperationId, PrincipalRef};

pub const CHANNEL_INPUT_QUESTION_DELIVERED: &str = "channel/input-question-delivered";
pub const CHANNEL_INPUT_DELIVERY_STATE: &str = "channel/input-delivery-state";

/// Trusted ingress identity carried by the one original accepted input.
/// The Channel Session id is the actual transport binding row, not an Agent
/// Session mirror; revocation or rebinding invalidates automatic delivery.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelInputSource {
    pub platform: String,
    pub channel_plugin_id: String,
    pub chat_id: String,
    pub channel_session_id: String,
    pub inbound_message_key: String,
}

impl ChannelInputSource {
    pub fn from_input(input:&serde_json::Value)->Result<Option<Self>,String> {
        input.get("channel_source").map(|value| {
            let source:Self=serde_json::from_value(value.clone()).map_err(|error|error.to_string())?;
            source.validate()?;Ok(source)
        }).transpose()
    }

    /// Receiving platform/plugin/chat determine output routing. Inbound
    /// command and authorization binding identities do not merge recipients.
    pub fn destination(&self)->(&str,&str,&str) {(&self.platform,&self.channel_plugin_id,&self.chat_id)}

    pub fn same_destination(left:Option<&Self>,right:Option<&Self>)->bool {
        match(left,right) {
            (None,None)=>true,
            (Some(left),Some(right))=>left.destination()==right.destination(),
            _=>false,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !bounded(&self.platform,64) || !bounded(&self.channel_plugin_id,512)
            || !bounded(&self.chat_id,1024) || !bounded(&self.channel_session_id,512)
            || !bounded(&self.inbound_message_key,1024) {
            return Err("invalid trusted Channel input source".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum ChannelInputDeliverySubject { Question {question_seq:u64}, Terminal }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag="state",rename_all="snake_case",deny_unknown_fields)]
pub enum ChannelInputDeliveryPhase {
    Started,
    Confirmed {provider_message_ids:Vec<String>},
    Unknown,
}

/// Write-ahead observation of external transport delivery. Content remains in
/// its original input/Turn; this fact never grants another execution or retry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelInputDeliveryStateV1 {
    pub agent_session_id:AgentSessionId,
    pub owner_ref:PrincipalRef,
    pub input_event_id:EventId,
    pub operation_id:OperationId,
    pub source:ChannelInputSource,
    pub subject:ChannelInputDeliverySubject,
    pub phase:ChannelInputDeliveryPhase,
}

impl ChannelInputDeliveryStateV1 {
    pub fn validate(&self)->Result<(),String> {
        self.source.validate()?;
        let uuid=|value:&str|uuid::Uuid::parse_str(value).is_ok_and(|id|id.get_version_num()==7&&id.to_string()==value);
        if !uuid(self.agent_session_id.as_ref()) || !uuid(self.input_event_id.as_ref())
            || self.owner_ref.principal_kind!="user" || !bounded(&self.owner_ref.principal_id,1024)
            || !bounded(self.operation_id.as_ref(),1024)
            || matches!(self.subject,ChannelInputDeliverySubject::Question {question_seq} if question_seq==0||question_seq>i64::MAX as u64)
            || matches!(&self.phase,ChannelInputDeliveryPhase::Confirmed {provider_message_ids} if provider_message_ids.len()>1024
                || provider_message_ids.iter().any(|id|!bounded(id,1024))) {
            return Err("invalid Channel delivery observation".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelInputQuestionDeliveredV1 {
    pub agent_session_id: AgentSessionId,
    pub owner_ref: PrincipalRef,
    pub operation_id: OperationId,
    pub source_message_id: EventId,
    pub question_seq: u64,
    pub request_id: String,
    pub platform: String,
    pub channel_plugin_id: String,
    pub chat_id: String,
    pub provider_message_id: String,
}

impl ChannelInputQuestionDeliveredV1 {
    pub fn validate(&self) -> Result<(), String> {
        let uuid = |value:&str| uuid::Uuid::parse_str(value).is_ok_and(|id|id.get_version_num()==7 && id.to_string()==value);
        if !uuid(self.agent_session_id.as_ref()) || !uuid(self.source_message_id.as_ref())
            || self.owner_ref.principal_kind != "user" || !bounded(&self.owner_ref.principal_id,1024)
            || !bounded(self.operation_id.as_ref(),1024) || self.question_seq==0 || self.question_seq>i64::MAX as u64
            || !bounded(&self.request_id,256) || !bounded(&self.platform,64)
            || !bounded(&self.channel_plugin_id,512) || !bounded(&self.chat_id,1024)
            || !bounded(&self.provider_message_id,1024) {
            return Err("invalid Channel input question delivery identity".into());
        }
        Ok(())
    }
}

fn bounded(value:&str,limit:usize) -> bool {
    !value.is_empty() && value.trim()==value && value.len()<=limit && !value.chars().any(char::is_control)
}
