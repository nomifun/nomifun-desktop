-- Migration 041: one enforceable execution-authority boundary.
--
-- The desktop backend and every child Agent run as the same OS account. SQL
-- row ownership is therefore not a process/filesystem sandbox. The canonical
-- installation owner retains host execution; every other authenticated user is
-- model-only and may keep ordinary Nomi conversations/cron without host tools.

-- Retire unsafe legacy secondary-user runtime state deterministically while
-- preserving conversation transcripts and AgentExecution audit graphs.
DELETE FROM conversation_mcp_servers
WHERE conversation_id IN (
    SELECT id FROM conversations WHERE user_id <> 'system_default_user'
);

-- Process-issued loopback capabilities never belong in persisted JSON. Remove
-- the former boolean marker and any accidentally persisted private bridge
-- configs for every owner as part of the hard cut; the factory now derives and
-- injects them after authority resolution.
UPDATE conversations
SET extra = json_remove(
        extra,
        '$.desktopGateway',
        '$.desktop_gateway',
        '$.gatewayMcpConfig',
        '$.gateway_mcp_config',
        '$.requirementMcpConfig',
        '$.requirement_mcp_config',
        '$.knowledgeMcpConfig',
        '$.knowledge_mcp_config'
    )
WHERE json_valid(extra);

UPDATE conversations
SET type = 'nomi',
    extra = '{}',
    delegation_policy = 'disabled',
    execution_model_pool = NULL,
    decision_policy = 'automatic',
    execution_template_id = NULL,
    channel_chat_id = NULL,
    preset_id = NULL,
    preset_revision = NULL,
    preset_snapshot = NULL,
    updated_at = MAX(updated_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
WHERE user_id <> 'system_default_user';

-- Preserve the part of a Nomi cron configuration that is necessary to create
-- a model-only Conversation, while deleting every host-capability field. This
-- avoids disabling valid new-conversation schedules merely because the old
-- JSON bag also carried workspace/preset/session settings.
UPDATE cron_jobs
SET agent_type = 'nomi',
    agent_config = CASE
        WHEN agent_type = 'nomi'
         AND agent_config IS NOT NULL
         AND json_valid(agent_config)
         AND json_type(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END) = 'object'
         AND json_type(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.backend') = 'text'
         AND trim(json_extract(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.backend')) <> ''
        THEN json_object(
            'backend', json_extract(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.backend'),
            'name', CASE
                WHEN json_type(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.name') = 'text'
                THEN json_extract(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.name')
                ELSE ''
            END,
            'model_id', CASE
                WHEN json_type(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.model_id') = 'text'
                 AND trim(json_extract(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.model_id')) <> ''
                THEN json_extract(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.model_id')
                ELSE NULL
            END,
            'clear_context_each_run', json(
                CASE WHEN json_extract(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.clear_context_each_run') = 1
                     THEN 'true' ELSE 'false' END
            )
        )
        ELSE NULL
    END,
    preset_id = NULL,
    preset_revision = NULL,
    preset_snapshot = NULL,
    skill_content = NULL,
    target_kind = 'agent',
    terminal_mode = NULL,
    terminal_session_id = NULL,
    terminal_command = NULL,
    terminal_args = NULL,
    terminal_script = NULL,
    enabled = CASE
        WHEN execution_mode = 'existing'
         AND EXISTS (
             SELECT 1
             FROM conversations conversation
             WHERE conversation.id = cron_jobs.conversation_id
               AND conversation.user_id = cron_jobs.user_id
               AND conversation.type = 'nomi'
               AND conversation.model IS NOT NULL
               AND json_valid(conversation.model)
               AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END) = 'object'
               AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.provider_id') = 'text'
               AND trim(json_extract(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.provider_id')) <> ''
               AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.model') = 'text'
               AND trim(json_extract(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.model')) <> ''
         )
        THEN enabled
        WHEN agent_type = 'nomi'
         AND agent_config IS NOT NULL
         AND json_valid(agent_config)
         AND json_type(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END) = 'object'
         AND json_type(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.backend') = 'text'
         AND trim(json_extract(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.backend')) <> ''
        THEN enabled
        ELSE 0
    END,
    last_error = CASE
        WHEN (
            execution_mode = 'existing'
            AND EXISTS (
                SELECT 1
                FROM conversations conversation
                WHERE conversation.id = cron_jobs.conversation_id
                  AND conversation.user_id = cron_jobs.user_id
                  AND conversation.type = 'nomi'
                  AND conversation.model IS NOT NULL
                  AND json_valid(conversation.model)
                  AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END) = 'object'
                  AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.provider_id') = 'text'
                  AND trim(json_extract(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.provider_id')) <> ''
                  AND json_type(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.model') = 'text'
                  AND trim(json_extract(CASE WHEN json_valid(conversation.model) THEN conversation.model ELSE '{}' END, '$.model')) <> ''
            )
        ) OR (
            agent_type = 'nomi'
            AND agent_config IS NOT NULL
            AND json_valid(agent_config)
            AND json_type(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END) = 'object'
            AND json_type(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.backend') = 'text'
            AND trim(json_extract(CASE WHEN json_valid(agent_config) THEN agent_config ELSE '{}' END, '$.backend')) <> ''
        )
        THEN last_error
        ELSE 'Disabled during execution-authority migration: choose a Nomi model before re-enabling'
    END,
    updated_at = MAX(updated_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
WHERE user_id <> 'system_default_user';

-- A retained graph is useful audit history but must never be recovered by the
-- scheduler. Tombstone rather than cascade-delete it.
UPDATE agent_executions
SET deleted_at = MAX(created_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000),
    lease_owner = NULL,
    lease_expires_at = NULL,
    updated_at = MAX(updated_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
WHERE user_id <> 'system_default_user'
  AND deleted_at IS NULL;

-- Templates are installation execution configuration, not runtime audit.
-- Clear Conversation references above, then remove non-owner templates and
-- their participant rows by FK cascade.
DELETE FROM agent_execution_templates
WHERE user_id <> 'system_default_user';

-- Existing secondary PTYs are arbitrary host processes and cannot be safely
-- retained under a model-only principal. Lifecycle/IDMM cleanup runs through
-- the existing raw-delete triggers.
DELETE FROM terminal_sessions
WHERE user_id <> 'system_default_user';

-- Model-only Conversation rows have one canonical executable shape. Extra JSON
-- is still allowed for backend-minted workspace metadata, but execution builds
-- ignore it for non-owners; the high-risk typed fields are constrained here.
-- Aggregate ownership is immutable for every Conversation/Terminal, not only
-- rows currently referenced by another domain. Ownership transfer is a new
-- aggregate operation; rewriting user_id in place can otherwise turn an
-- already-created host resource into a secondary user's capability.
CREATE TRIGGER conversation_owner_immutable
BEFORE UPDATE OF user_id ON conversations
WHEN NEW.user_id IS NOT OLD.user_id
BEGIN
    SELECT RAISE(ABORT, 'conversation owner is immutable');
END;

CREATE TRIGGER terminal_session_owner_immutable
BEFORE UPDATE OF user_id ON terminal_sessions
WHEN NEW.user_id IS NOT OLD.user_id
BEGIN
    SELECT RAISE(ABORT, 'terminal session owner is immutable');
END;

CREATE TRIGGER conversation_execution_authority_insert_guard
BEFORE INSERT ON conversations
WHEN NEW.user_id <> 'system_default_user'
 AND (
     NEW.type <> 'nomi'
     OR NEW.delegation_policy <> 'disabled'
     OR NEW.execution_model_pool IS NOT NULL
     OR NEW.execution_template_id IS NOT NULL
     OR NEW.channel_chat_id IS NOT NULL
     OR NEW.preset_id IS NOT NULL
     OR NEW.preset_revision IS NOT NULL
     OR NEW.preset_snapshot IS NOT NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'non-owner conversation must be model-only');
END;

CREATE TRIGGER conversation_execution_authority_update_guard
BEFORE UPDATE OF user_id, type, delegation_policy, execution_model_pool,
                 execution_template_id, channel_chat_id, preset_id,
                 preset_revision, preset_snapshot
ON conversations
WHEN NEW.user_id <> 'system_default_user'
 AND (
     NEW.type <> 'nomi'
     OR NEW.delegation_policy <> 'disabled'
     OR NEW.execution_model_pool IS NOT NULL
     OR NEW.execution_template_id IS NOT NULL
     OR NEW.channel_chat_id IS NOT NULL
     OR NEW.preset_id IS NOT NULL
     OR NEW.preset_revision IS NOT NULL
     OR NEW.preset_snapshot IS NOT NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'non-owner conversation must be model-only');
END;

CREATE TRIGGER cron_execution_authority_insert_guard
BEFORE INSERT ON cron_jobs
WHEN NEW.user_id <> 'system_default_user'
 AND (
     NEW.agent_type <> 'nomi'
     OR NEW.target_kind <> 'agent'
     OR NEW.preset_id IS NOT NULL
     OR NEW.preset_revision IS NOT NULL
     OR NEW.preset_snapshot IS NOT NULL
     OR NEW.skill_content IS NOT NULL
     OR NEW.terminal_mode IS NOT NULL
     OR NEW.terminal_session_id IS NOT NULL
     OR NEW.terminal_command IS NOT NULL
     OR NEW.terminal_args IS NOT NULL
     OR NEW.terminal_script IS NOT NULL
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
                 skill_content, target_kind, terminal_mode, terminal_session_id,
                 terminal_command, terminal_args, terminal_script
ON cron_jobs
WHEN NEW.user_id <> 'system_default_user'
 AND (
     NEW.agent_type <> 'nomi'
     OR NEW.target_kind <> 'agent'
     OR NEW.preset_id IS NOT NULL
     OR NEW.preset_revision IS NOT NULL
     OR NEW.preset_snapshot IS NOT NULL
     OR NEW.skill_content IS NOT NULL
     OR NEW.terminal_mode IS NOT NULL
     OR NEW.terminal_session_id IS NOT NULL
     OR NEW.terminal_command IS NOT NULL
     OR NEW.terminal_args IS NOT NULL
     OR NEW.terminal_script IS NOT NULL
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

CREATE TRIGGER agent_execution_owner_insert_guard
BEFORE INSERT ON agent_executions
WHEN NEW.user_id <> 'system_default_user'
BEGIN
    SELECT RAISE(ABORT, 'AgentExecution requires the installation owner');
END;

CREATE TRIGGER agent_execution_template_owner_insert_guard
BEFORE INSERT ON agent_execution_templates
WHEN NEW.user_id <> 'system_default_user'
BEGIN
    SELECT RAISE(ABORT, 'AgentExecutionTemplate requires the installation owner');
END;

CREATE TRIGGER terminal_execution_authority_insert_guard
BEFORE INSERT ON terminal_sessions
WHEN NEW.user_id <> 'system_default_user'
BEGIN
    SELECT RAISE(ABORT, 'terminal execution requires the installation owner');
END;
