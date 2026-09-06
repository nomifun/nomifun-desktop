-- Canonicalize the persisted runtime projection name.
--
-- `agent_snapshot` is the consumer-neutral, immutable Agent projection used
-- by Conversation, Cron, Agent Execution, and Execution Template rows.
-- This is a physical rename only: no compatibility alias or dual read/write
-- path is introduced.

ALTER TABLE conversations
    RENAME COLUMN preset_snapshot TO agent_snapshot;

ALTER TABLE agent_execution_participants
    RENAME COLUMN preset_snapshot TO agent_snapshot;

ALTER TABLE agent_execution_template_participants
    RENAME COLUMN preset_snapshot TO agent_snapshot;

ALTER TABLE cron_jobs
    RENAME COLUMN preset_snapshot TO agent_snapshot;
