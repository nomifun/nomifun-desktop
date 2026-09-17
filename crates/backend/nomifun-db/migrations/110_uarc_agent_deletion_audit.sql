-- Append-only non-private audit ledger for explicit AgentSession deletion
-- quarantine overrides. Existing generation-5 databases receive the same
-- table present in the clean-start baseline.
CREATE TABLE agent_deletion_audits (
    audit_id TEXT PRIMARY KEY CHECK (trim(audit_id) <> ''),
    agent_session_id TEXT NOT NULL,
    owner_ref_json TEXT NOT NULL CHECK (json_valid(owner_ref_json)),
    target_kind TEXT NOT NULL CHECK (target_kind IN ('effect', 'resource_cleanup')),
    target_id TEXT NOT NULL CHECK (trim(target_id) <> ''),
    authority TEXT NOT NULL CHECK (authority = 'installation_owner_manual_override'),
    risk_acknowledged INTEGER NOT NULL CHECK (risk_acknowledged = 1),
    reason_digest TEXT NOT NULL CHECK (length(reason_digest) = 64),
    recorded_at INTEGER NOT NULL CHECK (recorded_at >= 0),
    UNIQUE (agent_session_id, target_kind, target_id, reason_digest),
    FOREIGN KEY (agent_session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_agent_deletion_audits_session_time
    ON agent_deletion_audits(agent_session_id, recorded_at, audit_id);
