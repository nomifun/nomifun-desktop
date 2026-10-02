//! Source-bound model commitments, not semantic proof or Kernel authority.
//! Store parameter digests only. No raw file/stdin/env bodies in checkpoints.
use std::collections::BTreeMap;
use nomifun_agent_contracts::digest_bytes;
use nomifun_chat_model_broker::{ChatMessage, ChatToolCall};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExactAction {
    pub id: String,
    pub source: crate::AgentInputCitation,
    pub tool: String,
    pub fields: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub stdin_sha256: Option<String>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub receiver_ref: Option<String>,
    /// Host-owned binding; a digest is not a recoverable live handle.
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub receiver_digest: Option<String>,
    #[serde(default,skip_serializing_if="std::ops::Not::not")]
    pub receiver_closed: bool,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub attempted_call_id: Option<String>,
    #[serde(default)]
    pub settled: bool,
    #[serde(default)]
    pub succeeded: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExactActionInput {
    pub id: String,
    pub source: crate::AgentInputCitation,
    pub tool: String,
    pub expected_arguments: serde_json::Value,
    #[serde(default)]
    pub receiver_ref: Option<String>,
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::json!({"type":"array","maxItems":24,"description":"Ordered exact commitments, not intent proof or authority. Declare byte-sensitive phases before effects. Use receiver_ref for stdin referencing a prior start_process ID, so start+stdin can be committed before the process lifetime begins. The host binds only its real running receipt. No env, credentials or fabricated handles. Status-only updates omit exact_actions; once/outcome state cannot reset. stdin compares actual UTF-8 input plus append_newline; no normalization. Cleanup/poll remain ordinary owner controls, not exact_actions.",
        "items":{"type":"object","additionalProperties":false,"required":["id","source","tool","expected_arguments"],
            "properties":{"id":{"type":"string","minLength":1,"maxLength":64},"source":crate::requirements::citation_schema(),
                "tool":{"type":"string","minLength":1,"maxLength":128,
                    "enum":["write_file","apply_patch","exec_command","start_process","read_file","delete_path","write_process_stdin"]},
                "receiver_ref":{"type":"string","minLength":1,"maxLength":64,"description":"stdin only: ID of an earlier start_process exact action. Declare start+stdin before launching; the host binds only its real running receipt. Mutually exclusive with expected_arguments.process_id. No placeholder handle or new authority."},
                "expected_arguments":{"type":"object","minProperties":1,"maxProperties":8,
                    "description":"Frozen tool fields: path/content; files; command/args/cmd/tty/wait_ms/timeout_ms/cwd; path/format/missing_ok/expected_sha256; stdin input/append_newline plus either current process_id or receiver_ref. No env. receiver_ref omits process_id until the actual native call. Supplied values/arrays match exactly, including guards; guard-dependent declarations wait for the real source receipt."}}}})
}

fn sha(value: &serde_json::Value) -> Result<String,String> {
    serde_json::to_vec(value).map(|bytes|digest_bytes(&bytes).as_ref().to_owned()).map_err(|_|"Cannot digest exact parameter".into())
}

fn stdin_bytes(value: &serde_json::Value) -> Result<Vec<u8>,String> {
    let input=value["input"].as_str().ok_or("Exact stdin needs input text")?;
    let mut bytes=input.as_bytes().to_vec();
    if value.get("append_newline").and_then(serde_json::Value::as_bool).unwrap_or(false) {bytes.push(b'\n');}
    Ok(bytes)
}

impl ExactActionInput {
    pub(crate) fn compile(self, inputs:&[ChatMessage]) -> Result<AgentExactAction,String> {
        crate::requirements::validate_citation(&self.source,inputs,false)?;
        if self.id.is_empty() || self.id.len()>64 || !self.id.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_'))
            || self.tool.is_empty() || self.tool.len()>128 {
            return Err("Exact action needs a bounded ASCII ID and frozen tool name".into());
        }
        crate::stream_limits::serialized_size(&self.expected_arguments,48*1024).map_err(|_|"Exact action parameters exceed the existing 48 KiB plan budget")?;
        let allowed:&[&str]=match self.tool.as_str() {
            "write_file"=>&["path","content"],"apply_patch"=>&["files"],
            "exec_command"|"start_process"=>&["command","args","cmd","tty","wait_ms","timeout_ms","cwd"],
            "read_file"=>&["path","format","missing_ok","expected_sha256"],
            "delete_path"=>&["path"],"write_process_stdin"=>&["process_id","input","append_newline"],
            _=>return Err("Exact actions support only the listed workspace file/process tools; cleanup and unknown tools need ordinary owner controls".into()),
        };
        let object=self.expected_arguments.as_object().ok_or("Exact arguments must be an object")?;
        if object.is_empty()||object.keys().any(|key|!allowed.contains(&key.as_str())) {return Err("Exact arguments contain unsupported or private fields".into());}
        if self.tool=="write_file" && (!object.contains_key("path")||!object.contains_key("content")) {return Err("Exact file write requires both path and complete content".into());}
        if self.receiver_ref.as_ref().is_some_and(|id|self.tool!="write_process_stdin"||id.is_empty()||id.len()>64||object.contains_key("process_id")) {
            return Err("receiver_ref is stdin-only and exclusive with an explicit process_id".into());
        }
        if self.tool=="write_process_stdin"&&self.receiver_ref.is_none()&&!object.get("process_id").and_then(serde_json::Value::as_str).is_some_and(|id|!id.is_empty()&&id.len()<=128) {
            return Err("Exact stdin needs the actual current owned process_id; declare it only after the start/poll receipt, not a made-up or cold-recovered handle".into());
        }
        let stdin_sha256=if self.tool=="write_process_stdin" {Some(digest_bytes(&stdin_bytes(&self.expected_arguments)?).as_ref().to_owned())} else {None};
        let fields=object.iter().filter(|(key,_)|self.tool!="write_process_stdin"||!matches!(key.as_str(),"input"|"append_newline"))
            .map(|(key,value)|Ok((key.clone(),sha(value)?))).collect::<Result<_,String>>()?;
        Ok(AgentExactAction {id:self.id,source:self.source,tool:self.tool,fields,stdin_sha256,receiver_ref:self.receiver_ref,
            receiver_digest:None,receiver_closed:false,attempted_call_id:None,settled:false,succeeded:false})
    }
}

impl AgentExactAction {
    pub(crate) fn matches(&self,call:&ChatToolCall)->bool {
        !self.receiver_closed&&self.matches_observation(call)
    }
    pub(crate) fn matches_observation(&self,call:&ChatToolCall)->bool {
        self.tool==call.name && self.fields.iter().all(|(key,digest)|call.arguments.0.get(key).is_some_and(|value|sha(value).ok().as_ref()==Some(digest)))
            && (self.receiver_ref.is_none() || self.receiver_digest.as_ref().is_some_and(|digest|
                call.arguments.0.get("process_id").is_some_and(|value|sha(value).ok().as_ref()==Some(digest))))
            && self.stdin_sha256.as_ref().is_none_or(|digest|stdin_bytes(&call.arguments.0).ok()
                .is_some_and(|bytes|digest_bytes(&bytes).as_ref()==digest))
    }
}

pub(crate) fn pending(actions:&[AgentExactAction])->Option<&AgentExactAction> {
    actions.iter().find(|action|!action.succeeded)
}

pub(crate) fn validate_receivers(actions:&[AgentExactAction])->Result<(),String> {
    for (index,action) in actions.iter().enumerate() {
        if let Some(id)=&action.receiver_ref {
            if action.tool!="write_process_stdin"||action.fields.contains_key("process_id")
                || !actions[..index].iter().any(|start|start.id==*id&&start.tool=="start_process") {
                return Err("receiver_ref must name an earlier start_process commitment in this same plan".into());
            }
        } else if action.receiver_digest.is_some() {return Err("receiver binding requires its source start action".into());}
    }
    Ok(())
}

pub(crate) fn process_digest(value:&serde_json::Value)->Option<String> {
    value.as_str().filter(|id|!id.is_empty()&&id.len()<=128).and_then(|_|sha(value).ok())
}

pub(crate) fn close_receivers(actions:&mut [AgentExactAction]) {
    for action in actions.iter_mut().filter(|action|action.receiver_ref.is_some()&&!action.succeeded) {action.receiver_closed=true;}
}

pub(crate) fn proven_nonstart(call:&ChatToolCall,result:&crate::AgentToolResult)->bool {
    matches!(call.name.as_str(),"exec_command"|"start_process")
        && serde_json::from_str::<serde_json::Value>(&result.output_text()).is_ok_and(|value|
            value["schema"]=="nomifun.process-start-observation.v1"&&value["state"]=="not_started"
                &&value["user_code_started"]==false&&value["success"]==false&&value.get("process_id").is_none())
}

pub(crate) fn owner_succeeded(call:&ChatToolCall,result:&crate::AgentToolResult)->bool {
    if result.is_error {return false;}
    let Ok(value)=serde_json::from_str::<serde_json::Value>(&result.output_text()) else {return false;};
    let hash=|value:&serde_json::Value|value.as_str().is_some_and(|hash|hash.len()==64&&hash.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)));
    match call.name.as_str() {
        "write_file"=>value["written"]==true&&value["path"]==call.arguments.0["path"]
            &&call.arguments.0["content"].as_str().is_some_and(|content|value["bytes"].as_u64()==Some(content.len() as u64)
                &&value["sha256"].as_str()==Some(digest_bytes(content.as_bytes()).as_ref())),
        "apply_patch"=>value["files"].as_array().is_some_and(|files|!files.is_empty()&&Some(files.len() as u64)==value["file_count"].as_u64()
            &&call.arguments.0["files"].as_array().is_some_and(|requested|requested.len()==files.len()
                &&files.iter().zip(requested).all(|(file,request)|file["path"]==request["path"]&&hash(&file["written_sha256"])
                    &&file["bytes_before"].is_u64()&&file["bytes_after"].is_u64()
                    &&request["hunks"].as_array().is_some_and(|hunks|file["hunks_applied"].as_u64()==Some(hunks.len() as u64))))),
        "delete_path"=>value["deleted"]==true&&value["path"]==call.arguments.0["path"],
        "read_file"=>value["path"]==call.arguments.0["path"]&&(hash(&value["sha256"])||value["kind"]=="workspace_file_absent"),
        "exec_command"|"start_process"|"write_process_stdin"=>{
            let bound=value["process_id"].as_str().is_some_and(|id|!id.is_empty())
                &&(call.name!="write_process_stdin"||value["process_id"]==call.arguments.0["process_id"]);
            let exited=value["state"]=="exited"&&value["exit_code"]==0&&value["cleanup"]["reaped"]==true&&value["success"]==true;
            let running=call.name!="exec_command"&&value["state"]=="running"&&value.get("control_applied")!=Some(&serde_json::Value::Bool(false))
                &&value["pid"].as_u64().is_some_and(|pid|pid>0&&pid<=u32::MAX as u64)
                &&value["output"]["text"].is_string()&&value["output"]["next_cursor"].is_u64()
                &&value.get("success")!=Some(&serde_json::Value::Bool(false));
            bound&&(exited||running)
        },
        _=>false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::StrictJsonValue;
    use nomifun_chat_model_broker::ChatRole;
    fn action(tool:&str,mut args:serde_json::Value)->AgentExactAction {
        if tool=="write_process_stdin" {args["process_id"]=serde_json::json!("owned");}
        ExactActionInput {id:"one".into(),source:crate::AgentInputCitation {input:0,quote:"Exact requested bytes".into()},
            tool:tool.into(),expected_arguments:args,receiver_ref:None}.compile(&[crate::context_lifecycle::text_message(ChatRole::User,"Exact requested bytes".into())]).unwrap()
    }
    fn call(name:&str,args:serde_json::Value)->ChatToolCall {ChatToolCall {call_id:"proposed".into(),name:name.into(),arguments:StrictJsonValue(args),provider_metadata:None}}
    #[test]
    fn effective_stdin_bytes_accept_both_exact_forms_not_double_lf() {
        let input=action("write_process_stdin",serde_json::json!({"input":"你好 MAC-B\n"}));
        assert!(input.matches(&call("write_process_stdin",serde_json::json!({"process_id":"owned","input":"你好 MAC-B","append_newline":true}))));
        assert!(input.matches(&call("write_process_stdin",serde_json::json!({"process_id":"owned","input":"你好 MAC-B\n","append_newline":false}))));
        assert!(!input.matches(&call("write_process_stdin",serde_json::json!({"process_id":"owned","input":"你好 MAC-B\n","append_newline":true}))));
        let two=action("write_process_stdin",serde_json::json!({"input":"你好 MAC-B\n","append_newline":true}));
        assert!(two.matches(&call("write_process_stdin",serde_json::json!({"process_id":"owned","input":"你好 MAC-B\n\n"}))));
        assert!(!input.matches(&call("write_process_stdin",serde_json::json!({"process_id":"other-owned","input":"你好 MAC-B\n"}))));
        assert!(!serde_json::to_string(&input).unwrap().contains("你好"));
    }
    #[test]
    fn creation_commitment_preserves_intermediate_bytes_and_no_case_name_is_special() {
        let first=action("write_file",serde_json::json!({"path":"another.txt","content":"first\nbefore\n"}));
        assert!(!first.matches(&call("write_file",serde_json::json!({"path":"another.txt","content":"first\nafter\n"}))));
        assert!(!first.matches(&call("write_file",serde_json::json!({"path":"another.txt","content":"first\nbefore"}))));
        assert!(first.matches(&call("write_file",serde_json::json!({"path":"another.txt","content":"first\nbefore\n"}))));
        let final_only=action("write_file",serde_json::json!({"path":"other.txt","content":"final\n"}));
        assert!(final_only.matches(&call("write_file",serde_json::json!({"path":"other.txt","content":"final\n"}))));
    }

    #[test]
    fn only_positive_owner_receipts_satisfy_exact_calls() {
        let write=call("write_file",serde_json::json!({"path":"a","content":"saved"}));
        for body in [serde_json::json!({}),serde_json::json!({"written":false}),
            serde_json::json!({"state":"lost","success":null}),serde_json::json!({"state":"partial"})] {
            assert!(!owner_succeeded(&write,&crate::AgentToolResult::text(write.call_id.clone(),body.to_string(),false)));
        }
        let receipt=serde_json::json!({"path":"a","written":true,"bytes":5,"sha256":digest_bytes(b"saved")});
        assert!(owner_succeeded(&write,&crate::AgentToolResult::text(write.call_id.clone(),receipt.to_string(),false)));
        let input=call("write_process_stdin",serde_json::json!({"process_id":"owned","input":"line\n"}));
        let running=serde_json::json!({"process_id":"owned","state":"running","pid":123,"success":null,
            "output":{"text":"READY\n","next_cursor":6}});
        assert!(owner_succeeded(&input,&crate::AgentToolResult::text(input.call_id.clone(),running.to_string(),false)));
        let mut wrong=running.clone();wrong["process_id"]=serde_json::json!("other-owned");
        assert!(!owner_succeeded(&input,&crate::AgentToolResult::text(input.call_id.clone(),wrong.to_string(),false)));
        let mut lost=running;lost["state"]=serde_json::json!("lost");
        assert!(!owner_succeeded(&input,&crate::AgentToolResult::text(input.call_id.clone(),lost.to_string(),false)));
    }
}
