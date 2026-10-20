//! Read-only presentation of the original domain events. No presentation rows
//! or secondary lifecycle ledger are persisted.
use crate::{DbError, NewAgentExecutionEvent};
use nomifun_common::{AgentExecutionEventKind, DelegationProcessRecord, DelegationRecordAnchor,
    AgentExecutionSource as Source, DelegationRecordKind as Kind, DelegationRecordSource};
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transition { step_id: String, attempt_id: Option<String>, kind: Kind }

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Presentation { anchor: DelegationRecordAnchor, transitions: Vec<Transition> }

#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordCursor {
    session: String,
    sources: Option<Vec<String>>,
    after_source_seq: i64,
    through_source_seq: Option<i64>,
    execution_fences: std::collections::BTreeMap<String,i64>,
    before: String,
}

fn invalid(detail: impl std::fmt::Display) -> DbError { DbError::Conflict(format!("delegation presentation: {detail}")) }

async fn completed_receipt_matches(tx:&mut Transaction<'_,Sqlite>,execution:&str,transition:&Transition,
    payload:&serde_json::Value,owner:&str)->Result<bool,DbError> {
    let Some(attempt)=transition.attempt_id.as_deref() else{return Ok(false);};
    let sessions:Vec<String>=sqlx::query_scalar("SELECT DISTINCT conversation_id FROM conversation_execution_links WHERE execution_id=? AND step_id=? AND attempt_id=? AND relation IN ('attempt','automation')")
        .bind(execution).bind(&transition.step_id).bind(attempt).fetch_all(&mut **tx).await?;
    let [session]=sessions.as_slice() else{return Ok(false);};
    let operation=payload.get("canonical_operation_id").and_then(|value|value.as_str()).map(str::to_owned)
        .or_else(||payload.get("turn_operation_key").and_then(|value|value.as_str()).map(|key|format!("turn:user:{owner}:{session}:{key}")));
    let Some(operation)=operation else{return Ok(false);};
    let expected_terminal=payload.get("terminal_event_id").and_then(|value|value.as_str());
    let terminal:Option<String>=sqlx::query_scalar("SELECT event.event_id FROM agent_turns turn JOIN agent_events event ON event.session_id=turn.session_id AND event.event_id=turn.terminal_event_id WHERE turn.session_id=? AND turn.operation_id=? AND turn.state='completed' AND event.kind='turn/completed' AND event.kind_version=1")
        .bind(session).bind(operation).fetch_optional(&mut **tx).await?;
    Ok(terminal.is_some() && expected_terminal.is_none_or(|expected|terminal.as_deref()==Some(expected)))
}

async fn source_for_step(tx: &mut Transaction<'_, Sqlite>, execution: &str, step: &str)
    -> Result<Option<Source>, DbError> {
    let appended: Option<String> = sqlx::query_scalar(
        "SELECT event.payload FROM agent_execution_events event,json_each(event.payload,'$.added_step_ids') added \
         WHERE event.execution_id=? AND event.event_type='plan_changed' AND added.value=? ORDER BY event.sequence LIMIT 1")
        .bind(execution).bind(step).fetch_optional(&mut **tx).await?;
    if let Some(payload) = appended {
        let value: serde_json::Value = serde_json::from_str(&payload).map_err(invalid)?;
        // An appended task belongs to its actual caller. Missing old provenance
        // must never silently borrow the root's different tool invocation.
        return value.get("delegation_source").filter(|source|!source.is_null()).cloned().map(serde_json::from_value).transpose().map_err(invalid);
    }
    initial_source(tx,execution).await
}

async fn initial_source(tx:&mut Transaction<'_,Sqlite>,execution:&str)->Result<Option<Source>,DbError> {
    let input: String = sqlx::query_scalar("SELECT initial_plan_input FROM agent_executions WHERE execution_id=?")
        .bind(execution).fetch_one(&mut **tx).await?;
    let input: serde_json::Value = serde_json::from_str(&input).map_err(invalid)?;
    if input.get("mode").and_then(|mode|mode.as_str())==Some("automation") {return Ok(None);}
    input.get("source").filter(|source|!source.is_null()).cloned().map(serde_json::from_value).transpose().map_err(invalid)
}

async fn resolve_source(tx: &mut Transaction<'_, Sqlite>, source: &Source, owner: &str)
    -> Result<Option<DelegationRecordSource>, DbError> {
    let row: Option<(String, String, i64, String)> = sqlx::query_as(
        "SELECT turn.source_message_id,tool.event_id,tool.seq,json_extract(tool.inline_json,'$.call_id') FROM agent_turns turn \
         JOIN agent_sessions session ON session.agent_session_id=turn.session_id \
         JOIN agent_effects effect ON effect.session_id=turn.session_id AND effect.effect_id=? \
         JOIN agent_events started ON started.session_id=effect.session_id AND started.event_id=effect.started_event_id \
         JOIN agent_events tool ON tool.session_id=started.session_id AND tool.event_id=started.causation_event_id \
         WHERE turn.session_id=? AND turn.operation_id=? \
           AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
           AND json_extract(session.owner_ref_json,'$.principal_id')=? \
           AND effect.turn_id=turn.operation_id AND effect.operation_id=? \
           AND effect.owner_domain='wave5.product' AND effect.capability_module='agent.collaboration' \
           AND effect.action_id IN ('agent/delegate','agent/fork') \
           AND tool.kind='tool/call-started' AND tool.kind_version=1 \
           AND tool.causation_event_id=turn.started_event_id \
           AND json_extract(tool.inline_json,'$.operation_id')=effect.operation_id")
        .bind(&source.effect_id).bind(&source.agent_session_id).bind(&source.turn_operation_id)
        .bind(owner).bind(&source.tool_operation_id).fetch_optional(&mut **tx).await?;
    Ok(row.map(|(source_message_id,tool_event_id,tool_event_seq,tool_call_id)| DelegationRecordSource {
        agent_session_id:source.agent_session_id.clone(),turn_operation_id:source.turn_operation_id.clone(),
        source_message_id,tool_operation_id:source.tool_operation_id.clone(),effect_id:source.effect_id.clone(),
        tool_event_id,tool_event_seq,tool_call_id,
    }))
}

/// Called only by the original writer, after the business mutation and before
/// its event append. A source Session cursor is captured in that same SQLite
/// transaction; no timestamp or cross-domain sequence comparison is involved.
pub(crate) async fn annotate_event_tx(tx: &mut Transaction<'_, Sqlite>, execution: &str,
    event: &NewAgentExecutionEvent, owner: &str) -> Result<String, DbError> {
    let mut payload: serde_json::Value = serde_json::from_str(&event.payload).map_err(invalid)?;
    let mut transitions = Vec::new();
    if event.event_type == AgentExecutionEventKind::PlanChanged {
        let ids: Vec<String> = if let Some(ids) = payload.get("added_step_ids") {
            serde_json::from_value(ids.clone()).map_err(invalid)?
        } else if let Some(revision) = payload.get("plan_revision").and_then(|v|v.as_i64()) {
            sqlx::query_scalar("SELECT step_id FROM agent_execution_steps WHERE execution_id=? AND introduced_in_revision=? AND kind='agent' ORDER BY step_id")
                .bind(execution).bind(revision).fetch_all(&mut **tx).await?
        } else { Vec::new() };
        let agent_ids:Vec<String>=sqlx::query_scalar("SELECT step_id FROM agent_execution_steps WHERE execution_id=? AND kind='agent' AND step_id IN (SELECT value FROM json_each(?)) ORDER BY step_id")
            .bind(execution).bind(serde_json::to_string(&ids).map_err(invalid)?).fetch_all(&mut **tx).await?;
        for step_id in agent_ids { transitions.push(Transition {step_id,attempt_id:None,kind:Kind::Delegated}); }
    } else if let Some(step) = event.step_id.as_ref() {
        let settlement = event.event_type == AgentExecutionEventKind::AttemptChanged && (
            payload.get("step_status").is_some() || payload.get("reconciliation").is_some()
            || payload.get("change").and_then(|v|v.as_str()) == Some("output_adopted"));
        let step_terminal = event.event_type == AgentExecutionEventKind::StepChanged && payload.get("status").is_some();
        if settlement || step_terminal {
            let status: Option<String> = sqlx::query_scalar("SELECT status FROM agent_execution_steps WHERE execution_id=? AND step_id=? AND kind='agent'")
                .bind(execution).bind(step).fetch_optional(&mut **tx).await?;
            let kind = match status.as_deref() { Some("completed") => Some(Kind::Completed),
                Some("failed") => Some(Kind::Failed), Some("skipped") => Some(Kind::Skipped), _ => None };
            if let Some(kind) = kind { transitions.push(Transition {step_id:step.clone(),attempt_id:event.attempt_id.clone(),kind}); }
        }
    }
    if transitions.is_empty() { return Ok(event.payload.clone()); }
    // A plan append's source is in the event being committed, not yet in the
    // event table. All tasks introduced by that command share its exact cause.
    let appended_source: Option<Source> = payload.get("delegation_source").filter(|source|!source.is_null()).cloned()
        .map(serde_json::from_value).transpose().map_err(invalid)?;
    let plan_source=if event.event_type==AgentExecutionEventKind::PlanChanged {
        if payload.get("added_step_ids").is_some() {appended_source.clone()}else{initial_source(tx,execution).await?}
    }else{None};
    let mut grouped: Vec<Presentation> = Vec::new();
    let mut verified_sources:Vec<(Source,DelegationRecordSource,DelegationRecordAnchor)>=Vec::new();
    for transition in transitions {
        let source = if event.event_type==AgentExecutionEventKind::PlanChanged {plan_source.clone()}
            else {source_for_step(tx,execution,&transition.step_id).await?};
        let Some(source) = source else { continue; };
        if transition.kind==Kind::Completed && !completed_receipt_matches(tx,execution,&transition,&payload,owner).await? {continue;}
        let cached=verified_sources.iter().find(|(key,_,_)|*key==source).cloned();
        let (source,anchor)=if let Some((_,source,anchor))=cached {(source,anchor)}else{
            let Some(verified)=resolve_source(tx,&source,owner).await? else{continue;};
            let head: Option<(String,i64)> = sqlx::query_as(
            "SELECT event.event_id,event.seq FROM agent_session_heads head JOIN agent_events event \
             ON event.session_id=head.session_id AND event.seq=head.last_seq WHERE head.session_id=?")
            .bind(&source.agent_session_id).fetch_optional(&mut **tx).await?;
            let Some((event_id,seq))=head else{return Err(invalid("source Session has no canonical head"));};
            let anchor=DelegationRecordAnchor{agent_session_id:source.agent_session_id.clone(),event_id,seq};
            verified_sources.push((source,verified.clone(),anchor.clone()));
            (verified,anchor)
        };
        if let Some(group)=grouped.iter_mut().find(|group|group.anchor.agent_session_id==source.agent_session_id) {
            group.transitions.push(transition);
        } else { grouped.push(Presentation {anchor,transitions:vec![transition]}); }
    }
    if !grouped.is_empty() {
        payload.as_object_mut().ok_or_else(||invalid("domain event payload is not an object"))?
            .insert("delegation_presentation".into(),serde_json::to_value(grouped).map_err(invalid)?);
    }
    serde_json::to_string(&payload).map_err(invalid)
}

// The source window is resolved once inside SQLite. In particular, the query
// never loads unrelated Attempt/control payloads into the application and never
// recovers provenance from the currently selected Conversation execution.
const RECORD_SOURCES: &str = r#"
WITH executions AS (
 SELECT execution.* FROM agent_executions execution
 WHERE execution.user_id=?1 AND execution.deleted_at IS NULL
   AND (json_extract(execution.initial_plan_input,'$.source.agent_session_id')=?2
     OR EXISTS(SELECT 1 FROM agent_execution_events nested
       WHERE nested.execution_id=execution.execution_id AND nested.event_type='plan_changed'
         AND json_extract(nested.payload,'$.delegation_source.agent_session_id')=?2))
), subjects AS (
 SELECT execution.execution_id,execution.event_sequence,step.step_id,step.title,step.introduced_in_revision,
   CASE WHEN nested.sequence IS NOT NULL THEN json_extract(nested.payload,'$.delegation_source')
     ELSE json_extract(execution.initial_plan_input,'$.source') END AS source
 FROM executions execution JOIN agent_execution_steps step ON step.execution_id=execution.execution_id AND step.kind='agent'
 LEFT JOIN agent_execution_events nested ON nested.execution_id=execution.execution_id AND nested.sequence=(
   SELECT MIN(origin.sequence) FROM agent_execution_events origin,json_each(origin.payload,'$.added_step_ids') added
   WHERE origin.execution_id=execution.execution_id AND origin.event_type='plan_changed' AND added.value=step.step_id)
), resolved AS MATERIALIZED (
 SELECT subject.execution_id,subject.event_sequence,subject.step_id,subject.title,subject.introduced_in_revision,
   source_turn.session_id AS source_session,source_turn.operation_id AS source_operation,
   source_turn.source_message_id,source_request.seq AS source_request_seq,
   effect.operation_id AS tool_operation,effect.effect_id,tool.event_id AS tool_event,tool.seq AS tool_seq,
   json_extract(tool.inline_json,'$.call_id') AS tool_call_id
 FROM subjects subject
 JOIN agent_turns source_turn ON source_turn.session_id=?2
   AND source_turn.session_id=json_extract(subject.source,'$.agent_session_id')
   AND source_turn.operation_id=json_extract(subject.source,'$.turn_operation_id')
 JOIN agent_sessions source_session ON source_session.agent_session_id=source_turn.session_id
   AND json_extract(source_session.owner_ref_json,'$.principal_kind')='user'
   AND json_extract(source_session.owner_ref_json,'$.principal_id')=?1
 JOIN agent_events source_request ON source_request.session_id=source_turn.session_id AND source_request.event_id=source_turn.source_message_id
 JOIN agent_effects effect ON effect.session_id=source_turn.session_id AND effect.effect_id=json_extract(subject.source,'$.effect_id')
   AND effect.turn_id=source_turn.operation_id AND effect.operation_id=json_extract(subject.source,'$.tool_operation_id')
   AND effect.owner_domain='wave5.product' AND effect.capability_module='agent.collaboration'
   AND effect.action_id IN ('agent/delegate','agent/fork')
 JOIN agent_events effect_start ON effect_start.session_id=effect.session_id AND effect_start.event_id=effect.started_event_id
 JOIN agent_events tool ON tool.session_id=effect_start.session_id AND tool.event_id=effect_start.causation_event_id
   AND tool.kind='tool/call-started' AND tool.kind_version=1 AND tool.causation_event_id=source_turn.started_event_id
   AND json_extract(tool.inline_json,'$.operation_id')=effect.operation_id
 WHERE (?3 IS NOT NULL AND source_turn.source_message_id IN (SELECT value FROM json_each(?3)))
   OR (?3 IS NULL AND source_request.seq>?4 AND source_request.seq<=?5)
)
"#;

// A fixed per-execution fence lives only in this opaque read cursor. No new
// database cursor, lifecycle row or global sequence is created.
const RECORD_CANDIDATES: &str = r#"
, fenced_events AS MATERIALIZED (
 SELECT event.* FROM json_each(?6) fence JOIN agent_execution_events event
   ON event.execution_id=fence.key AND event.sequence<=CAST(fence.value AS INTEGER)
 WHERE event.event_type IN ('plan_changed','attempt_changed','step_changed')
   AND (json_type(event.payload,'$.delegation_presentation')='array'
     OR (event.event_type='plan_changed' AND (json_type(event.payload,'$.plan_revision')='integer' OR json_type(event.payload,'$.added_step_ids')='array'))
     OR json_extract(event.payload,'$.step_status') IN ('completed','failed','skipped')
     OR (event.event_type='step_changed' AND json_extract(event.payload,'$.status') IN ('failed','skipped'))
     OR json_extract(event.payload,'$.change')='output_adopted')
), candidates AS (
 SELECT event.execution_id,event.sequence,event.payload,json_extract(transition.value,'$.step_id') AS step_id,
   json_extract(transition.value,'$.attempt_id') AS attempt_id,json_extract(transition.value,'$.kind') AS kind,
   json_extract(presentation.value,'$.anchor.agent_session_id') AS anchor_session,
   json_extract(presentation.value,'$.anchor.event_id') AS anchor_event,
   json_extract(presentation.value,'$.anchor.seq') AS anchor_seq,0 AS source_scoped_order
 FROM fenced_events event,json_each(event.payload,'$.delegation_presentation') presentation,
   json_each(presentation.value,'$.transitions') transition
 UNION ALL
 SELECT event.execution_id,event.sequence,event.payload,source.step_id,NULL,'delegated',source.source_session,source.tool_event,source.tool_seq,1
 FROM fenced_events event JOIN resolved source ON source.execution_id=event.execution_id
 WHERE event.event_type='plan_changed' AND json_type(event.payload,'$.delegation_presentation') IS NULL
   AND (EXISTS(SELECT 1 FROM json_each(event.payload,'$.added_step_ids') added WHERE added.value=source.step_id)
     OR (json_type(event.payload,'$.added_step_ids') IS NULL AND source.introduced_in_revision=json_extract(event.payload,'$.plan_revision')))
 UNION ALL
 SELECT event.execution_id,event.sequence,event.payload,event.step_id,event.attempt_id,
   CASE WHEN json_extract(event.payload,'$.change')='output_adopted' THEN 'completed'
     ELSE COALESCE(json_extract(event.payload,'$.step_status'),json_extract(event.payload,'$.status')) END,
   source.source_session,source.tool_event,source.tool_seq,1
 FROM fenced_events event JOIN resolved source ON source.execution_id=event.execution_id AND source.step_id=event.step_id
 WHERE event.event_type IN ('attempt_changed','step_changed') AND json_type(event.payload,'$.delegation_presentation') IS NULL
   AND (json_extract(event.payload,'$.step_status') IN ('completed','failed','skipped')
     OR (event.event_type='step_changed' AND json_extract(event.payload,'$.status') IN ('failed','skipped'))
     OR json_extract(event.payload,'$.change')='output_adopted')
), records AS (
 SELECT candidate.execution_id,candidate.sequence,candidate.kind,candidate.step_id,candidate.attempt_id,source.title,
   source.source_session,source.source_operation,source.source_message_id,source.tool_operation,source.tool_call_id,
   source.effect_id,source.tool_event,source.tool_seq,candidate.anchor_event,candidate.anchor_seq,candidate.source_scoped_order,
   printf('%020d:%s:%020d:%s',candidate.anchor_seq,candidate.execution_id,candidate.sequence,candidate.step_id) AS order_key
 FROM candidates candidate JOIN resolved source ON source.execution_id=candidate.execution_id AND source.step_id=candidate.step_id
 JOIN agent_events anchor ON anchor.session_id=source.source_session AND anchor.session_id=candidate.anchor_session
   AND anchor.event_id=candidate.anchor_event AND anchor.seq=candidate.anchor_seq
 WHERE candidate.kind IN ('delegated','completed','failed','skipped')
   AND (candidate.kind<>'completed' OR EXISTS(
     SELECT 1 FROM conversation_execution_links link
     JOIN agent_turns child ON child.session_id=link.conversation_id
       AND child.operation_id=COALESCE(json_extract(candidate.payload,'$.canonical_operation_id'),
         'turn:user:'||?1||':'||link.conversation_id||':'||json_extract(candidate.payload,'$.turn_operation_key'))
       AND child.state='completed'
     JOIN agent_events terminal ON terminal.session_id=child.session_id AND terminal.event_id=child.terminal_event_id
       AND terminal.kind='turn/completed' AND terminal.kind_version=1
     WHERE link.execution_id=candidate.execution_id AND link.step_id=candidate.step_id AND link.attempt_id=candidate.attempt_id
       AND link.relation IN ('attempt','automation')
       AND (json_extract(candidate.payload,'$.terminal_event_id') IS NULL OR terminal.event_id=json_extract(candidate.payload,'$.terminal_event_id'))))
)
SELECT * FROM records WHERE (?7 IS NULL OR order_key<?7) ORDER BY order_key DESC LIMIT ?8
"#;

#[derive(sqlx::FromRow)]
struct RecordRow {
    execution_id:String,sequence:i64,kind:String,step_id:String,attempt_id:Option<String>,title:String,
    source_session:String,source_operation:String,source_message_id:String,tool_operation:String,tool_call_id:String,
    effect_id:String,tool_event:String,tool_seq:i64,anchor_event:String,anchor_seq:i64,source_scoped_order:bool,order_key:String,
}

/// Bounded newest-first keyset reads. Source-request membership and ordering
/// anchors are separate: a later parent Turn never adopts the original task.
pub async fn read_delegation_records(pool:&SqlitePool,owner:&str,session:&str,
    source_messages:Option<&[String]>,after:i64,through:Option<i64>,cursor:Option<&str>,limit:usize)
    ->Result<(Vec<DelegationProcessRecord>,i64,Option<String>),DbError> {
    let mut tx=pool.begin().await?;
    let head:Option<i64>=sqlx::query_scalar("SELECT head.last_seq FROM agent_session_heads head JOIN agent_sessions session ON session.agent_session_id=head.session_id WHERE head.session_id=? AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_kind')='user' AND json_extract(session.owner_ref_json,'$.principal_id')=?")
        .bind(session).bind(owner).fetch_optional(&mut *tx).await?;
    let head=head.ok_or_else(||invalid("source Session is unavailable"))?;
    let upper=through.unwrap_or(head).min(head);
    let mut sources=source_messages.map(|roots|roots.to_vec());
    if let Some(roots)=sources.as_mut(){roots.sort();roots.dedup();}
    let source_filter=sources.as_ref().map(serde_json::to_string).transpose().map_err(invalid)?;
    let saved:Option<RecordCursor>=cursor.map(|cursor| {
        let bytes=hex::decode(cursor).map_err(invalid)?;
        serde_json::from_slice(&bytes).map_err(invalid)
    }).transpose()?;
    if saved.as_ref().is_some_and(|saved|saved.session!=session || saved.sources!=sources
        || saved.after_source_seq!=after || saved.through_source_seq!=through) {
        return Err(invalid("pagination cursor belongs to a different source window"));
    }
    let execution_fences=if let Some(saved)=saved.as_ref(){saved.execution_fences.clone()}else{
        let sql=format!("{RECORD_SOURCES} SELECT DISTINCT execution_id,event_sequence FROM resolved");
        sqlx::query_as::<_,(String,i64)>(&sql).bind(owner).bind(session).bind(&source_filter).bind(after).bind(upper)
            .fetch_all(&mut *tx).await?.into_iter().collect::<std::collections::BTreeMap<_,_>>()
    };
    let fences=serde_json::to_string(&execution_fences).map_err(invalid)?;
    let sql=format!("{RECORD_SOURCES}{RECORD_CANDIDATES}");
    let limit=limit.clamp(1,500);
    let mut rows=sqlx::query_as::<_,RecordRow>(&sql).bind(owner).bind(session).bind(&source_filter).bind(after).bind(upper)
        .bind(&fences).bind(saved.as_ref().map(|saved|saved.before.as_str())).bind((limit+1)as i64)
        .fetch_all(&mut *tx).await?;
    let next=if rows.len()>limit {
        rows.truncate(limit);
        let next=RecordCursor{session:session.into(),sources,after_source_seq:after,through_source_seq:through,
            execution_fences,before:rows.last().expect("nonempty bounded page").order_key.clone()};
        Some(hex::encode(serde_json::to_vec(&next).map_err(invalid)?))
    }else{None};
    let records=rows.into_iter().map(|row|Ok(DelegationProcessRecord{
        id:format!("{}:{:020}:{}",row.execution_id,row.sequence,row.step_id),
        kind:serde_json::from_value(row.kind.into()).map_err(invalid)?,
        execution_id:row.execution_id,execution_event_sequence:row.sequence,step_id:row.step_id,attempt_id:row.attempt_id,title:row.title,
        source_scoped_order:row.source_scoped_order,
        source:DelegationRecordSource{agent_session_id:row.source_session.clone(),turn_operation_id:row.source_operation,
            source_message_id:row.source_message_id,tool_operation_id:row.tool_operation,tool_call_id:row.tool_call_id,
            effect_id:row.effect_id,tool_event_id:row.tool_event,tool_event_seq:row.tool_seq},
        anchor:DelegationRecordAnchor{agent_session_id:row.source_session,event_id:row.anchor_event,seq:row.anchor_seq},
    })).collect::<Result<Vec<_>,DbError>>()?;
    tx.commit().await?;
    Ok((records,head,next))
}
