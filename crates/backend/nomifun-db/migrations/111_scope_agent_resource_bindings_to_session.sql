-- A product resource may be frozen into more than one AgentSession. Binding
-- identity is therefore scoped by the owning Session, matching every effect
-- foreign key and the immutable AgentBinding projection.

-- Stage data without foreign keys, then rebuild both the parent and its only
-- child under their final names. This keeps foreign-key enforcement enabled
-- and makes the migrated schema byte-equivalent to the clean baseline.
CREATE TABLE agent_session_resources_stage AS
SELECT binding_id, session_id, resource_kind, resource_id, owner_id,
       operations_json, connection_config_ref, typed_parameters_json, binding_digest
FROM agent_session_resources;

CREATE TABLE agent_effects_stage AS
SELECT effect_id, session_id, turn_id, operation_id, owner_domain,
       capability_module, action_id, resource_binding_id, resource_key,
       input_digest, strategy, state, bounded_observation_json,
       started_event_id, terminal_event_id, created_at, settled_at
FROM agent_effects;

DROP TABLE agent_effects;
DROP TABLE agent_session_resources;

CREATE TABLE agent_session_resources (
    binding_id TEXT NOT NULL CHECK (trim(binding_id) <> ''),
    session_id TEXT NOT NULL,
    resource_kind TEXT NOT NULL CHECK (trim(resource_kind) <> ''),
    resource_id TEXT NOT NULL CHECK (trim(resource_id) <> ''),
    owner_id TEXT NOT NULL CHECK (trim(owner_id) <> ''),
    operations_json TEXT NOT NULL CHECK (json_valid(operations_json) AND json_type(operations_json) = 'array'),
    connection_config_ref TEXT,
    typed_parameters_json TEXT NOT NULL CHECK (json_valid(typed_parameters_json) AND json_type(typed_parameters_json) = 'object'),
    binding_digest TEXT NOT NULL CHECK (length(binding_digest) = 64),
    PRIMARY KEY (session_id, binding_id),
    FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

INSERT INTO agent_session_resources (
    binding_id, session_id, resource_kind, resource_id, owner_id,
    operations_json, connection_config_ref, typed_parameters_json, binding_digest
)
SELECT binding_id, session_id, resource_kind, resource_id, owner_id,
       operations_json, connection_config_ref, typed_parameters_json, binding_digest
FROM agent_session_resources_stage;

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

INSERT INTO agent_effects (
    effect_id, session_id, turn_id, operation_id, owner_domain,
    capability_module, action_id, resource_binding_id, resource_key,
    input_digest, strategy, state, bounded_observation_json,
    started_event_id, terminal_event_id, created_at, settled_at
)
SELECT effect_id, session_id, turn_id, operation_id, owner_domain,
       capability_module, action_id, resource_binding_id, resource_key,
       input_digest, strategy, state, bounded_observation_json,
       started_event_id, terminal_event_id, created_at, settled_at
FROM agent_effects_stage;

DROP TABLE agent_effects_stage;
DROP TABLE agent_session_resources_stage;

CREATE INDEX idx_agent_session_resources_session_kind
    ON agent_session_resources(session_id, resource_kind, binding_id);
CREATE INDEX idx_agent_effects_session_turn
    ON agent_effects(session_id, turn_id, created_at);
CREATE UNIQUE INDEX idx_agent_effects_resource_unsettled
    ON agent_effects(owner_domain, resource_key)
    WHERE state IN ('pending', 'unknown') AND resource_key IS NOT NULL;
