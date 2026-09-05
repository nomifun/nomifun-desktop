-- Nomi-core Remote persistence.
--
-- nomi_remote_sessions is only a durable Remote projection. Its
-- agent_session_id is the same logical identity as conversations.conversation_id;
-- it is not a second session aggregate.

CREATE TABLE remote_bindings (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    remote_binding_id        TEXT NOT NULL UNIQUE CHECK (
        length(remote_binding_id) = 36
        AND lower(remote_binding_id) = remote_binding_id
        AND remote_binding_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(remote_binding_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id            TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    name                     TEXT NOT NULL CHECK (length(trim(name)) > 0),
    agent_binding_json       TEXT NOT NULL CHECK (
        json_valid(agent_binding_json)
        AND json_type(agent_binding_json) = 'object'
    ),
    nomi_snapshot_json       TEXT NOT NULL CHECK (
        json_valid(nomi_snapshot_json)
        AND json_type(nomi_snapshot_json) = 'object'
    ),
    provenance_json           TEXT NOT NULL CHECK (
        json_valid(provenance_json)
        AND json_type(provenance_json) = 'object'
    ),
    agent_binding_digest     TEXT NOT NULL CHECK (
        length(agent_binding_digest) = 64
        AND lower(agent_binding_digest) = agent_binding_digest
        AND agent_binding_digest NOT GLOB '*[^0-9a-f]*'
    ),
    binding_version          INTEGER NOT NULL CHECK (binding_version > 0),
    created_at               INTEGER NOT NULL,
    updated_at               INTEGER NOT NULL
);

CREATE INDEX idx_remote_bindings_owner_user_id
    ON remote_bindings(owner_user_id);

CREATE TABLE nomi_remote_sessions (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_session_id         TEXT NOT NULL UNIQUE CHECK (
        length(agent_session_id) = 36
        AND lower(agent_session_id) = agent_session_id
        AND agent_session_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(agent_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id            TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    remote_binding_id        TEXT NOT NULL CHECK (
        length(remote_binding_id) = 36
        AND lower(remote_binding_id) = remote_binding_id
        AND remote_binding_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(remote_binding_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    open_idempotency_key     TEXT NOT NULL,
    binding_version          INTEGER NOT NULL CHECK (binding_version > 0),
    agent_binding_digest     TEXT NOT NULL CHECK (
        length(agent_binding_digest) = 64
        AND lower(agent_binding_digest) = agent_binding_digest
        AND agent_binding_digest NOT GLOB '*[^0-9a-f]*'
    ),
    initial_input_digest     TEXT CHECK (
        initial_input_digest IS NULL
        OR (
            length(initial_input_digest) = 64
            AND lower(initial_input_digest) = initial_input_digest
            AND initial_input_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    agent_binding_json       TEXT NOT NULL CHECK (
        json_valid(agent_binding_json)
        AND json_type(agent_binding_json) = 'object'
    ),
    nomi_snapshot_json       TEXT NOT NULL CHECK (
        json_valid(nomi_snapshot_json)
        AND json_type(nomi_snapshot_json) = 'object'
    ),
    provenance_json           TEXT NOT NULL CHECK (
        json_valid(provenance_json)
        AND json_type(provenance_json) = 'object'
    ),
    state                    TEXT NOT NULL CHECK (state IN ('opening', 'ready', 'failed', 'cancelled')),
    created_at               INTEGER NOT NULL,
    updated_at               INTEGER NOT NULL,
    UNIQUE (owner_user_id, open_idempotency_key)
);

CREATE INDEX idx_nomi_remote_sessions_owner_user_id
    ON nomi_remote_sessions(owner_user_id);
CREATE INDEX idx_nomi_remote_sessions_agent_session_id
    ON nomi_remote_sessions(agent_session_id);
CREATE INDEX idx_nomi_remote_sessions_remote_binding_id
    ON nomi_remote_sessions(remote_binding_id);
CREATE INDEX idx_nomi_remote_sessions_owner_open_key
    ON nomi_remote_sessions(owner_user_id, open_idempotency_key);

CREATE TABLE nomi_remote_events (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id                 TEXT NOT NULL UNIQUE CHECK (
        length(event_id) = 36
        AND lower(event_id) = event_id
        AND event_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(event_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    agent_session_id         TEXT NOT NULL CHECK (
        length(agent_session_id) = 36
        AND lower(agent_session_id) = agent_session_id
        AND agent_session_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(agent_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    seq                      INTEGER NOT NULL CHECK (seq > 0),
    event_type               TEXT NOT NULL CHECK (length(trim(event_type)) > 0),
    payload_json             TEXT NOT NULL CHECK (json_valid(payload_json)),
    created_at               INTEGER NOT NULL,
    UNIQUE (agent_session_id, seq)
);

CREATE INDEX idx_nomi_remote_events_agent_session_id_seq
    ON nomi_remote_events(agent_session_id, seq);

CREATE TRIGGER trg_nomi_remote_sessions_provenance_immutable
BEFORE UPDATE ON nomi_remote_sessions
WHEN NEW.agent_session_id IS NOT OLD.agent_session_id
  OR NEW.owner_user_id IS NOT OLD.owner_user_id
  OR NEW.remote_binding_id IS NOT OLD.remote_binding_id
  OR NEW.open_idempotency_key IS NOT OLD.open_idempotency_key
  OR NEW.binding_version IS NOT OLD.binding_version
  OR NEW.agent_binding_digest IS NOT OLD.agent_binding_digest
  OR NEW.initial_input_digest IS NOT OLD.initial_input_digest
  OR NEW.agent_binding_json IS NOT OLD.agent_binding_json
  OR NEW.nomi_snapshot_json IS NOT OLD.nomi_snapshot_json
  OR NEW.provenance_json IS NOT OLD.provenance_json
  OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'NOMI REMOTE SESSION PROVENANCE IS IMMUTABLE');
END;

CREATE TRIGGER trg_nomi_remote_events_append_only_update
BEFORE UPDATE ON nomi_remote_events
BEGIN
    SELECT RAISE(ABORT, 'NOMI REMOTE EVENTS ARE APPEND ONLY');
END;

CREATE TRIGGER trg_nomi_remote_events_append_only_delete
BEFORE DELETE ON nomi_remote_events
BEGIN
    SELECT RAISE(ABORT, 'NOMI REMOTE EVENTS ARE APPEND ONLY');
END;
