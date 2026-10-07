//! Read-only display links from typed Runtime output-limit recovery events.
//! No text is merged, rewritten, or inferred from wording/timestamps.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use nomifun_agent_contracts::{AgentSessionId, SessionPayloadBody};
use nomifun_agent_runtime::AgentEngineEvent;
use nomifun_agent_session::MessageProjection;
use nomifun_common::AppError;
use nomifun_db::{SqlitePool, sqlx};
use serde_json::Value;

fn links_for_turn(root: &str, events: &[AgentEngineEvent]) -> HashMap<String, String> {
    let mut truncated = BTreeSet::new();
    let mut not_public = BTreeSet::new();
    for event in events {
        match event {
            AgentEngineEvent::ModelOutputTruncated { step, continuation: true, discarded_tool_call_ids }
                if discarded_tool_call_ids.is_empty() => { truncated.insert(*step); }
            AgentEngineEvent::ModelResponseRejected { step, .. }
            | AgentEngineEvent::ToolCallCompleted { step, .. } => { not_public.insert(*step); }
            _ => {}
        }
    }
    let mut links = HashMap::new();
    for step in truncated {
        let Some(next) = step.checked_add(1) else { continue };
        if step == 0 || not_public.contains(&step) || not_public.contains(&next) { continue; }
        if let (Ok(previous), Ok(current)) = (
            super::super::engine_journal::canonical_assistant_step_message_id(root, step),
            super::super::engine_journal::canonical_assistant_step_message_id(root, next),
        ) { links.insert(current, previous); }
    }
    links
}

fn public_text_identity(presentation: &str, document: &Value) -> Option<(String,String)> {
    // Actual canonical projection schema has no type/role member. Accepted
    // user rows have state=accepted and no owning turn_id; only content-part
    // assistant rows have streaming/completed state and the canonical root.
    if presentation!="message" || !matches!(document.get("state")?.as_str()?,"streaming"|"completed")
        || document.get("content")?.as_str()?.trim().is_empty() {return None;}
    Some((document.get("correlation_id")?.as_str()?.into(),document.get("turn_id")?.as_str()?.into()))
}

fn retain_public_links(
    candidates: HashMap<String,(String,String)>, public: &HashMap<String,String>,
) -> HashMap<String,String> {
    candidates.into_iter().filter_map(|(current,(previous,root))| {
        (public.get(&previous)==Some(&root) && public.get(&current)==Some(&root))
            .then_some((current,previous))
    }).collect()
}

pub(super) async fn load(
    pool: &SqlitePool, session: &AgentSessionId, projections: &[MessageProjection],
) -> Result<HashMap<String, String>, AppError> {
    let roots = projections.iter().filter(|projection| projection.presentation_intent == "message")
        .filter_map(|projection| projection.projection.get("turn_id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    if roots.is_empty() { return Ok(HashMap::new()); }
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT turn.source_message_id,event.inline_json,payload.body FROM agent_events event \
         JOIN agent_turns turn ON turn.session_id=event.session_id AND turn.operation_id=event.correlation_id \
         LEFT JOIN agent_payloads payload ON payload.payload_id=event.payload_id AND payload.session_id=event.session_id WHERE event.session_id=");
    query.push_bind(session.as_ref()).push(" AND event.kind='runtime/progress-recorded' AND turn.source_message_id IN (");
    { let mut values=query.separated(",");for root in roots {values.push_bind(root);} }
    query.push(") AND COALESCE(json_extract(event.inline_json,'$.event.event'),json_extract(CAST(payload.body AS TEXT),'$.value.event.event')) \
        IN ('model_output_truncated','model_response_rejected','tool_call_completed') ORDER BY event.seq LIMIT 8193");
    let rows: Vec<(String,Option<String>,Option<Vec<u8>>)> = query.build_query_as().fetch_all(pool).await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    if rows.len()>8192 { return Ok(HashMap::new()); }
    let mut by_root = BTreeMap::<String,Vec<AgentEngineEvent>>::new();
    for (root,inline,body) in rows {
        let value: Value = match (inline,body) {
            (Some(inline),_)=>serde_json::from_str(&inline).map_err(|error|AppError::Internal(error.to_string()))?,
            (_,Some(body))=>match serde_json::from_slice::<SessionPayloadBody>(&body)
                .map_err(|error|AppError::Internal(error.to_string()))? {
                    SessionPayloadBody::Json(value)=>value.0,_=>continue,
                },
            _=>continue,
        };
        let Some(event)=value.get("event") else {continue};
        let event=serde_json::from_value::<AgentEngineEvent>(event.clone())
            .map_err(|error|AppError::Conflict(format!("continuation Runtime event is invalid: {error}")))?;
        by_root.entry(root).or_default().push(event);
    }
    let candidates=by_root.into_iter().flat_map(|(root,events)| {
        links_for_turn(&root,&events).into_iter().map(move |(current,previous)|
            (current,(previous,root.clone())))
    }).collect::<HashMap<_,_>>();
    // Query the exact canonical identities independently of the current page.
    // A genuine prefix outside that page keeps its link; reasoning-only steps
    // have no nonempty public projection and cannot create phantom prefixes.
    let ids=candidates.iter().flat_map(|(current,(previous,_))|[current.as_str(),previous.as_str()])
        .collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
    let mut public=HashMap::new();
    for chunk in ids.chunks(256) {
        let mut query=sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT presentation_intent,projection_json FROM agent_messages WHERE session_id=");
        query.push_bind(session.as_ref()).push(" AND projection_id IN (");
        {let mut values=query.separated(",");for id in chunk {values.push_bind(format!("message:{id}"));}}
        query.push(")");
        let rows:Vec<(String,String)>=query.build_query_as().fetch_all(pool).await
            .map_err(|error|AppError::Internal(error.to_string()))?;
        for (presentation,document) in rows {
            let document=serde_json::from_str::<Value>(&document).map_err(|error|AppError::Internal(error.to_string()))?;
            if let Some((id,root))=public_text_identity(&presentation,&document) {public.insert(id,root);}
        }
    }
    Ok(retain_public_links(candidates,&public))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_continuation_display_links_require_typed_text_only_recovery() {
        let root="0190f5fe-7c00-7a00-8000-000000000001";
        let event=AgentEngineEvent::ModelOutputTruncated {step:1,continuation:true,discarded_tool_call_ids:vec![]};
        let links=links_for_turn(root,&[event.clone()]);
        let previous=super::super::super::engine_journal::canonical_assistant_step_message_id(root,1).unwrap();
        let current=super::super::super::engine_journal::canonical_assistant_step_message_id(root,2).unwrap();
        assert_eq!(links.get(&current),Some(&previous));
        assert!(links_for_turn(root,&[AgentEngineEvent::ModelOutputTruncated {step:1,continuation:false,discarded_tool_call_ids:vec![]}]).is_empty());
        assert!(links_for_turn(root,&[AgentEngineEvent::ModelOutputTruncated {step:1,continuation:true,discarded_tool_call_ids:vec!["discarded".into()]}]).is_empty());
        assert!(links_for_turn(root,&[event,AgentEngineEvent::ModelResponseRejected {step:2,discarded_tool_call_ids:vec![],continuation:true,tool_hint:None}]).is_empty());
    }

    #[test]
    fn public_projection_check_rejects_reasoning_phantoms_and_foreign_turn_but_preserves_paged_prefix() {
        let document=|id:&str,root:&str,content:&str,state:&str|serde_json::json!({
            "correlation_id":id,"turn_id":root,"content":content,"state":state,
        });
        assert!(public_text_identity("message",&document("user","root","task","accepted")).is_none());
        assert!(public_text_identity("thinking",&document("prefix","root","private","completed")).is_none());
        assert!(public_text_identity("message",&document("prefix","root"," \n","completed")).is_none());
        let candidates=HashMap::from([("tail".into(),("prefix".into(),"root".into()))]);
        let (_,tail_root)=public_text_identity("message",&document("tail","root","尾段","completed")).unwrap();
        let mut public=HashMap::from([("tail".into(),tail_root)]);
        assert!(retain_public_links(candidates.clone(),&public).is_empty(),"reasoning-only prefix has no public message");
        public.insert("prefix".into(),"other-turn".into());
        assert!(retain_public_links(candidates.clone(),&public).is_empty(),"cross-Turn prefix is rejected");
        // Public observations represent the exact DB lookup, not the newest
        // renderer page; a real prefix outside that page remains addressed.
        let (id,root)=public_text_identity("message",&document("prefix","root","首段正文","completed")).unwrap();
        public.insert(id,root);
        assert_eq!(retain_public_links(candidates,&public).get("tail"),Some(&"prefix".to_owned()));
    }
}
