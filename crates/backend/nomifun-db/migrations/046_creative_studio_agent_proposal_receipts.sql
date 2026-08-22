-- Durable exactly-once receipts for Canvas Agent proposal application.
--
-- The clean v3 schema deliberately models relationships through the runtime
-- logical-reference registry instead of SQLite FOREIGN KEY declarations. The
-- project link below is therefore indexed and registered as CASCADE; the
-- workshop repository performs that cascade in the same transaction as
-- project deletion.

CREATE TABLE creative_studio_agent_proposal_receipts (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id           TEXT NOT NULL
                         CHECK (
                             length(project_id) = 36
                             AND lower(project_id) = project_id
                             AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                         ),
    assistant_message_id TEXT NOT NULL
                         CHECK (
                             length(assistant_message_id) = 36
                             AND lower(assistant_message_id) = assistant_message_id
                             AND assistant_message_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(assistant_message_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                         ),
    ops_fingerprint      TEXT NOT NULL
                         CHECK (
                             length(ops_fingerprint) = 64
                             AND lower(ops_fingerprint) = ops_fingerprint
                             AND ops_fingerprint NOT GLOB '*[^0-9a-f]*'
                         ),
    ops_json             TEXT NOT NULL
                         CHECK (json_valid(ops_json) AND json_type(ops_json) = 'array'),
    results_json         TEXT NOT NULL
                         CHECK (json_valid(results_json) AND json_type(results_json) = 'array'),
    applied_revision     INTEGER NOT NULL CHECK (applied_revision >= 2),
    created_at           INTEGER NOT NULL CHECK (created_at >= 0),
    UNIQUE (assistant_message_id)
);

CREATE INDEX idx_creative_agent_proposal_receipts_project
    ON creative_studio_agent_proposal_receipts(project_id);

CREATE INDEX idx_creative_agent_proposal_receipts_assistant_message
    ON creative_studio_agent_proposal_receipts(assistant_message_id);
