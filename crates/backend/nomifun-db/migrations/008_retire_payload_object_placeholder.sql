-- The startup-owned migration connection already suspends FK enforcement for
-- table replacements and restores it on success or failure. Preserve all
-- canonical payload bytes and identities; never reinterpret an object row.
CREATE TEMP TABLE payload_storage_integrity (
    valid INTEGER NOT NULL CHECK (valid = 1)
);
INSERT INTO payload_storage_integrity
SELECT CASE WHEN storage_kind = 'inline' AND object_ref IS NULL AND body IS NOT NULL
    THEN 1 ELSE 0 END FROM agent_payloads;

CREATE TABLE agent_payloads_next (
    payload_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    media_type TEXT NOT NULL,
    byte_len INTEGER NOT NULL CHECK (byte_len >= 0),
    digest TEXT NOT NULL CHECK (length(digest) = 64),
    body BLOB NOT NULL,
    FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;
INSERT INTO agent_payloads_next (payload_id, session_id, media_type, byte_len, digest, body)
SELECT payload_id, session_id, media_type, byte_len, digest, body FROM agent_payloads;
DROP TABLE agent_payloads;
ALTER TABLE agent_payloads_next RENAME TO agent_payloads;
CREATE INDEX idx_agent_payloads_session ON agent_payloads(session_id);

INSERT INTO payload_storage_integrity
SELECT CASE WHEN COUNT(*) = 0 THEN 1 ELSE 0 END FROM pragma_foreign_key_check;
DROP TABLE payload_storage_integrity;
