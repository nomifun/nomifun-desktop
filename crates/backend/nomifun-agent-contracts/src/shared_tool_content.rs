//! Exact Session-scoped shared tool body references. These are storage-owned
//! slots, never references supplied by a tool or recursively followed content.
use crate::{AgentSessionId,ArtifactId,DigestHex};
use serde::{Deserialize,Serialize};
use serde_json::Value;

pub const SHARED_TOOL_CONTENT_KEY:&str="_canonical_shared_content";
pub const SHARED_TOOL_CONTENT_MIN_BYTES:u64=1024;
pub const CANONICAL_PAYLOAD_MAX_BYTES:u64=1024*1024;

#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum SharedToolContentKind { ToolOutput,ToolArguments }

impl SharedToolContentKind {
    pub fn media_type(self)->&'static str {match self {
        Self::ToolOutput=>"application/vnd.nomifun.tool-output+json",
        Self::ToolArguments=>"application/vnd.nomifun.tool-arguments+json",
    }}
    pub fn id(self,session:&AgentSessionId,digest:&DigestHex)->ArtifactId {
        let kind=match self {Self::ToolOutput=>"output",Self::ToolArguments=>"arguments"};
        format!("shared-tool:{}:{kind}:{}",session.as_ref(),digest.as_ref()).into()
    }
    pub fn accepts(self,value:&Value)->bool {match self {
        Self::ToolOutput=>value.is_array(),Self::ToolArguments=>value.is_object(),
    }}
}

#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedToolContentReference {
    pub pointer:String,pub kind:SharedToolContentKind,pub payload_id:ArtifactId,
    pub digest:DigestHex,pub byte_len:u64,
}

impl SharedToolContentReference {
    pub fn is_for_slot(&self,session:&AgentSessionId,pointer:&str,kind:SharedToolContentKind)->bool {
        self.pointer==pointer&&self.kind==kind&&self.payload_id==kind.id(session,&self.digest)
            &&(SHARED_TOOL_CONTENT_MIN_BYTES..=CANONICAL_PAYLOAD_MAX_BYTES).contains(&self.byte_len)
    }
}
