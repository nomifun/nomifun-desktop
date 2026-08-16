-- Collapse the engine set to the native `nomi` executor.
--
-- ACP, OpenClaw Gateway, Nanobot and Remote are gone as product surfaces:
-- `AgentType` is now `Nomi` alone, and every runtime adapter, protocol module
-- and command that served the other four was removed in the same change. The
-- database must therefore stop carrying rows that no surviving code can
-- execute, read or render.
--
-- This is a DESTRUCTIVE data migration, not a preservation one. A non-nomi
-- Conversation names an engine that no longer exists in the binary, so it can
-- never be resumed, replayed or displayed again — keeping its transcript would
-- leave the user a permanently dead thread whose only remaining behaviour is to
-- fail. The rows go, and the release note says so.
--
-- ORDER — children before parents, and every child is deleted EXPLICITLY. The
-- baseline states the rule this file obeys: "SQLite does not own relation
-- deletion behavior" (001_v3_baseline.sql:12). There are no physical foreign
-- keys anywhere in the lineage (`validate_no_physical_foreign_keys` enforces
-- that at every boot), so nothing cascades on its own. Every table carrying a
-- conversation-scoped column is handled below or explained as deliberately
-- untouched.
--
-- The engine CHECK constraints on `conversations.type` and
-- `channel_sessions.agent_type` are deliberately LEFT AS A SUPERSET. SQLite
-- cannot ALTER a CHECK, and rebuilding those tables would silently destroy the
-- five registered `trg_conversations_running_*` guard triggers plus sixteen
-- indexes, which `validate_id_schema_contract` then rejects by name. Nothing
-- reads the enum text — the single-variant Rust `AgentType` is the real gate,
-- since serde refuses any other string on read.

-- ---------------------------------------------------------------------------
-- 0. Doomed conversations, resolved once into a temporary set.
--
--    Materialized rather than repeated as a correlated subquery because it is
--    read many times below, and because step 8 deletes the `conversations` rows
--    themselves — after which the predicate would no longer resolve.
--
--    Running rows are settled first: trg_conversations_running_delete_guard
--    (migration 008) aborts a DELETE of a Running Conversation, and reaching it
--    would abort the whole migration. Settling here is safe precisely because
--    the engine that owned that turn is gone from this build, so no finalizer
--    can ever arrive to claim it. The status write goes through the same
--    guarded path the runtime uses: trg_conversations_running_exit_guard
--    requires 'finished', a cleared owner and the next epoch, and additionally
--    that no accepted turn receipt remains — hence the receipt settlement
--    immediately before it.
-- ---------------------------------------------------------------------------
CREATE TEMPORARY TABLE doomed_conversations AS
SELECT conversation_id, user_id, status, active_turn_operation_id
FROM conversations
WHERE type <> 'nomi';

-- Terminal-outcome receipts for a doomed turn. `conversation_delivery_receipts`
-- is append-only and delete-guarded, and its lifecycle guard makes a completed
-- outcome immutable, so accepted rows are completed here exactly once with a
-- truthful failure code rather than removed.
UPDATE conversation_delivery_receipts
SET status = 'completed',
    result_ok = 0,
    result_text = '',
    result_error = 'The engine that owned this turn was removed from the product.',
    result_error_code = 'engine_removed',
    result_error_retryable = 0,
    completed_at = MAX(created_at, unixepoch('now','subsec')*1000)
WHERE status = 'accepted'
  AND kind = 'turn'
  AND conversation_id IN (SELECT conversation_id FROM doomed_conversations);

-- Retire the Running generation through the migration-008 exit guard.
UPDATE conversations
SET status = 'finished',
    active_turn_operation_id = NULL,
    admission_epoch = admission_epoch + 1,
    updated_at = MAX(updated_at, unixepoch('now','subsec')*1000)
WHERE status = 'running'
  AND conversation_id IN (SELECT conversation_id FROM doomed_conversations);

-- ---------------------------------------------------------------------------
-- 1. Restrict-policy blockers, cleared before their parent row disappears or
--    the boot orphan audit reports them.
--
--    `requirement_pre_effect_abandon_guards.owner_conversation_id` is Restrict
--    and the guard row is trigger-protected while its requirement is still
--    claimed. A doomed guard can never be consumed normally any more, because
--    the runtime that would settle it is gone.
-- ---------------------------------------------------------------------------
DELETE FROM requirement_pre_effect_abandon_guards
WHERE owner_conversation_id IN (SELECT conversation_id FROM doomed_conversations);

-- ---------------------------------------------------------------------------
-- 2. Grandchildren of the doomed conversations' cron jobs. `cron_jobs`
--    .conversation_id is Cascade, so the job dies with its conversation and its
--    own children must go first.
-- ---------------------------------------------------------------------------
UPDATE conversations
SET cron_job_id = NULL
WHERE cron_job_id IN (
    SELECT cron_job_id FROM cron_jobs
    WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations)
);

UPDATE conversation_artifacts
SET cron_job_id = NULL
WHERE cron_job_id IN (
    SELECT cron_job_id FROM cron_jobs
    WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations)
);

DELETE FROM cron_job_runs
WHERE cron_job_id IN (
    SELECT cron_job_id FROM cron_jobs
    WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations)
);

DELETE FROM cron_run_reservations
WHERE cron_job_id IN (
    SELECT cron_job_id FROM cron_jobs
    WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations)
);

-- ---------------------------------------------------------------------------
-- 3. Every cron job that names a deleted engine, whichever way it names it.
--
--    Two independent reasons a job is doomed: it targets a doomed conversation
--    (Cascade), or its own `agent_type` is a deleted engine. That column has no
--    CHECK — it is the job's OWN executor selection — so it must be filtered by
--    value. The conversations-side pass above already detached the surviving
--    references.
-- ---------------------------------------------------------------------------
UPDATE conversations
SET cron_job_id = NULL
WHERE cron_job_id IN (SELECT cron_job_id FROM cron_jobs WHERE agent_type <> 'nomi');

UPDATE conversation_artifacts
SET cron_job_id = NULL
WHERE cron_job_id IN (SELECT cron_job_id FROM cron_jobs WHERE agent_type <> 'nomi');

DELETE FROM cron_job_runs
WHERE cron_job_id IN (SELECT cron_job_id FROM cron_jobs WHERE agent_type <> 'nomi');

DELETE FROM cron_run_reservations
WHERE cron_job_id IN (SELECT cron_job_id FROM cron_jobs WHERE agent_type <> 'nomi');

DELETE FROM cron_jobs
WHERE agent_type <> 'nomi'
   OR conversation_id IN (SELECT conversation_id FROM doomed_conversations);

-- ---------------------------------------------------------------------------
-- 4. Direct children of the doomed conversations. Every column below is a
--    conversation-scoped reference registered Cascade.
-- ---------------------------------------------------------------------------

-- The engine-specific session store. Dropped wholesale in step 13; rows are
-- deleted first so the DROP cannot be mistaken for the only cleanup.
DELETE FROM acp_session
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

DELETE FROM knowledge_binding_bases
WHERE knowledge_binding_id IN (
    SELECT knowledge_binding_id FROM knowledge_bindings
    WHERE target_kind = 'conversation'
      AND target_conversation_id IN (SELECT conversation_id FROM doomed_conversations)
);

DELETE FROM knowledge_bindings
WHERE target_kind = 'conversation'
  AND target_conversation_id IN (SELECT conversation_id FROM doomed_conversations);

DELETE FROM message_correlations
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

DELETE FROM conversation_mcp_servers
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

DELETE FROM conversation_artifacts
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

DELETE FROM conversation_creation_keys
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

DELETE FROM idmm_action_reservations
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

DELETE FROM idmm_interventions
WHERE target_kind = 'conversation'
  AND target_id IN (SELECT conversation_id FROM doomed_conversations);

DELETE FROM conversation_execution_links
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

DELETE FROM channel_pending_prompts
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

DELETE FROM conversation_delivery_notify
WHERE requester_conversation_id IN (SELECT conversation_id FROM doomed_conversations);

-- ---------------------------------------------------------------------------
-- 5. Message-shaped references, detached before the messages disappear.
-- ---------------------------------------------------------------------------
UPDATE channel_inbound_receipts
SET message_id = NULL
WHERE message_id IN (
    SELECT message_id FROM messages
    WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations)
);

UPDATE conversation_delivery_receipts
SET projected_message_id = NULL,
    projected_conversation_id = NULL
WHERE projected_conversation_id IN (SELECT conversation_id FROM doomed_conversations);

DELETE FROM messages
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

-- ---------------------------------------------------------------------------
-- 6. Engine-specific transcript rows inside SURVIVING nomi conversations.
--
--    `acp_tool_call` was only ever emitted by the ACP translator; the native
--    engine emits `tool_call`. No surviving renderer can draw such a bubble,
--    and `MessageType` no longer has a variant that deserializes it — a leftover
--    row would fail the whole page read. A nomi conversation can hold one from a
--    pre-collapse Agent Execution attempt routed to an ACP participant.
--    `messages.type` has no CHECK, so the value is filtered, not constrained.
-- ---------------------------------------------------------------------------
UPDATE channel_inbound_receipts
SET message_id = NULL
WHERE message_id IN (SELECT message_id FROM messages WHERE type = 'acp_tool_call');

UPDATE conversation_delivery_receipts
SET projected_message_id = NULL
WHERE projected_message_id IN (SELECT message_id FROM messages WHERE type = 'acp_tool_call');

DELETE FROM message_correlations
WHERE message_type = 'acp_tool_call';

DELETE FROM messages WHERE type = 'acp_tool_call';

-- ---------------------------------------------------------------------------
-- 7. SetNull references, and the two owners that outlive their conversation.
-- ---------------------------------------------------------------------------
UPDATE channel_sessions
SET conversation_id = NULL
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

UPDATE channel_inbound_receipts
SET conversation_id = NULL
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

UPDATE cron_run_reservations
SET conversation_id = NULL
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

-- A mini-app is a finished artifact that keeps running from its stored HTML.
-- Forget the provenance link, keep the app — the same posture the repository's
-- explicit conversation-delete path takes.
UPDATE miniapps
SET source_conversation_id = NULL
WHERE source_conversation_id IN (SELECT conversation_id FROM doomed_conversations);

-- Requirements owned by a doomed conversation can never be resumed.
--
-- SHAPE IS FORCED BY A TRIGGER. `trg_requirements_active_identity_exit_guard`
-- (migration 009) aborts any UPDATE that changes claim_generation, claim_token,
-- owner_conversation_id, owner_terminal_id, active_turn_started_at, started_at
-- or attempt_count while OLD.status = 'in_progress' and NEW.status is not
-- 'pending'. So this CANNOT settle to needs_review and clear the owner in one
-- statement — the CASE-per-column form below is the only legal shape, and it is
-- exactly the one `SqliteConversationRepository::delete_with_cleanup` already
-- uses. `lease_expires_at` is deliberately the only authority column cleared
-- for an active row: it is the one column the exit guard does not compare.
--
-- Active/needs_review rows therefore KEEP their now-dangling
-- owner_conversation_id as immutable execution-history evidence. That is not an
-- orphan by the contract's own definition: the registered child predicate for
-- this reference exempts exactly `status IN ('in_progress', 'needs_review')`.
UPDATE requirements
SET status = CASE WHEN status = 'in_progress' THEN 'needs_review' ELSE status END,
    completion_note = CASE
        WHEN status = 'in_progress'
        THEN COALESCE(completion_note,
                      'The engine that was executing this requirement was removed from the product; the outcome is unknown and it was not restarted.')
        ELSE completion_note END,
    lease_expires_at = CASE WHEN status = 'in_progress' THEN NULL ELSE lease_expires_at END,
    owner_conversation_id = CASE
        WHEN status IN ('in_progress', 'needs_review') THEN owner_conversation_id
        ELSE NULL END,
    updated_at = MAX(updated_at, unixepoch('now','subsec')*1000)
WHERE owner_conversation_id IN (SELECT conversation_id FROM doomed_conversations);

-- ---------------------------------------------------------------------------
-- 8. The conversations themselves.
--
--    `agent_execution_events` is deliberately untouched: both its
--    conversation-shaped columns are registered KeepHistory /
--    AllowMissingHistoricalParent, and the table has an append-only posture —
--    it is historical actor evidence, not a live reference.
--    `channel_inbound_receipts.conversation_scope_id` and
--    `conversation_delivery_receipts.conversation_id` are likewise untouched:
--    they are immutable idempotency-scope tokens, not parent references. The
--    nullable `projected_*` columns are the real references and were detached
--    in step 5.
-- ---------------------------------------------------------------------------
DELETE FROM conversations
WHERE conversation_id IN (SELECT conversation_id FROM doomed_conversations);

DROP TABLE doomed_conversations;

-- ---------------------------------------------------------------------------
-- 9. Channel sessions whose own agent_type is a deleted engine. A session's
--    agent_type is its OWN routing choice, independent of whether it currently
--    points at a conversation, and the Rust reader now rejects any value but
--    'nomi' rather than coercing it.
-- ---------------------------------------------------------------------------
DELETE FROM channel_pending_prompts
WHERE channel_session_id IN (
    SELECT channel_session_id FROM channel_sessions WHERE agent_type <> 'nomi'
);

DELETE FROM channel_session_bindings
WHERE channel_session_id IN (
    SELECT channel_session_id FROM channel_sessions WHERE agent_type <> 'nomi'
);

DELETE FROM channel_sessions WHERE agent_type <> 'nomi';

-- ---------------------------------------------------------------------------
-- 10. The agent catalog. Every non-nomi row named a deleted engine.
--
--     Two references must be cleared first or the boot audit reports orphans:
--     `preset_agent_preferences.agent_id` (Restrict) and
--     `preset_user_state.preferred_agent_id` (SetNull).
--
--     `agent_execution_participants.source_agent_id` and
--     `agent_execution_template_participants.source_agent_id` are both NOT NULL
--     and cannot be walked back. The participant column is KeepHistory (a
--     missing parent is legal), so historical execution rows are left alone.
--     The TEMPLATE column is Restrict, so a template still naming a deleted
--     engine is deleted whole — a multi-seat template minus a required seat is
--     not runnable, and `agent_execution_templates.primary_participant_id` is
--     itself NOT NULL + Restrict scoped to the same template, so a partial
--     delete could strand the template's own primary.
-- ---------------------------------------------------------------------------
DELETE FROM preset_agent_preferences
WHERE agent_id IN (SELECT agent_id FROM agent_metadata WHERE agent_type <> 'nomi');

UPDATE preset_user_state
SET preferred_agent_id = NULL
WHERE preferred_agent_id IN (SELECT agent_id FROM agent_metadata WHERE agent_type <> 'nomi');

-- The doomed set is materialized FIRST, because the third statement below
-- deletes the very participant rows that define it.
CREATE TEMPORARY TABLE doomed_templates AS
SELECT DISTINCT template_id
FROM agent_execution_template_participants
WHERE source_agent_id NOT IN (SELECT agent_id FROM agent_metadata WHERE agent_type = 'nomi');

UPDATE conversations
SET execution_template_id = NULL
WHERE execution_template_id IN (SELECT template_id FROM doomed_templates);

DELETE FROM agent_execution_template_participants
WHERE template_id IN (SELECT template_id FROM doomed_templates);

-- Scoped to the doomed set on purpose. A blanket "template with no
-- participants" sweep would also delete a pre-existing zero-seat template that
-- has nothing to do with this collapse.
DELETE FROM agent_execution_templates
WHERE execution_template_id IN (SELECT template_id FROM doomed_templates);

DROP TABLE doomed_templates;

DELETE FROM agent_metadata WHERE agent_type <> 'nomi';

-- ---------------------------------------------------------------------------
-- 11. Engine identity keys left inside SURVIVING nomi conversations' `extra`.
--
--     `extra.$.agent_id` and `extra.$.custom_agent_id` are Restrict +
--     RequireParent JSON references to `agent_metadata.agent_id`. A nomi row can
--     hold one: the preset resolver writes `extra.agent_id` from whatever agent
--     the preset resolved and sets `type` to follow it, so a later preset
--     re-resolution can flip `type` to nomi while the stale key persists.
--     Deleting the agent row without stripping the key makes
--     `validate_id_data_contract` fail the NEXT BOOT with a RequireParent
--     orphan — the app would not start.
--
--     `backend` and `agent_source` are descriptive strings with no registry
--     entry; they go with `agent_id` because they described the deleted engine
--     and nothing reads them for a nomi row.
-- ---------------------------------------------------------------------------
UPDATE conversations
SET extra = json_remove(extra, '$.agent_id', '$.backend', '$.agent_source')
WHERE json_extract(extra, '$.agent_id') IS NOT NULL
  AND json_extract(extra, '$.agent_id') NOT IN (SELECT agent_id FROM agent_metadata);

UPDATE conversations
SET extra = json_remove(extra, '$.custom_agent_id')
WHERE json_extract(extra, '$.custom_agent_id') IS NOT NULL
  AND json_extract(extra, '$.custom_agent_id') NOT IN (SELECT agent_id FROM agent_metadata);

-- ---------------------------------------------------------------------------
-- 12. Remote agents. The whole table was the Remote engine's store.
--
--     `conversations.extra.$.remote_agent_id` was a Restrict + RequireParent
--     JSON reference to it. Every conversation that could legitimately hold one
--     was type 'remote' and is already gone, but the key is stripped from any
--     surviving row for the same next-boot reason as step 11.
-- ---------------------------------------------------------------------------
UPDATE conversations
SET extra = json_remove(extra, '$.remote_agent_id')
WHERE json_extract(extra, '$.remote_agent_id') IS NOT NULL;

DELETE FROM remote_agents;

-- ---------------------------------------------------------------------------
-- 13. Drop the two engine-owned tables and their indexes.
--
--     Indexes are dropped explicitly, matching 015_drop_model_profiles.sql.
--     `idx_conversations_extra_remote_agent_id` is an expression index on a
--     surviving table whose only registered purpose was the JSON reference
--     removed in step 12.
-- ---------------------------------------------------------------------------
DROP INDEX idx_acp_session_conversation_id;
DROP INDEX idx_acp_session_agent_id;
DROP TABLE acp_session;

DROP INDEX idx_remote_agents_status;
DROP TABLE remote_agents;

DROP INDEX idx_conversations_extra_remote_agent_id;
