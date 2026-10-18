//! On-demand Companion history reads from exact closed canonical Turns.
//! The canonical public-result reducer owns response selection and sealing.
use super::*;
use nomifun_agent_contracts::AgentTurnOutcome;
use nomifun_companion::store::CompanionMemoryMessageSource;
use nomifun_companion::{
    CompanionHistoryFragment, CompanionHistoryRecallCursor, CompanionHistoryRecallPage,
    CompanionHistoryRecallQuery,
};

const MAX_PUBLIC_TURN_EVENTS: i64 = 4096;
const MAX_PUBLIC_TURN_BYTES: i64 = 512 * 1024;
const MAX_RECALL_SESSIONS: i64 = 32;
const MAX_RECALL_TURNS: i64 = 160;
const MAX_RECALL_LIMIT: u32 = 12;
/// Conservative SQL recognition of the existing host archive command. This is
/// only a history DATA floor: neither these references nor a reason string grant
/// execution authority. The HTTP clear command uses user-context-clear:<key>
/// and a fixed user_requested reason, so user text cannot enter this namespace.
/// Unknown/legacy/malformed shapes remain hard clears. Keep the exact writer
/// shape here so SQL can skip any number of automatic archives without hydrating
/// a growing list of clear payloads into runtime memory.
fn trusted_archive_clear_sql() -> String {
    fn uuid_v7(expr: &str) -> String {
        format!(
            "length({expr})=36 AND {expr} GLOB '????????-????-7???-[89ab]???-????????????' AND length(replace({expr},'-',''))=32 AND replace({expr},'-','') NOT GLOB '*[^0-9a-f]*'"
        )
    }
    fn object(path: &str, keys: &[&str]) -> String {
        let allowed = keys
            .iter()
            .map(|key| format!("'{key}'"))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "json_type(c.inline_json,'{path}')='object' AND (SELECT COUNT(*) FROM json_each(c.inline_json,'{path}'))={} AND NOT EXISTS(SELECT 1 FROM json_each(c.inline_json,'{path}') WHERE key NOT IN ({allowed}))",
            keys.len()
        )
    }
    fn integer(path: &str, minimum: u64) -> String {
        // SQLite's JSON token type is still 'integer' for a decimal beyond
        // u64, although extraction converts it to REAL. Only exact signed
        // integer storage is conservatively recognizable without typed decode.
        format!(
            "json_type(c.inline_json,'{path}')='integer' AND typeof(json_extract(c.inline_json,'{path}'))='integer' AND (c.inline_json -> '{path}') NOT GLOB '*[^0-9]*' AND json_extract(c.inline_json,'{path}')>={minimum}"
        )
    }
    fn text(path: &str, digest: bool) -> String {
        let value = format!("json_extract(c.inline_json,'{path}')");
        if digest {
            format!(
                "json_type(c.inline_json,'{path}')='text' AND length({value})=64 AND {value} NOT GLOB '*[^0-9a-f]*'"
            )
        } else {
            format!("json_type(c.inline_json,'{path}')='text' AND length(trim({value}))>0")
        }
    }
    let mut terms=vec![
        "c.kind_version=1".to_owned(),"c.producer_id='session_api'".to_owned(),
        "c.correlation_id=c.session_id".to_owned(),"c.payload_id IS NULL".to_owned(),
        "json_extract(c.inline_json,'$.reason')='companion_archive'".to_owned(),
        "substr(c.idempotency_key,1,26)='companion-archive-context:'".to_owned(),
        uuid_v7("substr(c.idempotency_key,27)"),
        "substr(c.event_id,1,16)='context-cleared:'".to_owned(),uuid_v7("substr(c.event_id,17)"),
        "EXISTS(SELECT 1 FROM agent_events predecessor WHERE predecessor.session_id=c.session_id AND predecessor.event_id=c.causation_event_id AND predecessor.seq<c.seq AND predecessor.kind_version=1 AND predecessor.kind IN ('session/ready','turn/completed','turn/failed','turn/cancelled','context/cleared'))".to_owned(),
        object("$",&["reason","admission"]),
        object("$.admission",&["binding_ref","active_set_generation"]),
        integer("$.admission.active_set_generation",0),
        object("$.admission.binding_ref",&["preset_revision_ref","resolved_snapshot_ref","binding_version"]),
        integer("$.admission.binding_ref.binding_version",1),
        object("$.admission.binding_ref.preset_revision_ref",&["preset_id","revision","revision_digest"]),
        integer("$.admission.binding_ref.preset_revision_ref.revision",1),
        text("$.admission.binding_ref.preset_revision_ref.preset_id",false),
        text("$.admission.binding_ref.preset_revision_ref.revision_digest",true),
        object("$.admission.binding_ref.resolved_snapshot_ref",&["snapshot_id","snapshot_digest"]),
        text("$.admission.binding_ref.resolved_snapshot_ref.snapshot_id",false),
        text("$.admission.binding_ref.resolved_snapshot_ref.snapshot_digest",true),
    ];
    // Explicit type check distinguishes a string from a JSON numeric coercion.
    terms.push("json_type(c.inline_json,'$.reason')='text'".to_owned());
    terms.join(" AND ")
}

/// SQL retains at most the latest hard boundary. A long-lived Session remains
/// searchable after arbitrarily many valid automatic archives; only one bounded
/// payload is hydrated and formally decoded, with no second clear index/ledger.
pub(super) async fn explicit_history_floor(
    pool: &sqlx::SqlitePool,
    session: &AgentSessionId,
) -> Result<i64, AppError> {
    let query = format!(
        "SELECT c.seq,c.event_id,c.kind,c.kind_version,c.correlation_id,c.producer_id,c.idempotency_key,c.causation_event_id, \
        c.inline_json,c.payload_id,p.session_id AS payload_session_id, \
        CASE WHEN length(p.body)<=65536 THEN p.body END AS payload_body,p.digest AS payload_digest, \
        p.byte_len AS payload_byte_len \
        FROM agent_events c LEFT JOIN agent_payloads p ON p.payload_id=c.payload_id \
        WHERE c.session_id=? AND c.kind='context/cleared' AND NOT COALESCE(({}),0) ORDER BY c.seq DESC LIMIT 1",
        trusted_archive_clear_sql()
    );
    let candidate = sqlx::query_as::<_, SourceEvent>(&query)
        .bind(session.as_ref())
        .fetch_optional(pool)
        .await
        .map_err(failure)?;
    let Some(candidate) = candidate else {
        return Ok(0);
    };
    // Typed decoding still owns the readable clear contract. A malformed clear
    // fails closed rather than being silently treated as an archive or absence.
    let _: nomifun_agent_contracts::ContextClearedPayloadV1 =
        serde_json::from_value(candidate.value(session.as_ref())?).map_err(failure)?;
    Ok(candidate.seq)
}

pub(super) struct PublicAnswer {
    pub(super) turn_id: String,
    pub(super) event_ids: Vec<String>,
    pub(super) text: String,
}

pub(super) enum PublicAnswerRead {
    Answer(PublicAnswer),
    Unavailable,
    OutOfBudget,
}

/// A closed Turn is immutable through its exact terminal. Preflight byte and
/// event counts bound the official exact-Turn reader before it hydrates any
/// Payload. Nothing outside that closed interval can extend this allowance.
pub(super) async fn closed_turn_answer(
    pool: &sqlx::SqlitePool,
    store: &AgentSessionStore,
    session: &AgentSessionId,
    operation: &str,
    context_floor: i64,
) -> Result<PublicAnswerRead, AppError> {
    closed_turn_answer_at_floor(pool, store, session, operation, context_floor, true).await
}

pub(super) async fn closed_turn_answer_at_floor(
    pool: &sqlx::SqlitePool,
    store: &AgentSessionStore,
    session: &AgentSessionId,
    operation: &str,
    context_floor: i64,
    honor_all_clears: bool,
) -> Result<PublicAnswerRead, AppError> {
    let bounds: Option<(i64, i64)> = sqlx::query_as(
        "SELECT start.seq,terminal.seq FROM agent_turns t \
         JOIN agent_events start ON start.event_id=t.started_event_id \
         JOIN agent_events terminal ON terminal.event_id=t.terminal_event_id \
         JOIN agent_sessions s ON s.agent_session_id=t.session_id \
         WHERE t.session_id=? AND t.operation_id=? AND t.state='completed' AND s.state='live' \
           AND start.seq>? AND (?=0 OR start.seq>(SELECT COALESCE(MAX(c.seq),0) FROM agent_events c \
               WHERE c.session_id=t.session_id AND c.kind='context/cleared' AND c.kind_version=1)) \
           AND terminal.kind='turn/completed' AND terminal.kind_version=1",
    )
    .bind(session.as_ref())
    .bind(operation)
    .bind(context_floor)
    .bind(honor_all_clears)
    .fetch_optional(pool)
    .await
    .map_err(failure)?;
    let Some((start, end)) = bounds else {
        return Ok(PublicAnswerRead::Unavailable);
    };
    let (count, bytes): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),COALESCE(SUM(COALESCE(length(CAST(e.inline_json AS BLOB)),length(p.body),0)),0) \
         FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id \
         WHERE e.session_id=? AND e.seq>=? AND e.seq<=?")
        .bind(session.as_ref()).bind(start).bind(end).fetch_one(pool).await.map_err(failure)?;
    if count > MAX_PUBLIC_TURN_EVENTS || bytes > MAX_PUBLIC_TURN_BYTES {
        return Ok(PublicAnswerRead::OutOfBudget);
    }
    let Some(facts) = source_read(
        store
            .turn_output_facts(session, &operation.to_owned().into())
            .await,
        true,
    )?
    else {
        return Ok(PublicAnswerRead::Unavailable);
    };
    let Some(receipt) = source_read(
        store
            .read_turn_receipt(session, &operation.to_owned().into())
            .await,
        true,
    )?
    else {
        return Ok(PublicAnswerRead::Unavailable);
    };
    // Recheck the exact immutable interval, not a later successor Turn/head.
    if receipt.started_event.as_ref().map(|event| event.seq) != u64::try_from(start).ok()
        || receipt.terminal_event.as_ref().map(|event| event.seq) != u64::try_from(end).ok()
    {
        return Err(failure("closed public response boundary changed"));
    }
    let result =
        nomifun_agent_session::canonical_turn_public_result(&facts, &receipt).map_err(failure)?;
    let (Some(AgentTurnOutcome::Answered { message_ids }), Some(text)) =
        (result.turn_outcome, result.result_text)
    else {
        return Ok(PublicAnswerRead::Unavailable);
    };
    let mut event_ids = facts
        .events
        .iter()
        .filter(|event| {
            event.kind.0 == "message/completed"
                && message_ids
                    .iter()
                    .any(|id| event.correlation_id.as_ref() == id)
        })
        .map(|event| event.event_id.as_ref().to_owned())
        .collect::<Vec<_>>();
    if event_ids.is_empty() || event_ids.len() > 16 {
        return Ok(PublicAnswerRead::OutOfBudget);
    }
    event_ids.push(
        receipt
            .terminal_event
            .as_ref()
            .expect("checked terminal")
            .event_id
            .as_ref()
            .into(),
    );
    Ok(PublicAnswerRead::Answer(PublicAnswer {
        turn_id: operation.into(),
        event_ids,
        text,
    }))
}

#[derive(sqlx::FromRow)]
struct RecallTurn {
    session_id: String,
    operation_id: String,
    source_event_id: String,
    recorded_at: i64,
    source_seq: i64,
}

#[derive(Deserialize)]
struct DirectedPlanSource {
    mode: String,
    #[serde(default)]
    source: Option<nomifun_agent_execution::DirectedDelegationSource>,
}

/// Follow existing typed result/Execution dependencies only for privacy
/// filtering. This never creates a task relationship or grants target access.
/// Root and accepted steering are both inputs to the public response; losing
/// either source suppresses the derived prose from automatic/history recall.
pub(super) async fn turn_has_retracted_input(
    pool: &sqlx::SqlitePool,
    store: &AgentSessionStore,
    service: &nomifun_companion::CompanionService,
    principal: &PrincipalRef,
    companion_id: &str,
    session: &AgentSessionId,
    operation: &str,
) -> Result<bool, AppError> {
    let Some(inputs) = resolve_turn_inputs(
        pool,
        store,
        principal,
        companion_id,
        session,
        operation,
        None,
    )
    .await?
    else {
        return Ok(true);
    };
    for source in inputs.all_sources {
        if service
            .memory_message_is_retracted(companion_id, &source.session_id, &source.message_id)
            .await?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

struct ResolvedTurnInputs {
    all_sources: std::collections::BTreeSet<CompanionMemoryMessageSource>,
    human_sources: std::collections::BTreeSet<CompanionMemoryMessageSource>,
}

/// Reuse the privacy graph to attribute a write to existing human messages.
/// The caller proves its exact running Effect and supplies that Effect's
/// canonical causation sequence; no later steering may become a write source.
/// None means unproven/over-budget, never a guessed hidden user message.
pub(crate) async fn resolve_companion_turn_human_sources(
    pool: &sqlx::SqlitePool,
    store: &AgentSessionStore,
    principal: &PrincipalRef,
    companion_id: &str,
    session: &AgentSessionId,
    operation: &str,
    before_seq: u64,
) -> Result<Option<Vec<CompanionMemoryMessageSource>>, AppError> {
    if before_seq == 0 || before_seq > i64::MAX as u64 {
        return Err(failure("invalid memory source causation boundary"));
    }
    Ok(resolve_turn_inputs(
        pool,
        store,
        principal,
        companion_id,
        session,
        operation,
        Some(before_seq),
    )
    .await?
    .map(|sources| sources.human_sources.into_iter().collect()))
}

async fn resolve_turn_inputs(
    pool: &sqlx::SqlitePool,
    store: &AgentSessionStore,
    principal: &PrincipalRef,
    companion_id: &str,
    session: &AgentSessionId,
    operation: &str,
    before_seq: Option<u64>,
) -> Result<Option<ResolvedTurnInputs>, AppError> {
    let mut result = ResolvedTurnInputs {
        all_sources: Default::default(),
        human_sources: Default::default(),
    };
    let mut pending = vec![(session.clone(), operation.to_owned(), before_seq)];
    let mut visited = std::collections::BTreeSet::new();
    while let Some((session, operation, before_seq)) = pending.pop() {
        if !visited.insert((session.clone(), operation.clone(), before_seq)) {
            continue;
        }
        if visited.len() > 16 {
            return Ok(None);
        }
        let Some(scope) = read_product_origin(store, principal, &session, true).await? else {
            return Ok(None);
        };
        if scope.companion_id != companion_id {
            return Err(failure("result dependency belongs to another Companion"));
        }
        let root: Option<String> = sqlx::query_scalar(
            "SELECT source_message_id FROM agent_turns WHERE session_id=? AND operation_id=?",
        )
        .bind(session.as_ref())
        .bind(&operation)
        .fetch_optional(pool)
        .await
        .map_err(failure)?;
        let Some(root) = root else {
            return Ok(None);
        };
        let steers: Vec<String> = sqlx::query_scalar(
            "SELECT event_id FROM agent_events WHERE session_id=? AND correlation_id=? \
            AND kind='turn/steer-accepted' AND kind_version IN (1,2) AND seq<? ORDER BY seq LIMIT 18",
        )
        .bind(session.as_ref())
        .bind(&operation)
        .bind(before_seq.unwrap_or(i64::MAX as u64) as i64)
        .fetch_all(pool)
        .await
        .map_err(failure)?;
        if steers.len() > 16 {
            return Ok(None);
        }
        let Some(source) = read_accepted_source(pool, &session, &root).await? else {
            return Ok(None);
        };
        if before_seq.is_some_and(|limit| source.seq as u64 >= limit) {
            return Ok(None);
        }
        let source_value = source.value(session.as_ref())?;
        // This formal reader verifies root/start/operation identity. Preflight
        // above bounds its payload before it hydrates the canonical value.
        let (accepted_record, accepted_value) = store
            .turn_accepted_input(&session, &operation.clone().into())
            .await
            .map_err(failure)?;
        if accepted_record.event_id.as_ref() != root
            || accepted_record.seq != source.seq as u64
            || accepted_value.0 != source_value
        {
            return Err(failure("memory source root identity changed"));
        }
        let accepted: AcceptedInput = serde_json::from_value(accepted_value.0).map_err(failure)?;
        let reference = |message_id: String| CompanionMemoryMessageSource {
            session_id: session.as_ref().into(),
            message_id,
        };
        result.all_sources.insert(reference(root.clone()));
        if accepted.is_user_speech(&scope) {
            result.human_sources.insert(reference(root.clone()));
        }
        for steer in steers {
            let Some(event) = read_turn_input_source(pool, &session, &steer).await? else {
                return Ok(None);
            };
            if event.kind != "turn/steer-accepted" || event.correlation_id != operation {
                return Err(failure(
                    "memory steering source identity differs from its Turn",
                ));
            }
            let claim = event.value(session.as_ref())?;
            let original = if event.kind_version == 2 {
                let input_id=claim.get("input_event_id").and_then(Value::as_str)
                    .ok_or_else(||failure("steering claim has no original input reference"))?;
                let Some(source)=read_accepted_source(pool,&session,input_id).await? else {return Ok(None);};
                Some(source)
            } else {None};
            // The Store verifies the immutable claim and returns the original
            // input, never a mirrored body or projection-derived provenance.
            let (source, input)=store.steering_input_source(&session,&steer.clone().into()).await.map_err(failure)?;
            if let Some(original)=original.as_ref() {
                if original.event_id!=source.event_id.as_ref() || original.seq as u64!=source.seq
                    || original.value(session.as_ref())?!=input.0 {return Err(failure("steering source changed during bounded read"));}
            } else if source.event_id.as_ref()!=steer {return Err(failure("exact steering source changed"));}
            let input:AcceptedInput=serde_json::from_value(input.0).map_err(failure)?;
            let source_id=source.event_id.as_ref().to_owned();
            result.all_sources.insert(reference(source_id.clone()));
            if input.is_user_speech(&scope) {
                result.human_sources.insert(reference(source_id));
            }
        }
        if result.all_sources.len() > 64 {
            return Ok(None);
        }
        let mut related = false;
        if let Some(AgentInputSource::DirectedDelegationResult {
            source_turn_operation_id,
            ..
        }) = &accepted.input_source
        {
            pending.push((
                session.clone(),
                source_turn_operation_id.as_ref().into(),
                None,
            ));
            related = true;
        }
        // Directed Attempt creation and its canonical forks retain the exact
        // source in the existing Execution. Do not infer it from task text or a
        // selected Companion resource on an ordinary Session.
        let plans:Vec<String>=sqlx::query_scalar("WITH RECURSIVE ancestry(id,depth) AS (SELECT ?,0 UNION ALL \
            SELECT s.parent_agent_session_id,ancestry.depth+1 FROM agent_sessions s JOIN ancestry ON s.agent_session_id=ancestry.id \
            WHERE s.parent_agent_session_id IS NOT NULL AND ancestry.depth<64) \
            SELECT execution.initial_plan_input FROM ancestry \
            JOIN conversation_execution_links link ON link.conversation_id=ancestry.id AND link.relation='attempt' \
            JOIN agent_executions execution ON execution.execution_id=link.execution_id \
            WHERE execution.user_id=? AND execution.deleted_at IS NULL LIMIT 17")
            .bind(session.as_ref()).bind(&principal.principal_id).fetch_all(pool).await.map_err(failure)?;
        if plans.len() > 16 {
            return Ok(None);
        }
        for plan in plans {
            let plan: DirectedPlanSource = serde_json::from_str(&plan).map_err(failure)?;
            if plan.mode == "directed" {
                let source = plan
                    .source
                    .ok_or_else(|| failure("directed result dependency lost its source"))?;
                let parent = AgentSessionId::from(source.agent_session_id);
                let Some(effect) = store
                    .read_effect(&parent, &source.effect_id)
                    .await
                    .map_err(failure)?
                else {
                    return Ok(None);
                };
                if effect.agent_session_id != parent
                    || effect.turn_id.as_ref() != source.turn_operation_id
                    || effect.operation_id.as_ref() != source.tool_operation_id
                    || effect.capability_module.as_ref() != "agent.collaboration"
                    || effect.action_id.as_ref() != "agent/delegate"
                {
                    return Err(failure(
                        "Directed human source differs from its exact delegation Effect",
                    ));
                }
                let cause = store
                    .committed_tool_call_causation(
                        &parent,
                        &effect.turn_id,
                        &effect.operation_id,
                        &effect.capability_module,
                        &effect.action_id,
                    )
                    .await
                    .map_err(failure)?;
                if cause.seq == 0 || cause.seq > i64::MAX as u64 {
                    return Err(failure(
                        "Directed human source causation boundary is invalid",
                    ));
                }
                pending.push((parent, source.turn_operation_id, Some(cause.seq)));
                related = true;
            }
        }
        if !accepted.is_user_speech(&scope) && !related {
            return Ok(None);
        }
    }
    if result.human_sources.is_empty() {
        Ok(None)
    } else {
        Ok(Some(result))
    }
}

/// Bounded lexical recall, deliberately separate from the small automatically
/// injected recent window. The caller resolves the Companion from its frozen
/// resource; this reader independently verifies canonical product membership.
pub(crate) async fn recall_companion_history(
    pool: &sqlx::SqlitePool,
    store: &AgentSessionStore,
    service: &nomifun_companion::CompanionService,
    owner_id: &str,
    companion_id: &str,
    request: CompanionHistoryRecallQuery,
) -> Result<CompanionHistoryRecallPage, AppError> {
    if request.query.trim().is_empty()
        || request.query.chars().count() > 1024
        || request.limit == 0
        || request.limit > MAX_RECALL_LIMIT
        || request.byte_budget < 256
        || request.byte_budget > 32 * 1024
        || request
            .since_ts
            .zip(request.until_ts)
            .is_some_and(|(since, until)| since > until)
    {
        return Err(AppError::BadRequest(
            "Companion history query or bounded window is invalid".into(),
        ));
    }
    let principal = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: owner_id.into(),
    };
    if service.authoritative_user_id() != owner_id {
        return Err(AppError::Forbidden(
            "Companion history belongs to another dataset owner".into(),
        ));
    }
    service.get_companion(companion_id).await?;
    let owner_json = serde_json::to_string(&principal).map_err(failure)?;
    let legacy_key = format!("user:{owner_id}:open:companion-session:{companion_id}");
    let mut verified = std::collections::BTreeMap::new();
    if let Some(id) = request.source_session_id.as_ref() {
        let id = AgentSessionId::from(id.clone());
        let origin = product_origin(store, &principal, &id)
            .await?
            .filter(|scope| scope.companion_id == companion_id)
            .ok_or_else(|| {
                AppError::Forbidden("History source is not a live Session of this Companion".into())
            })?;
        verified.insert(id, origin);
    }
    if let Some(cursor) = request.before.as_ref() {
        if cursor.recorded_at < 0 || cursor.source_seq == 0 || cursor.source_seq > i64::MAX as u64 {
            return Err(AppError::BadRequest(
                "Companion history cursor is invalid".into(),
            ));
        }
        let id = AgentSessionId::from(cursor.session_id.clone());
        let origin = product_origin(store, &principal, &id)
            .await?
            .filter(|scope| scope.companion_id == companion_id)
            .ok_or_else(|| {
                AppError::Forbidden("History cursor belongs to another Companion".into())
            })?;
        let exact:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns t JOIN agent_events source ON source.event_id=t.source_message_id \
            WHERE t.session_id=? AND t.accepted_at=? AND source.seq=? AND source.kind='message/user-accepted' AND source.kind_version=1)")
            .bind(id.as_ref()).bind(cursor.recorded_at).bind(cursor.source_seq as i64).fetch_one(pool).await.map_err(failure)?;
        if !exact {
            return Err(AppError::BadRequest(
                "History cursor does not identify a canonical source".into(),
            ));
        }
        verified.insert(id, origin);
    }
    let mut turns = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT t.session_id,t.operation_id,t.source_message_id AS source_event_id, t.accepted_at AS recorded_at,source.seq AS source_seq \
         FROM agent_turns t JOIN agent_events source ON source.event_id=t.source_message_id \
         JOIN agent_sessions s ON s.agent_session_id=t.session_id \
         JOIN agent_events opening ON opening.session_id=t.session_id AND opening.seq=1 \
         LEFT JOIN agent_payloads sp ON sp.payload_id=source.payload_id \
         WHERE s.state='live' AND s.owner_ref_json=",
    );
    turns
        .push_bind(owner_json)
        .push(
            " AND opening.kind='session/opening' AND opening.kind_version=1 \
        AND (json_extract(opening.inline_json,'$.metadata.companion.companion_id')=",
        )
        .push_bind(companion_id)
        .push(" OR (opening.producer_id='session_api' AND opening.idempotency_key=")
        .push_bind(legacy_key)
        .push(") OR opening.payload_id IS NOT NULL)");
    if let Some(id) = request.source_session_id.as_ref() {
        turns.push(" AND t.session_id=").push_bind(id);
    }
    // The conservative complete host signature is shared with the exact floor
    // reader; unknown clears stay hard boundaries and cannot consume pages of
    // stale sources after a user clear.
    turns.push(" AND source.kind='message/user-accepted' AND source.kind_version=1 \
        AND source.seq>(SELECT COALESCE(MAX(c.seq),0) FROM agent_events c WHERE c.session_id=t.session_id AND c.kind='context/cleared' \
            AND NOT COALESCE((").push(trusted_archive_clear_sql()).push("),0))");
    if let Some(since) = request.since_ts {
        turns.push(" AND t.accepted_at>=").push_bind(since);
    }
    if let Some(until) = request.until_ts {
        turns.push(" AND t.accepted_at<=").push_bind(until);
    }
    if let Some(cursor) = request.before.as_ref() {
        turns
            .push(" AND (t.accepted_at<")
            .push_bind(cursor.recorded_at)
            .push(" OR (t.accepted_at=")
            .push_bind(cursor.recorded_at)
            .push(" AND t.session_id<")
            .push_bind(&cursor.session_id)
            .push(") OR (t.accepted_at=")
            .push_bind(cursor.recorded_at)
            .push(" AND t.session_id=")
            .push_bind(&cursor.session_id)
            .push(" AND source.seq<")
            .push_bind(cursor.source_seq as i64)
            .push("))");
    }
    let needles = lexical_needles(&request.query);
    // The input can be filtered lexically before the page bound. Completed
    // Turns remain candidates because a relevant final answer may be absent
    // from the user's wording; only the sealed canonical reducer can decide it.
    turns.push(" AND (t.state='completed' OR (");
    for (index, needle) in needles.iter().enumerate() {
        if index > 0 {
            turns.push(" OR ");
        }
        let pattern = format!(
            "%{}%",
            needle
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        turns.push("LOWER(json_extract(COALESCE(source.inline_json,json_extract(CAST(sp.body AS TEXT),'$.value')),'$.content')) LIKE ")
            .push_bind(pattern).push(" ESCAPE '\\'");
    }
    turns.push("))");
    turns
        .push(" ORDER BY t.accepted_at DESC,t.session_id DESC,source.seq DESC LIMIT ")
        .push_bind(MAX_RECALL_TURNS + 1);
    let rows = turns
        .build_query_as::<RecallTurn>()
        .fetch_all(pool)
        .await
        .map_err(failure)?;
    let mut page = CompanionHistoryRecallPage {
        fragments: Vec::new(),
        has_more: rows.len() > MAX_RECALL_TURNS as usize,
        next_cursor: None,
        truncated: false,
    };
    let mut last_scanned = None;
    let mut history_floors = std::collections::BTreeMap::new();
    for turn in rows.into_iter().take(MAX_RECALL_TURNS as usize) {
        let id = AgentSessionId::from(turn.session_id.clone());
        if !verified.contains_key(&id) {
            if verified.len() >= MAX_RECALL_SESSIONS as usize {
                page.has_more = true;
                break;
            }
            let Some(origin) = read_product_origin(store, &principal, &id, true).await? else {
                continue;
            };
            verified.insert(id.clone(), origin);
        }
        let origin = verified.get(&id).expect("verified source");
        let cursor = CompanionHistoryRecallCursor {
            recorded_at: turn.recorded_at,
            session_id: turn.session_id.clone(),
            source_seq: turn.source_seq as u64,
        };
        if origin.companion_id != companion_id {
            return Err(failure("history source affiliation changed"));
        }
        // Page continuation advances only past sources actually scanned. A
        // fragment that does not fit below is retried on the following page.
        let previous_cursor = last_scanned.clone();
        last_scanned = Some(cursor);
        page.next_cursor = last_scanned.clone();
        let source = read_accepted_source(pool, &id, &turn.source_event_id).await?;
        let Some(source) = source else {
            page.truncated = true;
            continue;
        };
        let payload = source.value(id.as_ref())?;
        let accepted: AcceptedInput = serde_json::from_value(payload).map_err(failure)?;
        let human_input = accepted.is_user_speech(origin);
        if service
            .memory_message_is_retracted(companion_id, id.as_ref(), &turn.source_event_id)
            .await?
        {
            continue;
        }
        let floor = if let Some(floor) = history_floors.get(&id) {
            *floor
        } else {
            let floor = explicit_history_floor(pool, &id).await?;
            history_floors.insert(id.clone(), floor);
            floor
        };
        if turn.source_seq <= floor {
            continue;
        }
        let derived_source_withdrawn = turn_has_retracted_input(
            pool,
            store,
            service,
            &principal,
            companion_id,
            &id,
            &turn.operation_id,
        )
        .await?;
        // Hidden task/report inputs are not human speech. Their sealed answer
        // may still be recalled when the existing typed Directed source graph
        // proves this Companion's original user source is current. Never expose
        // an unproven hidden host prompt as recalled user wording.
        if !human_input && derived_source_withdrawn {
            continue;
        }
        let answer = if derived_source_withdrawn {
            PublicAnswerRead::Unavailable
        } else {
            closed_turn_answer_at_floor(pool, store, &id, &turn.operation_id, floor, false).await?
        };
        let mut search_text = if human_input {
            accepted.request.content.clone()
        } else {
            String::new()
        };
        let mut text = if human_input {
            format!("Historical user input: {}", accepted.request.content)
        } else {
            "Verified-source directed task response (historical data):".into()
        };
        let mut event_ids = vec![turn.source_event_id];
        let mut answer_omitted = derived_source_withdrawn;
        match answer {
            PublicAnswerRead::Answer(answer) => {
                text.push_str("\nSealed assistant response (historical data): ");
                text.push_str(&answer.text);
                search_text.push_str(&answer.text);
                event_ids.extend(answer.event_ids);
            }
            PublicAnswerRead::OutOfBudget => {
                page.truncated = true;
                answer_omitted = true;
            }
            PublicAnswerRead::Unavailable if !human_input => {
                continue;
            }
            PublicAnswerRead::Unavailable => {}
        }
        let normalized = search_text.to_lowercase();
        if !needles.iter().any(|needle| normalized.contains(needle)) {
            continue;
        }
        // Source metadata itself remains intact. Only data text is clipped,
        // and the returned fragment explicitly tells callers when it was.
        let (excerpt, truncated) = text_excerpt(&text, request.byte_budget.min(8 * 1024));
        let fragment = CompanionHistoryFragment {
            session_id: turn.session_id,
            turn_id: turn.operation_id,
            event_ids,
            recorded_at: turn.recorded_at,
            text: excerpt.into(),
            truncated: truncated || answer_omitted,
        };
        page.truncated |= fragment.truncated;
        if page.fragments.len() >= request.limit as usize {
            page.has_more = true;
            last_scanned = previous_cursor;
            break;
        }
        let current_cursor = last_scanned.clone();
        page.fragments.push(fragment);
        if serde_json::to_vec(&page).map_err(failure)?.len() > request.byte_budget {
            let mut fragment = page.fragments.pop().expect("just inserted");
            // Default to retrying this unreturned source on the next page.
            // Advance only after its clipped fragment really fits below.
            last_scanned = previous_cursor.clone();
            let overhead = serde_json::to_vec(&page).map_err(failure)?.len() + 512;
            let available = request.byte_budget.saturating_sub(overhead);
            if available > 64 {
                fragment.text = text_excerpt(&fragment.text, available / 6).0.into();
                fragment.truncated = true;
                page.truncated = true;
                page.fragments.push(fragment);
                if serde_json::to_vec(&page).map_err(failure)?.len() > request.byte_budget {
                    page.fragments.pop();
                } else {
                    last_scanned = current_cursor;
                }
            }
            if page.fragments.is_empty() {
                last_scanned = previous_cursor;
            }
            page.has_more = true;
            break;
        }
    }
    page.next_cursor = if page.has_more { last_scanned } else { None };
    if page.has_more && page.next_cursor.is_none() {
        return Err(AppError::BadRequest("History response budget cannot fit one source with its continuation; increase byte_budget".into()));
    }
    if serde_json::to_vec(&page).map_err(failure)?.len() > request.byte_budget {
        return Err(failure(
            "history response including continuation exceeds its byte budget",
        ));
    }
    Ok(page)
}

fn lexical_needles(query: &str) -> Vec<String> {
    let lowered = query.trim().to_lowercase();
    let mut needles = lowered
        .split(|ch: char| ch.is_whitespace() || ch.is_ascii_punctuation())
        .filter(|piece| !piece.is_empty())
        .take(8)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if needles.is_empty() {
        needles.push(lowered);
    }
    needles
}

async fn read_accepted_source(
    pool: &sqlx::SqlitePool,
    session: &AgentSessionId,
    event_id: &str,
) -> Result<Option<SourceEvent>, AppError> {
    let value = read_turn_input_source(pool, session, event_id).await?;
    if value
        .as_ref()
        .is_some_and(|value| value.kind != "message/user-accepted")
    {
        return Err(failure("accepted source has another canonical kind"));
    }
    Ok(value)
}

async fn read_turn_input_source(
    pool: &sqlx::SqlitePool,
    session: &AgentSessionId,
    event_id: &str,
) -> Result<Option<SourceEvent>, AppError> {
    sqlx::query_as::<_,SourceEvent>(
        "SELECT e.seq,e.event_id,e.kind,e.kind_version,e.correlation_id,e.producer_id,e.idempotency_key,e.causation_event_id, \
         e.inline_json,e.payload_id,p.session_id AS payload_session_id,p.body AS payload_body,p.digest AS payload_digest, \
         p.byte_len AS payload_byte_len \
         FROM agent_events e JOIN agent_sessions s ON s.agent_session_id=e.session_id \
         LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id WHERE s.state='live' AND e.session_id=? AND e.event_id=? \
         AND ((e.kind='message/user-accepted' AND e.kind_version=1) OR (e.kind='turn/steer-accepted' AND e.kind_version IN (1,2))) \
         AND (e.inline_json IS NULL OR length(CAST(e.inline_json AS BLOB))<=?) AND (p.body IS NULL OR length(p.body)<=?)")
        .bind(session.as_ref()).bind(event_id).bind(MAX_SOURCE_PAYLOAD_BYTES).bind(MAX_SOURCE_PAYLOAD_BYTES)
        .fetch_optional(pool).await.map_err(failure)
}
