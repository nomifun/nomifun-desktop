//! Bounded source-scoped observation references. They acknowledge observations,
//! never result bodies, questions answered, task success or execution authority.
use super::*;
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    executions: BTreeMap<String, AgentCollaborationObservation>,
}

#[derive(Deserialize)]
pub(super) struct StepObservation {
    pub terminal_digest: Option<String>,
    pub page_source_digest: Option<String>,
    pub ready_digest:Option<String>,
}
impl StepObservation {
    pub(super) fn validate(&self)->Result<(),AgentEngineError> {
        if self.terminal_digest.iter().chain(self.page_source_digest.iter()).chain(self.ready_digest.iter()).any(|digest|!valid_digest(digest)) {
            return Err(AgentEngineError::InvalidContract("Step source header digest is invalid".into()));
        }
        Ok(())
    }
}

impl AgentCollaborationSnapshot {
    pub(crate) fn decision_blocked(&self)->Result<bool,AgentEngineError> {
        match self.result.0.get("wait_outcome") {
            None=>Ok(false),
            Some(serde_json::Value::String(value)) if value=="decision_blocked"=>Ok(true),
            Some(serde_json::Value::String(value)) if value=="parent_input_pending"=>Ok(false),
            _=>Err(AgentEngineError::InvalidContract("child wait returned an unknown typed outcome".into())),
        }
    }
    pub(crate) fn parent_input_pending(&self)->bool {self.result.0["wait_outcome"]=="parent_input_pending"}

    pub(crate) fn ready_sources(&self)->Result<BTreeMap<String,BTreeMap<String,String>>,AgentEngineError> {
        let mut result=BTreeMap::new();
        let mut references=Self::observation_references(&self.cursor)?;
        for row in self.result.0["executions"].as_array().ok_or_else(||AgentEngineError::InvalidContract("ready observation lacks Execution scope".into()))? {
            let id=row["execution_id"].as_str().ok_or_else(||AgentEngineError::InvalidContract("ready observation lacks Execution identity".into()))?;
            let reference=references.remove(id).ok_or_else(||AgentEngineError::InvalidContract("ready observation duplicates or exceeds cursor scope".into()))?;
            if row["observation_cursor"]!=serde_json::to_value(reference).map_err(|error|AgentEngineError::InvalidContract(error.to_string()))? {
                return Err(AgentEngineError::InvalidContract("ready source heads differ from their scoped observation".into()));
            }
            let heads:BTreeMap<String,StepObservation>=serde_json::from_value(row["step_observations"].clone())
                .map_err(|error|AgentEngineError::InvalidContract(format!("ready observation lacks full Step source heads: {error}")))?;
            let owned=row["owned_step_ids"].as_array().ok_or_else(||AgentEngineError::InvalidContract("ready observation lacks owned Steps".into()))?;
            let owned_set=owned.iter().filter_map(|id|id.as_str()).collect::<std::collections::BTreeSet<_>>();
            if heads.len()!=owned.len()||owned_set.len()!=owned.len()||owned.len()>128||owned.iter().any(|id|id.as_str().is_none_or(|id|!valid_id(id)||!heads.contains_key(id))) {
                return Err(AgentEngineError::InvalidContract("ready source heads differ from owned Step scope".into()));
            }
            let mut sources=BTreeMap::new();
            for (step,head) in heads {head.validate()?;if let Some(digest)=head.ready_digest {sources.insert(format!("step:{step}"),digest);}}
            let terminal=&row["execution_terminal_receipt"];let attention=&row["execution_attention"];
            if !terminal.is_null()||!attention.is_null() {
                let digest=nomifun_agent_contracts::digest_payload(&serde_json::json!({"terminal":terminal,"attention":attention}))
                    .map_err(|error|AgentEngineError::InvalidContract(error.to_string()))?;
                sources.insert("@execution".into(),digest.as_ref().into());
            }
            result.insert(id.into(),sources);
        }
        if !references.is_empty() {return Err(AgentEngineError::InvalidContract("ready cursor contains unreturned Execution scope".into()));}
        Ok(result)
    }
    pub fn observation_cursor(executions: BTreeMap<String,AgentCollaborationObservation>) -> Result<String, AgentEngineError> {
        if executions.len()>64 || executions.iter().any(|(id,cursor)|!valid_id(id)||!cursor.validate()) {
            return Err(AgentEngineError::InvalidContract("invalid exact Execution observation references".into()));
        }
        let encoded=serde_json::to_string(&Cursor {version:1,executions})
            .map_err(|error|AgentEngineError::InvalidContract(error.to_string()))?;
        if encoded.len()>16384 {return Err(AgentEngineError::InvalidContract("Execution observation scope exceeds its bound".into()));}
        Ok(encoded)
    }

    pub fn observation_references(cursor: &str) -> Result<BTreeMap<String,AgentCollaborationObservation>, AgentEngineError> {
        if cursor.is_empty()||cursor.len()>16384||cursor.contains('\0') {
            return Err(AgentEngineError::InvalidContract("child observation cursor exceeds its bound".into()));
        }
        let decoded:Cursor=serde_json::from_str(cursor).map_err(|_|AgentEngineError::InvalidContract(
            "child observation cursor has no exact Execution scope".into()))?;
        if decoded.version!=1 {return Err(AgentEngineError::InvalidContract("child observation cursor version differs".into()));}
        if Self::observation_cursor(decoded.executions.clone())?!=cursor {
            return Err(AgentEngineError::InvalidContract("child observation cursor is not canonical".into()));
        }
        Ok(decoded.executions)
    }

    pub fn has_new_ready_observation(&self,previous:Option<&str>)->Result<bool,AgentEngineError> {
        let previous=previous.map(Self::observation_references).transpose()?.unwrap_or_default();
        Ok(Self::observation_references(&self.cursor)?.iter().any(|(id,current)|
            previous.get(id).is_none_or(|old|old.ready!=current.ready)))
    }
}

impl AgentCollaborationProgress {
    pub(crate) fn has_new_ready_source(&self,snapshot:&AgentCollaborationSnapshot)->Result<bool,AgentEngineError> {
        Ok(snapshot.ready_sources()?.iter().any(|(id,sources)|sources.iter().any(|(key,digest)|
            self.results.get(id).and_then(|read|read.ready_source_digests.get(key)
                .or_else(||key.strip_prefix("step:").and_then(|step|read.source_digests.get(step))))!=Some(digest))))
    }
    /// One normalization for explicit calls, automatic waits and journal replay.
    /// A newer successful native observation dominates stale caller data.
    pub(crate) fn normalize_wait(&self, wait: &AgentChildrenWait) -> Result<AgentChildrenWait,AgentEngineError> {
        wait.validate()?;
        let mut wait=wait.clone();wait.execution_ids.sort();
        let mut references=wait.after_cursor.as_deref().map(AgentCollaborationSnapshot::observation_references)
            .transpose()?.unwrap_or_default();
        if references.keys().any(|id|!wait.execution_ids.contains(id)) {
            return Err(AgentEngineError::InvalidContract("wait cursor belongs to another Execution scope".into()));
        }
        for id in &wait.execution_ids {
            if let Some(cursor)=self.results.get(id).and_then(|read|read.observation_cursor.as_ref()) {
                references.insert(id.clone(),cursor.clone());
            }
        }
        wait.after_cursor=(!references.is_empty()).then(||AgentCollaborationSnapshot::observation_cursor(references)).transpose()?;
        Ok(wait)
    }

    pub(crate) fn record_wake(&mut self, cursor: &str) -> Result<(),AgentEngineError> {
        for (id,reference) in AgentCollaborationSnapshot::observation_references(cursor)? {
            if !self.pending_child_executions.contains(&id)&&!self.results.contains_key(&id) {
                return Err(AgentEngineError::ReplayContract("child wake is outside accepted original work".into()));
            }
            self.results.entry(id.clone()).or_default().observation_cursor=Some(reference);
            self.unread_wakes.insert(id);
        }
        self.validate()
    }
}

/// Fake owner metadata for isolated Runtime tests. Production observations
/// must provide their own complete source headers; no reader fallback exists.
#[cfg(test)]
pub(crate) fn test_snapshot(mut result:serde_json::Value)->AgentCollaborationSnapshot {
    let mut references=BTreeMap::new();
    for row in result["executions"].as_array_mut().expect("fixture Executions") {
        let id=row["execution_id"].as_str().unwrap().to_owned();
        let owned=row["owned_step_ids"].as_array().unwrap().iter().map(|id|id.as_str().unwrap().to_owned()).collect::<std::collections::BTreeSet<_>>();
        let terminal=row["owned_terminal_step_ids"].as_array().unwrap().iter().map(|id|id.as_str().unwrap().to_owned()).collect::<std::collections::BTreeSet<_>>();
        let mut heads=row.get("step_observations").and_then(|value|value.as_object()).cloned().unwrap_or_default();
        heads.retain(|step,_|owned.contains(step));
        for child in row["children"].as_array().unwrap() {
            let step=child["step_id"].as_str().unwrap();
            let identity=serde_json::json!({"terminal_receipt":child["terminal_receipt"],"attempt_id":child["attempt_id"],"source":child["source"]});
            let page=if child.get("source").is_some() {serde_json::json!({"attempt_id":child["attempt_id"],"source":child["source"]})}
                else {serde_json::json!({"attempt_id":child["attempt_id"],"terminal_receipt":child["terminal_receipt"]})};
            let ready=terminal.contains(step)||child["status"]=="waiting_input";
            heads.insert(step.into(),serde_json::json!({"attempt_id":child["attempt_id"],"source":child["source"],
                "status":child["status"],"terminal_digest":terminal.contains(step).then(||nomifun_agent_contracts::digest_payload(&identity).unwrap()),
                "page_source_digest":terminal.contains(step).then(||nomifun_agent_contracts::digest_payload(&page).unwrap()),
                "ready_digest":if terminal.contains(step) {Some(nomifun_agent_contracts::digest_payload(&identity).unwrap())}
                    else {ready.then(||nomifun_agent_contracts::digest_payload(&serde_json::json!({"identity":identity,
                        "review":child["review_blocked"],"question":child["question_source"]})).unwrap())}}));
        }
        for step in &owned {heads.entry(step.clone()).or_insert(serde_json::json!({"terminal_digest":null,"page_source_digest":null}));}
        row["step_observations"]=serde_json::Value::Object(heads);
        let reference=nomifun_agent_contracts::digest_payload(&serde_json::json!({"execution":id,"steps":row["step_observations"],
            "terminal":row["execution_terminal_receipt"],"attention":row["execution_attention"]})).unwrap();
        let reference=AgentCollaborationObservation{state:reference.as_ref().into(),ready:reference.as_ref().into()};
        row["observation_cursor"]=serde_json::json!(reference);
        references.insert(id,reference);
    }
    AgentCollaborationSnapshot {cursor:AgentCollaborationSnapshot::observation_cursor(references).unwrap(),result:StrictJsonValue(result)}
}
