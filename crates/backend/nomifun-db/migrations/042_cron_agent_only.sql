-- Migration 042: scheduled work has one execution target -- an Agent.
--
-- Terminal scheduling was removed from the runtime before this migration, but
-- its discriminator and five terminal-only columns remained in storage and on
-- the API. Those rows could no longer be listed, executed, or removed through
-- the service. This is a hard schema cut, not a compatibility layer: retire the
-- already-inert terminal rows and rebuild the aggregate in its canonical form.
--
-- `run_migrations` disables FK enforcement and enables legacy_alter_table
-- outside the sqlx transaction. References to retired rows are detached
-- explicitly below; references to retained Agent rows continue to target the
-- table after the standard SQLite rebuild.

CREATE TABLE cron_agent_only_migration_guard (
    ok INTEGER NOT NULL CHECK (ok = 1)
);

-- Refuse unknown historical discriminators instead of silently deleting data
-- whose meaning this migration cannot prove.
INSERT INTO cron_agent_only_migration_guard(ok)
SELECT CASE WHEN target_kind IN ('agent', 'terminal') THEN 1 ELSE 0 END
FROM cron_jobs;

-- Terminal schedules were already non-executable. Detach their audit/UI
-- references before retiring the rows, matching the former FK actions even
-- though foreign-key enforcement is deliberately disabled during migrations.
UPDATE conversations
SET cron_job_id = NULL
WHERE cron_job_id IN (
    SELECT id FROM cron_jobs WHERE target_kind = 'terminal'
);

UPDATE conversation_artifacts
SET cron_job_id = NULL
WHERE cron_job_id IN (
    SELECT id FROM cron_jobs WHERE target_kind = 'terminal'
);

DELETE FROM cron_job_runs
WHERE job_id IN (
    SELECT id FROM cron_jobs WHERE target_kind = 'terminal'
);

CREATE TABLE cron_jobs_agent_only (
    id                   TEXT    PRIMARY KEY NOT NULL,
    user_id              TEXT    NOT NULL CHECK(length(trim(user_id)) > 0),
    name                 TEXT    NOT NULL,
    enabled              INTEGER NOT NULL DEFAULT 1,
    schedule_kind        TEXT    NOT NULL CHECK(schedule_kind IN ('at', 'every', 'cron')),
    schedule_value       TEXT    NOT NULL,
    schedule_tz          TEXT,
    schedule_description TEXT,
    payload_message      TEXT    NOT NULL,
    execution_mode       TEXT    NOT NULL DEFAULT 'existing'
                                 CHECK(execution_mode IN ('existing', 'new_conversation')),
    agent_config         TEXT,
    preset_id            TEXT,
    preset_revision      INTEGER,
    preset_snapshot      TEXT,
    conversation_id      INTEGER,
    conversation_title   TEXT,
    agent_type           TEXT    NOT NULL,
    created_by           TEXT    NOT NULL CHECK(created_by IN ('user', 'agent')),
    skill_content        TEXT,
    description          TEXT,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    next_run_at          INTEGER,
    last_run_at          INTEGER,
    last_status          TEXT    CHECK(last_status IN ('ok', 'error', 'skipped', 'missed')),
    last_error           TEXT,
    run_count            INTEGER NOT NULL DEFAULT 0,
    retry_count          INTEGER NOT NULL DEFAULT 0,
    max_retries          INTEGER NOT NULL DEFAULT 3,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE SET NULL
);

INSERT INTO cron_jobs_agent_only (
    id, user_id, name, enabled, schedule_kind, schedule_value, schedule_tz,
    schedule_description, payload_message, execution_mode, agent_config,
    preset_id, preset_revision, preset_snapshot, conversation_id,
    conversation_title, agent_type, created_by, skill_content, description,
    created_at, updated_at, next_run_at, last_run_at, last_status, last_error,
    run_count, retry_count, max_retries
)
SELECT
    id, user_id, name, enabled, schedule_kind, schedule_value, schedule_tz,
    schedule_description, payload_message, execution_mode, agent_config,
    preset_id, preset_revision, preset_snapshot, conversation_id,
    conversation_title, agent_type, created_by, skill_content, description,
    created_at, updated_at, next_run_at, last_run_at, last_status, last_error,
    run_count, retry_count, max_retries
FROM cron_jobs
WHERE target_kind = 'agent';

DROP TABLE cron_jobs;
ALTER TABLE cron_jobs_agent_only RENAME TO cron_jobs;
DROP TABLE cron_agent_only_migration_guard;

CREATE INDEX idx_cron_jobs_user
    ON cron_jobs(user_id, created_at);
CREATE INDEX idx_cron_jobs_conversation
    ON cron_jobs(conversation_id);
CREATE INDEX idx_cron_jobs_user_conversation
    ON cron_jobs(user_id, conversation_id, created_at);
CREATE INDEX idx_cron_jobs_next_run
    ON cron_jobs(next_run_at) WHERE enabled = 1;
CREATE INDEX idx_cron_jobs_agent_type
    ON cron_jobs(agent_type);
CREATE INDEX idx_cron_jobs_preset_id
    ON cron_jobs(preset_id);

-- Ownership is immutable. Moving a job means creating a new aggregate.
CREATE TRIGGER cron_job_owner_immutable
BEFORE UPDATE OF user_id ON cron_jobs
WHEN NEW.user_id IS NOT OLD.user_id
BEGIN
    SELECT RAISE(ABORT, 'cron job owner is immutable');
END;

-- Both halves of the optional Conversation binding must remain same-owner.
CREATE TRIGGER cron_job_conversation_owner_insert
BEFORE INSERT ON cron_jobs
WHEN NEW.conversation_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM conversations conversation
     WHERE conversation.id = NEW.conversation_id
       AND conversation.user_id = NEW.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'cron job conversation owner mismatch');
END;

CREATE TRIGGER cron_job_conversation_owner_update
BEFORE UPDATE OF conversation_id ON cron_jobs
WHEN NEW.conversation_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM conversations conversation
     WHERE conversation.id = NEW.conversation_id
       AND conversation.user_id = NEW.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'cron job conversation owner mismatch');
END;

-- Non-owner scheduled work has the same model-only ceiling as an interactive
-- Nomi Conversation. The allowlist is deliberately structural and rejects
-- unknown JSON keys so host capabilities cannot be smuggled in later.
CREATE TRIGGER cron_execution_authority_insert_guard
BEFORE INSERT ON cron_jobs
WHEN NEW.user_id <> 'system_default_user'
 AND (
     NEW.agent_type <> 'nomi'
     OR NEW.preset_id IS NOT NULL
     OR NEW.preset_revision IS NOT NULL
     OR NEW.preset_snapshot IS NOT NULL
     OR NEW.skill_content IS NOT NULL
     OR (
         NEW.enabled = 1
         AND (
             (
                 (NEW.execution_mode <> 'existing' OR NEW.conversation_id IS NULL)
                 AND NEW.agent_config IS NULL
             )
             OR (
                 NEW.execution_mode = 'existing'
                 AND NEW.conversation_id IS NOT NULL
                 AND NOT EXISTS (
                     SELECT 1
                     FROM conversations conversation
                     WHERE conversation.id = NEW.conversation_id
                       AND conversation.user_id = NEW.user_id
                       AND conversation.type = 'nomi'
                       AND conversation.model IS NOT NULL
                       AND json_valid(conversation.model)
                       AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END) = 'object'
                       AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.provider_id') = 'text'
                       AND trim(json_extract(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.provider_id')) <> ''
                       AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.model') = 'text'
                       AND trim(json_extract(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.model')) <> ''
                 )
             )
         )
     )
     OR CASE
         WHEN NEW.agent_config IS NULL THEN 0
         WHEN NOT json_valid(NEW.agent_config) THEN 1
         WHEN json_type(NEW.agent_config) <> 'object' THEN 1
         WHEN json_type(NEW.agent_config, '$.backend') IS NOT 'text'
           OR trim(json_extract(NEW.agent_config, '$.backend')) = '' THEN 1
         WHEN json_type(NEW.agent_config, '$.name') IS NOT 'text' THEN 1
         WHEN json_type(NEW.agent_config, '$.model_id') IS NOT NULL
          AND json_type(NEW.agent_config, '$.model_id') NOT IN ('text', 'null') THEN 1
         WHEN json_type(NEW.agent_config, '$.clear_context_each_run') IS NOT NULL
          AND json_type(NEW.agent_config, '$.clear_context_each_run') NOT IN ('true', 'false') THEN 1
         WHEN EXISTS (
             SELECT 1 FROM json_each(NEW.agent_config)
             WHERE key NOT IN ('backend', 'name', 'model_id', 'clear_context_each_run')
         ) THEN 1
         ELSE 0
     END
 )
BEGIN
    SELECT RAISE(ABORT, 'non-owner cron job must be model-only');
END;

CREATE TRIGGER cron_execution_authority_update_guard
BEFORE UPDATE OF user_id, enabled, execution_mode, conversation_id, agent_type,
                 agent_config, preset_id, preset_revision, preset_snapshot,
                 skill_content
ON cron_jobs
WHEN NEW.user_id <> 'system_default_user'
 AND (
     NEW.agent_type <> 'nomi'
     OR NEW.preset_id IS NOT NULL
     OR NEW.preset_revision IS NOT NULL
     OR NEW.preset_snapshot IS NOT NULL
     OR NEW.skill_content IS NOT NULL
     OR (
         NEW.enabled = 1
         AND (
             (
                 (NEW.execution_mode <> 'existing' OR NEW.conversation_id IS NULL)
                 AND NEW.agent_config IS NULL
             )
             OR (
                 NEW.execution_mode = 'existing'
                 AND NEW.conversation_id IS NOT NULL
                 AND NOT EXISTS (
                     SELECT 1
                     FROM conversations conversation
                     WHERE conversation.id = NEW.conversation_id
                       AND conversation.user_id = NEW.user_id
                       AND conversation.type = 'nomi'
                       AND conversation.model IS NOT NULL
                       AND json_valid(conversation.model)
                       AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END) = 'object'
                       AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.provider_id') = 'text'
                       AND trim(json_extract(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.provider_id')) <> ''
                       AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.model') = 'text'
                       AND trim(json_extract(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.model')) <> ''
                 )
             )
         )
     )
     OR CASE
         WHEN NEW.agent_config IS NULL THEN 0
         WHEN NOT json_valid(NEW.agent_config) THEN 1
         WHEN json_type(NEW.agent_config) <> 'object' THEN 1
         WHEN json_type(NEW.agent_config, '$.backend') IS NOT 'text'
           OR trim(json_extract(NEW.agent_config, '$.backend')) = '' THEN 1
         WHEN json_type(NEW.agent_config, '$.name') IS NOT 'text' THEN 1
         WHEN json_type(NEW.agent_config, '$.model_id') IS NOT NULL
          AND json_type(NEW.agent_config, '$.model_id') NOT IN ('text', 'null') THEN 1
         WHEN json_type(NEW.agent_config, '$.clear_context_each_run') IS NOT NULL
          AND json_type(NEW.agent_config, '$.clear_context_each_run') NOT IN ('true', 'false') THEN 1
         WHEN EXISTS (
             SELECT 1 FROM json_each(NEW.agent_config)
             WHERE key NOT IN ('backend', 'name', 'model_id', 'clear_context_each_run')
         ) THEN 1
         ELSE 0
     END
 )
BEGIN
    SELECT RAISE(ABORT, 'non-owner cron job must be model-only');
END;
