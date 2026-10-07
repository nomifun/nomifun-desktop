//! Engine-local control state. A plan never grants a platform capability.
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_chat_model_broker::{ChatMessage, ChatToolCall, ChatToolDefinition};
use serde::{Deserialize, Serialize};

use crate::{AgentEngineError, AgentEngineEvent, AgentEventSink, AgentToolResult};

pub(crate) const TOOL_NAME: &str = "update_plan";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPlanStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanStep {
    pub step: String,
    pub status: AgentPlanStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlan {
    pub revision: u32,
    pub explanation: String,
    pub steps: Vec<AgentPlanStep>,
    pub needs_replan: bool,
    #[serde(default)]
    pub requirements: Vec<crate::AgentTaskRequirement>,
    #[serde(default, skip_serializing_if="Vec::is_empty")]
    pub exact_actions: Vec<crate::AgentExactAction>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdatePlan {
    #[serde(default)]
    explanation: Option<String>,
    plan: Vec<AgentPlanStep>,
    #[serde(default)]
    requirements: Vec<crate::AgentTaskRequirement>,
    #[serde(default)]
    exact_actions: Vec<crate::exact_actions::ExactActionInput>,
}

pub(crate) fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        name: TOOL_NAME.into(),
        description: "Maintain a concise execution plan. At most one step in_progress. A short explanation is optional. Usually omit requirements: the engine records every otherwise-unaccounted accepted input as one full-scope obligation without losing its original text or constraints. Add finer-grained IDs only when you can copy a short, exact contiguous quote from that indexed accepted user input (input 0=original, later indices=corrections); never paraphrase or cite a tool result. On status updates omit existing requirements; old obligations cannot be rewritten or dropped. The response lists the ledger IDs that report_completion must cover. Repeated unchanged plans succeed idempotently but are not progress. Replan after uncertain effects or changed scope. This never authorizes verification or widens user scope.".into(),
        deferred: false,
        input_schema: StrictJsonValue(serde_json::json!({
            "type":"object", "additionalProperties":false,
            "required":["plan"],
            "properties":{
                "explanation":{"type":"string","minLength":1,"maxLength":2048,
                    "description":"Optional short explanation of the plan or its revision. The engine preserves accepted requirements independently of this text."},
                "requirements":crate::requirements::schema(),
                "exact_actions":crate::exact_actions::schema(),
                "plan":{"type":"array","minItems":1,"maxItems":16,"items":{
                    "type":"object","additionalProperties":false,"required":["step","status"],
                    "properties":{"step":{"type":"string","minLength":1,"maxLength":512},
                        "status":{"type":"string","enum":["pending","in_progress","completed","blocked"]}}
                }}
            }
        })),
    }
}

impl AgentPlan {
    pub(crate) async fn update(
        &mut self,
        call: &ChatToolCall,
        inputs: &[ChatMessage],
        sink: &dyn AgentEventSink,
    ) -> Result<AgentToolResult, AgentEngineError> {
        if crate::stream_limits::serialized_size(&call.arguments, 48 * 1024).is_err() {
            return Ok(self.feedback(call, "rejected", "Plan/requirements exceed the 48 KiB argument budget"));
        }
        let update = serde_json::from_value::<UpdatePlan>(call.arguments.0.clone());
        let update = match update {
            Ok(value) => value,
            Err(error) => {
                return Ok(self.feedback(call, "rejected", &format!("Invalid plan: {error}")));
            }
        };
        let explanation = update.explanation.unwrap_or_else(|| {
            if self.explanation.is_empty() { "Execution plan for the accepted task.".to_owned() }
            else { self.explanation.clone() }
        });
        let mut names = std::collections::BTreeSet::new();
        if explanation.trim().is_empty()
            || explanation.chars().count() > 2048
            || update.plan.is_empty()
            || update.plan.len() > 16
            || update.plan.iter().any(|item| {
                item.step.trim().is_empty()
                    || item.step.chars().count() > 512
                    || !names.insert(item.step.trim())
            })
            || update
                .plan
                .iter()
                .filter(|item| item.status == AgentPlanStatus::InProgress)
                .count()
                > 1
        {
            return Ok(self.feedback(call, "rejected",
                "Invalid plan: provide 1..16 unique steps (1..512 characters each), a nonempty explanation (up to 2048 characters), and at most one in_progress step. The current plan was not changed."));
        }
        let requirements =
            match crate::requirements::merge_with_location(&self.requirements, &update.requirements, inputs) {
                Ok(requirements) => requirements,
                Err(reason) => {
                    return Ok(self.feedback_with_source_location(call,"rejected",&reason.message,
                        reason.source_location.as_ref().map(|(path,input)|(path.as_str(),*input))));
                }
            };
        let ignored_restatements = update.requirements.iter().filter(|item| {
            self.requirements.iter().any(|existing| existing.id == item.id
                && (existing.description != item.description || existing.source != item.source))
        }).map(|item| item.id.as_str()).collect::<Vec<_>>();
        let mut exact_actions=self.exact_actions.clone();
        for proposal in update.exact_actions {
            let action=match proposal.compile(inputs) {Ok(action)=>action,Err(reason)=>return Ok(self.feedback(call,"rejected",&reason))};
            if let Some(prior)=exact_actions.iter().find(|prior|prior.id==action.id) {
                if prior.source!=action.source||prior.tool!=action.tool||prior.fields!=action.fields||prior.stdin_sha256!=action.stdin_sha256||prior.receiver_ref!=action.receiver_ref {
                    return Ok(self.feedback(call,"rejected","Exact action IDs cannot rewrite bytes, tools or sources, reset once state or erase receipts; add a new source-bound action only for genuinely new authorized work."));
                }
            } else {
                if action.receiver_ref.as_ref().is_some_and(|id|exact_actions.iter().any(|start|start.id==*id&&start.attempted_call_id.is_some())) {
                    return Ok(self.feedback(call,"rejected","Declare receiver_ref stdin before its start action is attempted. For an already-live receiver use its actual current process_id; never rebind from old/cold receipts."));
                }
                if exact_actions.iter().any(|prior|prior.succeeded&&prior.source==action.source&&prior.tool==action.tool
                    &&prior.fields==action.fields&&prior.stdin_sha256==action.stdin_sha256
                    &&(action.tool=="write_process_stdin"||prior.receiver_ref==action.receiver_ref)) {
                    return Ok(self.feedback(call,"rejected","A new exact action ID cannot repeat a satisfied commitment under the same user-source citation. Later user authorization needs its distinct later input source."));
                }
                exact_actions.push(action);
            }
        }
        if let Err(reason)=crate::exact_actions::validate_receivers(&exact_actions) {return Ok(self.feedback(call,"rejected",&reason));}
        if exact_actions.len()>24 || crate::stream_limits::serialized_size(&exact_actions,8192).is_err() {
            return Ok(self.feedback(call,"rejected","Exact action ledger exceeds 24 items / 8 KiB digest metadata"));
        }
        if !self.needs_replan && update.plan == self.steps && requirements == self.requirements && exact_actions==self.exact_actions {
            return Ok(self.feedback(call, "unchanged",
                "Plan already has these step statuses and immutable requirements; this idempotent update succeeded without recording a new revision or invalidating completion evidence. Perform the next authorized action, or report_completion if the work is finished. Repeating this update is not task progress."));
        }
        let next = Self {
            revision: self.revision.checked_add(1).ok_or_else(|| {
                AgentEngineError::InvalidContract("plan revision counter exhausted".into())
            })?,
            explanation,
            steps: update.plan,
            needs_replan: false,
            requirements,
            exact_actions,
        };
        // Persist before publishing/using the new control state.
        sink.emit(AgentEngineEvent::PlanUpdated { plan: next.clone() })
            .await?;
        *self = next;
        let mut feedback = "Plan and source-anchored requirements recorded. For later status-only updates omit requirements; the immutable ledger persists and completion must cover every ID. Statuses and interpretation are model-authored, not proof of successful effects, verification, or complete user-intent extraction.".to_owned();
        if !ignored_restatements.is_empty() {
            feedback.push_str(&format!(" Existing requirement IDs {} were restated differently and kept unchanged. Omit requirements on future plan-status updates; submit only genuinely new IDs.",
                serde_json::to_string(&ignored_restatements).unwrap_or_default()));
        }
        Ok(self.feedback(call, "updated", &feedback))
    }

    fn feedback(&self, call: &ChatToolCall, status: &str, message: &str) -> AgentToolResult {
        self.feedback_with_source_location(call,status,message,None)
    }

    fn feedback_with_source_location(&self,call:&ChatToolCall,status:&str,message:&str,location:Option<(&str,usize)>) -> AgentToolResult {
        let mut value=serde_json::json!({
            "status": status,
            "plan_revision": self.revision,
            "needs_replan": self.needs_replan,
            "plan": self.steps,
            "requirement_ids": self.requirements.iter().map(|item| &item.id).collect::<Vec<_>>(),
            "message": message,
        });
        if let Some((path,input))=location {
            value["rejected_parameter_path"]=serde_json::json!(path);
            value["source_input_index"]=serde_json::json!(input);
        }
        AgentToolResult::text(call.call_id.clone(),value.to_string(),status=="rejected")
    }

    pub(crate) fn effect_gate(&self) -> Option<&'static str> {
        if self.needs_replan {
            Some(
                "The plan needs reconsideration after an uncertain effect, newly accepted user input, or changed repository instructions. This proposed effect was not executed. Call update_plan alone now: explain the recovery and put one step in_progress. Preserve existing requirements; add exact accepted-input citations only for genuinely new requirements or inputs.",
            )
        } else if self.revision == 0 && self.steps.is_empty() {
            // Adaptive accounting may start after the model has already
            // proposed a valid batch. An absent optional plan is not a closed
            // plan and must not retroactively block that batch or hide tools.
            // Completion still requires accounting for every accepted input.
            None
        } else if !self
            .steps
            .iter()
            .any(|step| step.status == AgentPlanStatus::InProgress)
        {
            Some(
                "The plan is closed; no further command or mutation was executed. If a new check is truly needed, call update_plan ALONE to reopen one verification step as in_progress, run the check, then close the plan and report_completion without starting another command. Otherwise use an already current successful observation in report_completion.",
            )
        } else {
            None
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.needs_replan
            || self.steps.iter().any(|step| {
                matches!(
                    step.status,
                    AgentPlanStatus::Pending | AgentPlanStatus::InProgress
                )
            })
    }

    pub(crate) fn context(&self) -> Result<String, AgentEngineError> {
        let mut projected=serde_json::to_value(self).map_err(|error|AgentEngineError::ContextAssembly(error.to_string()))?;
        if !self.exact_actions.is_empty() {
            projected["exact_actions"]=serde_json::json!(self.exact_actions.iter().map(|action|serde_json::json!([action.id,
                if action.succeeded {"succeeded"} else if action.attempted_call_id.is_some() {"attempted_not_satisfied"} else {"pending"}])).collect::<Vec<_>>());
            projected["next_exact_action"]=serde_json::json!(crate::exact_actions::pending(&self.exact_actions));
        }
        serde_json::to_string(&projected).map(|value| format!("Current engine plan (derived control state, not user authority or completion evidence): {value}"))
            .map_err(|error| AgentEngineError::ContextAssembly(error.to_string()))
    }

    pub(crate) fn exact_action_gate(&self,call:&ChatToolCall,protected:bool)->Option<String> {
        if self.exact_actions.is_empty() {return None;}
        let current=crate::exact_actions::pending(&self.exact_actions);
        if !current.is_some_and(|action|action.matches(call)) && let Some(action)=self.exact_actions.iter().find(|action|action.succeeded&&action.matches(call)) {
            return Some(format!("Not executed: exact action {} already has a successful owner receipt; do not repeat it for plan/report repair.",action.id));
        }
        let Some(current)=current else {return None;};
        if !protected && !current.matches(call) {return None;}
        if current.attempted_call_id.is_some() {
            return Some(format!("Not executed: exact action {} has an attempted, failed or unsettled owner outcome. A fresh read/plan status cannot reset once state; report blocked or obtain real reconciliation, never replay it blindly.",current.id));
        }
        if !current.matches(call) {return Some(format!("Not executed: proposed tool/parameters do not match the next exact action {}. Preserve every committed byte and operation order; parameters were not corrected or normalized.",current.id));}
        None
    }

    pub(crate) async fn arm_exact_action(&mut self,call:&ChatToolCall,sink:&dyn AgentEventSink)->Result<bool,AgentEngineError> {
        let Some(action)=self.exact_actions.iter_mut().find(|action|!action.succeeded) else {return Ok(false);};
        if !action.matches(call)||action.attempted_call_id.is_some() {return Ok(false);}
        action.attempted_call_id=Some(call.call_id.as_ref().to_owned());
        sink.emit(AgentEngineEvent::PlanUpdated {plan:self.clone()}).await?;
        Ok(true)
    }

    pub(crate) async fn settle_exact_action(&mut self,call:&ChatToolCall,result:&AgentToolResult,not_applied:bool,sink:&dyn AgentEventSink)->Result<(),AgentEngineError> {
        if !self.apply_exact_outcome(call,result,not_applied) {return Ok(());}
        self.bind_exact_receiver(call,result);
        sink.emit(AgentEngineEvent::PlanUpdated {plan:self.clone()}).await?;
        Ok(())
    }

    pub(crate) fn apply_exact_outcome(&mut self,call:&ChatToolCall,result:&AgentToolResult,not_applied:bool)->bool {
        let Some(action)=self.exact_actions.iter_mut().find(|action|action.attempted_call_id.as_deref()==Some(call.call_id.as_ref())&&action.matches_observation(call)) else {return false;};
        if not_applied {action.attempted_call_id=None;action.settled=false;action.succeeded=false;} else {
            action.succeeded=crate::exact_actions::owner_succeeded(call,result);
            action.settled=action.succeeded;
        }
        true
    }

    fn bind_exact_receiver(&mut self,call:&ChatToolCall,result:&AgentToolResult) {
        let Some(start)=self.exact_actions.iter().find(|action|action.tool=="start_process"&&action.succeeded
            &&action.attempted_call_id.as_deref()==Some(call.call_id.as_ref())) else {return;};
        let Ok(value)=serde_json::from_str::<serde_json::Value>(&result.output_text()) else {return;};
        if value["state"]!="running" {return;}
        let Some(digest)=crate::exact_actions::process_digest(&value["process_id"]) else {return;};
        let id=start.id.clone();
        for input in self.exact_actions.iter_mut().filter(|action|action.receiver_ref.as_deref()==Some(id.as_str())) {
            if !input.receiver_closed&&input.receiver_digest.is_none()&&input.attempted_call_id.is_none() {input.receiver_digest=Some(digest.clone());}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentInputCitation, AgentTaskRequirement, NoopAgentEventSink};
    use nomifun_chat_model_broker::{ChatContentPart, ChatRole, ChatToolResultPart};

    #[tokio::test]
    async fn receiver_reference_binds_only_real_prior_start_and_preserves_definition() {
        let inputs=vec![crate::context_lifecycle::text_message(ChatRole::User,"Start helper then send exactly one line.".into())];
        let source=serde_json::json!({"input":0,"quote":"Start helper then send exactly one line."});
        let start=serde_json::json!({"id":"helper","tool":"start_process","source":source,
            "expected_arguments":{"command":"bun","args":["helper.mjs"],"tty":false,"wait_ms":0}});
        let input=serde_json::json!({"id":"input","tool":"write_process_stdin","source":source,"receiver_ref":"helper",
            "expected_arguments":{"input":"你好\n"}});
        let update=|actions:serde_json::Value|ChatToolCall {call_id:"plan".into(),name:TOOL_NAME.into(),provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"plan":[{"step":"Interact","status":"in_progress"}],"exact_actions":actions}))};
        let mut plan=AgentPlan::default();
        assert!(plan.update(&update(serde_json::json!([input.clone(),start.clone()])),&inputs,&NoopAgentEventSink).await.unwrap().is_error);
        assert!(!plan.update(&update(serde_json::json!([start.clone(),input.clone()])),&inputs,&NoopAgentEventSink).await.unwrap().is_error);
        let stdin=|pid:&str|ChatToolCall {call_id:"send".into(),name:"write_process_stdin".into(),provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"process_id":pid,"input":"你好","append_newline":true}))};
        assert!(!plan.exact_actions[1].matches(&stdin("owned")));
        let launch=ChatToolCall {call_id:"launch".into(),name:"start_process".into(),provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"command":"bun","args":["helper.mjs"],"tty":false,"wait_ms":0}))};
        plan.arm_exact_action(&launch,&NoopAgentEventSink).await.unwrap();
        let mut unknown=plan.clone();
        unknown.settle_exact_action(&launch,&AgentToolResult::text(launch.call_id.clone(),"unknown owner outcome",true),false,&NoopAgentEventSink).await.unwrap();
        assert!(unknown.exact_actions[1].receiver_digest.is_none());
        assert!(!unknown.exact_actions[1].matches(&stdin("owned")));
        let mut exited=plan.clone();
        let ended=AgentToolResult::text(launch.call_id.clone(),serde_json::json!({"process_id":"owned","state":"exited",
            "exit_code":0,"success":true,"cleanup":{"reaped":true}}).to_string(),false);
        exited.settle_exact_action(&launch,&ended,false,&NoopAgentEventSink).await.unwrap();
        assert!(exited.exact_actions[1].receiver_digest.is_none(),"a completed start cannot create a live receiver");
        let receipt=AgentToolResult::text(launch.call_id.clone(),serde_json::json!({"process_id":"owned","state":"running","pid":123,
            "output":{"text":"READY\n","next_cursor":6},"success":null}).to_string(),false);
        plan.settle_exact_action(&launch,&receipt,false,&NoopAgentEventSink).await.unwrap();
        assert!(plan.exact_actions[1].matches(&stdin("owned")));
        assert!(!plan.exact_actions[1].matches(&stdin("other-owned")));
        let binding=plan.exact_actions[1].receiver_digest.clone();
        let mut late=input.clone();late["id"]=serde_json::json!("late-ref");
        assert!(plan.update(&update(serde_json::json!([late])),&inputs,&NoopAgentEventSink).await.unwrap().is_error);
        assert!(!plan.update(&update(serde_json::json!([start,input])),&inputs,&NoopAgentEventSink).await.unwrap().is_error);
        assert_eq!(plan.exact_actions[1].receiver_digest,binding);
        let serialized=serde_json::to_string(&plan).unwrap();assert!(!serialized.contains("owned"));
        let mut cold:AgentPlan=serde_json::from_str(&serialized).unwrap();
        crate::exact_actions::close_receivers(&mut cold.exact_actions);
        assert!(!cold.exact_actions[1].matches(&stdin("owned")));
        assert!(cold.exact_actions[1].matches_observation(&stdin("owned")));
        assert_eq!(cold.exact_actions[1].receiver_digest,binding);
        let send=stdin("owned");plan.arm_exact_action(&send,&NoopAgentEventSink).await.unwrap();
        plan.settle_exact_action(&send,&AgentToolResult::text(send.call_id.clone(),serde_json::json!({"process_id":"owned","state":"running","pid":123,
            "output":{"text":"ECHO\n","next_cursor":11},"success":null}).to_string(),false),false,&NoopAgentEventSink).await.unwrap();
        let new_start=serde_json::json!({"id":"other-helper","tool":"start_process","source":source,
            "expected_arguments":{"command":"bun","args":["other.mjs"],"tty":false,"wait_ms":0}});
        let alias=serde_json::json!({"id":"renamed-input","tool":"write_process_stdin","source":source,"receiver_ref":"other-helper",
            "expected_arguments":{"input":"你好\n"}});
        assert!(plan.update(&update(serde_json::json!([new_start,alias])),&inputs,&NoopAgentEventSink).await.unwrap().is_error,
            "successful same-source stdin cannot repeat by changing receiver aliases");
    }

    #[tokio::test]
    async fn satisfied_exact_action_cannot_be_renamed_or_its_once_state_rewritten() {
        let inputs=vec![crate::context_lifecycle::text_message(ChatRole::User,"Save this exact file once.".into())];
        let spec=|id:&str|serde_json::json!({"id":id,"source":{"input":0,"quote":"Save this exact file once."},
            "tool":"write_file","expected_arguments":{"path":"a","content":"exact\n"}});
        let update=|id:&str|ChatToolCall {call_id:"plan".into(),name:TOOL_NAME.into(),provider_metadata:None,
            arguments:StrictJsonValue(serde_json::json!({"plan":[{"step":"Save","status":"in_progress"}],"exact_actions":[spec(id)]}))};
        let mut plan=AgentPlan::default();
        assert!(!plan.update(&update("original"),&inputs,&NoopAgentEventSink).await.unwrap().is_error);
        let call=ChatToolCall {call_id:"saved".into(),name:"write_file".into(),arguments:StrictJsonValue(serde_json::json!({"path":"a","content":"exact\n"})),provider_metadata:None};
        plan.arm_exact_action(&call,&NoopAgentEventSink).await.unwrap();
        plan.settle_exact_action(&call,&AgentToolResult::text(call.call_id.clone(),serde_json::json!({"written":true,"path":"a","bytes":6,
            "sha256":nomifun_agent_contracts::digest_bytes(b"exact\n")}).to_string(),false),false,&NoopAgentEventSink).await.unwrap();
        assert!(plan.update(&update("renamed"),&inputs,&NoopAgentEventSink).await.unwrap().is_error);
        assert_eq!(plan.exact_actions.len(),1);assert!(plan.exact_actions[0].succeeded);
        let mut wrong=update("original");wrong.arguments.0["exact_actions"][0]["expected_arguments"]["content"]=serde_json::json!("changed");
        assert!(plan.update(&wrong,&inputs,&NoopAgentEventSink).await.unwrap().is_error);
        let mut later=inputs;later.push(crate::context_lifecycle::text_message(ChatRole::User,"Repeat that save now.".into()));
        let mut repeat=update("new-user-authorized");repeat.arguments.0["exact_actions"][0]["source"]=serde_json::json!({"input":1,"quote":"Repeat that save now."});
        assert!(!plan.update(&repeat,&later,&NoopAgentEventSink).await.unwrap().is_error);
        assert_eq!(plan.exact_actions.len(),2);
    }

    #[derive(Default)]
    struct RecordingSink(std::sync::Mutex<Vec<AgentEngineEvent>>);

    #[async_trait::async_trait]
    impl AgentEventSink for RecordingSink {
        async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    struct FailingSink;

    #[async_trait::async_trait]
    impl AgentEventSink for FailingSink {
        async fn emit(&self, _: AgentEngineEvent) -> Result<(), AgentEngineError> {
            Err(AgentEngineError::InvalidContract("fixture persistence failure".into()))
        }
    }

    fn inputs() -> Vec<ChatMessage> {
        vec![crate::context_lifecycle::text_message(ChatRole::User, "inspect".into())]
    }

    fn update_call(status: &str) -> ChatToolCall {
        ChatToolCall {
            call_id: "plan-call".into(), name: TOOL_NAME.into(), provider_metadata: None,
            arguments: StrictJsonValue(serde_json::json!({
                "explanation":"Inspect and verify the requested work",
                "plan":[{"step":"inspect", "status":status}],
                "requirements":[{"id":"R1", "description":"inspect", "source":{"input":0,"quote":"inspect"}}],
            })),
        }
    }

    #[tokio::test]
    async fn rejected_new_citation_locates_the_middle_source_and_preserves_committed_plan() {
        let mut plan=AgentPlan::default();
        let sink=RecordingSink::default();
        let mut accepted=inputs();
        plan.update(&update_call("in_progress"),&accepted,&sink).await.unwrap();
        plan.needs_replan=true;
        let before=plan.clone();
        accepted.push(crate::context_lifecycle::text_message(ChatRole::User,
            "Write final.txt once; keep helper running until Stop. PRIVATE_INPUT_SENTINEL".into()));
        let mut call=update_call("in_progress");
        call.arguments.0["requirements"]=serde_json::json!([
            {"id":"name","description":"Use final.txt","source":{"input":1,"quote":"Write final.txt once"}},
            {"id":"keep","description":"Keep helper running","source":{"input":1,"quote":"keep helper alive"}},
            {"id":"stop","description":"Wait for Stop","source":{"input":1,"quote":"Stop"}},
        ]);
        let original_args=call.arguments.clone();
        let rejected=plan.update(&call,&accepted,&sink).await.unwrap();
        assert!(rejected.is_error);
        let feedback:serde_json::Value=serde_json::from_str(&rejected.output_text()).unwrap();
        assert_eq!(feedback["status"],"rejected");
        assert_eq!(feedback["rejected_parameter_path"],"/requirements/1/source");
        assert_eq!(feedback["source_input_index"],1);
        assert!(!rejected.output_text().contains("PRIVATE_INPUT_SENTINEL"));
        assert!(!rejected.output_text().contains("keep helper alive"));
        assert_eq!(plan,before);
        assert_eq!(call.arguments,original_args);
        assert_eq!(sink.0.lock().unwrap().len(),1,"a rejected source cannot persist a partial plan");
        call.arguments.0["requirements"][1]["source"]["quote"]=serde_json::json!("keep helper running until Stop");
        assert!(!plan.update(&call,&accepted,&sink).await.unwrap().is_error);
        assert_eq!(plan.requirements[0],before.requirements[0]);
        assert_eq!(plan.requirements.len(),4);
        assert_eq!(plan.requirements[1].source.quote,"Write final.txt once");
        assert_eq!(plan.requirements[3].source.quote,"Stop");
        assert!(plan.effect_gate().is_none());
    }

    #[tokio::test]
    async fn a_schema_valid_plan_without_requirements_is_executable_and_idempotent() {
        let mut call = update_call("in_progress");
        call.arguments.0.as_object_mut().unwrap().remove("requirements");
        let mut plan = AgentPlan::default();
        let sink = RecordingSink::default();
        assert!(!plan.update(&call, &inputs(), &sink).await.unwrap().is_error);
        assert_eq!(plan.requirements[0].id, "input_0");
        assert!(plan.effect_gate().is_none());
        assert!(!plan.update(&call, &inputs(), &sink).await.unwrap().is_error);
        assert_eq!(plan.revision, 1);
        assert_eq!(sink.0.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn replaying_plan_is_idempotent_but_replanning_is_a_real_transition() {
        let sink = RecordingSink::default();
        let mut plan = AgentPlan::default();
        let call = update_call("in_progress");
        assert!(!plan.update(&call, &inputs(), &sink).await.unwrap().is_error);
        let original = plan.clone();
        for _ in 0..10 {
            assert!(!plan.update(&call, &inputs(), &sink).await.unwrap().is_error);
        }
        assert_eq!(plan, original);
        assert_eq!(sink.0.lock().unwrap().len(), 1);

        plan.needs_replan = true;
        assert!(!plan.update(&call, &inputs(), &sink).await.unwrap().is_error);
        assert_eq!(plan.revision, 2);
        assert!(!plan.needs_replan);
        assert_eq!(sink.0.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn revisions_survive_long_tasks_without_an_arbitrary_sixty_four_update_limit() {
        let mut plan = AgentPlan::default();
        assert!(!plan.update(&update_call("in_progress"), &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
        for index in 2..=130 {
            let status = if index % 2 == 0 { "pending" } else { "in_progress" };
            assert!(!plan.update(&update_call(status), &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
            assert_eq!(plan.revision, index);
        }
        plan.revision = u32::from(u16::MAX);
        assert!(!plan.update(&update_call("completed"), &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
        assert_eq!(plan.revision, 65_536);
        plan.revision = u32::MAX;
        let before = plan.clone();
        assert!(plan.update(&update_call("in_progress"), &inputs(), &NoopAgentEventSink).await.is_err());
        assert_eq!(plan, before, "revision overflow cannot wrap or mutate the plan");
    }

    #[tokio::test]
    async fn persistence_failure_and_invalid_proposal_preserve_the_committed_plan() {
        let mut plan = AgentPlan::default();
        plan.update(&update_call("in_progress"), &inputs(), &NoopAgentEventSink).await.unwrap();
        let before = plan.clone();
        assert!(plan.update(&update_call("completed"), &inputs(), &FailingSink).await.is_err());
        assert_eq!(plan, before);
        let mut invalid = update_call("in_progress");
        invalid.arguments.0["plan"] = serde_json::json!([
            {"step":"first", "status":"in_progress"},
            {"step":"second", "status":"in_progress"},
        ]);
        let result = plan.update(&invalid, &inputs(), &NoopAgentEventSink).await.unwrap();
        assert!(result.is_error);
        let feedback: serde_json::Value = serde_json::from_str(&result.output_text()).unwrap();
        assert_eq!(feedback["status"], "rejected");
        assert_eq!(feedback["plan_revision"], 1);
        assert_eq!(feedback["plan"][0]["step"], "inspect");
        assert_eq!(plan, before);
    }

    #[tokio::test]
    async fn schema_character_limits_do_not_reject_valid_chinese_plans_as_byte_overflows() {
        let quote = "检查".repeat(200);
        let input = vec![crate::context_lifecycle::text_message(ChatRole::User, quote.clone())];
        let mut call = update_call("in_progress");
        call.arguments.0["explanation"] = serde_json::Value::String("解释".repeat(800));
        call.arguments.0["plan"][0]["step"] = serde_json::Value::String("步骤".repeat(256));
        call.arguments.0["requirements"][0]["description"] = serde_json::Value::String("需求".repeat(256));
        call.arguments.0["requirements"][0]["source"]["quote"] = serde_json::Value::String(quote);
        let mut plan = AgentPlan::default();
        let result = plan.update(&call, &input, &NoopAgentEventSink).await.unwrap();
        assert!(!result.is_error, "{}", result.output_text());
        assert_eq!(plan.steps[0].step.chars().count(), 512);
    }

    #[tokio::test]
    async fn status_update_preserves_rephrased_requirement_and_explains_omission() {
        let original = AgentTaskRequirement {
            id: "R1".into(), description: "Fix the failing tests".into(),
            source: AgentInputCitation { input: 0, quote: "Fix the failing tests".into() },
            origin: None,
        };
        let mut plan = AgentPlan {
            revision: 1, explanation: "Start".into(),
            steps: vec![AgentPlanStep { step: "Fix tests".into(), status: AgentPlanStatus::InProgress }],
            needs_replan: false, requirements: vec![original.clone()],exact_actions:Vec::new(),
        };
        let call = ChatToolCall {
            call_id: "update-2".into(), name: TOOL_NAME.into(), provider_metadata: None,
            arguments: StrictJsonValue(serde_json::json!({
                "explanation":"Tests passed", "plan":[{"step":"Fix tests","status":"completed"}],
                "requirements":[{"id":"R1","description":"A weakened restatement",
                    "source":{"input":0,"quote":"Fix the failing tests"}}]
            })),
        };
        let input = ChatMessage { role: ChatRole::User, provider_round_id: None,
            content: vec![ChatContentPart::Text { text: "Fix the failing tests".into() }] };
        let result = plan.update(&call, &[input], &NoopAgentEventSink).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(plan.requirements, vec![original]);
        assert_eq!(plan.steps[0].status, AgentPlanStatus::Completed);
        assert!(plan.effect_gate().is_some_and(|reason|
            reason.contains("already current successful observation")));
        assert!(matches!(&result.output[0], ChatToolResultPart::Text { text }
            if text.contains("kept unchanged") && text.contains("Omit requirements")));
        let revision = plan.revision;
        let repeated = plan.update(&call, &[crate::context_lifecycle::text_message(
            ChatRole::User, "Fix the failing tests".into(),
        )], &NoopAgentEventSink).await.unwrap();
        assert!(!repeated.is_error);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&repeated.output_text()).unwrap()["status"], "unchanged");
        assert_eq!(plan.revision, revision);
    }

    #[tokio::test]
    async fn optional_explanation_does_not_block_revisions_or_drop_accepted_requirements() {
        let mut plan = AgentPlan::default();
        let mut status = update_call("completed");
        let schema = definition().input_schema.0;
        let validator = jsonschema::options().build(&schema).unwrap();
        assert!(validator.is_valid(&status.arguments.0));
        status.arguments.0.as_object_mut().unwrap().remove("explanation");
        assert!(validator.is_valid(&status.arguments.0));
        assert!(!plan.update(&status, &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
        plan.update(&update_call("in_progress"), &inputs(), &NoopAgentEventSink).await.unwrap();
        let requirements = plan.requirements.clone();
        let explanation = plan.explanation.clone();
        // Preserve compatibility for already-recorded status-only calls.
        let result = plan.update(&status, &inputs(), &NoopAgentEventSink).await.unwrap();
        assert!(!result.is_error, "{}", result.output_text());
        assert_eq!(plan.requirements, requirements);
        assert_eq!(plan.explanation, explanation);
        assert_eq!(plan.steps[0].status, AgentPlanStatus::Completed);
        status.arguments.0["plan"][0]["step"] = serde_json::json!("Different work");
        assert!(!plan.update(&status, &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
        assert_eq!(plan.requirements, requirements);
        plan.needs_replan = true;
        let mut recovery = update_call("in_progress");
        recovery.arguments.0.as_object_mut().unwrap().remove("explanation");
        assert!(!plan.update(&recovery, &inputs(), &NoopAgentEventSink).await.unwrap().is_error);
        assert!(!plan.needs_replan);
        assert_eq!(plan.requirements, requirements);
    }

    #[test]
    fn absent_plan_allows_work_but_recovery_and_explicit_closure_still_gate_effects() {
        let mut plan = AgentPlan::default();
        assert!(plan.effect_gate().is_none());
        plan.needs_replan = true;
        assert!(plan.effect_gate().is_some());
        plan.needs_replan = false;
        plan.revision = 1;
        plan.steps.push(AgentPlanStep { step: "Create files".into(), status: AgentPlanStatus::Completed });
        assert!(plan.effect_gate().is_some());
        plan.steps[0].status = AgentPlanStatus::InProgress;
        assert!(plan.effect_gate().is_none());
    }
}
