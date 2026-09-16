-- UARC-011 installs the canonical Agent Store into the existing main SQLite
-- database. Legacy Agent tables remain readable only until the Wave 6 cutover;
-- UARC-054 replaces this migration sequence with one clean baseline.

CREATE TABLE schema_metadata (
    singleton_key TEXT PRIMARY KEY CHECK (singleton_key = 'canonical'),
    data_generation INTEGER NOT NULL CHECK (data_generation = 5),
    root_instance_id TEXT NOT NULL,
    migration_head INTEGER NOT NULL CHECK (migration_head >= 1),
    seed_manifest_digest TEXT NOT NULL CHECK (length(seed_manifest_digest) = 64),
    canonical_schema_manifest_digest TEXT NOT NULL CHECK (length(canonical_schema_manifest_digest) = 64),
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version >= 1)
) STRICT;

INSERT INTO schema_metadata VALUES (
    'canonical', 5, 'main-sqlite-agent-store', 1,
    '9a4f4144f2927e6d67b0dbf430e97d2859a8f5043a3e0fc4c43d59183fd093eb',
    '29946a8d60e5d17bad7f200ee9b77a28c2da877e842394d4d8880ad23681df72',
    1
);

CREATE TABLE agent_preset_templates (
    template_key TEXT PRIMARY KEY,
    source_kind TEXT NOT NULL CHECK (source_kind = 'official'),
    template_json TEXT NOT NULL CHECK (json_valid(template_json)),
    template_digest TEXT NOT NULL CHECK (length(template_digest) = 64)
) STRICT;

CREATE TABLE agent_presets (
    preset_id TEXT PRIMARY KEY,
    owner_ref_json TEXT NOT NULL CHECK (json_valid(owner_ref_json)),
    source_json TEXT NOT NULL CHECK (json_valid(source_json)),
    display_json TEXT NOT NULL CHECK (json_valid(display_json)),
    current_stable_revision INTEGER,
    created_at INTEGER NOT NULL,
    retired_at_ms INTEGER CHECK (retired_at_ms IS NULL OR retired_at_ms >= 0),
    CHECK (current_stable_revision IS NULL OR current_stable_revision >= 1)
) STRICT;

CREATE TABLE agent_preset_revisions (
    revision_id TEXT PRIMARY KEY,
    preset_id TEXT NOT NULL,
    revision_no INTEGER NOT NULL CHECK (revision_no >= 1),
    schema_version TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    revision_digest TEXT NOT NULL CHECK (length(revision_digest) = 64),
    created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    reason TEXT,
    UNIQUE (preset_id, revision_no),
    FOREIGN KEY (preset_id) REFERENCES agent_presets (preset_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE agent_preset_contribution_locks (
    revision_id TEXT NOT NULL,
    contribution_id TEXT NOT NULL CHECK (trim(contribution_id) <> ''),
    lock_json TEXT NOT NULL CHECK (json_valid(lock_json)),
    PRIMARY KEY (revision_id, contribution_id),
    FOREIGN KEY (revision_id) REFERENCES agent_preset_revisions (revision_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE agent_bindings (
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    agent_binding_json TEXT NOT NULL CHECK (json_valid(agent_binding_json)),
    PRIMARY KEY (target_kind, target_id)
) STRICT;

CREATE TABLE agent_runtime_snapshots (
    snapshot_id TEXT PRIMARY KEY,
    snapshot_digest TEXT NOT NULL UNIQUE CHECK (length(snapshot_digest) = 64),
    content_json TEXT NOT NULL CHECK (json_valid(content_json)),
    envelope_json TEXT NOT NULL CHECK (json_valid(envelope_json))
) STRICT;

CREATE TABLE agent_sessions (
    agent_session_id TEXT PRIMARY KEY,
    owner_ref_json TEXT NOT NULL CHECK (json_valid(owner_ref_json)),
    state TEXT NOT NULL CHECK (state IN ('live', 'deleting', 'deleted')),
    title TEXT,
    archived INTEGER CHECK (archived IN (0, 1)),
    pinned INTEGER CHECK (pinned IN (0, 1)),
    agent_binding_json TEXT CHECK (agent_binding_json IS NULL OR json_valid(agent_binding_json)),
    remote_binding_id TEXT,
    remote_binding_version INTEGER,
    parent_agent_session_id TEXT,
    fork_base_payload_id TEXT,
    next_seq INTEGER,
    created_at INTEGER,
    deleted_at INTEGER,
    CHECK (
        (state IN ('live', 'deleting') AND agent_binding_json IS NOT NULL
            AND archived IS NOT NULL AND pinned IS NOT NULL
            AND next_seq IS NOT NULL AND next_seq >= 1
            AND created_at IS NOT NULL AND deleted_at IS NULL)
        OR
        (state = 'deleted' AND title IS NULL AND archived IS NULL AND pinned IS NULL
            AND agent_binding_json IS NULL AND remote_binding_id IS NULL
            AND remote_binding_version IS NULL AND parent_agent_session_id IS NULL
            AND fork_base_payload_id IS NULL AND next_seq IS NULL
            AND created_at IS NULL AND deleted_at IS NOT NULL)
    ),
    FOREIGN KEY (parent_agent_session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE agent_turns (
    session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL CHECK (trim(turn_id) <> ''),
    operation_id TEXT NOT NULL CHECK (trim(operation_id) <> ''),
    idempotency_key TEXT NOT NULL CHECK (trim(idempotency_key) <> ''),
    source_message_id TEXT,
    admission_json TEXT CHECK (admission_json IS NULL OR json_valid(admission_json)),
    state TEXT NOT NULL CHECK (state IN ('accepted', 'running', 'completed', 'failed', 'cancelled', 'interrupted')),
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    error_json TEXT CHECK (error_json IS NULL OR json_valid(error_json)),
    started_event_id TEXT,
    terminal_event_id TEXT,
    accepted_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    PRIMARY KEY (session_id, turn_id),
    UNIQUE (session_id, operation_id),
    UNIQUE (session_id, idempotency_key),
    FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    FOREIGN KEY (started_event_id) REFERENCES agent_events (event_id)
        ON UPDATE RESTRICT ON DELETE SET NULL,
    FOREIGN KEY (terminal_event_id) REFERENCES agent_events (event_id)
        ON UPDATE RESTRICT ON DELETE SET NULL,
    CHECK (
        (state IN ('accepted', 'running') AND terminal_event_id IS NULL AND finished_at IS NULL)
        OR
        (state IN ('completed', 'failed', 'cancelled', 'interrupted')
            AND terminal_event_id IS NOT NULL AND finished_at IS NOT NULL)
    )
) STRICT;

CREATE TABLE agent_session_resources (
    binding_id TEXT PRIMARY KEY CHECK (trim(binding_id) <> ''),
    session_id TEXT NOT NULL,
    resource_kind TEXT NOT NULL CHECK (trim(resource_kind) <> ''),
    resource_id TEXT NOT NULL CHECK (trim(resource_id) <> ''),
    owner_id TEXT NOT NULL CHECK (trim(owner_id) <> ''),
    operations_json TEXT NOT NULL CHECK (json_valid(operations_json) AND json_type(operations_json) = 'array'),
    connection_config_ref TEXT,
    typed_parameters_json TEXT NOT NULL CHECK (json_valid(typed_parameters_json) AND json_type(typed_parameters_json) = 'object'),
    binding_digest TEXT NOT NULL CHECK (length(binding_digest) = 64),
    UNIQUE (session_id, binding_id),
    FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE agent_payloads (
    payload_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    media_type TEXT NOT NULL,
    byte_len INTEGER NOT NULL CHECK (byte_len >= 0),
    digest TEXT NOT NULL CHECK (length(digest) = 64),
    storage_kind TEXT NOT NULL CHECK (storage_kind IN ('inline', 'object')),
    body BLOB,
    object_ref TEXT,
    CHECK (
        (storage_kind = 'inline' AND body IS NOT NULL AND object_ref IS NULL)
        OR
        (storage_kind = 'object' AND body IS NULL AND object_ref = 'objects/' || digest)
    ),
    FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE agent_events (
    session_id TEXT NOT NULL,
    seq INTEGER NOT NULL CHECK (seq >= 1),
    event_id TEXT NOT NULL UNIQUE,
    producer_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    runtime_binding_id TEXT,
    runtime_producer_seq INTEGER CHECK (runtime_producer_seq IS NULL OR runtime_producer_seq >= 1),
    kind TEXT NOT NULL,
    kind_version INTEGER NOT NULL CHECK (kind_version >= 1),
    correlation_id TEXT NOT NULL,
    causation_event_id TEXT,
    inline_json TEXT CHECK (inline_json IS NULL OR json_valid(inline_json)),
    payload_id TEXT,
    PRIMARY KEY (session_id, seq),
    UNIQUE (producer_id, idempotency_key),
    UNIQUE (runtime_binding_id, runtime_producer_seq),
    CHECK (inline_json IS NULL OR payload_id IS NULL),
    FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    FOREIGN KEY (payload_id) REFERENCES agent_payloads (payload_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (causation_event_id) REFERENCES agent_events (event_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE agent_effects (
    effect_id TEXT PRIMARY KEY CHECK (trim(effect_id) <> ''),
    session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL CHECK (trim(turn_id) <> ''),
    operation_id TEXT NOT NULL CHECK (trim(operation_id) <> ''),
    owner_domain TEXT NOT NULL CHECK (trim(owner_domain) <> ''),
    capability_module TEXT NOT NULL CHECK (trim(capability_module) <> ''),
    action_id TEXT NOT NULL CHECK (trim(action_id) <> ''),
    resource_binding_id TEXT,
    resource_key TEXT,
    input_digest TEXT NOT NULL CHECK (length(input_digest) = 64),
    strategy TEXT NOT NULL CHECK (strategy IN ('managed_effect', 'external_uncertain_effect')),
    state TEXT NOT NULL CHECK (state IN ('pending', 'returned', 'rejected', 'cancelled', 'unknown')),
    bounded_observation_json TEXT CHECK (bounded_observation_json IS NULL OR json_valid(bounded_observation_json)),
    started_event_id TEXT NOT NULL,
    terminal_event_id TEXT,
    created_at INTEGER NOT NULL,
    settled_at INTEGER,
    FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    FOREIGN KEY (session_id, turn_id) REFERENCES agent_turns (session_id, turn_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (session_id, resource_binding_id) REFERENCES agent_session_resources (session_id, binding_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (started_event_id) REFERENCES agent_events (event_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (terminal_event_id) REFERENCES agent_events (event_id)
        ON UPDATE RESTRICT ON DELETE SET NULL,
    CHECK (
        (state = 'pending' AND terminal_event_id IS NULL AND settled_at IS NULL)
        OR
        (state <> 'pending' AND terminal_event_id IS NOT NULL AND settled_at IS NOT NULL)
    )
) STRICT;

CREATE TABLE agent_session_heads (
    session_id TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    active_turn_id TEXT,
    active_set_generation INTEGER NOT NULL CHECK (active_set_generation >= 0),
    runtime_checkpoint_locator TEXT,
    runtime_checkpoint_digest TEXT CHECK (runtime_checkpoint_digest IS NULL OR length(runtime_checkpoint_digest) = 64),
    runtime_bound_event_id TEXT,
    runtime_protocol_version TEXT,
    snapshot_digest TEXT CHECK (snapshot_digest IS NULL OR length(snapshot_digest) = 64),
    checkpoint_through_seq INTEGER CHECK (checkpoint_through_seq IS NULL OR checkpoint_through_seq >= 0),
    last_seq INTEGER NOT NULL CHECK (last_seq >= 0),
    unread_count INTEGER NOT NULL CHECK (unread_count >= 0),
    FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    FOREIGN KEY (runtime_bound_event_id) REFERENCES agent_events (event_id)
        ON UPDATE RESTRICT ON DELETE SET NULL
) STRICT;

CREATE TABLE agent_messages (
    session_id TEXT NOT NULL,
    projection_id TEXT NOT NULL,
    first_seq INTEGER NOT NULL CHECK (first_seq >= 1),
    last_seq INTEGER NOT NULL CHECK (last_seq >= first_seq),
    presentation_intent TEXT NOT NULL,
    projection_json TEXT NOT NULL CHECK (json_valid(projection_json)),
    semantic_digest TEXT NOT NULL CHECK (length(semantic_digest) = 64),
    PRIMARY KEY (session_id, projection_id),
    FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_agent_presets_active ON agent_presets(preset_id) WHERE retired_at_ms IS NULL;
CREATE INDEX idx_agent_preset_revisions_preset ON agent_preset_revisions(preset_id, revision_no);
CREATE INDEX idx_agent_sessions_owner_state ON agent_sessions(owner_ref_json, state);
CREATE INDEX idx_agent_turns_session_state ON agent_turns(session_id, state, accepted_at);
CREATE INDEX idx_agent_session_resources_session_kind ON agent_session_resources(session_id, resource_kind, binding_id);
CREATE INDEX idx_agent_events_correlation ON agent_events(session_id, correlation_id, seq);
CREATE INDEX idx_agent_payloads_session ON agent_payloads(session_id);
CREATE INDEX idx_agent_messages_sequence ON agent_messages(session_id, first_seq, last_seq);
CREATE INDEX idx_agent_effects_session_turn ON agent_effects(session_id, turn_id, created_at);
CREATE UNIQUE INDEX idx_agent_effects_resource_unsettled
    ON agent_effects(owner_domain, resource_key)
    WHERE state IN ('pending', 'unknown') AND resource_key IS NOT NULL;
