-- UARC generation-5 canonical database baseline.
-- Fresh installations create only the final schema; historical Agent tables
-- are authenticated and removed by the one-time Rust cutover for existing data.

PRAGMA foreign_keys = ON;

CREATE TABLE agent_bindings (
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    agent_binding_json TEXT NOT NULL CHECK (json_valid(agent_binding_json)),
    PRIMARY KEY (target_kind, target_id)
) STRICT;

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

CREATE TABLE agent_execution_attempts (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    attempt_id       TEXT NOT NULL UNIQUE
                     CHECK (
                         length(attempt_id) = 36
                         AND lower(attempt_id) = attempt_id
                         AND attempt_id GLOB '????????-????-7???-[89ab]???-????????????'
                         AND replace(attempt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                     ),
    execution_id     TEXT NOT NULL,
    step_id          TEXT NOT NULL
                     CHECK (
                         length(step_id) = 36
                         AND lower(step_id) = step_id
                         AND step_id GLOB '????????-????-7???-[89ab]???-????????????'
                         AND replace(step_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                     ),
    attempt_no       INTEGER NOT NULL CHECK (attempt_no >= 0),
    participant_id   TEXT
                     CHECK (
                         participant_id IS NULL
                         OR (
                             length(participant_id) = 36
                             AND lower(participant_id) = participant_id
                             AND participant_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(participant_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                         )
                     ),
    status           TEXT NOT NULL CHECK (status IN (
                         'queued', 'running', 'waiting_input', 'completed',
                         'failed', 'cancelled', 'interrupted'
                     )),
    trigger_reason   TEXT NOT NULL CHECK (trim(trigger_reason) <> ''),
    effective_config TEXT NOT NULL DEFAULT '{}',
    question         TEXT,
    error            TEXT,
    output_summary   TEXT,
    output_files     TEXT NOT NULL DEFAULT '[]',
    tokens           INTEGER,
    retry_after      INTEGER,
    runtime_state    TEXT,
    started_at       INTEGER,
    finished_at      INTEGER,
    version          INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    UNIQUE (execution_id, step_id, attempt_no),
    CHECK (length(execution_id) = 36 AND lower(execution_id) = execution_id AND execution_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(execution_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE agent_execution_events (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    execution_id          TEXT NOT NULL,
    sequence              INTEGER NOT NULL CHECK (sequence > 0),
    event_type            TEXT NOT NULL CHECK (event_type IN (
                              'created', 'status_changed', 'plan_changed',
                              'step_changed', 'attempt_changed', 'decision_requested',
                              'decision_answered', 'deleted'
                          )),
    step_id               TEXT
                          CHECK (
                              step_id IS NULL
                              OR (
                                  length(step_id) = 36
                                  AND lower(step_id) = step_id
                                  AND step_id GLOB '????????-????-7???-[89ab]???-????????????'
                                  AND replace(step_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                              )
                          ),
    attempt_id            TEXT
                          CHECK (
                              attempt_id IS NULL
                              OR (
                                  length(attempt_id) = 36
                                  AND lower(attempt_id) = attempt_id
                                  AND attempt_id GLOB '????????-????-7???-[89ab]???-????????????'
                                  AND replace(attempt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                              )
                          ),
    actor_type            TEXT NOT NULL CHECK (actor_type IN ('system', 'user', 'agent')),
    actor_id              TEXT,
    actor_conversation_id TEXT,
    actor_attempt_id      TEXT
                          CHECK (
                              actor_attempt_id IS NULL
                              OR (
                                  length(actor_attempt_id) = 36
                                  AND lower(actor_attempt_id) = actor_attempt_id
                                  AND actor_attempt_id GLOB '????????-????-7???-[89ab]???-????????????'
                                  AND replace(actor_attempt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                              )
                          ),
    on_behalf_of_user_id  TEXT NOT NULL,
    payload               TEXT NOT NULL CHECK (json_valid(payload)),
    created_at            INTEGER NOT NULL,
    published_at          INTEGER,
    UNIQUE (execution_id, sequence),
    CHECK (attempt_id IS NULL OR step_id IS NOT NULL),
    CHECK (actor_conversation_id IS NULL OR (length(actor_conversation_id) = 36 AND lower(actor_conversation_id) = actor_conversation_id AND actor_conversation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(actor_conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (actor_id IS NULL OR (length(actor_id) = 36 AND lower(actor_id) = actor_id AND actor_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(actor_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (length(execution_id) = 36 AND lower(execution_id) = execution_id AND execution_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(execution_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    CHECK (length(on_behalf_of_user_id) = 36 AND lower(on_behalf_of_user_id) = on_behalf_of_user_id AND on_behalf_of_user_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(on_behalf_of_user_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE agent_execution_participants (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    participant_id          TEXT NOT NULL UNIQUE
                            CHECK (
                                length(participant_id) = 36
                                AND lower(participant_id) = participant_id
                                AND participant_id GLOB '????????-????-7???-[89ab]???-????????????'
                                AND replace(participant_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                            ),
    execution_id            TEXT NOT NULL,
    source_agent_id         TEXT NOT NULL
                            CHECK (
                                length(source_agent_id) = 36
                                AND lower(source_agent_id) = source_agent_id
                                AND source_agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                                AND replace(source_agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                            ),
    preset_id               TEXT,
    preset_revision         INTEGER CHECK (preset_revision IS NULL OR preset_revision > 0),
    agent_snapshot         TEXT,
    provider_id             TEXT,
    model                   TEXT,
    role                    TEXT,
    capability              TEXT,
    constraints             TEXT,
    description             TEXT,
    system_prompt           TEXT,
    enabled_skills          TEXT NOT NULL DEFAULT '[]',
    disabled_builtin_skills TEXT NOT NULL DEFAULT '[]',
    sort_order              INTEGER NOT NULL DEFAULT 0,
    introduced_in_revision  INTEGER NOT NULL CHECK (introduced_in_revision >= 0),
    retired_in_revision     INTEGER,
    created_at              INTEGER NOT NULL,
    CHECK (
        (provider_id IS NULL AND model IS NULL)
        OR (provider_id IS NOT NULL AND model IS NOT NULL)
    ),
    CHECK (length(execution_id) = 36 AND lower(execution_id) = execution_id AND execution_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(execution_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    CHECK (preset_id IS NULL OR (length(preset_id) = 36 AND lower(preset_id) = preset_id AND preset_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(preset_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (provider_id IS NULL OR (length(provider_id) = 36 AND lower(provider_id) = provider_id AND provider_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*'))
);

CREATE TABLE agent_execution_step_dependencies (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    execution_id           TEXT NOT NULL,
    blocker_step_id        TEXT NOT NULL
                           CHECK (
                               length(blocker_step_id) = 36
                               AND lower(blocker_step_id) = blocker_step_id
                               AND blocker_step_id GLOB '????????-????-7???-[89ab]???-????????????'
                               AND replace(blocker_step_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                           ),
    blocked_step_id        TEXT NOT NULL
                           CHECK (
                               length(blocked_step_id) = 36
                               AND lower(blocked_step_id) = blocked_step_id
                               AND blocked_step_id GLOB '????????-????-7???-[89ab]???-????????????'
                               AND replace(blocked_step_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                           ),
    introduced_in_revision INTEGER NOT NULL CHECK (introduced_in_revision >= 0),
    superseded_in_revision INTEGER,
    UNIQUE (execution_id, blocker_step_id, blocked_step_id, introduced_in_revision),
    CHECK (blocker_step_id <> blocked_step_id),
    CHECK (length(execution_id) = 36 AND lower(execution_id) = execution_id AND execution_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(execution_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE agent_execution_steps (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    step_id                 TEXT NOT NULL UNIQUE
                            CHECK (
                                length(step_id) = 36
                                AND lower(step_id) = step_id
                                AND step_id GLOB '????????-????-7???-[89ab]???-????????????'
                                AND replace(step_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                            ),
    execution_id            TEXT NOT NULL,
    title                   TEXT NOT NULL CHECK (trim(title) <> ''),
    spec                    TEXT NOT NULL,
    role                    TEXT,
    tool_policy             TEXT NOT NULL DEFAULT 'full'
                            CHECK (tool_policy IN ('full', 'read_only', 'read_shell')),
    kind                    TEXT NOT NULL CHECK (kind IN ('agent', 'verify', 'judge', 'loop')),
    agent_mode              TEXT,
    profile                 TEXT,
    fanout_group            TEXT,
    control_policy          TEXT,
    delegation_depth        INTEGER NOT NULL DEFAULT 0 CHECK (delegation_depth BETWEEN 0 AND 4),
    status                  TEXT NOT NULL CHECK (status IN (
                                'pending', 'running', 'waiting_input', 'completed',
                                'failed', 'skipped', 'cancelled'
                            )),
    assigned_participant_id TEXT
                            CHECK (
                                assigned_participant_id IS NULL
                                OR (
                                    length(assigned_participant_id) = 36
                                    AND lower(assigned_participant_id) = assigned_participant_id
                                    AND assigned_participant_id GLOB '????????-????-7???-[89ab]???-????????????'
                                    AND replace(assigned_participant_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                                )
                            ),
    assignment_score        REAL,
    assignment_rationale    TEXT,
    assignment_source       TEXT,
    assignment_locked       INTEGER NOT NULL DEFAULT 0 CHECK (assignment_locked IN (0, 1)),
    failure_policy          TEXT NOT NULL DEFAULT 'fail_execution'
                            CHECK (failure_policy IN ('fail_execution', 'skip_dependents')),
    preset_prompt           TEXT,
    graph_x                 REAL,
    graph_y                 REAL,
    dispatch_after          INTEGER,
    version                 INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    introduced_in_revision  INTEGER NOT NULL CHECK (introduced_in_revision >= 0),
    superseded_in_revision  INTEGER,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    CHECK (length(execution_id) = 36 AND lower(execution_id) = execution_id AND execution_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(execution_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE agent_execution_template_participants (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    template_participant_id TEXT NOT NULL UNIQUE
                            CHECK (
                                length(template_participant_id) = 36
                                AND lower(template_participant_id) = template_participant_id
                                AND template_participant_id GLOB '????????-????-7???-[89ab]???-????????????'
                                AND replace(template_participant_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                            ),
    template_id             TEXT NOT NULL,
    source_agent_id         TEXT NOT NULL
                            CHECK (
                                length(source_agent_id) = 36
                                AND lower(source_agent_id) = source_agent_id
                                AND source_agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                                AND replace(source_agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                            ),
    preset_id               TEXT,
    preset_revision         INTEGER,
    agent_snapshot         TEXT,
    provider_id             TEXT,
    model                   TEXT,
    role                    TEXT,
    capability              TEXT,
    constraints             TEXT,
    description             TEXT,
    system_prompt           TEXT,
    enabled_skills          TEXT NOT NULL DEFAULT '[]',
    disabled_builtin_skills TEXT NOT NULL DEFAULT '[]',
    sort_order              INTEGER NOT NULL DEFAULT 0,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    CHECK (preset_id IS NULL OR (length(preset_id) = 36 AND lower(preset_id) = preset_id AND preset_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(preset_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (provider_id IS NULL OR (length(provider_id) = 36 AND lower(provider_id) = provider_id AND provider_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (length(template_id) = 36 AND lower(template_id) = template_id AND template_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(template_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE agent_execution_templates (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    execution_template_id  TEXT NOT NULL UNIQUE
                           CHECK (
                               length(execution_template_id) = 36
                               AND lower(execution_template_id) = execution_template_id
                               AND execution_template_id GLOB '????????-????-7???-[89ab]???-????????????'
                               AND replace(execution_template_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                           ),
    user_id                TEXT NOT NULL,
    name                   TEXT NOT NULL CHECK (trim(name) <> ''),
    description            TEXT,
    max_parallel           INTEGER CHECK (max_parallel IS NULL OR max_parallel BETWEEN 1 AND 64),
    work_dir               TEXT,
    context                TEXT CHECK (context IS NULL OR json_valid(context)),
    primary_participant_id TEXT NOT NULL
                           CHECK (
                               length(primary_participant_id) = 36
                               AND lower(primary_participant_id) = primary_participant_id
                               AND primary_participant_id GLOB '????????-????-7???-[89ab]???-????????????'
                               AND replace(primary_participant_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                           ),
    version                INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (length(user_id) = 36 AND lower(user_id) = user_id AND user_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE agent_executions (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    execution_id        TEXT NOT NULL UNIQUE
                        CHECK (
                            length(execution_id) = 36
                            AND lower(execution_id) = execution_id
                            AND execution_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(execution_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    user_id             TEXT NOT NULL,
    goal                TEXT NOT NULL CHECK (trim(goal) <> ''),
    status              TEXT NOT NULL CHECK (status IN (
                            'planning', 'awaiting_approval', 'running', 'paused',
                            'waiting_input', 'completed', 'completed_with_failures',
                            'failed', 'cancelled'
                        )),
    plan_gate           TEXT NOT NULL CHECK (plan_gate IN ('automatic', 'require_approval')),
    adaptation_policy   TEXT NOT NULL CHECK (adaptation_policy IN ('fixed', 'adaptive')),
    decision_policy     TEXT NOT NULL CHECK (decision_policy IN ('automatic', 'ask_user')),
    delegation_policy   TEXT NOT NULL CHECK (delegation_policy IN ('disabled', 'automatic', 'prefer_parallel')),
    max_parallel        INTEGER NOT NULL DEFAULT 4 CHECK (max_parallel BETWEEN 1 AND 64),
    work_dir            TEXT,
    initial_plan_input  TEXT NOT NULL CHECK (
                            json_valid(initial_plan_input)
                            AND json_type(initial_plan_input) = 'object'
                        ),
    summary             TEXT,
    version             INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    plan_revision       INTEGER NOT NULL DEFAULT 0 CHECK (plan_revision >= 0),
    event_sequence      INTEGER NOT NULL DEFAULT 0 CHECK (event_sequence >= 0),
    lease_owner         TEXT,
    lease_expires_at    INTEGER,
    deleted_at          INTEGER,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    CHECK (
        (lease_owner IS NULL AND lease_expires_at IS NULL)
        OR (trim(lease_owner) <> '' AND lease_expires_at IS NOT NULL)
    ),
    CHECK (updated_at >= created_at),
    CHECK (length(user_id) = 36 AND lower(user_id) = user_id AND user_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

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

CREATE TABLE agent_metadata (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id            TEXT NOT NULL UNIQUE
                        CHECK (
                            length(agent_id) = 36
                            AND lower(agent_id) = agent_id
                            AND agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    icon                TEXT,
    name                TEXT NOT NULL,
    name_i18n           TEXT,
    description         TEXT,
    description_i18n    TEXT,
    backend             TEXT,
    agent_type          TEXT NOT NULL,
    agent_source        TEXT NOT NULL,
    agent_source_info   TEXT,
    source_key          TEXT UNIQUE
                        CHECK (source_key IS NULL OR trim(source_key) <> ''),
    enabled             INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    command             TEXT,
    args                TEXT,
    env                 TEXT,
    native_skills_dirs  TEXT,
    behavior_policy     TEXT,
    yolo_id             TEXT,
    agent_capabilities  TEXT,
    auth_methods        TEXT,
    config_options      TEXT,
    available_modes     TEXT,
    available_models    TEXT,
    available_commands  TEXT,
    sort_order          INTEGER NOT NULL DEFAULT 1000,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

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

CREATE TABLE agent_preset_contribution_locks (
    revision_id TEXT NOT NULL,
    contribution_id TEXT NOT NULL CHECK (trim(contribution_id) <> ''),
    lock_json TEXT NOT NULL CHECK (json_valid(lock_json)),
    PRIMARY KEY (revision_id, contribution_id),
    FOREIGN KEY (revision_id) REFERENCES agent_preset_revisions (revision_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
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

CREATE TABLE agent_runtime_snapshots (
    snapshot_id TEXT PRIMARY KEY,
    snapshot_digest TEXT NOT NULL UNIQUE CHECK (length(snapshot_digest) = 64),
    content_json TEXT NOT NULL CHECK (json_valid(content_json)),
    envelope_json TEXT NOT NULL CHECK (json_valid(envelope_json))
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

CREATE TABLE attachments (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    attachment_id  TEXT NOT NULL UNIQUE
                   CHECK (
                       length(attachment_id) = 36
                       AND lower(attachment_id) = attachment_id
                       AND attachment_id GLOB '????????-????-7???-[89ab]???-????????????'
                       AND replace(attachment_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                   ),
    requirement_id TEXT NOT NULL,
    file_name      TEXT NOT NULL,
    rel_path       TEXT NOT NULL,
    mime           TEXT NOT NULL,
    size_bytes     INTEGER NOT NULL,
    created_by     TEXT,
    created_at     INTEGER NOT NULL,
    UNIQUE (requirement_id, file_name),
    CHECK (length(requirement_id) = 36 AND lower(requirement_id) = requirement_id AND requirement_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(requirement_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE channel_inbound_receipts (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_key       TEXT NOT NULL UNIQUE
                        CHECK (
                            length(operation_key) = 83
                            AND operation_key GLOB 'channel-inbound:v1:[0-9a-f]*'
                            AND substr(operation_key, 20) NOT GLOB '*[^0-9a-f]*'
                        ),
    user_scope_id       TEXT NOT NULL
                        CHECK (
                            length(user_scope_id) = 36
                            AND lower(user_scope_id) = user_scope_id
                            AND user_scope_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(user_scope_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    user_id             TEXT
                        CHECK (
                            user_id IS NULL
                            OR (
                                length(user_id) = 36
                                AND lower(user_id) = user_id
                                AND user_id GLOB '????????-????-7???-[89ab]???-????????????'
                                AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                            )
                        ),
    channel_plugin_scope_id TEXT NOT NULL
                        CHECK (
                            length(channel_plugin_scope_id) = 36
                            AND lower(channel_plugin_scope_id) = channel_plugin_scope_id
                            AND channel_plugin_scope_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(channel_plugin_scope_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    channel_plugin_id   TEXT
                        CHECK (
                            channel_plugin_id IS NULL
                            OR (
                                length(channel_plugin_id) = 36
                                AND lower(channel_plugin_id) = channel_plugin_id
                                AND channel_plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
                                AND replace(channel_plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                            )
                        ),
    platform            TEXT NOT NULL CHECK (length(platform) BETWEEN 1 AND 64),
    chat_id             TEXT NOT NULL CHECK (length(chat_id) BETWEEN 1 AND 512),
    provider_event_id   TEXT NOT NULL CHECK (length(provider_event_id) BETWEEN 1 AND 512),
    payload_hash        TEXT NOT NULL
                        CHECK (
                            length(payload_hash) = 64
                            AND lower(payload_hash) = payload_hash
                            AND payload_hash NOT GLOB '*[^0-9a-f]*'
                        ),
    status              TEXT NOT NULL DEFAULT 'accepted'
                        CHECK (status IN ('accepted', 'completed', 'failed')),
    phase               TEXT NOT NULL DEFAULT 'claimed'
                        CHECK (phase IN ('claimed', 'effects_started', 'settled')),
    owner_generation    INTEGER NOT NULL DEFAULT 1 CHECK (owner_generation >= 1),
    conversation_scope_id TEXT,
    message_scope_id    TEXT,
    conversation_id     TEXT,
    message_id          TEXT,
    outcome_json        TEXT CHECK (
                            outcome_json IS NULL
                            OR (json_valid(outcome_json) AND json_type(outcome_json) = 'object')
                        ),
    error_text          TEXT,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    completed_at        INTEGER,
    CHECK (
        (status = 'accepted' AND phase IN ('claimed', 'effects_started') AND completed_at IS NULL)
        OR
        (status IN ('completed', 'failed') AND phase = 'settled' AND completed_at IS NOT NULL)
    ),
    CHECK (conversation_scope_id IS NULL OR (length(conversation_scope_id) = 36 AND lower(conversation_scope_id) = conversation_scope_id AND conversation_scope_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(conversation_scope_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (message_scope_id IS NULL OR (length(message_scope_id) = 36 AND lower(message_scope_id) = message_scope_id AND message_scope_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(message_scope_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (conversation_id IS NULL OR (length(conversation_id) = 36 AND lower(conversation_id) = conversation_id AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (message_id IS NULL OR (length(message_id) = 36 AND lower(message_id) = message_id AND message_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(message_id, '-', '') NOT GLOB '*[^0-9a-f]*'))
);

CREATE TABLE channel_pairing_codes (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    code              TEXT NOT NULL UNIQUE,
    platform_user_id  TEXT NOT NULL,
    platform_type     TEXT NOT NULL,
    channel_plugin_id TEXT
                      CHECK (
                          channel_plugin_id IS NULL
                          OR (
                              length(channel_plugin_id) = 36
                              AND lower(channel_plugin_id) = channel_plugin_id
                              AND channel_plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
                              AND replace(channel_plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                          )
                      ),
    display_name      TEXT,
    requested_at      INTEGER NOT NULL,
    expires_at        INTEGER NOT NULL,
    status            TEXT NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending', 'approved', 'rejected', 'expired'))
);

CREATE TABLE channel_pending_prompts (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    prompt_id           TEXT NOT NULL UNIQUE
                        CHECK (
                            length(prompt_id) = 36
                            AND lower(prompt_id) = prompt_id
                            AND prompt_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(prompt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    channel_plugin_id   TEXT NOT NULL
                        CHECK (
                            length(channel_plugin_id) = 36
                            AND lower(channel_plugin_id) = channel_plugin_id
                            AND channel_plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(channel_plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    chat_id             TEXT NOT NULL CHECK (length(chat_id) BETWEEN 1 AND 512),
    channel_session_id  TEXT NOT NULL
                        CHECK (
                            length(channel_session_id) = 36
                            AND lower(channel_session_id) = channel_session_id
                            AND channel_session_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(channel_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    conversation_id     TEXT NOT NULL
                        CHECK (
                            length(conversation_id) = 36
                            AND lower(conversation_id) = conversation_id
                            AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    text                TEXT NOT NULL,
    idempotency_key     TEXT NOT NULL,
    state               TEXT NOT NULL DEFAULT 'queued'
                        CHECK (state IN ('queued','delivered','expired','cancelled','failed')),
    attempts            INTEGER NOT NULL DEFAULT 0,
    queued_at           INTEGER NOT NULL,
    settled_at          INTEGER
);

CREATE TABLE "channel_plugins" (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_plugin_id TEXT NOT NULL UNIQUE
                      CHECK (
                          length(channel_plugin_id) = 36
                          AND lower(channel_plugin_id) = channel_plugin_id
                          AND channel_plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(channel_plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    type              TEXT NOT NULL,
    name              TEXT NOT NULL,
    enabled           INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    config            TEXT NOT NULL,
    status            TEXT,
    last_connected    INTEGER,
    companion_id      TEXT,
    bot_key           TEXT,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL, owner_domain TEXT NOT NULL DEFAULT 'companion'
    CHECK (owner_domain IN ('companion', 'customer_service')), group_access_mode TEXT NOT NULL DEFAULT 'allowlist'
        CHECK (group_access_mode IN ('all_members', 'allowlist', 'disabled')),
    CHECK (
        companion_id IS NULL
        OR (
            length(companion_id) = 36
            AND lower(companion_id) = companion_id
            AND companion_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(companion_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    )
);

CREATE TABLE channel_session_bindings (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_plugin_id   TEXT NOT NULL
                        CHECK (
                            length(channel_plugin_id) = 36
                            AND lower(channel_plugin_id) = channel_plugin_id
                            AND channel_plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(channel_plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    channel_user_id     TEXT NOT NULL
                        CHECK (
                            length(channel_user_id) = 36
                            AND lower(channel_user_id) = channel_user_id
                            AND channel_user_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(channel_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    chat_id             TEXT NOT NULL CHECK (length(chat_id) BETWEEN 1 AND 512),
    channel_session_id  TEXT NOT NULL UNIQUE
                        CHECK (
                            length(channel_session_id) = 36
                            AND lower(channel_session_id) = channel_session_id
                            AND channel_session_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(channel_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    created_at          INTEGER NOT NULL,
    UNIQUE (channel_plugin_id, channel_user_id, chat_id)
);

CREATE TABLE channel_sessions (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_session_id TEXT NOT NULL UNIQUE
                       CHECK (
                           length(channel_session_id) = 36
                           AND lower(channel_session_id) = channel_session_id
                           AND channel_session_id GLOB '????????-????-7???-[89ab]???-????????????'
                           AND replace(channel_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                       ),
    channel_user_id    TEXT NOT NULL
                       CHECK (
                           length(channel_user_id) = 36
                           AND lower(channel_user_id) = channel_user_id
                           AND channel_user_id GLOB '????????-????-7???-[89ab]???-????????????'
                           AND replace(channel_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                       ),
    agent_type         TEXT NOT NULL
                       CHECK (agent_type IN (
                           'acp',
                           'openclaw-gateway',
                           'nanobot',
                           'remote',
                           'nomi'
                       )),
    conversation_id    TEXT,
    workspace          TEXT,
    chat_id            TEXT,
    channel_plugin_id  TEXT
                       CHECK (
                           channel_plugin_id IS NULL
                           OR (
                               length(channel_plugin_id) = 36
                               AND lower(channel_plugin_id) = channel_plugin_id
                               AND channel_plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
                               AND replace(channel_plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                           )
                       ),
    created_at         INTEGER NOT NULL,
    last_activity      INTEGER NOT NULL, chat_kind TEXT NOT NULL DEFAULT 'unknown'
        CHECK (chat_kind IN ('unknown', 'direct', 'group')),
    CHECK (conversation_id IS NULL OR (length(conversation_id) = 36 AND lower(conversation_id) = conversation_id AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'))
);

CREATE TABLE channel_users (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_user_id    TEXT NOT NULL UNIQUE
                       CHECK (
                           length(channel_user_id) = 36
                           AND lower(channel_user_id) = channel_user_id
                           AND channel_user_id GLOB '????????-????-7???-[89ab]???-????????????'
                           AND replace(channel_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                       ),
    platform_user_id   TEXT NOT NULL,
    platform_type      TEXT NOT NULL,
    channel_plugin_id  TEXT
                       CHECK (
                           channel_plugin_id IS NULL
                           OR (
                               length(channel_plugin_id) = 36
                               AND lower(channel_plugin_id) = channel_plugin_id
                               AND channel_plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
                               AND replace(channel_plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                           )
                       ),
    display_name       TEXT,
    authorized_at      INTEGER NOT NULL,
    last_active        INTEGER, authorization_kind TEXT NOT NULL DEFAULT 'approved'
        CHECK (authorization_kind IN ('approved', 'auto_group')),
    UNIQUE (platform_user_id, platform_type, channel_plugin_id)
);

CREATE TABLE client_preferences (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    key        TEXT NOT NULL UNIQUE,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE conversation_execution_links (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id      TEXT NOT NULL,
    execution_id         TEXT NOT NULL,
    relation             TEXT NOT NULL CHECK (relation IN ('lead', 'attempt', 'automation')),
    step_id              TEXT
                         CHECK (
                             step_id IS NULL
                             OR (
                                 length(step_id) = 36
                                 AND lower(step_id) = step_id
                                 AND step_id GLOB '????????-????-7???-[89ab]???-????????????'
                                 AND replace(step_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                             )
                         ),
    attempt_id           TEXT
                         CHECK (
                             attempt_id IS NULL
                             OR (
                                 length(attempt_id) = 36
                                 AND lower(attempt_id) = attempt_id
                                 AND attempt_id GLOB '????????-????-7???-[89ab]???-????????????'
                                 AND replace(attempt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                             )
                         ),
    active               INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    cleanup_completed_at INTEGER,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    CHECK (
        (relation = 'lead' AND step_id IS NULL AND attempt_id IS NULL)
        OR (relation IN ('attempt', 'automation') AND step_id IS NOT NULL AND attempt_id IS NOT NULL)
    ),
    CHECK (length(conversation_id) = 36 AND lower(conversation_id) = conversation_id AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    CHECK (length(execution_id) = 36 AND lower(execution_id) = execution_id AND execution_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(execution_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE "creation_tasks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    creation_task_id TEXT NOT NULL UNIQUE
        CHECK (
            length(creation_task_id) = 36
            AND lower(creation_task_id) = creation_task_id
            AND creation_task_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(creation_task_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    conversation_id TEXT CHECK (conversation_id IS NULL OR (
        length(conversation_id) = 36 AND lower(conversation_id) = conversation_id
        AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    message_id TEXT CHECK (message_id IS NULL OR (
        length(message_id) = 36 AND lower(message_id) = message_id
        AND message_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(message_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    project_id TEXT
        CHECK (
            project_id IS NULL
            OR (
                length(project_id) = 36
                AND lower(project_id) = project_id
                AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    template_id TEXT
        CHECK (
            template_id IS NULL
            OR (
                length(template_id) = 36
                AND lower(template_id) = template_id
                AND template_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(template_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    template_run_id TEXT
        CHECK (
            template_run_id IS NULL
            OR (
                length(template_run_id) = 36
                AND lower(template_run_id) = template_run_id
                AND template_run_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(template_run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    template_step_id TEXT
        CHECK (
            template_step_id IS NULL
            OR (
                length(template_step_id) = 36
                AND lower(template_step_id) = template_step_id
                AND template_step_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(template_step_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    node_id TEXT
        CHECK (
            node_id IS NULL
            OR (
                length(node_id) = 36
                AND lower(node_id) = node_id
                AND node_id GLOB '????????-????-7???-[89ab]???-????????????'
                AND replace(node_id, '-', '') NOT GLOB '*[^0-9a-f]*'
            )
        ),
    provider_id TEXT NOT NULL
        CHECK (
            length(provider_id) = 36
            AND lower(provider_id) = provider_id
            AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    model TEXT NOT NULL,
    capability TEXT NOT NULL,
    params TEXT NOT NULL,
    input_bindings TEXT
        CHECK (
            input_bindings IS NULL
            OR (json_valid(input_bindings) AND json_type(input_bindings) = 'array')
        ),
    status TEXT NOT NULL,
    error TEXT,
    result_asset_ids TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(result_asset_ids) AND json_type(result_asset_ids) = 'array'),
    remote_task_id TEXT,
    attempt INTEGER NOT NULL DEFAULT 0,
    submitted_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    deleted_at INTEGER
        CHECK (
            deleted_at IS NULL
            OR (
                deleted_at >= 0
                AND deleted_at >= submitted_at
                AND conversation_id IS NOT NULL AND message_id IS NOT NULL
                AND node_id IS NULL
                AND template_id IS NULL
                AND template_run_id IS NULL
                AND template_step_id IS NULL
                AND status IN ('failed', 'canceled', 'succeeded')
            )
        ),
    request_fingerprint TEXT NOT NULL
        CHECK (
            json_valid(request_fingerprint)
            AND json_type(request_fingerprint) = 'object'
        ),
    CHECK (
      (conversation_id IS NULL AND message_id IS NULL AND (
        -- Canvas node owner. `project_id` is the published storage name for
        -- the canvas business ID until the Canvas facade lands.
        (
            project_id IS NOT NULL
            AND node_id IS NOT NULL
            AND template_id IS NULL
            AND template_run_id IS NULL
            AND template_step_id IS NULL
        )
        OR
        -- Template step owner.
        (
            project_id IS NULL
            AND node_id IS NULL
            AND template_id IS NOT NULL
            AND template_run_id IS NOT NULL
            AND template_step_id IS NOT NULL
        )
      )) OR (conversation_id IS NOT NULL AND message_id IS NOT NULL
        AND project_id IS NULL AND node_id IS NULL
        AND template_id IS NULL AND template_run_id IS NULL AND template_step_id IS NULL)
    )
);

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

CREATE TABLE creative_studio_agent_sessions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_id        TEXT NOT NULL
                    CHECK (
                        length(owner_id) = 36
                        AND lower(owner_id) = owner_id
                        AND owner_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(owner_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    project_id      TEXT NOT NULL
                    CHECK (
                        length(project_id) = 36
                        AND lower(project_id) = project_id
                        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    session_id      TEXT NOT NULL
                    CHECK (
                        length(session_id) = 36
                        AND lower(session_id) = session_id
                        AND session_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    conversation_id TEXT NOT NULL
                    CHECK (
                        length(conversation_id) = 36
                        AND lower(conversation_id) = conversation_id
                        AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (owner_id, project_id, session_id)
);

CREATE TABLE creative_studio_projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL UNIQUE
        CHECK (
            length(project_id) = 36
            AND lower(project_id) = project_id
            AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    title TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    node_count INTEGER NOT NULL DEFAULT 0 CHECK (node_count >= 0),
    connection_count INTEGER NOT NULL DEFAULT 0 CHECK (connection_count >= 0),
    document_json TEXT NOT NULL
        CHECK (json_valid(document_json))
        CHECK (
            json_type(document_json, '$.schema') = 'text'
            AND json_extract(document_json, '$.schema') = 'nomifun.creative-studio/v1'
        )
        CHECK (
            json_type(document_json, '$.projectId') = 'text'
            AND json_extract(document_json, '$.projectId') = project_id
        ),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE creative_studio_template_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    template_run_id TEXT NOT NULL UNIQUE
        CHECK (
            length(template_run_id) = 36
            AND lower(template_run_id) = template_run_id
            AND template_run_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(template_run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    template_id TEXT NOT NULL
        CHECK (
            length(template_id) = 36
            AND lower(template_id) = template_id
            AND template_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(template_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    template_revision INTEGER NOT NULL CHECK (template_revision >= 1),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    status TEXT NOT NULL
        CHECK (status IN (
            'requested', 'awaiting-review', 'queued', 'running',
            'succeeded', 'failed', 'cancelled'
        )),
    step_ids_json TEXT NOT NULL
        CHECK (
            json_valid(step_ids_json)
            AND json_type(step_ids_json) = 'array'
            AND json_array_length(step_ids_json) BETWEEN 1 AND 128
        ),
    aggregate_json TEXT NOT NULL
        CHECK (json_valid(aggregate_json) AND json_type(aggregate_json) = 'object')
        CHECK (json_extract(aggregate_json, '$.kind') = 'nomifun.creative-studio.template-run')
        CHECK (json_extract(aggregate_json, '$.version') = 1)
        CHECK (json_extract(aggregate_json, '$.revision') = revision)
        CHECK (json_extract(aggregate_json, '$.templateSnapshot.id') = template_id)
        CHECK (json_extract(aggregate_json, '$.templateSnapshot.revision') = template_revision)
        CHECK (json_extract(aggregate_json, '$.request.id') = template_run_id)
        CHECK (json_extract(aggregate_json, '$.request.templateId') = template_id)
        CHECK (json_extract(aggregate_json, '$.request.templateRevision') = template_revision)
        CHECK (json_extract(aggregate_json, '$.record.requestId') = template_run_id)
        CHECK (json_extract(aggregate_json, '$.record.templateId') = template_id)
        CHECK (json_extract(aggregate_json, '$.record.status') = status),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
);

CREATE TABLE creative_studio_templates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    template_id TEXT NOT NULL UNIQUE
        CHECK (
            length(template_id) = 36
            AND lower(template_id) = template_id
            AND template_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(template_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 120),
    description TEXT NOT NULL CHECK (length(description) <= 2000),
    category TEXT NOT NULL CHECK (length(category) <= 80),
    visibility TEXT NOT NULL CHECK (visibility IN ('private', 'public')),
    definition_json TEXT NOT NULL
        CHECK (json_valid(definition_json))
        CHECK (
            json_type(definition_json, '$.id') = 'text'
            AND json_extract(definition_json, '$.id') = template_id
        )
        CHECK (
            json_type(definition_json, '$.revision') = 'integer'
            AND json_extract(definition_json, '$.revision') = revision
        ),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
);

CREATE TABLE cron_job_runs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    cron_job_run_id TEXT NOT NULL UNIQUE
                    CHECK (
                        length(cron_job_run_id) = 36
                        AND lower(cron_job_run_id) = cron_job_run_id
                        AND cron_job_run_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(cron_job_run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    cron_job_id     TEXT NOT NULL
                    CHECK (
                        length(cron_job_id) = 36
                        AND lower(cron_job_id) = cron_job_id
                        AND cron_job_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(cron_job_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    ),
    executed_at_ms  INTEGER NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('ok', 'error', 'skipped', 'missed')),
    created_at_ms   INTEGER NOT NULL
);

CREATE TABLE cron_jobs (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    cron_job_id          TEXT NOT NULL UNIQUE
                         CHECK (
                             length(cron_job_id) = 36
                             AND lower(cron_job_id) = cron_job_id
                             AND cron_job_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(cron_job_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                         ),
    user_id              TEXT NOT NULL,
    name                 TEXT NOT NULL,
    enabled              INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    schedule_kind        TEXT NOT NULL CHECK (schedule_kind IN ('at', 'every', 'cron')),
    schedule_value       TEXT NOT NULL,
    schedule_tz          TEXT,
    schedule_description TEXT,
    payload_message      TEXT NOT NULL,
    execution_mode       TEXT NOT NULL DEFAULT 'existing'
                         CHECK (execution_mode IN ('existing', 'new_conversation')),
    agent_config         TEXT CHECK (
                             agent_config IS NULL
                             OR (json_valid(agent_config) AND json_type(agent_config) = 'object')
                         ),
    preset_id            TEXT,
    preset_revision      INTEGER,
    agent_snapshot      TEXT,
    conversation_id      TEXT,
    conversation_title   TEXT,
    agent_type           TEXT NOT NULL,
    created_by           TEXT NOT NULL CHECK (created_by IN ('user', 'agent')),
    skill_content        TEXT,
    description          TEXT,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    next_run_at          INTEGER,
    last_run_at          INTEGER,
    last_status          TEXT CHECK (last_status IN ('ok', 'error', 'skipped', 'missed')),
    last_error           TEXT,
    run_count            INTEGER NOT NULL DEFAULT 0,
    retry_count          INTEGER NOT NULL DEFAULT 0,
    max_retries          INTEGER NOT NULL DEFAULT 3 CHECK (max_retries >= 0), schedule_revision INTEGER NOT NULL DEFAULT 1
        CHECK (schedule_revision > 0),
    CHECK (conversation_id IS NULL OR (length(conversation_id) = 36 AND lower(conversation_id) = conversation_id AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (preset_id IS NULL OR (length(preset_id) = 36 AND lower(preset_id) = preset_id AND preset_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(preset_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (length(user_id) = 36 AND lower(user_id) = user_id AND user_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE cron_run_reservations (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    cron_job_run_id     TEXT NOT NULL UNIQUE
                        CHECK (
                            length(cron_job_run_id) = 36
                            AND lower(cron_job_run_id) = cron_job_run_id
                            AND cron_job_run_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(cron_job_run_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    cron_job_id         TEXT NOT NULL
                        CHECK (
                            length(cron_job_id) = 36
                            AND lower(cron_job_id) = cron_job_id
                            AND cron_job_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(cron_job_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    trigger_kind        TEXT NOT NULL CHECK (trigger_kind IN ('scheduled', 'run_now')),
    operation_key       TEXT NOT NULL UNIQUE CHECK (length(operation_key) BETWEEN 1 AND 1024),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) BETWEEN 1 AND 1024),
    schedule_revision   INTEGER CHECK (schedule_revision > 0),
    planned_at_ms       INTEGER,
    status              TEXT NOT NULL DEFAULT 'reserved'
                        CHECK (status IN ('reserved', 'ok', 'error', 'skipped', 'missed')),
    conversation_id     TEXT,
    result_error        TEXT,
    created_at_ms       INTEGER NOT NULL,
    updated_at_ms       INTEGER NOT NULL,
    settled_at_ms       INTEGER, job_projection_state TEXT NOT NULL DEFAULT 'legacy_unknown'
        CHECK (job_projection_state IN ('legacy_unknown', 'pending', 'applied')), job_projected_at_ms INTEGER,
    CHECK (
        (trigger_kind = 'scheduled' AND schedule_revision IS NOT NULL AND planned_at_ms IS NOT NULL)
        OR
        (trigger_kind = 'run_now' AND schedule_revision IS NULL AND planned_at_ms IS NULL)
    ),
    CHECK (
        (status = 'reserved' AND settled_at_ms IS NULL)
        OR
        (status <> 'reserved' AND settled_at_ms IS NOT NULL)
    ),
    CHECK (
        conversation_id IS NULL
        OR (
            length(conversation_id) = 36
            AND lower(conversation_id) = conversation_id
            AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    )
);

CREATE TABLE cs_agent_capability_receipts (
    id                         INTEGER PRIMARY KEY AUTOINCREMENT,
    cs_agent_capability_receipt_id TEXT NOT NULL UNIQUE
                               CHECK (
                                   length(cs_agent_capability_receipt_id) = 36
                                   AND lower(cs_agent_capability_receipt_id) = cs_agent_capability_receipt_id
                                   AND cs_agent_capability_receipt_id GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(cs_agent_capability_receipt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                               ),
    owner_user_id              TEXT NOT NULL
                               CHECK (
                                   length(owner_user_id) = 36
                                   AND lower(owner_user_id) = owner_user_id
                                   AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                               ),
    cs_agent_id                TEXT NOT NULL
                               CHECK (
                                   length(cs_agent_id) = 36
                                   AND lower(cs_agent_id) = cs_agent_id
                                   AND cs_agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(cs_agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                               ),
    capability_id              TEXT NOT NULL CHECK (length(capability_id) BETWEEN 1 AND 256),
    idempotency_key            TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 512),
    request_digest             TEXT NOT NULL
                               CHECK (
                                   length(request_digest) = 64
                                   AND lower(request_digest) = request_digest
                                   AND request_digest NOT GLOB '*[^0-9a-f]*'
                               ),
    result_json                TEXT NOT NULL CHECK (json_valid(result_json)),
    created_at                 INTEGER NOT NULL,
    UNIQUE(owner_user_id, capability_id, idempotency_key)
);

CREATE TABLE cs_agents (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    cs_agent_id          TEXT NOT NULL UNIQUE
                         CHECK (
                             length(cs_agent_id) = 36
                             AND lower(cs_agent_id) = cs_agent_id
                             AND cs_agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(cs_agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                         ),
    name                 TEXT NOT NULL,
    greeting             TEXT NOT NULL DEFAULT '',
    persona              TEXT NOT NULL DEFAULT '',
    service_policy       TEXT NOT NULL DEFAULT '',
    provider_id          TEXT
                         CHECK (
                             provider_id IS NULL
                             OR (
                                 length(provider_id) = 36
                                 AND lower(provider_id) = provider_id
                                 AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
                                 AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                             )
                         ),
    model                TEXT,
    knowledge_base_ids   TEXT NOT NULL DEFAULT '[]'
                         CHECK (json_valid(knowledge_base_ids) AND json_type(knowledge_base_ids) = 'array'),
    enabled              INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    max_concurrent       INTEGER NOT NULL DEFAULT 8 CHECK (max_concurrent BETWEEN 1 AND 64),
    audit_retention_days INTEGER NOT NULL DEFAULT 30 CHECK (audit_retention_days >= 1),
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE TABLE cs_audit_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    cs_agent_id TEXT NOT NULL
                CHECK (
                    length(cs_agent_id) = 36
                    AND lower(cs_agent_id) = cs_agent_id
                    AND cs_agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                    AND replace(cs_agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                ),
    kind        TEXT NOT NULL,
    platform    TEXT NOT NULL DEFAULT '',
    detail      TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL
);

CREATE TABLE cs_channel_bindings (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    cs_agent_id       TEXT NOT NULL
                      CHECK (
                          length(cs_agent_id) = 36
                          AND lower(cs_agent_id) = cs_agent_id
                          AND cs_agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(cs_agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    channel_plugin_id TEXT NOT NULL
                      CHECK (
                          length(channel_plugin_id) = 36
                          AND lower(channel_plugin_id) = channel_plugin_id
                          AND channel_plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(channel_plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    created_at        INTEGER NOT NULL
);

CREATE TABLE cs_dialogues (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    cs_dialogue_id    TEXT NOT NULL UNIQUE
                      CHECK (
                          length(cs_dialogue_id) = 36
                          AND lower(cs_dialogue_id) = cs_dialogue_id
                          AND cs_dialogue_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(cs_dialogue_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    cs_agent_id       TEXT NOT NULL
                      CHECK (
                          length(cs_agent_id) = 36
                          AND lower(cs_agent_id) = cs_agent_id
                          AND cs_agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(cs_agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    channel_plugin_id TEXT NOT NULL
                      CHECK (
                          length(channel_plugin_id) = 36
                          AND lower(channel_plugin_id) = channel_plugin_id
                          AND channel_plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(channel_plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    channel_user_id   TEXT NOT NULL
                      CHECK (
                          length(channel_user_id) = 36
                          AND lower(channel_user_id) = channel_user_id
                          AND channel_user_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(channel_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    chat_id           TEXT NOT NULL,
    state             TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'closed')),
    created_at        INTEGER NOT NULL,
    last_activity     INTEGER NOT NULL
);

CREATE TABLE cs_handoffs (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    cs_handoff_id     TEXT NOT NULL UNIQUE
                      CHECK (
                          length(cs_handoff_id) = 36
                          AND lower(cs_handoff_id) = cs_handoff_id
                          AND cs_handoff_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(cs_handoff_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    cs_agent_id       TEXT NOT NULL
                      CHECK (
                          length(cs_agent_id) = 36
                          AND lower(cs_agent_id) = cs_agent_id
                          AND cs_agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(cs_agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    cs_dialogue_id    TEXT NOT NULL
                      CHECK (
                          length(cs_dialogue_id) = 36
                          AND lower(cs_dialogue_id) = cs_dialogue_id
                          AND cs_dialogue_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(cs_dialogue_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    requested_by      TEXT NOT NULL
                      CHECK (
                          length(requested_by) = 36
                          AND lower(requested_by) = requested_by
                          AND requested_by GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(requested_by, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    idempotency_key   TEXT NOT NULL UNIQUE CHECK (length(idempotency_key) BETWEEN 1 AND 512),
    reason            TEXT NOT NULL DEFAULT '' CHECK (length(reason) <= 4000),
    summary           TEXT NOT NULL DEFAULT '' CHECK (length(summary) <= 12000),
    status            TEXT NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending', 'claimed', 'resolved', 'cancelled')),
    claimed_by        TEXT
                      CHECK (
                          claimed_by IS NULL
                          OR (
                              length(claimed_by) = 36
                              AND lower(claimed_by) = claimed_by
                              AND claimed_by GLOB '????????-????-7???-[89ab]???-????????????'
                              AND replace(claimed_by, '-', '') NOT GLOB '*[^0-9a-f]*'
                          )
                      ),
    updated_by        TEXT NOT NULL
                      CHECK (
                          length(updated_by) = 36
                          AND lower(updated_by) = updated_by
                          AND updated_by GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(updated_by, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    resolution        TEXT NOT NULL DEFAULT '' CHECK (length(resolution) <= 12000),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

CREATE TABLE cs_messages (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    cs_message_id  TEXT NOT NULL UNIQUE
                   CHECK (
                       length(cs_message_id) = 36
                       AND lower(cs_message_id) = cs_message_id
                       AND cs_message_id GLOB '????????-????-7???-[89ab]???-????????????'
                       AND replace(cs_message_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                   ),
    cs_dialogue_id TEXT NOT NULL
                   CHECK (
                       length(cs_dialogue_id) = 36
                       AND lower(cs_dialogue_id) = cs_dialogue_id
                       AND cs_dialogue_id GLOB '????????-????-7???-[89ab]???-????????????'
                       AND replace(cs_dialogue_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                   ),
    role           TEXT NOT NULL CHECK (role IN ('visitor', 'agent', 'system')),
    content        TEXT NOT NULL,
    created_at     INTEGER NOT NULL
);

CREATE TABLE cs_notes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    cs_note_id  TEXT NOT NULL UNIQUE
                CHECK (
                    length(cs_note_id) = 36
                    AND lower(cs_note_id) = cs_note_id
                    AND cs_note_id GLOB '????????-????-7???-[89ab]???-????????????'
                    AND replace(cs_note_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                ),
    -- NULL = shared across every customer-service agent.
    cs_agent_id TEXT
                CHECK (
                    cs_agent_id IS NULL
                    OR (
                        length(cs_agent_id) = 36
                        AND lower(cs_agent_id) = cs_agent_id
                        AND cs_agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                        AND replace(cs_agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                    )
                ),
    kind        TEXT NOT NULL DEFAULT 'faq',
    content     TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
, search_text TEXT NOT NULL DEFAULT '', aliases TEXT NOT NULL DEFAULT '');

CREATE VIRTUAL TABLE cs_notes_fts USING fts5(
  search_text, content='cs_notes', content_rowid='id', tokenize='trigram'
);

CREATE TABLE installation_identity (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    singleton_key TEXT NOT NULL UNIQUE CHECK (singleton_key = 'installation'),
    owner_user_id TEXT NOT NULL UNIQUE,
    CHECK (length(owner_user_id) = 36 AND lower(owner_user_id) = owner_user_id AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE installation_role_bindings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    role_id TEXT NOT NULL UNIQUE CHECK (trim(role_id) <> ''),
    role_contract_ref_json TEXT NOT NULL CHECK (
        json_valid(role_contract_ref_json)
        AND json_type(role_contract_ref_json) = 'object'
        AND json_extract(role_contract_ref_json, '$.key.role_id') = role_id
    ),
    provider_mount_id TEXT NOT NULL CHECK (trim(provider_mount_id) <> ''),
    binding_version INTEGER NOT NULL CHECK (binding_version >= 1),
    updated_at INTEGER NOT NULL
);

CREATE TABLE instance_access_token (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    singleton_key TEXT NOT NULL UNIQUE CHECK (singleton_key = 'instance'),
    token_hash    TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE javascript_runtime_selection (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    singleton_key            TEXT NOT NULL UNIQUE CHECK (
        singleton_key = 'javascript_runtime_selection'
    ),
    selected_runtime_json    TEXT CHECK (
        selected_runtime_json IS NULL OR (
            json_valid(selected_runtime_json)
            AND json_type(selected_runtime_json) = 'object'
        )
    ),
    pending_candidate_json   TEXT CHECK (
        pending_candidate_json IS NULL OR (
            json_valid(pending_candidate_json)
            AND json_type(pending_candidate_json) = 'object'
        )
    ),
    validation_result_json   TEXT CHECK (
        validation_result_json IS NULL OR (
            json_valid(validation_result_json)
            AND json_type(validation_result_json) = 'object'
        )
    ),
    last_error_code          TEXT CHECK (
        last_error_code IS NULL OR (
            length(last_error_code) BETWEEN 1 AND 256
            AND last_error_code NOT GLOB '*[^!-~]*'
        )
    ),
    non_recommended_warning_acknowledged_json TEXT NOT NULL DEFAULT '[]' CHECK (
        json_valid(non_recommended_warning_acknowledged_json)
        AND json_type(non_recommended_warning_acknowledged_json) = 'array'
    ),
    revision                 INTEGER NOT NULL CHECK (revision >= 1),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= 0), selected_executable_path TEXT, pending_candidate_executable_path TEXT,
    CHECK (
        validation_result_json IS NULL
        OR pending_candidate_json IS NOT NULL
    )
);

CREATE TABLE knowledge_bases (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_base_id TEXT NOT NULL UNIQUE
                      CHECK (
                          length(knowledge_base_id) = 36
                          AND lower(knowledge_base_id) = knowledge_base_id
                          AND knowledge_base_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(knowledge_base_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    name              TEXT NOT NULL,
    description       TEXT NOT NULL DEFAULT '',
    root_path         TEXT NOT NULL,
    managed           INTEGER NOT NULL DEFAULT 1 CHECK (managed IN (0, 1)),
    extra             TEXT NOT NULL DEFAULT '{}',
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    tags              TEXT
, tree_revision INTEGER NOT NULL DEFAULT 0
    CHECK (tree_revision >= 0), tree_access TEXT NOT NULL DEFAULT 'editable'
    CHECK (tree_access IN ('editable', 'read_only')));

CREATE TABLE knowledge_binding_bases (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_binding_id TEXT NOT NULL
                         CHECK (
                             length(knowledge_binding_id) = 36
                             AND lower(knowledge_binding_id) = knowledge_binding_id
                             AND knowledge_binding_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(knowledge_binding_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                         ),
    knowledge_base_id    TEXT NOT NULL
                         CHECK (
                             length(knowledge_base_id) = 36
                             AND lower(knowledge_base_id) = knowledge_base_id
                             AND knowledge_base_id GLOB '????????-????-7???-[89ab]???-????????????'
                             AND replace(knowledge_base_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                         ),
    position             INTEGER NOT NULL DEFAULT 0,
    UNIQUE (knowledge_binding_id, knowledge_base_id)
);

CREATE TABLE knowledge_bindings (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_binding_id   TEXT NOT NULL UNIQUE
                           CHECK (
                               length(knowledge_binding_id) = 36
                               AND lower(knowledge_binding_id) = knowledge_binding_id
                               AND knowledge_binding_id GLOB '????????-????-7???-[89ab]???-????????????'
                               AND replace(knowledge_binding_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                           ),
    target_kind            TEXT NOT NULL,
    target_workpath        TEXT,
    target_conversation_id TEXT
                           CHECK (
                               target_conversation_id IS NULL
                               OR (
                                   length(target_conversation_id) = 36
                                   AND lower(target_conversation_id) = target_conversation_id
                                   AND target_conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(target_conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                               )
                           ),
    target_terminal_id     TEXT
                           CHECK (
                               target_terminal_id IS NULL
                               OR (
                                   length(target_terminal_id) = 36
                                   AND lower(target_terminal_id) = target_terminal_id
                                   AND target_terminal_id GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(target_terminal_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                               )
                           ),
    target_companion_id    TEXT,
    enabled                INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    writeback              INTEGER NOT NULL DEFAULT 0 CHECK (writeback IN (0, 1)),
    updated_at             INTEGER NOT NULL,
    channel_write_enabled  INTEGER NOT NULL DEFAULT 0
                           CHECK (channel_write_enabled IN (0, 1)), writeback_eagerness TEXT NOT NULL
    DEFAULT 'manual' CHECK (writeback_eagerness IN ('manual', 'auto')),
    CHECK (
        (target_kind = 'workpath' AND target_workpath IS NOT NULL
            AND target_conversation_id IS NULL AND target_terminal_id IS NULL AND target_companion_id IS NULL)
        OR (target_kind = 'conversation' AND target_conversation_id IS NOT NULL
            AND target_workpath IS NULL AND target_terminal_id IS NULL AND target_companion_id IS NULL)
        OR (target_kind = 'terminal' AND target_terminal_id IS NOT NULL
            AND target_workpath IS NULL AND target_conversation_id IS NULL AND target_companion_id IS NULL)
        OR (target_kind = 'companion' AND target_companion_id IS NOT NULL
            AND target_workpath IS NULL AND target_conversation_id IS NULL AND target_terminal_id IS NULL)
    ),
    CHECK (
        target_companion_id IS NULL
        OR (
            length(target_companion_id) = 36
            AND lower(target_companion_id) = target_companion_id
            AND target_companion_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(target_companion_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    )
);

CREATE TABLE knowledge_entries (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_entry_id  TEXT NOT NULL UNIQUE
                        CHECK (
                            length(knowledge_entry_id) = 36
                            AND lower(knowledge_entry_id) = knowledge_entry_id
                            AND knowledge_entry_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(knowledge_entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    knowledge_base_id   TEXT NOT NULL
                        CHECK (
                            length(knowledge_base_id) = 36
                            AND lower(knowledge_base_id) = knowledge_base_id
                            AND knowledge_base_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(knowledge_base_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    parent_entry_id     TEXT
                        CHECK (
                            parent_entry_id IS NULL
                            OR (
                                length(parent_entry_id) = 36
                                AND lower(parent_entry_id) = parent_entry_id
                                AND parent_entry_id GLOB '????????-????-7???-[89ab]???-????????????'
                                AND replace(parent_entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                            )
                        ),
    name                TEXT NOT NULL
                        CHECK (
                            name <> ''
                            AND name NOT IN ('.', '..')
                            AND instr(name, '/') = 0
                            AND instr(name, '\') = 0
                            AND instr(name, char(0)) = 0
                        ),
    kind                TEXT NOT NULL CHECK (kind IN ('file', 'directory')),
    origin              TEXT NOT NULL CHECK (origin IN ('user', 'url_snapshot', 'generated')),
    rel_path            TEXT NOT NULL
                        CHECK (
                            rel_path <> ''
                            AND substr(rel_path, 1, 1) <> '/'
                            AND substr(rel_path, -1, 1) <> '/'
                            AND instr(rel_path, '\') = 0
                            AND instr(rel_path, '//') = 0
                            AND instr(rel_path, char(0)) = 0
                        ),
    portable_rel_path   TEXT NOT NULL
                        CHECK (
                            portable_rel_path <> ''
                            AND substr(portable_rel_path, 1, 1) <> '/'
                            AND substr(portable_rel_path, -1, 1) <> '/'
                            AND instr(portable_rel_path, '\') = 0
                            AND instr(portable_rel_path, '//') = 0
                            AND instr(portable_rel_path, char(0)) = 0
                        ),
    fs_identity         TEXT,
    content_hash        TEXT,
    revision            INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    deleted_at          INTEGER,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    CHECK (parent_entry_id IS NULL OR parent_entry_id <> knowledge_entry_id),
    CHECK (deleted_at IS NULL OR deleted_at >= created_at),
    CHECK (updated_at >= created_at)
);

CREATE TABLE knowledge_entry_provenance (
    id                        INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_entry_id        TEXT NOT NULL
                              CHECK (
                                  length(knowledge_entry_id) = 36
                                  AND lower(knowledge_entry_id) = knowledge_entry_id
                                  AND knowledge_entry_id GLOB '????????-????-7???-[89ab]???-????????????'
                                  AND replace(knowledge_entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                              ),
    knowledge_source_item_id  TEXT NOT NULL
                              CHECK (
                                  length(knowledge_source_item_id) = 36
                                  AND lower(knowledge_source_item_id) = knowledge_source_item_id
                                  AND knowledge_source_item_id GLOB '????????-????-7???-[89ab]???-????????????'
                                  AND replace(knowledge_source_item_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                              ),
    relationship              TEXT NOT NULL CHECK (relationship IN ('managed', 'detached', 'copy')),
    derived_from_entry_id     TEXT
                              CHECK (
                                  derived_from_entry_id IS NULL
                                  OR (
                                      length(derived_from_entry_id) = 36
                                      AND lower(derived_from_entry_id) = derived_from_entry_id
                                      AND derived_from_entry_id GLOB '????????-????-7???-[89ab]???-????????????'
                                      AND replace(derived_from_entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                                  )
                              ),
    revision                  INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    detached_at               INTEGER CHECK (detached_at IS NULL OR detached_at >= 0),
    created_at                INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at                INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (derived_from_entry_id IS NULL OR derived_from_entry_id <> knowledge_entry_id),
    CHECK (
        (relationship = 'managed' AND derived_from_entry_id IS NULL AND detached_at IS NULL)
        OR (relationship = 'detached' AND derived_from_entry_id IS NULL AND detached_at IS NOT NULL)
        OR (relationship = 'copy' AND derived_from_entry_id IS NOT NULL AND detached_at IS NULL)
    )
);

CREATE TABLE knowledge_source_items (
    id                        INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_source_item_id  TEXT NOT NULL UNIQUE
                              CHECK (
                                  length(knowledge_source_item_id) = 36
                                  AND lower(knowledge_source_item_id) = knowledge_source_item_id
                                  AND knowledge_source_item_id GLOB '????????-????-7???-[89ab]???-????????????'
                                  AND replace(knowledge_source_item_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                              ),
    knowledge_source_id       TEXT NOT NULL
                              CHECK (
                                  length(knowledge_source_id) = 36
                                  AND lower(knowledge_source_id) = knowledge_source_id
                                  AND knowledge_source_id GLOB '????????-????-7???-[89ab]???-????????????'
                                  AND replace(knowledge_source_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                              ),
    requested_url             TEXT NOT NULL
                              CHECK (
                                  length(requested_url) BETWEEN 1 AND 8192
                                  AND trim(requested_url) = requested_url
                                  AND instr(requested_url, char(0)) = 0
                              ),
    normalized_url            TEXT NOT NULL
                              CHECK (
                                  length(normalized_url) BETWEEN 1 AND 8192
                                  AND trim(normalized_url) = normalized_url
                                  AND instr(normalized_url, char(0)) = 0
                              ),
    final_url                 TEXT
                              CHECK (
                                  final_url IS NULL
                                  OR (
                                      length(final_url) BETWEEN 1 AND 8192
                                      AND trim(final_url) = final_url
                                      AND instr(final_url, char(0)) = 0
                                  )
                              ),
    rendered                  INTEGER NOT NULL DEFAULT 0 CHECK (rendered IN (0, 1)),
    title                     TEXT
                              CHECK (
                                  title IS NULL
                                  OR (
                                      length(title) BETWEEN 1 AND 1024
                                      AND trim(title) = title
                                      AND instr(title, char(0)) = 0
                                  )
                              ),
    ordinal                   INTEGER NOT NULL CHECK (ordinal >= 0),
    state                     TEXT NOT NULL CHECK (state IN ('active', 'paused', 'removed')),
    sync_status               TEXT NOT NULL CHECK (sync_status IN (
                                  'pending',
                                  'syncing',
                                  'synced',
                                  'failed',
                                  'conflicted',
                                  'missing'
                              )),
    revision                  INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    etag                      TEXT
                              CHECK (
                                  etag IS NULL
                                  OR (length(etag) BETWEEN 1 AND 4096 AND instr(etag, char(0)) = 0)
                              ),
    http_last_modified        TEXT
                              CHECK (
                                  http_last_modified IS NULL
                                  OR (
                                      length(http_last_modified) BETWEEN 1 AND 512
                                      AND instr(http_last_modified, char(0)) = 0
                                  )
                              ),
    last_attempt_at           INTEGER CHECK (last_attempt_at IS NULL OR last_attempt_at >= 0),
    last_success_at           INTEGER CHECK (last_success_at IS NULL OR last_success_at >= 0),
    last_error                TEXT
                              CHECK (
                                  last_error IS NULL
                                  OR (length(last_error) BETWEEN 1 AND 8192 AND instr(last_error, char(0)) = 0)
                              ),
    last_published_hash       TEXT
                              CHECK (
                                  last_published_hash IS NULL
                                  OR (
                                      length(last_published_hash) = 64
                                      AND lower(last_published_hash) = last_published_hash
                                      AND last_published_hash NOT GLOB '*[^0-9a-f]*'
                                  )
                              ),
    removed_at                INTEGER,
    created_at                INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at                INTEGER NOT NULL CHECK (updated_at >= created_at), pending_published_hash TEXT
    CHECK (
        pending_published_hash IS NULL
        OR (
            length(pending_published_hash) = 64
            AND lower(pending_published_hash) = pending_published_hash
            AND pending_published_hash NOT GLOB '*[^0-9a-f]*'
            AND state = 'active'
            AND sync_status = 'syncing'
        )
    ), pending_final_url TEXT
    CHECK (
        pending_final_url IS NULL
        OR (
            pending_published_hash IS NOT NULL
            AND length(pending_final_url) BETWEEN 1 AND 8192
            AND trim(pending_final_url) = pending_final_url
            AND instr(pending_final_url, char(0)) = 0
        )
    ), pending_title TEXT
    CHECK (
        pending_title IS NULL
        OR (
            pending_published_hash IS NOT NULL
            AND length(pending_title) BETWEEN 1 AND 1024
            AND trim(pending_title) = pending_title
            AND instr(pending_title, char(0)) = 0
        )
    ), pending_publication_at INTEGER
    CHECK (
        (pending_published_hash IS NULL AND pending_publication_at IS NULL)
        OR (
            pending_published_hash IS NOT NULL
            AND pending_publication_at IS NOT NULL
            AND pending_publication_at >= 0
        )
    ),
    CHECK (removed_at IS NULL OR removed_at >= created_at),
    CHECK (state <> 'removed' OR sync_status <> 'syncing'),
    CHECK (sync_status <> 'syncing' OR last_attempt_at IS NOT NULL),
    CHECK (
        (state = 'removed' AND removed_at IS NOT NULL)
        OR (state <> 'removed' AND removed_at IS NULL)
    )
);

CREATE TABLE knowledge_sources (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_source_id      TEXT NOT NULL UNIQUE
                             CHECK (
                                 length(knowledge_source_id) = 36
                                 AND lower(knowledge_source_id) = knowledge_source_id
                                 AND knowledge_source_id GLOB '????????-????-7???-[89ab]???-????????????'
                                 AND replace(knowledge_source_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                             ),
    knowledge_base_id        TEXT NOT NULL
                             CHECK (
                                 length(knowledge_base_id) = 36
                                 AND lower(knowledge_base_id) = knowledge_base_id
                                 AND knowledge_base_id GLOB '????????-????-7???-[89ab]???-????????????'
                                 AND replace(knowledge_base_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                             ),
    kind                     TEXT NOT NULL CHECK (kind = 'url'),
    mode                     TEXT NOT NULL CHECK (mode IN ('live', 'snapshot')),
    state                    TEXT NOT NULL CHECK (state IN ('active', 'paused', 'removed')),
    revision                 INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    default_parent_entry_id  TEXT
                             CHECK (
                                 default_parent_entry_id IS NULL
                                 OR (
                                     length(default_parent_entry_id) = 36
                                     AND lower(default_parent_entry_id) = default_parent_entry_id
                                     AND default_parent_entry_id GLOB '????????-????-7???-[89ab]???-????????????'
                                     AND replace(default_parent_entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                                 )
                             ),
    removed_at               INTEGER,
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (removed_at IS NULL OR removed_at >= created_at),
    CHECK (state <> 'removed' OR default_parent_entry_id IS NULL),
    CHECK (
        (state = 'removed' AND removed_at IS NOT NULL)
        OR (state <> 'removed' AND removed_at IS NULL)
    )
);

CREATE TABLE knowledge_tags (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    key        TEXT NOT NULL UNIQUE,
    label      TEXT NOT NULL,
    color      TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE knowledge_tree_operations (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id             TEXT NOT NULL UNIQUE
                             CHECK (
                                 length(operation_id) = 36
                                 AND lower(operation_id) = operation_id
                                 AND operation_id GLOB '????????-????-7???-[89ab]???-????????????'
                                 AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                             ),
    knowledge_base_id        TEXT NOT NULL
                             CHECK (
                                 length(knowledge_base_id) = 36
                                 AND lower(knowledge_base_id) = knowledge_base_id
                                 AND knowledge_base_id GLOB '????????-????-7???-[89ab]???-????????????'
                                 AND replace(knowledge_base_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                             ),
    request_id               TEXT NOT NULL
                             CHECK (
                                 length(request_id) BETWEEN 1 AND 128
                                 AND request_id NOT GLOB '*[^!-~]*'
                             ),
    fingerprint              TEXT NOT NULL
                             CHECK (
                                 length(fingerprint) = 64
                                 AND lower(fingerprint) = fingerprint
                                 AND fingerprint NOT GLOB '*[^0-9a-f]*'
                             ),
    source_rel_path          TEXT NOT NULL
                             CHECK (
                                 source_rel_path <> ''
                                 AND substr(source_rel_path, 1, 1) <> '/'
                                 AND substr(source_rel_path, -1, 1) <> '/'
                                 AND instr(source_rel_path, '\') = 0
                                 AND instr(source_rel_path, '//') = 0
                                 AND instr(source_rel_path, char(0)) = 0
                             ),
    destination_rel_path     TEXT NOT NULL
                             CHECK (
                                 destination_rel_path <> ''
                                 AND substr(destination_rel_path, 1, 1) <> '/'
                                 AND substr(destination_rel_path, -1, 1) <> '/'
                                 AND instr(destination_rel_path, '\') = 0
                                 AND instr(destination_rel_path, '//') = 0
                                 AND instr(destination_rel_path, char(0)) = 0
                             ),
    -- Physical identity observed while intent is prepared. On platforms that
    -- expose inode/file-index identity, recovery uses this to distinguish our
    -- completed rename from an unrelated file later occupying the target.
    source_fs_identity       TEXT CHECK (
                                 source_fs_identity IS NULL
                                 OR length(source_fs_identity) BETWEEN 1 AND 512
                             ),
    state                    TEXT NOT NULL DEFAULT 'prepared'
                             CHECK (state IN (
                                 'prepared',
                                 'filesystem_committed',
                                 'committed',
                                 'needs_recovery'
                             )),
    receipt_json             TEXT
                             CHECK (receipt_json IS NULL OR json_valid(receipt_json)),
    error_message            TEXT
                             CHECK (
                                 error_message IS NULL
                                 OR length(error_message) BETWEEN 1 AND 8192
                             ),
    event_status             TEXT NOT NULL DEFAULT 'none'
                             CHECK (event_status IN ('none', 'pending', 'published')),
    event_payload_json       TEXT
                             CHECK (
                                 event_payload_json IS NULL
                                 OR json_valid(event_payload_json)
                             ),
    filesystem_committed_at  INTEGER,
    committed_at             INTEGER,
    event_published_at       INTEGER,
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at),

    UNIQUE (knowledge_base_id, request_id),

    CHECK (
        filesystem_committed_at IS NULL
        OR filesystem_committed_at >= created_at
    ),
    CHECK (
        committed_at IS NULL
        OR (
            filesystem_committed_at IS NOT NULL
            AND committed_at >= filesystem_committed_at
        )
    ),
    CHECK (
        event_published_at IS NULL
        OR (
            committed_at IS NOT NULL
            AND event_published_at >= committed_at
        )
    ),
    CHECK (
        (state = 'prepared'
            AND filesystem_committed_at IS NULL
            AND committed_at IS NULL
            AND receipt_json IS NULL
            AND error_message IS NULL)
        OR
        (state = 'filesystem_committed'
            AND filesystem_committed_at IS NOT NULL
            AND committed_at IS NULL
            AND receipt_json IS NULL
            AND error_message IS NULL)
        OR
        (state = 'committed'
            AND filesystem_committed_at IS NOT NULL
            AND committed_at IS NOT NULL
            AND receipt_json IS NOT NULL
            AND error_message IS NULL)
        OR
        (state = 'needs_recovery'
            AND committed_at IS NULL
            AND receipt_json IS NULL
            AND error_message IS NOT NULL)
    ),
    CHECK (
        (state = 'committed'
            AND event_status IN ('pending', 'published')
            AND event_payload_json IS NOT NULL)
        OR
        (state <> 'committed'
            AND event_status = 'none'
            AND event_payload_json IS NULL
            AND event_published_at IS NULL)
    ),
    CHECK (
        (event_status = 'published' AND event_published_at IS NOT NULL)
        OR
        (event_status <> 'published' AND event_published_at IS NULL)
    )
);

CREATE TABLE mcp_servers (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    mcp_server_id    TEXT NOT NULL UNIQUE
                     CHECK (
                         length(mcp_server_id) = 36
                         AND lower(mcp_server_id) = mcp_server_id
                         AND mcp_server_id GLOB '????????-????-7???-[89ab]???-????????????'
                         AND replace(mcp_server_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                     ),
    name             TEXT NOT NULL UNIQUE,
    description      TEXT,
    enabled          INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    transport_type   TEXT NOT NULL,
    transport_config TEXT NOT NULL,
    tools            TEXT,
    last_test_status TEXT NOT NULL DEFAULT 'disconnected',
    last_connected   INTEGER,
    original_json    TEXT,
    builtin          INTEGER NOT NULL DEFAULT 0 CHECK (builtin IN (0, 1)),
    deleted_at       INTEGER,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

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

CREATE TABLE nomi_wave1_memory_action_receipts (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_user_id     TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    agent_session_id  TEXT NOT NULL CHECK (
        length(agent_session_id) = 36
        AND lower(agent_session_id) = agent_session_id
        AND agent_session_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(agent_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    capability_id     TEXT NOT NULL CHECK (
        length(capability_id) BETWEEN 1 AND 256
        AND capability_id NOT GLOB '*[^!-~]*'
    ),
    idempotency_key   TEXT NOT NULL CHECK (
        length(idempotency_key) BETWEEN 1 AND 256
        AND idempotency_key NOT GLOB '*[^!-~]*'
    ),
    request_digest    TEXT NOT NULL CHECK (
        length(request_digest) = 64
        AND lower(request_digest) = request_digest
        AND request_digest NOT GLOB '*[^0-9a-f]*'
    ),
    state             TEXT NOT NULL CHECK (
        state IN ('in_flight', 'completed', 'failed', 'outcome_unknown')
    ),
    process_lease_id  TEXT CHECK (
        process_lease_id IS NULL OR (
            length(process_lease_id) = 36
            AND lower(process_lease_id) = process_lease_id
            AND process_lease_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(process_lease_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    output_json       TEXT CHECK (
        output_json IS NULL OR json_valid(output_json)
    ),
    error_code        TEXT CHECK (
        error_code IS NULL OR (
            length(error_code) BETWEEN 1 AND 256
            AND error_code NOT GLOB '*[^!-~]*'
        )
    ),
    error_message     TEXT CHECK (
        error_message IS NULL OR length(error_message) BETWEEN 1 AND 4096
    ),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (
        owner_user_id,
        agent_session_id,
        capability_id,
        idempotency_key
    ),
    CHECK (
        (state = 'in_flight'
            AND process_lease_id IS NOT NULL
            AND output_json IS NULL
            AND error_code IS NULL
            AND error_message IS NULL)
        OR
        (state = 'completed'
            AND process_lease_id IS NULL
            AND output_json IS NOT NULL
            AND error_code IS NULL
            AND error_message IS NULL)
        OR
        (state = 'failed'
            AND process_lease_id IS NULL
            AND output_json IS NULL
            AND error_code IS NOT NULL
            AND error_message IS NOT NULL)
        OR
        (state = 'outcome_unknown'
            AND process_lease_id IS NULL
            AND output_json IS NULL
            AND error_code IS NULL
            AND error_message IS NULL)
    )
);

CREATE TABLE nomi_wave4_action_receipts (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_user_id     TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    agent_session_id  TEXT NOT NULL CHECK (
        length(agent_session_id) = 36
        AND lower(agent_session_id) = agent_session_id
        AND agent_session_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(agent_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    capability_id     TEXT NOT NULL,
    idempotency_key   TEXT NOT NULL,
    request_digest    TEXT NOT NULL,
    state             TEXT NOT NULL CHECK (
        state IN ('in_flight', 'completed', 'failed', 'outcome_unknown')
    ),
    process_lease_id  TEXT,
    output_json       TEXT,
    error_code        TEXT,
    error_message     TEXT,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    UNIQUE (
        owner_user_id,
        agent_session_id,
        capability_id,
        idempotency_key
    ),
    CHECK (
        (state = 'in_flight'
            AND process_lease_id IS NOT NULL
            AND output_json IS NULL
            AND error_code IS NULL
            AND error_message IS NULL)
        OR
        (state = 'completed'
            AND process_lease_id IS NULL
            AND output_json IS NOT NULL
            AND error_code IS NULL
            AND error_message IS NULL)
        OR
        (state = 'failed'
            AND process_lease_id IS NULL
            AND output_json IS NULL
            AND error_code IS NOT NULL
            AND error_message IS NOT NULL)
        OR
        (state = 'outcome_unknown'
            AND process_lease_id IS NULL
            AND output_json IS NULL
            AND error_code IS NULL
            AND error_message IS NULL)
    )
);

CREATE TABLE oauth_tokens (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    server_url    TEXT NOT NULL UNIQUE,
    access_token  TEXT NOT NULL,
    refresh_token TEXT,
    token_type    TEXT NOT NULL DEFAULT 'bearer',
    expires_at    INTEGER,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE plugin_artifacts (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    artifact_id              TEXT NOT NULL UNIQUE CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest          TEXT NOT NULL UNIQUE CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    package_id               TEXT NOT NULL CHECK (
        length(package_id) BETWEEN 1 AND 255
        AND package_id NOT GLOB '*[^A-Za-z0-9._-]*'
    ),
    package_version          TEXT NOT NULL CHECK (
        length(package_version) BETWEEN 1 AND 128
        AND package_version NOT GLOB '*[^!-~]*'
    ),
    manifest_digest          TEXT NOT NULL CHECK (
        length(manifest_digest) = 64
        AND lower(manifest_digest) = manifest_digest
        AND manifest_digest NOT GLOB '*[^0-9a-f]*'
    ),
    manifest_json            TEXT NOT NULL CHECK (
        json_valid(manifest_json)
        AND json_type(manifest_json) = 'object'
    ),
    managed_path             TEXT NOT NULL UNIQUE CHECK (
        managed_path <> ''
        AND substr(managed_path, 1, 1) <> '/'
        AND substr(managed_path, -1, 1) <> '/'
        AND instr(managed_path, '\') = 0
        AND instr(managed_path, '//') = 0
        AND instr('/' || managed_path || '/', '/../') = 0
        AND instr('/' || managed_path || '/', '/./') = 0
        AND instr(managed_path, char(0)) = 0
    ),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0)
);

CREATE TABLE plugin_build_operation_lineage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id TEXT NOT NULL UNIQUE CHECK (
        length(operation_id) = 36
        AND lower(operation_id) = operation_id
        AND operation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    plugin_product_id TEXT NOT NULL CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    project_id TEXT NOT NULL CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    project_revision INTEGER NOT NULL CHECK (project_revision >= 1),
    source_snapshot_digest TEXT NOT NULL CHECK (
        length(source_snapshot_digest) = 64
        AND lower(source_snapshot_digest) = source_snapshot_digest
        AND source_snapshot_digest NOT GLOB '*[^0-9a-f]*'
    ),
    dependency_lock_digest TEXT NOT NULL CHECK (
        length(dependency_lock_digest) = 64
        AND lower(dependency_lock_digest) = dependency_lock_digest
        AND dependency_lock_digest NOT GLOB '*[^0-9a-f]*'
    ),
    build_profile_version TEXT NOT NULL CHECK (
        length(build_profile_version) BETWEEN 1 AND 64
        AND build_profile_version NOT GLOB '*[^!-~]*'
    ),
    build_generation INTEGER NOT NULL CHECK (build_generation >= 1),
    started_at_ms INTEGER NOT NULL CHECK (started_at_ms > 0)
);

CREATE TABLE plugin_candidate_test_receipts (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    receipt_id               TEXT NOT NULL UNIQUE CHECK (
        length(receipt_id) = 36
        AND lower(receipt_id) = receipt_id
        AND receipt_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(receipt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    candidate_id             TEXT NOT NULL UNIQUE CHECK (
        length(candidate_id) = 36
        AND lower(candidate_id) = candidate_id
        AND candidate_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(candidate_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    candidate_digest         TEXT NOT NULL CHECK (
        length(candidate_digest) = 64
        AND lower(candidate_digest) = candidate_digest
        AND candidate_digest NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_id              TEXT NOT NULL CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest          TEXT NOT NULL CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    receipt_digest           TEXT NOT NULL UNIQUE CHECK (
        length(receipt_digest) = 64
        AND lower(receipt_digest) = receipt_digest
        AND receipt_digest NOT GLOB '*[^0-9a-f]*'
    ),
    runtime_fingerprint_digest TEXT NOT NULL CHECK (
        length(runtime_fingerprint_digest) = 64
        AND lower(runtime_fingerprint_digest) = runtime_fingerprint_digest
        AND runtime_fingerprint_digest NOT GLOB '*[^0-9a-f]*'
    ),
    receipt_json             TEXT NOT NULL CHECK (
        json_valid(receipt_json)
        AND json_type(receipt_json) = 'object'
    ),
    tested_at                INTEGER NOT NULL CHECK (tested_at >= 0)
);

CREATE TABLE plugin_catalog_publications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_product_id TEXT NOT NULL UNIQUE CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    active_release_id TEXT NOT NULL CHECK (
        length(active_release_id) = 36
        AND lower(active_release_id) = active_release_id
        AND active_release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(active_release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    active_release_digest TEXT NOT NULL CHECK (
        length(active_release_digest) = 64
        AND lower(active_release_digest) = active_release_digest
        AND active_release_digest NOT GLOB '*[^0-9a-f]*'
    ),
    active_release_epoch INTEGER NOT NULL CHECK (active_release_epoch > 0),
    catalog_digest TEXT NOT NULL CHECK (
        length(catalog_digest) = 64
        AND lower(catalog_digest) = catalog_digest
        AND catalog_digest NOT GLOB '*[^0-9a-f]*'
    )
);

CREATE TABLE plugin_credential_binding_mutations (
    id                          INTEGER PRIMARY KEY AUTOINCREMENT,
    mount_id                    TEXT NOT NULL UNIQUE CHECK (
        length(mount_id) = 36
        AND lower(mount_id) = mount_id
        AND mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    expected_mount_revision    INTEGER NOT NULL CHECK (expected_mount_revision >= 0),
    expected_current_artifact_digest TEXT CHECK (
        expected_current_artifact_digest IS NULL OR (
            length(expected_current_artifact_digest) = 64
            AND lower(expected_current_artifact_digest) = expected_current_artifact_digest
            AND expected_current_artifact_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    expected_bindings_revision INTEGER NOT NULL CHECK (expected_bindings_revision >= 0),
    target_bindings_revision   INTEGER NOT NULL CHECK (target_bindings_revision >= 1),
    allow_delete_pending       INTEGER NOT NULL DEFAULT 0 CHECK (allow_delete_pending IN (0, 1)),
    updated_at                 INTEGER NOT NULL CHECK (updated_at >= 0)
);

CREATE TABLE plugin_credential_bindings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_product_id TEXT NOT NULL CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    slot_key TEXT NOT NULL CHECK (
        length(slot_key) BETWEEN 1 AND 128
        AND slot_key NOT GLOB '*[^!-~]*'
    ),
    credential_id TEXT NOT NULL CHECK (
        length(credential_id) BETWEEN 1 AND 512
        AND credential_id NOT GLOB '*[^!-~]*'
    ),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (owner_user_id, plugin_product_id, slot_key)
);

CREATE TABLE plugin_deletion_intents (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_product_id      TEXT NOT NULL UNIQUE CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id   TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    operation_id    TEXT NOT NULL UNIQUE CHECK (
        length(operation_id) = 36
        AND lower(operation_id) = operation_id
        AND operation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    started_at_ms   INTEGER NOT NULL CHECK (started_at_ms > 0),
    last_error_code TEXT CHECK (
        last_error_code IS NULL OR (
            length(last_error_code) BETWEEN 1 AND 256
            AND last_error_code NOT GLOB '*[^!-~]*'
        )
    )
);

CREATE TABLE plugin_dependency_mutation_commits (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    intent_id  TEXT NOT NULL UNIQUE CHECK (
        length(intent_id) = 36
        AND lower(intent_id) = intent_id
        AND intent_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(intent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    created_at INTEGER NOT NULL CHECK (created_at > 0)
);

CREATE TABLE plugin_dependency_mutation_intents (
    id                          INTEGER PRIMARY KEY AUTOINCREMENT,
    intent_id                   TEXT NOT NULL UNIQUE CHECK (
        length(intent_id) = 36
        AND lower(intent_id) = intent_id
        AND intent_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(intent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    project_id                  TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id               TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    expected_project_updated_at INTEGER NOT NULL CHECK (expected_project_updated_at >= 0),
    expected_build_generation   INTEGER NOT NULL CHECK (expected_build_generation >= 0),
    expected_source_digest      TEXT NOT NULL CHECK (
        length(expected_source_digest) = 64
        AND lower(expected_source_digest) = expected_source_digest
        AND expected_source_digest NOT GLOB '*[^0-9a-f]*'
    ),
    expected_lock_digest        TEXT NOT NULL CHECK (
        length(expected_lock_digest) = 64
        AND lower(expected_lock_digest) = expected_lock_digest
        AND expected_lock_digest NOT GLOB '*[^0-9a-f]*'
    ),
    next_source_digest          TEXT NOT NULL CHECK (
        length(next_source_digest) = 64
        AND lower(next_source_digest) = next_source_digest
        AND next_source_digest NOT GLOB '*[^0-9a-f]*'
    ),
    next_lock_digest            TEXT NOT NULL CHECK (
        length(next_lock_digest) = 64
        AND lower(next_lock_digest) = next_lock_digest
        AND next_lock_digest NOT GLOB '*[^0-9a-f]*'
    ),
    created_at                  INTEGER NOT NULL CHECK (created_at > 0),
    CHECK (
        expected_source_digest <> next_source_digest
        OR expected_lock_digest <> next_lock_digest
    )
);

CREATE TABLE plugin_kv (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_product_id TEXT NOT NULL CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    namespace TEXT NOT NULL CHECK (
        length(namespace) BETWEEN 1 AND 128
        AND namespace NOT GLOB '*[^!-~]*'
    ),
    key TEXT NOT NULL CHECK (
        length(key) BETWEEN 1 AND 256
        AND key NOT GLOB '*[^!-~]*'
    ),
    value_json TEXT NOT NULL CHECK (json_valid(value_json)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at), key_generation INTEGER NOT NULL DEFAULT 1
        CHECK (key_generation >= 1), is_tombstone INTEGER NOT NULL DEFAULT 0
        CHECK (is_tombstone IN (0, 1)),
    UNIQUE (owner_user_id, plugin_product_id, namespace, key)
);

CREATE TABLE plugin_library_state (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    singleton_key TEXT NOT NULL CHECK (singleton_key = 'plugin_runtime'),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    UNIQUE (singleton_key, owner_user_id)
);

CREATE TABLE plugin_mount_credential_bindings (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    mount_id                 TEXT NOT NULL CHECK (
        length(mount_id) = 36
        AND lower(mount_id) = mount_id
        AND mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    slot                     TEXT NOT NULL CHECK (
        length(slot) BETWEEN 1 AND 128
        AND slot NOT GLOB '*[^!-~]*'
    ),
    credential_id            TEXT NOT NULL CHECK (
        length(credential_id) BETWEEN 1 AND 512
        AND credential_id NOT GLOB '*[^!-~]*'
    ),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (mount_id, slot)
);

CREATE TABLE plugin_mount_kv (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    mount_id                 TEXT NOT NULL CHECK (
        length(mount_id) = 36
        AND lower(mount_id) = mount_id
        AND mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    namespace                TEXT NOT NULL CHECK (
        length(namespace) BETWEEN 1 AND 128
        AND namespace NOT GLOB '*[^!-~]*'
    ),
    key                      TEXT NOT NULL CHECK (
        length(key) BETWEEN 1 AND 256
        AND key NOT GLOB '*[^!-~]*'
    ),
    value_json               TEXT NOT NULL CHECK (json_valid(value_json)),
    revision                 INTEGER NOT NULL CHECK (revision >= 1),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (mount_id, namespace, key)
);

CREATE TABLE plugin_mount_revisions (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    mount_revision_id        TEXT NOT NULL UNIQUE CHECK (
        length(mount_revision_id) = 36
        AND lower(mount_revision_id) = mount_revision_id
        AND mount_revision_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_revision_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    mount_id                 TEXT NOT NULL CHECK (
        length(mount_id) = 36
        AND lower(mount_id) = mount_id
        AND mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    revision                 INTEGER NOT NULL CHECK (revision >= 1),
    artifact_id              TEXT NOT NULL CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest          TEXT NOT NULL CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    candidate_key            TEXT NOT NULL UNIQUE CHECK (
        length(candidate_key) = 36
        AND lower(candidate_key) = candidate_key
        AND candidate_key GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(candidate_key, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    candidate_digest         TEXT NOT NULL CHECK (
        length(candidate_digest) = 64
        AND lower(candidate_digest) = candidate_digest
        AND candidate_digest NOT GLOB '*[^0-9a-f]*'
    ),
    base_target_digest       TEXT CHECK (
        base_target_digest IS NULL OR (
            length(base_target_digest) = 64
            AND lower(base_target_digest) = base_target_digest
            AND base_target_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    applied_at               INTEGER NOT NULL CHECK (applied_at >= 0), apply_authorization_kind TEXT NOT NULL
    DEFAULT 'manual_user_confirmation'
    CHECK (apply_authorization_kind IN ('manual_user_confirmation', 'standing_auto')), auto_apply_authorization_revision INTEGER CHECK (
    auto_apply_authorization_revision IS NULL OR auto_apply_authorization_revision > 0
),
    UNIQUE (mount_id, revision)
);

CREATE TABLE plugin_mounts (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    mount_id                 TEXT NOT NULL UNIQUE CHECK (
        length(mount_id) = 36
        AND lower(mount_id) = mount_id
        AND mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    package_id               TEXT NOT NULL UNIQUE CHECK (
        length(package_id) BETWEEN 1 AND 255
        AND package_id NOT GLOB '*[^A-Za-z0-9._-]*'
    ),
    current_artifact_digest  TEXT CHECK (
        current_artifact_digest IS NULL OR (
            length(current_artifact_digest) = 64
            AND lower(current_artifact_digest) = current_artifact_digest
            AND current_artifact_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    previous_artifact_digest TEXT CHECK (
        previous_artifact_digest IS NULL OR (
            length(previous_artifact_digest) = 64
            AND lower(previous_artifact_digest) = previous_artifact_digest
            AND previous_artifact_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    current_revision_id      TEXT CHECK (
        current_revision_id IS NULL OR (
            length(current_revision_id) = 36
            AND lower(current_revision_id) = current_revision_id
            AND current_revision_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(current_revision_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    previous_revision_id     TEXT CHECK (
        previous_revision_id IS NULL OR (
            length(previous_revision_id) = 36
            AND lower(previous_revision_id) = previous_revision_id
            AND previous_revision_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(previous_revision_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    enabled                  INTEGER NOT NULL DEFAULT 0 CHECK (
        typeof(enabled) = 'integer' AND enabled IN (0, 1)
    ),
    retained                 INTEGER NOT NULL DEFAULT 1 CHECK (
        typeof(retained) = 'integer' AND retained IN (0, 1)
    ),
    delete_pending           INTEGER NOT NULL DEFAULT 0 CHECK (
        typeof(delete_pending) = 'integer' AND delete_pending IN (0, 1)
    ),
    revision                 INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    config_json              TEXT NOT NULL DEFAULT '{}' CHECK (
        json_valid(config_json)
        AND json_type(config_json) = 'object'
    ),
    data_dir_path            TEXT NOT NULL UNIQUE CHECK (
        data_dir_path <> ''
        AND substr(data_dir_path, 1, 1) <> '/'
        AND substr(data_dir_path, -1, 1) <> '/'
        AND instr(data_dir_path, '\') = 0
        AND instr(data_dir_path, '//') = 0
        AND instr('/' || data_dir_path || '/', '/../') = 0
        AND instr('/' || data_dir_path || '/', '/./') = 0
        AND instr(data_dir_path, char(0)) = 0
    ),
    last_error               TEXT CHECK (
        last_error IS NULL OR length(last_error) BETWEEN 1 AND 8192
    ),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at), config_schema_digest TEXT CHECK (
    config_schema_digest IS NULL OR (
        length(config_schema_digest) = 64
        AND lower(config_schema_digest) = config_schema_digest
        AND config_schema_digest NOT GLOB '*[^0-9a-f]*'
    )
), config_revision INTEGER NOT NULL DEFAULT 0 CHECK (
    config_revision >= 0
), credential_bindings_revision INTEGER NOT NULL DEFAULT 0 CHECK (
    credential_bindings_revision >= 0
),
    CHECK (
        (current_artifact_digest IS NULL) = (current_revision_id IS NULL)
    ),
    CHECK (
        (previous_artifact_digest IS NULL) = (previous_revision_id IS NULL)
    ),
    CHECK (
        current_revision_id IS NULL
        OR previous_revision_id IS NULL
        OR current_revision_id <> previous_revision_id
    ),
    CHECK (
        current_artifact_digest IS NULL
        OR previous_artifact_digest IS NULL
        OR current_artifact_digest <> previous_artifact_digest
    ),
    CHECK (
        enabled = 0
        OR (
            current_artifact_digest IS NOT NULL
            AND retained = 0
            AND delete_pending = 0
        )
    ),
    CHECK (
        retained = 0
        OR (
            enabled = 0
            AND current_artifact_digest IS NULL
            AND previous_artifact_digest IS NULL
            AND current_revision_id IS NULL
            AND previous_revision_id IS NULL
        )
    ),
    CHECK (
        delete_pending = 0
        OR retained = 1
    ),
    CHECK (
        retained = 1
        OR current_artifact_digest IS NOT NULL
    )
);

CREATE TABLE plugin_product_documents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36 AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    document_key TEXT NOT NULL CHECK (length(document_key) BETWEEN 1 AND 100),
    revision INTEGER NOT NULL CHECK (revision > 0),
    content_json TEXT NOT NULL CHECK (json_valid(content_json)),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    UNIQUE (owner_user_id, document_key)
);

CREATE TABLE plugin_products (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_product_id TEXT NOT NULL UNIQUE CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    product_revision INTEGER NOT NULL DEFAULT 1 CHECK (product_revision >= 1),
    display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 255),
    description TEXT,
    icon_asset_id TEXT,
    kind TEXT NOT NULL CHECK (kind = 'plugin'),
    lifecycle TEXT NOT NULL DEFAULT 'disabled'
        CHECK (lifecycle IN ('enabled', 'disabled', 'trashed', 'deleting')),
    pointer_revision INTEGER NOT NULL DEFAULT 1 CHECK (pointer_revision >= 1),
    active_release_epoch INTEGER NOT NULL DEFAULT 0
        CHECK (active_release_epoch >= 0),
    ready_release_id TEXT,
    ready_release_digest TEXT,
    active_release_id TEXT,
    active_release_digest TEXT,
    previous_release_id TEXT,
    previous_release_digest TEXT,
    materialized_catalog_digest TEXT NOT NULL CHECK (
        length(materialized_catalog_digest) = 64
        AND lower(materialized_catalog_digest) = materialized_catalog_digest
        AND materialized_catalog_digest NOT GLOB '*[^0-9a-f]*'
    ),
    config_schema_json TEXT NOT NULL DEFAULT '{"type":"object"}'
        CHECK (json_valid(config_schema_json) AND json_type(config_schema_json) = 'object'),
    config_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(config_json) AND json_type(config_json) = 'object'),
    config_revision INTEGER NOT NULL DEFAULT 1 CHECK (config_revision >= 1),
    credential_bindings_revision INTEGER NOT NULL DEFAULT 1
        CHECK (credential_bindings_revision >= 1),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK ((ready_release_id IS NULL) = (ready_release_digest IS NULL)),
    CHECK ((active_release_id IS NULL) = (active_release_digest IS NULL)),
    CHECK ((previous_release_id IS NULL) = (previous_release_digest IS NULL)),
    CHECK (ready_release_digest IS NULL OR (
        length(ready_release_digest) = 64
        AND lower(ready_release_digest) = ready_release_digest
        AND ready_release_digest NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (active_release_digest IS NULL OR (
        length(active_release_digest) = 64
        AND lower(active_release_digest) = active_release_digest
        AND active_release_digest NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (previous_release_digest IS NULL OR (
        length(previous_release_digest) = 64
        AND lower(previous_release_digest) = previous_release_digest
        AND previous_release_digest NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (ready_release_id IS NULL OR (
        length(ready_release_id) = 36
        AND lower(ready_release_id) = ready_release_id
        AND ready_release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(ready_release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (icon_asset_id IS NULL OR (
        length(icon_asset_id) = 36
        AND lower(icon_asset_id) = icon_asset_id
        AND icon_asset_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(icon_asset_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (active_release_id IS NULL OR (
        length(active_release_id) = 36
        AND lower(active_release_id) = active_release_id
        AND active_release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(active_release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (previous_release_id IS NULL OR (
        length(previous_release_id) = 36
        AND lower(previous_release_id) = previous_release_id
        AND previous_release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(previous_release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK ((active_release_id IS NULL) = (active_release_epoch = 0)),
    CHECK (active_release_id IS NULL OR (
        active_release_id <> ready_release_id
        AND active_release_id <> previous_release_id
    )),
    CHECK (previous_release_id IS NULL OR previous_release_id <> ready_release_id)
);

CREATE TABLE plugin_projects (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id               TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id            TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    package_id               TEXT CHECK (
        package_id IS NULL OR (
            length(package_id) BETWEEN 1 AND 255
            AND package_id NOT GLOB '*[^A-Za-z0-9._-]*'
        )
    ),
    plugin_product_id        TEXT UNIQUE CHECK (
        plugin_product_id IS NULL OR (
            length(plugin_product_id) = 36
            AND lower(plugin_product_id) = plugin_product_id
            AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    project_revision         INTEGER NOT NULL DEFAULT 1 CHECK (project_revision >= 1),
    source_state             TEXT NOT NULL DEFAULT 'package'
        CHECK (source_state IN ('package', 'empty', 'editable', 'runtime_only')),
    build_profile_version    TEXT CHECK (
        build_profile_version IS NULL OR (
            length(build_profile_version) BETWEEN 1 AND 64
            AND build_profile_version NOT GLOB '*[^!-~]*'
        )
    ),
    managed_source_path      TEXT CHECK (
        managed_source_path IS NULL OR (
            managed_source_path <> ''
            AND substr(managed_source_path, 1, 1) <> '/'
            AND substr(managed_source_path, -1, 1) <> '/'
            AND instr(managed_source_path, '\') = 0
            AND instr(managed_source_path, '//') = 0
            AND instr('/' || managed_source_path || '/', '/../') = 0
            AND instr('/' || managed_source_path || '/', '/./') = 0
            AND instr(managed_source_path, char(0)) = 0
        )
    ),
    source_head_digest       TEXT CHECK (
        source_head_digest IS NULL OR (
            length(source_head_digest) = 64
            AND lower(source_head_digest) = source_head_digest
            AND source_head_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    dependency_lock_digest   TEXT CHECK (
        dependency_lock_digest IS NULL OR (
            length(dependency_lock_digest) = 64
            AND lower(dependency_lock_digest) = dependency_lock_digest
            AND dependency_lock_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    build_generation         INTEGER NOT NULL DEFAULT 0 CHECK (build_generation >= 0),
    linked_mount_id          TEXT UNIQUE CHECK (
        linked_mount_id IS NULL OR (
            length(linked_mount_id) = 36
            AND lower(linked_mount_id) = linked_mount_id
            AND linked_mount_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(linked_mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    ready_candidate_id       TEXT UNIQUE CHECK (
        ready_candidate_id IS NULL OR (
            length(ready_candidate_id) = 36
            AND lower(ready_candidate_id) = ready_candidate_id
            AND ready_candidate_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(ready_candidate_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at), display_name TEXT NOT NULL
    DEFAULT 'Plugin Runtime Project'
    CHECK (
        length(display_name) BETWEEN 1 AND 255
        AND trim(display_name) <> ''
        AND instr(display_name, char(0)) = 0
    ), description TEXT NOT NULL
    DEFAULT ''
    CHECK (
        length(description) <= 4096
        AND instr(description, char(0)) = 0
    ), apply_mode TEXT NOT NULL
    DEFAULT 'ask_before_apply'
    CHECK (apply_mode IN ('ask_before_apply', 'auto_compatible_when_idle')), auto_apply_mount_id TEXT CHECK (
    auto_apply_mount_id IS NULL OR (
        length(auto_apply_mount_id) = 36
        AND lower(auto_apply_mount_id) = auto_apply_mount_id
        AND auto_apply_mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(auto_apply_mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
), auto_apply_authorization_revision INTEGER NOT NULL
    DEFAULT 0 CHECK (auto_apply_authorization_revision >= 0), auto_apply_authorized_at INTEGER CHECK (
    auto_apply_authorized_at IS NULL OR auto_apply_authorized_at > 0
),
    CHECK (
        managed_source_path IS NOT NULL
        OR (source_head_digest IS NULL AND dependency_lock_digest IS NULL)
    ),
    CHECK (
        source_head_digest IS NOT NULL
        OR dependency_lock_digest IS NULL
    ),
    CHECK (
        (
            plugin_product_id IS NULL
            AND package_id IS NOT NULL
            AND source_state = 'package'
            AND project_revision = 1
            AND build_profile_version IS NULL
        )
        OR
        (
            plugin_product_id IS NOT NULL
            AND package_id IS NULL
            AND source_state IN ('empty', 'editable', 'runtime_only')
        )
    ),
    CHECK (
        source_state = 'package'
        OR (
            source_state = 'empty'
            AND managed_source_path IS NULL
            AND source_head_digest IS NULL
            AND dependency_lock_digest IS NULL
            AND build_profile_version IS NULL
            AND build_generation = 0
        )
        OR (
            source_state = 'editable'
            AND managed_source_path IS NOT NULL
            AND source_head_digest IS NOT NULL
            AND dependency_lock_digest IS NOT NULL
            AND build_profile_version IS NOT NULL
            AND build_generation > 0
        )
        OR (
            source_state = 'runtime_only'
            AND managed_source_path IS NULL
            AND source_head_digest IS NULL
            AND dependency_lock_digest IS NULL
            AND build_profile_version IS NULL
            AND build_generation = 0
        )
    )
);

CREATE TABLE plugin_publish_authorizations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    authorization_id TEXT NOT NULL UNIQUE CHECK (
        length(authorization_id) = 36
        AND lower(authorization_id) = authorization_id
        AND authorization_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(authorization_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    plugin_product_id TEXT NOT NULL UNIQUE CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    user_authorized_at_ms INTEGER NOT NULL CHECK (user_authorized_at_ms > 0)
);

CREATE TABLE plugin_ready_candidates (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    candidate_id             TEXT NOT NULL UNIQUE CHECK (
        length(candidate_id) = 36
        AND lower(candidate_id) = candidate_id
        AND candidate_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(candidate_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    project_id               TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    candidate_digest         TEXT NOT NULL UNIQUE CHECK (
        length(candidate_digest) = 64
        AND lower(candidate_digest) = candidate_digest
        AND candidate_digest NOT GLOB '*[^0-9a-f]*'
    ),
    origin_kind              TEXT NOT NULL CHECK (origin_kind IN ('build', 'import')),
    artifact_id              TEXT NOT NULL CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest          TEXT NOT NULL CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    base_target_digest       TEXT CHECK (
        base_target_digest IS NULL OR (
            length(base_target_digest) = 64
            AND lower(base_target_digest) = base_target_digest
            AND base_target_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    source_snapshot_digest   TEXT CHECK (
        source_snapshot_digest IS NULL OR (
            length(source_snapshot_digest) = 64
            AND lower(source_snapshot_digest) = source_snapshot_digest
            AND source_snapshot_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    dependency_lock_digest   TEXT CHECK (
        dependency_lock_digest IS NULL OR (
            length(dependency_lock_digest) = 64
            AND lower(dependency_lock_digest) = dependency_lock_digest
            AND dependency_lock_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    contract_diff_json       TEXT NOT NULL CHECK (
        json_valid(contract_diff_json)
        AND json_type(contract_diff_json) = 'object'
    ),
    origin_operation_id      TEXT NOT NULL CHECK (
        length(origin_operation_id) = 36
        AND lower(origin_operation_id) = origin_operation_id
        AND origin_operation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(origin_operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    build_generation         INTEGER NOT NULL CHECK (build_generation >= 0),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0), imported_test_provenance_json TEXT CHECK (
    imported_test_provenance_json IS NULL OR (
        json_valid(imported_test_provenance_json)
        AND json_type(imported_test_provenance_json) = 'object'
    )
),
    CHECK (
        (source_snapshot_digest IS NULL AND dependency_lock_digest IS NULL)
        OR (source_snapshot_digest IS NOT NULL AND dependency_lock_digest IS NOT NULL)
    )
);

CREATE TABLE plugin_release_artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    artifact_id TEXT NOT NULL UNIQUE CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest TEXT NOT NULL UNIQUE CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    manifest_digest TEXT NOT NULL CHECK (
        length(manifest_digest) = 64
        AND lower(manifest_digest) = manifest_digest
        AND manifest_digest NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_record_json TEXT NOT NULL
        CHECK (json_valid(artifact_record_json)
               AND json_type(artifact_record_json) = 'object'),
    managed_path TEXT NOT NULL UNIQUE CHECK (
        managed_path <> ''
        AND substr(managed_path, 1, 1) <> '/'
        AND substr(managed_path, -1, 1) <> '/'
        AND instr(managed_path, '\') = 0
        AND instr(managed_path, '//') = 0
        AND instr('/' || managed_path || '/', '/../') = 0
        AND instr('/' || managed_path || '/', '/./') = 0
        AND instr(managed_path, char(0)) = 0
    ),
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
);

CREATE TABLE "plugin_releases" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    release_id TEXT NOT NULL UNIQUE CHECK (
        length(release_id) = 36
        AND lower(release_id) = release_id
        AND release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    plugin_product_id TEXT NOT NULL CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_id TEXT NOT NULL CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest TEXT NOT NULL CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    manifest_digest TEXT NOT NULL CHECK (
        length(manifest_digest) = 64
        AND lower(manifest_digest) = manifest_digest
        AND manifest_digest NOT GLOB '*[^0-9a-f]*'
    ),
    release_digest TEXT NOT NULL CHECK (
        length(release_digest) = 64
        AND lower(release_digest) = release_digest
        AND release_digest NOT GLOB '*[^0-9a-f]*'
    ),
    origin_kind TEXT NOT NULL CHECK (origin_kind IN ('build', 'import')),
    origin_operation_id TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('managed', 'runtime_only')),
    project_id TEXT,
    source_snapshot_digest TEXT,
    dependency_lock_digest TEXT,
    build_profile_version TEXT,
    build_generation INTEGER CHECK (build_generation IS NULL OR build_generation > 0),
    release_record_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(release_record_json)
               AND json_type(release_record_json) = 'object'),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    CHECK (project_id IS NULL OR (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (
        length(origin_operation_id) = 36
        AND lower(origin_operation_id) = origin_operation_id
        AND origin_operation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(origin_operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    CHECK (
        (source_kind = 'managed'
         AND project_id IS NOT NULL
         AND source_snapshot_digest IS NOT NULL
         AND dependency_lock_digest IS NOT NULL
         AND build_profile_version IS NOT NULL
         AND build_generation IS NOT NULL)
        OR
        (source_kind = 'runtime_only'
         AND project_id IS NULL
         AND source_snapshot_digest IS NULL
         AND dependency_lock_digest IS NULL
         AND build_profile_version IS NULL
         AND build_generation IS NULL)
    ),
    CHECK (origin_kind <> 'build' OR source_kind = 'managed'),
    CHECK (source_snapshot_digest IS NULL OR (
        length(source_snapshot_digest) = 64
        AND lower(source_snapshot_digest) = source_snapshot_digest
        AND source_snapshot_digest NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (dependency_lock_digest IS NULL OR (
        length(dependency_lock_digest) = 64
        AND lower(dependency_lock_digest) = dependency_lock_digest
        AND dependency_lock_digest NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (build_profile_version IS NULL OR (
        length(build_profile_version) BETWEEN 1 AND 64
        AND build_profile_version NOT GLOB '*[^!-~]*'
    ))
);

CREATE TABLE plugin_service_test_receipts (
    id                                  INTEGER PRIMARY KEY AUTOINCREMENT,
    receipt_id                          TEXT NOT NULL UNIQUE CHECK (
        length(receipt_id) = 36
        AND lower(receipt_id) = receipt_id
        AND receipt_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(receipt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id                       TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    plugin_product_id                          TEXT NOT NULL CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    release_id                          TEXT NOT NULL CHECK (
        length(release_id) = 36
        AND lower(release_id) = release_id
        AND release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    release_digest                      TEXT NOT NULL CHECK (
        length(release_digest) = 64
        AND lower(release_digest) = release_digest
        AND release_digest NOT GLOB '*[^0-9a-f]*'
    ),
    service_run_key                     TEXT NOT NULL CHECK (
        length(service_run_key) = 64
        AND lower(service_run_key) = service_run_key
        AND service_run_key NOT GLOB '*[^0-9a-f]*'
    ),
    outcome                             TEXT NOT NULL CHECK (
        outcome IN ('passed', 'failed', 'needs_test_input')
    ),
    error_code                          TEXT CHECK (
        error_code IS NULL OR (
            length(error_code) BETWEEN 1 AND 256
            AND error_code NOT GLOB '*[^!-~]*'
        )
    ),
    receipt_digest                      TEXT NOT NULL CHECK (
        length(receipt_digest) = 64
        AND lower(receipt_digest) = receipt_digest
        AND receipt_digest NOT GLOB '*[^0-9a-f]*'
    ),
    runtime_fingerprint_digest          TEXT NOT NULL CHECK (
        length(runtime_fingerprint_digest) = 64
        AND lower(runtime_fingerprint_digest) = runtime_fingerprint_digest
        AND runtime_fingerprint_digest NOT GLOB '*[^0-9a-f]*'
    ),
    resolved_test_input_digest          TEXT NOT NULL CHECK (
        length(resolved_test_input_digest) = 64
        AND lower(resolved_test_input_digest) = resolved_test_input_digest
        AND resolved_test_input_digest NOT GLOB '*[^0-9a-f]*'
    ),
    tested_product_revision             INTEGER NOT NULL CHECK (
        tested_product_revision >= 1
    ),
    tested_pointer_revision             INTEGER NOT NULL CHECK (
        tested_pointer_revision >= 1
    ),
    tested_config_revision              INTEGER NOT NULL CHECK (
        tested_config_revision >= 1
    ),
    tested_credential_bindings_revision INTEGER NOT NULL CHECK (
        tested_credential_bindings_revision >= 1
    ),
    receipt_json                        TEXT NOT NULL CHECK (
        json_valid(receipt_json)
        AND json_type(receipt_json) = 'object'
    ),
    issued_at_ms                        INTEGER NOT NULL CHECK (issued_at_ms > 0)
);

CREATE TABLE plugin_source_mutation_commits (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    intent_id  TEXT NOT NULL UNIQUE CHECK (
        length(intent_id) = 36
        AND lower(intent_id) = intent_id
        AND intent_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(intent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    created_at INTEGER NOT NULL CHECK (created_at > 0)
);

CREATE TABLE plugin_source_mutation_intents (
    id                        INTEGER PRIMARY KEY AUTOINCREMENT,
    intent_id                 TEXT NOT NULL UNIQUE CHECK (
        length(intent_id) = 36
        AND lower(intent_id) = intent_id
        AND intent_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(intent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id             TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    plugin_product_id                TEXT NOT NULL CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    project_id                TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    expected_product_revision INTEGER NOT NULL CHECK (expected_product_revision >= 1),
    expected_project_revision INTEGER NOT NULL CHECK (expected_project_revision >= 1),
    expected_build_generation INTEGER NOT NULL CHECK (expected_build_generation >= 1),
    expected_source_digest    TEXT NOT NULL CHECK (
        length(expected_source_digest) = 64
        AND lower(expected_source_digest) = expected_source_digest
        AND expected_source_digest NOT GLOB '*[^0-9a-f]*'
    ),
    next_source_digest        TEXT NOT NULL CHECK (
        length(next_source_digest) = 64
        AND lower(next_source_digest) = next_source_digest
        AND next_source_digest NOT GLOB '*[^0-9a-f]*'
    ),
    next_build_generation     INTEGER NOT NULL CHECK (
        next_build_generation = expected_build_generation + 1
    ),
    created_at                INTEGER NOT NULL CHECK (created_at > 0),
    CHECK (expected_source_digest <> next_source_digest)
);

CREATE TABLE plugin_surface_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    surface_session_id TEXT NOT NULL UNIQUE CHECK (
        length(surface_session_id) = 36
        AND lower(surface_session_id) = surface_session_id
        AND surface_session_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(surface_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    plugin_product_id TEXT NOT NULL UNIQUE CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    generation INTEGER NOT NULL CHECK (generation >= 1),
    capability_digest TEXT NOT NULL UNIQUE CHECK (
        length(capability_digest) = 64
        AND lower(capability_digest) = capability_digest
        AND capability_digest NOT GLOB '*[^0-9a-f]*'
    ),
    active_release_id TEXT NOT NULL CHECK (
        length(active_release_id) = 36
        AND lower(active_release_id) = active_release_id
        AND active_release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(active_release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    active_release_digest TEXT NOT NULL CHECK (
        length(active_release_digest) = 64
        AND lower(active_release_digest) = active_release_digest
        AND active_release_digest NOT GLOB '*[^0-9a-f]*'
    ),
    active_release_epoch INTEGER NOT NULL CHECK (active_release_epoch > 0),
    issued_at_ms INTEGER NOT NULL CHECK (issued_at_ms > 0), conversation_id TEXT CHECK (
        conversation_id IS NULL OR (
            length(conversation_id) = 36
            AND lower(conversation_id) = conversation_id
            AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    )
);

CREATE TABLE product_agent_selections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36 AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    target_kind TEXT NOT NULL CHECK (target_kind IN ('companion', 'robot', 'customer', 'creative_studio_canvas')),
    target_id TEXT NOT NULL CHECK (length(trim(target_id)) > 0),
    selection_json TEXT NOT NULL CHECK (json_valid(selection_json) AND json_type(selection_json) = 'object'),
    UNIQUE (owner_user_id, target_kind, target_id)
);

CREATE TABLE product_operations (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id             TEXT NOT NULL UNIQUE CHECK (
        length(operation_id) = 36
        AND lower(operation_id) = operation_id
        AND operation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    kind                     TEXT NOT NULL CHECK (
        kind IN ('build', 'import', 'export', 'plugin_permanent_delete')
    ),
    owner_kind               TEXT NOT NULL CHECK (
        owner_kind IN ('plugin_project', 'plugin_mount', 'plugin')
    ),
    owner_id                 TEXT NOT NULL CHECK (
        length(owner_id) = 36
        AND lower(owner_id) = owner_id
        AND owner_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    state                    TEXT NOT NULL CHECK (
        state IN ('running', 'succeeded', 'failed', 'canceled')
    ),
    progress_percent         INTEGER CHECK (progress_percent BETWEEN 0 AND 100),
    last_error_code          TEXT CHECK (
        last_error_code IS NULL OR (
            length(last_error_code) BETWEEN 1 AND 256
            AND last_error_code NOT GLOB '*[^!-~]*'
        )
    ),
    bounded_log_tail_json    TEXT NOT NULL DEFAULT '[]' CHECK (
        json_valid(bounded_log_tail_json)
        AND json_type(bounded_log_tail_json) = 'array'
        AND json_array_length(bounded_log_tail_json) <= 200
    ),
    started_at_ms            INTEGER NOT NULL CHECK (started_at_ms > 0),
    finished_at_ms           INTEGER CHECK (
        finished_at_ms IS NULL OR finished_at_ms >= started_at_ms
    ), result_artifact_digests_json TEXT NOT NULL
    DEFAULT '{}' CHECK (
        json_valid(result_artifact_digests_json)
        AND json_type(result_artifact_digests_json) = 'object'
    ),
    CHECK (
        (kind = 'build' AND owner_kind IN ('plugin_project', 'plugin'))
        OR
        (kind IN ('import', 'export')
            AND owner_kind IN ('plugin_project', 'plugin_mount', 'plugin'))
        OR
        (kind = 'plugin_permanent_delete' AND owner_kind = 'plugin')
    ),
    CHECK (
        kind <> 'plugin_permanent_delete'
        OR progress_percent IS NULL
    ),
    CHECK (
        (state = 'running'
            AND finished_at_ms IS NULL
            AND last_error_code IS NULL)
        OR
        (state = 'succeeded'
            AND finished_at_ms IS NOT NULL
            AND last_error_code IS NULL
            AND (
                kind = 'plugin_permanent_delete'
                OR progress_percent = 100
            ))
        OR
        (state = 'failed'
            AND finished_at_ms IS NOT NULL
            AND last_error_code IS NOT NULL)
        OR
        (state = 'canceled'
            AND kind <> 'plugin_permanent_delete'
            AND finished_at_ms IS NOT NULL
            AND last_error_code IS NULL)
    )
);

CREATE TABLE "provider_connections" (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_id         TEXT NOT NULL UNIQUE,
    provider_id           TEXT NOT NULL,
    role                  TEXT NOT NULL CHECK (trim(role) <> '' AND role <> 'default'),
    label                 TEXT,
    base_url              TEXT NOT NULL,
    auth_scheme           TEXT NOT NULL CHECK (trim(auth_scheme) <> ''),
    credentials_encrypted TEXT NOT NULL,
    extra                 TEXT NOT NULL DEFAULT '{}',
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL,
    UNIQUE (provider_id, role),
    CHECK (length(connection_id) = 36 AND lower(connection_id) = connection_id
           AND connection_id GLOB '????????-????-7???-[89ab]???-????????????'
           AND replace(connection_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    CHECK (length(provider_id) = 36 AND lower(provider_id) = provider_id
           AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
           AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    CHECK (json_valid(extra) AND json_type(extra) = 'object')
);

CREATE TABLE "provider_model_capabilities" (
    id                             INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id                    TEXT NOT NULL,
    model                          TEXT NOT NULL,
    task                           TEXT NOT NULL,
    traits                         TEXT NOT NULL DEFAULT '[]',
    protocol                       TEXT NOT NULL CHECK (trim(protocol) <> ''),
    connection_role                TEXT NOT NULL DEFAULT 'default'
                                           CHECK (trim(connection_role) <> ''),
    base_url_override              TEXT,
    endpoint                       TEXT,
    poll_endpoint                  TEXT,
    content_endpoint               TEXT,
    realtime_endpoint              TEXT,
    allow_cross_origin_credentials INTEGER NOT NULL DEFAULT 0
                                           CHECK (allow_cross_origin_credentials IN (0, 1)),
    provider_params                TEXT NOT NULL DEFAULT '{}',
    context_limit                  INTEGER,
    health                         TEXT,
    health_checked_at              INTEGER,
    created_at                     INTEGER NOT NULL,
    updated_at                     INTEGER NOT NULL, output_limit INTEGER
    CHECK (output_limit IS NULL OR output_limit > 0),
    UNIQUE (provider_id, model, task),
    CHECK (task IN (
        'chat', 'realtime_conversation', 'image_generation', 'image_edit',
        'video_generation', 'music_generation', 'speech_synthesis', 'speech_recognition',
        'embedding', 'rerank'
    )),
    CHECK (json_valid(traits) AND json_type(traits) = 'array'),
    CHECK (json_valid(provider_params) AND json_type(provider_params) = 'object'),
    CHECK (health IS NULL OR json_valid(health)),
    CHECK (context_limit IS NULL OR context_limit > 0),
    CHECK (length(provider_id) = 36 AND lower(provider_id) = provider_id
           AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
           AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE "provider_models" (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL,
    model       TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    sort_order  INTEGER NOT NULL DEFAULT 0,
    description TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL, display_name TEXT
CHECK (
    display_name IS NULL
    OR (
        display_name = trim(display_name)
        AND length(display_name) BETWEEN 1 AND 128
    )
),
    UNIQUE (provider_id, model),
    CHECK (length(provider_id) = 36 AND lower(provider_id) = provider_id
           AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
           AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE "providers" (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id       TEXT NOT NULL UNIQUE,
    platform          TEXT NOT NULL,
    name              TEXT NOT NULL,
    base_url          TEXT NOT NULL,
    auth_scheme       TEXT NOT NULL CHECK (trim(auth_scheme) <> ''),
    credentials_encrypted TEXT NOT NULL,
    enabled           INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    bedrock_config    TEXT,
    sort_order        INTEGER NOT NULL DEFAULT 0,
    config_revision   INTEGER NOT NULL DEFAULT 0 CHECK (config_revision >= 0),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    CHECK (length(provider_id) = 36 AND lower(provider_id) = provider_id
           AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
           AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

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

CREATE TABLE requirement_display_sequence (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    singleton_key TEXT NOT NULL UNIQUE CHECK (singleton_key = 'requirements'),
    last_no       INTEGER NOT NULL DEFAULT 0 CHECK (last_no >= 0)
);

CREATE TABLE requirement_pre_effect_abandon_guards (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    requirement_id          TEXT NOT NULL
                            CHECK (
                                length(requirement_id) = 36
                                AND lower(requirement_id) = requirement_id
                                AND requirement_id GLOB '????????-????-7???-[89ab]???-????????????'
                                AND replace(requirement_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                            ),
    claim_generation        INTEGER NOT NULL CHECK (claim_generation >= 1),
    claim_token             TEXT NOT NULL
                            CHECK (
                                length(claim_token) = 64
                                AND lower(claim_token) = claim_token
                                AND claim_token NOT GLOB '*[^0-9a-f]*'
                            ),
    owner_conversation_id   TEXT
                            CHECK (
                                owner_conversation_id IS NULL
                                OR (
                                    length(owner_conversation_id) = 36
                                    AND lower(owner_conversation_id) = owner_conversation_id
                                    AND owner_conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
                                    AND replace(owner_conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                                )
                            ),
    owner_terminal_id       TEXT
                            CHECK (
                                owner_terminal_id IS NULL
                                OR (
                                    length(owner_terminal_id) = 36
                                    AND lower(owner_terminal_id) = owner_terminal_id
                                    AND owner_terminal_id GLOB '????????-????-7???-[89ab]???-????????????'
                                    AND replace(owner_terminal_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                                )
                            ),
    created_at              INTEGER NOT NULL,
    CHECK (
        (owner_conversation_id IS NOT NULL AND owner_terminal_id IS NULL)
        OR
        (owner_conversation_id IS NULL AND owner_terminal_id IS NOT NULL)
    )
);

CREATE TABLE requirement_tags (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    tag                   TEXT NOT NULL UNIQUE,
    paused                INTEGER NOT NULL DEFAULT 0 CHECK (paused IN (0, 1)),
    paused_reason         TEXT,
    paused_requirement_id TEXT,
    paused_at             INTEGER,
    CHECK (paused_requirement_id IS NULL OR (length(paused_requirement_id) = 36 AND lower(paused_requirement_id) = paused_requirement_id AND paused_requirement_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(paused_requirement_id, '-', '') NOT GLOB '*[^0-9a-f]*'))
);

CREATE TABLE requirements (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    requirement_id         TEXT NOT NULL UNIQUE
                           CHECK (
                               length(requirement_id) = 36
                               AND lower(requirement_id) = requirement_id
                               AND requirement_id GLOB '????????-????-7???-[89ab]???-????????????'
                               AND replace(requirement_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                           ),
    display_no             INTEGER NOT NULL UNIQUE CHECK (display_no > 0),
    title                  TEXT NOT NULL,
    content                TEXT NOT NULL DEFAULT '',
    tag                    TEXT NOT NULL,
    order_key              TEXT NOT NULL DEFAULT '',
    sort_seq               TEXT NOT NULL DEFAULT '',
    status                 TEXT NOT NULL DEFAULT 'pending',
    priority               INTEGER NOT NULL DEFAULT 0,
    completion_note        TEXT,
    owner_conversation_id  TEXT,
    owner_terminal_id      TEXT,
    active_turn_started_at INTEGER,
    lease_expires_at       INTEGER,
    started_at             INTEGER,
    completed_at           INTEGER,
    attempt_count          INTEGER NOT NULL DEFAULT 0,
    created_by             TEXT NOT NULL DEFAULT 'user',
    extra                  TEXT NOT NULL DEFAULT '{}',
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL, claim_generation INTEGER NOT NULL DEFAULT 0
        CHECK (claim_generation >= 0), claim_token TEXT
        CHECK (
            claim_token IS NULL
            OR (
                length(claim_token) = 64
                AND lower(claim_token) = claim_token
                AND claim_token NOT GLOB '*[^0-9a-f]*'
            )
        ),
    CHECK (owner_conversation_id IS NULL OR owner_terminal_id IS NULL),
    CHECK (owner_conversation_id IS NULL OR (length(owner_conversation_id) = 36 AND lower(owner_conversation_id) = owner_conversation_id AND owner_conversation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(owner_conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*')),
    CHECK (owner_terminal_id IS NULL OR (length(owner_terminal_id) = 36 AND lower(owner_terminal_id) = owner_terminal_id AND owner_terminal_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(owner_terminal_id, '-', '') NOT GLOB '*[^0-9a-f]*'))
);

CREATE TABLE schema_metadata (
    singleton_key TEXT PRIMARY KEY CHECK (singleton_key = 'canonical'),
    data_generation INTEGER NOT NULL CHECK (data_generation = 6),
    root_instance_id TEXT NOT NULL,
    migration_head INTEGER NOT NULL CHECK (migration_head >= 1),
    seed_manifest_digest TEXT NOT NULL CHECK (length(seed_manifest_digest) = 64),
    canonical_schema_manifest_digest TEXT NOT NULL CHECK (length(canonical_schema_manifest_digest) = 64),
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version >= 1)
) STRICT;

CREATE TABLE skill_tags (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    skill_name    TEXT NOT NULL UNIQUE,
    audience_tags TEXT,
    scenario_tags TEXT,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE ssh_hosts (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    ssh_host_id             TEXT NOT NULL UNIQUE
                            CHECK (
                                length(ssh_host_id) = 36
                                AND lower(ssh_host_id) = ssh_host_id
                                AND ssh_host_id GLOB '????????-????-7???-[89ab]???-????????????'
                                AND replace(ssh_host_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                            ),
    user_id                 TEXT NOT NULL,
    name                    TEXT NOT NULL,
    host                    TEXT NOT NULL,
    port                    INTEGER NOT NULL DEFAULT 22,
    username                TEXT NOT NULL,
    -- One of: "password", "key", "certificate", "agent".
    auth_type               TEXT NOT NULL,
    -- AES-256-GCM encrypted credential material (nullable per auth_type).
    password_encrypted      TEXT,
    private_key_encrypted   TEXT,
    passphrase_encrypted    TEXT,
    certificate_encrypted   TEXT,
    sudo_password_encrypted TEXT,
    -- SHA256 host-key fingerprint recorded on first connect (for display).
    host_fingerprint        TEXT,
    -- One of: "unknown", "connected", "error".
    status                  TEXT NOT NULL DEFAULT 'unknown',
    last_connected_at       INTEGER,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    -- user_id is a logical reference to users.user_id (CanonicalUuidV7), so the
    -- column carries the same UUIDv7 CHECK the contract requires (mirrors
    -- terminal_sessions.user_id).
    CHECK (length(user_id) = 36 AND lower(user_id) = user_id AND user_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE system_settings (
    id                        INTEGER PRIMARY KEY AUTOINCREMENT,
    singleton_key             TEXT NOT NULL UNIQUE CHECK (singleton_key = 'system'),
    language                  TEXT NOT NULL DEFAULT 'en-US',
    notification_enabled      INTEGER NOT NULL DEFAULT 1 CHECK (notification_enabled IN (0, 1)),
    cron_notification_enabled INTEGER NOT NULL DEFAULT 0 CHECK (cron_notification_enabled IN (0, 1)),
    command_queue_enabled     INTEGER NOT NULL DEFAULT 0 CHECK (command_queue_enabled IN (0, 1)),
    save_upload_to_workspace  INTEGER NOT NULL DEFAULT 0 CHECK (save_upload_to_workspace IN (0, 1)),
    updated_at                INTEGER NOT NULL
);

CREATE TABLE tag_settings (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    tag           TEXT NOT NULL UNIQUE,
    webhook_id    TEXT
                  CHECK (
                      webhook_id IS NULL
                      OR (
                          length(webhook_id) = 36
                          AND lower(webhook_id) = webhook_id
                          AND webhook_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(webhook_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      )
                  ),
    description   TEXT NOT NULL DEFAULT '',
    updated_at    INTEGER NOT NULL,
    notify_events TEXT NOT NULL DEFAULT 'done,failed,needs_review'
);

CREATE TABLE terminal_scrollback (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    terminal_id TEXT NOT NULL UNIQUE,
    data        BLOB NOT NULL,
    updated_at  INTEGER NOT NULL,
    CHECK (length(terminal_id) = 36 AND lower(terminal_id) = terminal_id AND terminal_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(terminal_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE terminal_sessions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    terminal_id   TEXT NOT NULL UNIQUE
                  CHECK (
                      length(terminal_id) = 36
                      AND lower(terminal_id) = terminal_id
                      AND terminal_id GLOB '????????-????-7???-[89ab]???-????????????'
                      AND replace(terminal_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                  ),
    name          TEXT NOT NULL,
    cwd           TEXT NOT NULL,
    command       TEXT NOT NULL,
    args          TEXT NOT NULL DEFAULT '[]',
    env           TEXT,
    backend       TEXT,
    mode          TEXT,
    cols          INTEGER NOT NULL DEFAULT 80,
    rows          INTEGER NOT NULL DEFAULT 24,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    last_status   TEXT NOT NULL DEFAULT 'running'
                  CHECK (last_status IN ('running', 'exited', 'error')),
    exit_code     INTEGER,
    user_id       TEXT NOT NULL,
    pinned        INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    pinned_at     INTEGER,
    autowork      TEXT,
    idmm          TEXT CHECK (
                      idmm IS NULL
                      OR (json_valid(idmm) AND json_type(idmm) = 'object')
                  ),
    CHECK (length(user_id) = 36 AND lower(user_id) = user_id AND user_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE terminal_turn_admissions (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    turn_token          TEXT NOT NULL UNIQUE
                        CHECK (
                            length(turn_token) = 36
                            AND lower(turn_token) = turn_token
                            AND turn_token GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(turn_token, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    terminal_id         TEXT NOT NULL
                        CHECK (
                            length(terminal_id) = 36
                            AND lower(terminal_id) = terminal_id
                            AND terminal_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(terminal_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    pty_epoch           INTEGER NOT NULL CHECK (pty_epoch >= 0),
    requirement_id      TEXT NOT NULL
                        CHECK (
                            length(requirement_id) = 36
                            AND lower(requirement_id) = requirement_id
                            AND requirement_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(requirement_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    claim_generation    INTEGER NOT NULL CHECK (claim_generation >= 1),
    phase               TEXT NOT NULL DEFAULT 'admitted'
                        CHECK (
                            phase IN (
                                'admitted',
                                'body_written',
                                'effects_started',
                                'settled'
                            )
                        ),
    outcome             TEXT
                        CHECK (
                            outcome IS NULL
                            OR outcome IN ('done', 'failed', 'needs_review', 'cancelled')
                        ),
    detail              TEXT,
    admitted_at         INTEGER NOT NULL,
    effects_started_at  INTEGER,
    settled_at          INTEGER, claim_token TEXT
        CHECK (
            claim_token IS NULL
            OR (
                length(claim_token) = 64
                AND lower(claim_token) = claim_token
                AND claim_token NOT GLOB '*[^0-9a-f]*'
            )
        ),
    CHECK (
        (
            phase = 'admitted'
            AND outcome IS NULL
            AND effects_started_at IS NULL
            AND settled_at IS NULL
        )
        OR (
            phase = 'body_written'
            AND outcome IS NULL
            AND effects_started_at IS NULL
            AND settled_at IS NULL
        )
        OR (
            phase = 'effects_started'
            AND outcome IS NULL
            AND effects_started_at IS NOT NULL
            AND settled_at IS NULL
        )
        OR (
            phase = 'settled'
            AND outcome IS NOT NULL
            AND settled_at IS NOT NULL
        )
    )
);

CREATE TABLE users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id       TEXT NOT NULL UNIQUE
                  CHECK (
                      length(user_id) = 36
                      AND lower(user_id) = user_id
                      AND user_id GLOB '????????-????-7???-[89ab]???-????????????'
                      AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                  ),
    username      TEXT NOT NULL UNIQUE,
    email         TEXT UNIQUE,
    password_hash TEXT NOT NULL,
    avatar_path   TEXT,
    jwt_secret    TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    last_login    INTEGER
);

CREATE TABLE webhooks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    webhook_id  TEXT NOT NULL UNIQUE
                CHECK (
                    length(webhook_id) = 36
                    AND lower(webhook_id) = webhook_id
                    AND webhook_id GLOB '????????-????-7???-[89ab]???-????????????'
                    AND replace(webhook_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                ),
    name        TEXT NOT NULL,
    platform    TEXT NOT NULL DEFAULT 'lark',
    url         TEXT NOT NULL,
    secret      TEXT,
    description TEXT NOT NULL DEFAULT '',
    enabled     INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE workshop_assets (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id       TEXT NOT NULL UNIQUE
                   CHECK (
                       length(asset_id) = 36
                       AND lower(asset_id) = asset_id
                       AND asset_id GLOB '????????-????-7???-[89ab]???-????????????'
                       AND replace(asset_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                   ),
    kind           TEXT NOT NULL,
    title          TEXT NOT NULL,
    collection     TEXT,
    tags           TEXT NOT NULL DEFAULT '[]',
    rel_path       TEXT,
    thumb_rel_path TEXT,
    mime           TEXT,
    width          INTEGER,
    height         INTEGER,
    bytes          INTEGER,
    text_content   TEXT,
    in_library     INTEGER NOT NULL DEFAULT 1 CHECK (in_library IN (0, 1)),
    origin         TEXT CHECK (
                       origin IS NULL
                       OR (
                           json_valid(origin)
                           AND json_type(origin) = 'object'
                           AND json_type(origin, '$.task_id') IS NULL
                           AND json_type(origin, '$.providerId') IS NULL
                           AND json_type(origin, '$.canvasId') IS NULL
                           AND json_type(origin, '$.nodeId') IS NULL
                           AND json_type(origin, '$.creationTaskId') IS NULL
                           AND (
                               json_type(origin, '$.provider_id') IS NULL
                               OR (
                                   json_type(origin, '$.provider_id') = 'text'
                                   AND length(json_extract(origin, '$.provider_id')) = 36
                                   AND lower(json_extract(origin, '$.provider_id')) =
                                       json_extract(origin, '$.provider_id')
                                   AND json_extract(origin, '$.provider_id')
                                       GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(json_extract(origin, '$.provider_id'), '-', '')
                                       NOT GLOB '*[^0-9a-f]*'
                               )
                           )
                           AND (
                               json_type(origin, '$.canvas_id') IS NULL
                               OR (
                                   json_type(origin, '$.canvas_id') = 'text'
                                   AND length(json_extract(origin, '$.canvas_id')) = 36
                                   AND lower(json_extract(origin, '$.canvas_id')) =
                                       json_extract(origin, '$.canvas_id')
                                   AND json_extract(origin, '$.canvas_id')
                                       GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(json_extract(origin, '$.canvas_id'), '-', '')
                                       NOT GLOB '*[^0-9a-f]*'
                               )
                           )
                           AND (
                               json_type(origin, '$.node_id') IS NULL
                               OR (
                                   json_type(origin, '$.node_id') = 'text'
                                   AND length(json_extract(origin, '$.node_id')) = 36
                                   AND lower(json_extract(origin, '$.node_id')) =
                                       json_extract(origin, '$.node_id')
                                   AND json_extract(origin, '$.node_id')
                                       GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(json_extract(origin, '$.node_id'), '-', '')
                                       NOT GLOB '*[^0-9a-f]*'
                               )
                           )
                           AND (
                               json_type(origin, '$.creation_task_id') IS NULL
                               OR (
                                   json_type(origin, '$.creation_task_id') = 'text'
                                   AND length(json_extract(origin, '$.creation_task_id')) = 36
                                   AND lower(json_extract(origin, '$.creation_task_id')) =
                                       json_extract(origin, '$.creation_task_id')
                                   AND json_extract(origin, '$.creation_task_id')
                                       GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(json_extract(origin, '$.creation_task_id'), '-', '')
                                       NOT GLOB '*[^0-9a-f]*'
                               )
                           )
                       )
                   ),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
, deleted_at INTEGER
    CHECK (deleted_at IS NULL OR (typeof(deleted_at) = 'integer' AND deleted_at >= 0)), content_deleted_at INTEGER
    CHECK (
        (deleted_at IS NULL OR (in_library = 0 AND text_content IS NULL))
        AND (
            content_deleted_at IS NULL
            OR (
                typeof(content_deleted_at) = 'integer'
                AND deleted_at IS NOT NULL
                AND content_deleted_at >= deleted_at
                AND rel_path IS NULL
                AND thumb_rel_path IS NULL
            )
        )
    ));

INSERT INTO "agent_metadata" (id, agent_id, icon, name, name_i18n, description, description_i18n, backend, agent_type, agent_source, agent_source_info, source_key, enabled, command, args, env, native_skills_dirs, behavior_policy, yolo_id, agent_capabilities, auth_methods, config_options, available_modes, available_models, available_commands, sort_order, created_at, updated_at) VALUES (20, '0190f5fe-7c00-7a00-8000-000000000114', '/api/assets/logos/brand/nomi.svg', 'Nomi', NULL, NULL, NULL, NULL, 'nomi', 'internal', '{}', 'agent_builtin_nomi', 1, NULL, '[]', '[]', '[".nomi/skills"]', '{}', 'yolo', NULL, NULL, NULL, NULL, NULL, NULL, 100, 1789741863859, 1789741863859);

INSERT INTO "requirement_display_sequence" (id, singleton_key, last_no) VALUES (1, 'requirements', 0);

INSERT INTO "schema_metadata" (singleton_key, data_generation, root_instance_id, migration_head, seed_manifest_digest, canonical_schema_manifest_digest, projection_schema_version) VALUES ('canonical', 6, 'main-sqlite-agent-store', 1, '9cafc5df531a50ad4b63000a21938588d7ebada7010ff20a80449687f089fa1b', '00056199f1d6f9d9f79f285f3b32924993b8f39efd50ba9af0cc07c622adb65f', 1);

INSERT INTO "system_settings" (id, singleton_key, language, notification_enabled, cron_notification_enabled, command_queue_enabled, save_upload_to_workspace, updated_at) VALUES (1, 'system', 'en-US', 1, 0, 0, 0, 1789741863859);

-- Physical indexes are workload access paths, not a mirror of every logical reference.
-- Keep each table within five total SQLite B-trees (including UNIQUE auto-indexes);
-- the runtime schema contract below rejects budget regressions.
CREATE INDEX idx_agent_bindings_preset ON agent_bindings(json_extract(agent_binding_json, '$.preset_revision_ref.preset_id'));
CREATE INDEX idx_agent_deletion_audits_session_time ON agent_deletion_audits(agent_session_id, recorded_at, audit_id);
CREATE UNIQUE INDEX idx_agent_effects_resource_unsettled ON agent_effects(owner_domain, resource_key) WHERE state IN ('pending', 'unknown') AND resource_key IS NOT NULL;
CREATE INDEX idx_agent_effects_session_turn ON agent_effects(session_id, turn_id, created_at);
CREATE INDEX idx_agent_events_correlation ON agent_events(session_id, correlation_id, seq);
CREATE INDEX idx_agent_executions_owner_updated ON agent_executions(user_id, updated_at DESC);
CREATE INDEX idx_agent_executions_status_lease ON agent_executions(status, lease_expires_at);
CREATE INDEX idx_agent_messages_sequence ON agent_messages(session_id, first_seq, last_seq);
CREATE INDEX idx_agent_payloads_session ON agent_payloads(session_id);
CREATE INDEX idx_agent_presets_owner_active ON agent_presets(json_extract(owner_ref_json, '$.user_id'), preset_id) WHERE retired_at_ms IS NULL;
CREATE INDEX idx_agent_presets_ui_plugin ON agent_presets(json_extract(display_json, '$.ui_binding.selection.plugin_id'));
CREATE INDEX idx_agent_runtime_snapshots_revision ON agent_runtime_snapshots( json_extract(content_json, '$.preset_revision_ref.preset_id'), json_extract(content_json, '$.preset_revision_ref.revision'), json_extract(content_json, '$.preset_revision_ref.revision_digest') );
CREATE INDEX idx_agent_session_resources_session_kind ON agent_session_resources(session_id, resource_kind, binding_id);
CREATE INDEX idx_agent_sessions_owner_state ON agent_sessions(owner_ref_json, state);
CREATE INDEX idx_agent_turns_session_state ON agent_turns(session_id, state, accepted_at);
CREATE INDEX idx_channel_inbound_receipts_channel_plugin_id ON channel_inbound_receipts(channel_plugin_id);
CREATE INDEX idx_channel_pairing_codes_channel_plugin_id ON channel_pairing_codes(channel_plugin_id);
CREATE INDEX idx_channel_plugins_companion_id ON channel_plugins(companion_id);
CREATE INDEX idx_channel_session_bindings_user_id ON channel_session_bindings(channel_user_id);
CREATE INDEX idx_channel_sessions_channel_plugin_id ON channel_sessions(channel_plugin_id);
CREATE INDEX idx_channel_sessions_channel_user_id ON channel_sessions(channel_user_id);
CREATE INDEX idx_channel_sessions_conversation_id ON channel_sessions(conversation_id);
CREATE INDEX idx_channel_users_channel_plugin_id ON channel_users(channel_plugin_id);
CREATE INDEX idx_conversation_execution_links_conversation_id ON conversation_execution_links(conversation_id, relation, active, updated_at DESC);
CREATE INDEX idx_conversation_execution_links_execution_id ON conversation_execution_links(execution_id, relation, active, step_id, attempt_id);
CREATE INDEX idx_cpp_conversation_state ON channel_pending_prompts(conversation_id, state, id);
CREATE INDEX idx_cpp_plugin_chat ON channel_pending_prompts(channel_plugin_id, chat_id, state);
CREATE INDEX idx_cpp_session ON channel_pending_prompts(channel_session_id);
CREATE INDEX idx_creation_tasks_conversation ON creation_tasks(conversation_id, submitted_at, creation_task_id) WHERE conversation_id IS NOT NULL;
CREATE INDEX idx_creation_tasks_live ON creation_tasks(submitted_at, creation_task_id) WHERE status IN ('queued', 'running') AND deleted_at IS NULL;
CREATE INDEX idx_creation_tasks_live_project ON creation_tasks(project_id, submitted_at, creation_task_id) WHERE status IN ('queued', 'running') AND node_id IS NOT NULL;
CREATE INDEX idx_creation_tasks_live_provider ON creation_tasks(provider_id, model, submitted_at, creation_task_id) WHERE status IN ('queued', 'running');
CREATE INDEX idx_creative_agent_proposal_receipts_project ON creative_studio_agent_proposal_receipts(project_id);
CREATE UNIQUE INDEX idx_creative_agent_sessions_conversation ON creative_studio_agent_sessions(conversation_id);
CREATE INDEX idx_creative_agent_sessions_project ON creative_studio_agent_sessions(project_id);
CREATE UNIQUE INDEX idx_creative_agent_sessions_session ON creative_studio_agent_sessions(session_id);
CREATE INDEX idx_creative_studio_projects_updated ON creative_studio_projects(updated_at DESC, id DESC);
CREATE INDEX idx_creative_studio_templates_category ON creative_studio_templates(category, updated_at DESC, id DESC);
CREATE INDEX idx_creative_studio_templates_updated ON creative_studio_templates(updated_at DESC, id DESC);
CREATE INDEX idx_creative_template_runs_status ON creative_studio_template_runs(status, updated_at DESC, id DESC);
CREATE INDEX idx_creative_template_runs_template_id ON creative_studio_template_runs(template_id, updated_at DESC, id DESC);
CREATE INDEX idx_cron_job_runs_cron_job_id ON cron_job_runs(cron_job_id);
CREATE INDEX idx_cron_jobs_next_run ON cron_jobs(enabled, next_run_at);
CREATE INDEX idx_cron_jobs_owner_conversation ON cron_jobs(user_id, conversation_id, created_at);
CREATE INDEX idx_cron_run_reservations_cron_job_id ON cron_run_reservations(cron_job_id, status, created_at_ms, id);
CREATE INDEX idx_cron_run_reservations_projection ON cron_run_reservations(status, job_projection_state, created_at_ms);
CREATE INDEX idx_cs_agent_capability_receipts_agent ON cs_agent_capability_receipts(cs_agent_id, capability_id, created_at DESC);
CREATE INDEX idx_cs_audit_agent_time ON cs_audit_events(cs_agent_id, created_at);
CREATE INDEX idx_cs_channel_bindings_agent ON cs_channel_bindings(cs_agent_id);
CREATE UNIQUE INDEX idx_cs_channel_bindings_plugin ON cs_channel_bindings(channel_plugin_id);
CREATE INDEX idx_cs_dialogues_agent ON cs_dialogues(cs_agent_id, last_activity);
CREATE INDEX idx_cs_dialogues_channel_user ON cs_dialogues(channel_user_id);
CREATE UNIQUE INDEX idx_cs_dialogues_identity ON cs_dialogues(channel_plugin_id, channel_user_id, chat_id);
CREATE INDEX idx_cs_handoffs_agent_status ON cs_handoffs(cs_agent_id, status, created_at DESC);
CREATE INDEX idx_cs_messages_dialogue ON cs_messages(cs_dialogue_id, id);
CREATE INDEX idx_cs_notes_agent ON cs_notes(cs_agent_id);
CREATE INDEX idx_execution_events_unpublished ON agent_execution_events(execution_id, sequence) WHERE published_at IS NULL;
CREATE INDEX idx_execution_participants_execution_id ON agent_execution_participants(execution_id, retired_in_revision, participant_id);
CREATE INDEX idx_execution_participants_provider_id ON agent_execution_participants(provider_id, retired_in_revision, execution_id);
CREATE INDEX idx_execution_steps_execution_id ON agent_execution_steps(execution_id, superseded_in_revision, step_id);
CREATE INDEX idx_execution_templates_user_id ON agent_execution_templates(user_id);
CREATE INDEX idx_knowledge_binding_bases_knowledge_base_id ON knowledge_binding_bases(knowledge_base_id);
CREATE INDEX idx_knowledge_entries_knowledge_base_id ON knowledge_entries(knowledge_base_id, parent_entry_id, deleted_at, name);
CREATE INDEX idx_knowledge_entry_provenance_source_item_id ON knowledge_entry_provenance(knowledge_source_item_id, relationship, knowledge_entry_id);
CREATE INDEX idx_knowledge_source_items_knowledge_source_id ON knowledge_source_items(knowledge_source_id, state, ordinal, knowledge_source_item_id);
CREATE INDEX idx_knowledge_sources_default_parent_entry_id ON knowledge_sources(default_parent_entry_id) WHERE default_parent_entry_id IS NOT NULL;
CREATE INDEX idx_knowledge_sources_knowledge_base_id ON knowledge_sources(knowledge_base_id, state, created_at, knowledge_source_id);
CREATE INDEX idx_knowledge_tree_operations_knowledge_base_id ON knowledge_tree_operations(knowledge_base_id, created_at, operation_id);
CREATE INDEX idx_knowledge_tree_operations_pending_events ON knowledge_tree_operations(event_status, committed_at, operation_id) WHERE event_status = 'pending';
CREATE INDEX idx_knowledge_tree_operations_recovery ON knowledge_tree_operations(state, created_at, operation_id) WHERE state <> 'committed';
CREATE INDEX idx_nomi_remote_sessions_remote_binding_id ON nomi_remote_sessions(remote_binding_id);
CREATE INDEX idx_nomi_wave1_memory_receipts_agent_session_id ON nomi_wave1_memory_action_receipts(agent_session_id);
CREATE INDEX idx_nomi_wave1_memory_receipts_orphan_sweep ON nomi_wave1_memory_action_receipts(updated_at, agent_session_id);
CREATE INDEX idx_nomi_wave4_receipts_agent_session_id ON nomi_wave4_action_receipts(agent_session_id);
CREATE INDEX idx_nomi_wave4_receipts_orphan_sweep ON nomi_wave4_action_receipts(updated_at, agent_session_id);
CREATE INDEX idx_plugin_artifacts_package_id ON plugin_artifacts(package_id, package_version, artifact_id);
CREATE INDEX idx_plugin_build_operation_lineage_owner ON plugin_build_operation_lineage(owner_user_id, plugin_product_id, started_at_ms DESC);
CREATE INDEX idx_plugin_build_operation_lineage_project ON plugin_build_operation_lineage(project_id, owner_user_id, build_generation);
CREATE INDEX idx_plugin_catalog_publications_active_release_id ON plugin_catalog_publications(active_release_id);
CREATE INDEX idx_plugin_credential_bindings_credential_id ON plugin_credential_bindings(credential_id);
CREATE INDEX idx_plugin_library_state_owner ON plugin_library_state(owner_user_id, revision);
CREATE INDEX idx_plugin_mount_credential_bindings_credential_id ON plugin_mount_credential_bindings(credential_id);
CREATE INDEX idx_plugin_mount_revisions_artifact_id ON plugin_mount_revisions(artifact_id);
CREATE INDEX idx_plugin_products_owner ON plugin_products(owner_user_id, updated_at DESC, id DESC);
CREATE INDEX idx_plugin_projects_owner ON plugin_projects(owner_user_id, plugin_product_id, updated_at DESC);
CREATE INDEX idx_plugin_releases_artifact_id ON plugin_releases(artifact_id);
CREATE INDEX idx_plugin_releases_origin_operation_id ON plugin_releases(origin_operation_id);
CREATE INDEX idx_plugin_releases_product ON plugin_releases(owner_user_id, plugin_product_id, created_at DESC);
CREATE INDEX idx_plugin_releases_release_digest ON plugin_releases(owner_user_id, plugin_product_id, release_digest);
CREATE INDEX idx_plugin_service_test_receipts_current ON plugin_service_test_receipts( owner_user_id, plugin_product_id, release_id, release_digest, issued_at_ms DESC );
CREATE INDEX idx_plugin_surface_sessions_conversation_id ON plugin_surface_sessions(conversation_id);
CREATE INDEX idx_product_operations_plugin_mount_owner_id ON product_operations(owner_id, started_at_ms, operation_id) WHERE owner_kind = 'plugin_mount';
CREATE INDEX idx_product_operations_plugin_owner_id ON product_operations(owner_id, started_at_ms, operation_id) WHERE owner_kind = 'plugin';
CREATE INDEX idx_product_operations_plugin_project_owner_id ON product_operations(owner_id, started_at_ms, operation_id) WHERE owner_kind = 'plugin_project';
CREATE INDEX idx_provider_model_capabilities_task ON provider_model_capabilities(task, provider_id, model);
CREATE INDEX idx_requirement_pre_effect_abandon_owner_conversation ON requirement_pre_effect_abandon_guards(owner_conversation_id) WHERE owner_conversation_id IS NOT NULL;
CREATE INDEX idx_requirement_pre_effect_abandon_owner_terminal ON requirement_pre_effect_abandon_guards(owner_terminal_id) WHERE owner_terminal_id IS NOT NULL;
CREATE UNIQUE INDEX idx_requirement_pre_effect_abandon_requirement_id ON requirement_pre_effect_abandon_guards(requirement_id);
CREATE INDEX idx_requirement_tags_paused_requirement_id ON requirement_tags(paused_requirement_id);
CREATE INDEX idx_requirements_tag_order ON requirements(tag, sort_seq);
CREATE INDEX idx_ssh_hosts_user_id ON ssh_hosts(user_id);
CREATE INDEX idx_tag_settings_webhook_id ON tag_settings(webhook_id);
CREATE INDEX idx_template_participants_provider_id ON agent_execution_template_participants(provider_id, template_id);
CREATE INDEX idx_template_participants_template_id ON agent_execution_template_participants(template_id, template_participant_id);
CREATE INDEX idx_terminal_sessions_user_id ON terminal_sessions(user_id);
CREATE INDEX idx_workshop_assets_live_library_kind ON workshop_assets(in_library, kind, updated_at DESC, id DESC) WHERE deleted_at IS NULL;
CREATE INDEX idx_workshop_assets_live_updated ON workshop_assets(updated_at DESC, id DESC) WHERE deleted_at IS NULL;
CREATE INDEX idx_workshop_assets_pending_content_deletion ON workshop_assets(deleted_at, asset_id) WHERE deleted_at IS NOT NULL AND content_deleted_at IS NULL;
CREATE UNIQUE INDEX uq_channel_plugins_type_bot_key ON channel_plugins(type, bot_key) WHERE bot_key IS NOT NULL;
CREATE UNIQUE INDEX uq_cron_run_reservations_scheduled_occurrence ON cron_run_reservations(cron_job_id, schedule_revision, planned_at_ms) WHERE trigger_kind = 'scheduled';
CREATE UNIQUE INDEX uq_cs_handoffs_active_dialogue ON cs_handoffs(cs_dialogue_id) WHERE status IN ('pending', 'claimed');
CREATE UNIQUE INDEX uq_knowledge_bindings_target_companion_id ON knowledge_bindings(target_companion_id) WHERE target_kind = 'companion' AND target_companion_id IS NOT NULL;
CREATE UNIQUE INDEX uq_knowledge_bindings_target_conversation_id ON knowledge_bindings(target_conversation_id) WHERE target_kind = 'conversation' AND target_conversation_id IS NOT NULL;
CREATE UNIQUE INDEX uq_knowledge_bindings_target_terminal_id ON knowledge_bindings(target_terminal_id) WHERE target_kind = 'terminal' AND target_terminal_id IS NOT NULL;
CREATE UNIQUE INDEX uq_knowledge_bindings_target_workpath ON knowledge_bindings(target_workpath) WHERE target_kind = 'workpath' AND target_workpath IS NOT NULL;
CREATE UNIQUE INDEX uq_knowledge_entries_live_portable_path ON knowledge_entries(knowledge_base_id, portable_rel_path) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX uq_knowledge_entries_live_rel_path ON knowledge_entries(knowledge_base_id, rel_path) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX uq_knowledge_entry_provenance_entry_id ON knowledge_entry_provenance(knowledge_entry_id);
CREATE UNIQUE INDEX uq_knowledge_entry_provenance_managed_source_item ON knowledge_entry_provenance(knowledge_source_item_id) WHERE relationship = 'managed';
CREATE UNIQUE INDEX uq_knowledge_source_items_live_normalized_url ON knowledge_source_items(knowledge_source_id, normalized_url) WHERE state <> 'removed';
CREATE UNIQUE INDEX uq_knowledge_source_items_live_ordinal ON knowledge_source_items(knowledge_source_id, ordinal) WHERE state <> 'removed';
CREATE UNIQUE INDEX uq_knowledge_sources_live_kind ON knowledge_sources(knowledge_base_id, kind) WHERE state <> 'removed';
CREATE UNIQUE INDEX uq_requirements_active_conversation_owner ON requirements(owner_conversation_id) WHERE status = 'in_progress' AND owner_conversation_id IS NOT NULL;
CREATE UNIQUE INDEX uq_requirements_active_terminal_owner ON requirements(owner_terminal_id) WHERE status = 'in_progress' AND owner_terminal_id IS NOT NULL;
CREATE UNIQUE INDEX uq_terminal_turn_admissions_exact_claim ON terminal_turn_admissions( terminal_id, pty_epoch, requirement_id, claim_generation );
CREATE UNIQUE INDEX uq_terminal_turn_admissions_requirement_claim ON terminal_turn_admissions(requirement_id, claim_generation);
CREATE UNIQUE INDEX uq_workshop_assets_prompt_library_identity ON workshop_assets( json_extract(origin, '$.prompt_library_source'), json_extract(origin, '$.prompt_library_id') ) WHERE kind = 'text' AND deleted_at IS NULL AND json_type(origin, '$.prompt_library_source') = 'text' AND json_type(origin, '$.prompt_library_id') = 'text';
CREATE TRIGGER channel_inbound_receipts_identity_immutable
BEFORE UPDATE OF
    operation_key,
    user_scope_id,
    channel_plugin_scope_id,
    platform,
    chat_id,
    provider_event_id,
    payload_hash,
    created_at
ON channel_inbound_receipts
BEGIN
    SELECT RAISE(ABORT, 'channel inbound receipt identity is immutable');
END;

CREATE TRIGGER channel_inbound_receipts_no_delete
BEFORE DELETE ON channel_inbound_receipts
BEGIN
    SELECT RAISE(ABORT, 'channel inbound receipts are retained indefinitely');
END;

CREATE TRIGGER channel_inbound_receipts_scope_set_once
BEFORE UPDATE OF conversation_scope_id, message_scope_id
ON channel_inbound_receipts
WHEN OLD.phase <> 'effects_started'
  OR NEW.phase <> 'settled'
  OR OLD.conversation_scope_id IS NOT NULL
  OR OLD.message_scope_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'channel inbound outcome scope can only be set while settling');
END;

CREATE TRIGGER channel_session_bindings_identity_immutable
BEFORE UPDATE OF
    channel_plugin_id,
    channel_user_id,
    chat_id,
    channel_session_id,
    created_at
ON channel_session_bindings
BEGIN
    SELECT RAISE(ABORT, 'channel session binding identity is immutable');
END;

CREATE TRIGGER prevent_workshop_asset_content_resurrection
BEFORE UPDATE ON workshop_assets
WHEN OLD.deleted_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'deleted workshop asset cannot be restored')
    WHERE NEW.deleted_at IS NOT OLD.deleted_at
       OR NEW.asset_id IS NOT OLD.asset_id
       OR (NEW.rel_path IS NOT NULL AND NEW.rel_path IS NOT OLD.rel_path)
       OR (NEW.thumb_rel_path IS NOT NULL AND NEW.thumb_rel_path IS NOT OLD.thumb_rel_path)
       OR (OLD.content_deleted_at IS NOT NULL
           AND NEW.content_deleted_at IS NOT OLD.content_deleted_at);
END;

CREATE TRIGGER restrict_creation_task_deleted_assets_insert
BEFORE INSERT ON creation_tasks
BEGIN
    SELECT RAISE(ABORT, 'creation task references a deleted workshop asset')
    WHERE EXISTS (
        SELECT 1 FROM workshop_assets asset
        WHERE asset.deleted_at IS NOT NULL AND (
            EXISTS (SELECT 1 FROM json_each(NEW.input_bindings) input
                    WHERE json_extract(input.value, '$.asset_id') = asset.asset_id)
            OR EXISTS (SELECT 1 FROM json_each(NEW.result_asset_ids) result
                       WHERE result.value = asset.asset_id)
        )
    );
END;

CREATE TRIGGER restrict_creation_task_deleted_assets_update
BEFORE UPDATE OF input_bindings, result_asset_ids, status ON creation_tasks
BEGIN
    SELECT RAISE(ABORT, 'creation task references a deleted workshop asset')
    WHERE EXISTS (
        SELECT 1 FROM workshop_assets asset
        WHERE asset.deleted_at IS NOT NULL AND (
            EXISTS (
                SELECT 1 FROM json_each(NEW.input_bindings) input
                WHERE json_extract(input.value, '$.asset_id') = asset.asset_id
                  AND (NEW.status IN ('queued', 'running') OR NOT EXISTS (
                      SELECT 1 FROM json_each(OLD.input_bindings) old_input
                      WHERE json_extract(old_input.value, '$.asset_id') = asset.asset_id
                  ))
            )
            OR EXISTS (
                SELECT 1 FROM json_each(NEW.result_asset_ids) result
                WHERE result.value = asset.asset_id
                  AND (NEW.status IN ('queued', 'running') OR NOT EXISTS (
                      SELECT 1 FROM json_each(OLD.result_asset_ids) old_result
                      WHERE old_result.value = asset.asset_id
                  ))
            )
        )
    );
END;

CREATE TRIGGER restrict_template_run_deleted_assets_insert
BEFORE INSERT ON creative_studio_template_runs
BEGIN
    SELECT RAISE(ABORT, 'template run references a deleted workshop asset')
    WHERE EXISTS (
        SELECT 1 FROM workshop_assets asset
        JOIN json_tree(NEW.aggregate_json) ref ON ref.value = asset.asset_id
        LEFT JOIN json_tree(NEW.aggregate_json) parent ON parent.id = ref.parent
        WHERE asset.deleted_at IS NOT NULL AND ref.type = 'text' AND (
            ref.key IN ('assetId', 'defaultAssetId')
            OR parent.key IN ('assetIds', 'defaultAssetIds', 'referenceAssetIds', 'resultAssetIds')
        )
    );
END;

CREATE TRIGGER restrict_template_run_deleted_assets_update
BEFORE UPDATE OF aggregate_json, status ON creative_studio_template_runs
BEGIN
    SELECT RAISE(ABORT, 'template run references a deleted workshop asset')
    WHERE EXISTS (
        SELECT 1 FROM workshop_assets asset
        JOIN json_tree(NEW.aggregate_json) ref ON ref.value = asset.asset_id
        LEFT JOIN json_tree(NEW.aggregate_json) parent ON parent.id = ref.parent
        WHERE asset.deleted_at IS NOT NULL AND ref.type = 'text' AND (
            ref.key IN ('assetId', 'defaultAssetId')
            OR parent.key IN ('assetIds', 'defaultAssetIds', 'referenceAssetIds', 'resultAssetIds')
        ) AND (
            NEW.status NOT IN ('succeeded', 'failed', 'cancelled')
            OR NOT EXISTS (
                SELECT 1 FROM json_tree(OLD.aggregate_json) old_ref
                LEFT JOIN json_tree(OLD.aggregate_json) old_parent ON old_parent.id = old_ref.parent
                WHERE old_ref.value = asset.asset_id AND old_ref.type = 'text' AND (
                    old_ref.key IN ('assetId', 'defaultAssetId')
                    OR old_parent.key IN ('assetIds', 'defaultAssetIds', 'referenceAssetIds', 'resultAssetIds')
                )
            )
        )
    );
END;

CREATE TRIGGER restrict_workshop_asset_delete_creation_task_refs
BEFORE DELETE ON workshop_assets
WHEN EXISTS (
    SELECT 1
    FROM creation_tasks task
    WHERE EXISTS (
        SELECT 1 FROM json_each(task.input_bindings) input
        WHERE json_extract(input.value, '$.asset_id') = OLD.asset_id
    ) OR EXISTS (
        SELECT 1 FROM json_each(task.result_asset_ids) result
        WHERE result.value = OLD.asset_id
    )
)
BEGIN
    SELECT RAISE(ABORT, 'workshop asset is referenced by creation task input or result');
END;

CREATE TRIGGER trg_channel_plugins_owner_domain_insert_guard
BEFORE INSERT ON channel_plugins
WHEN NEW.owner_domain = 'customer_service' AND NEW.companion_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'customer-service channel bots cannot carry a companion binding');
END;

CREATE TRIGGER trg_channel_plugins_owner_domain_update_guard
BEFORE UPDATE OF owner_domain, companion_id ON channel_plugins
WHEN NEW.owner_domain = 'customer_service' AND NEW.companion_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'customer-service channel bots cannot carry a companion binding');
END;

CREATE TRIGGER trg_nomi_remote_events_append_only_delete
BEFORE DELETE ON nomi_remote_events
BEGIN
    SELECT RAISE(ABORT, 'NOMI REMOTE EVENTS ARE APPEND ONLY');
END;

CREATE TRIGGER trg_nomi_remote_events_append_only_update
BEFORE UPDATE ON nomi_remote_events
BEGIN
    SELECT RAISE(ABORT, 'NOMI REMOTE EVENTS ARE APPEND ONLY');
END;

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

CREATE TRIGGER trg_plugin_artifacts_immutable
BEFORE UPDATE ON plugin_artifacts
BEGIN
    SELECT RAISE(ABORT, 'plugin artifacts are immutable');
END;

CREATE TRIGGER trg_plugin_auto_apply_dependency_fence
BEFORE UPDATE OF
    apply_mode,
    auto_apply_mount_id,
    auto_apply_authorization_revision,
    auto_apply_authorized_at
ON plugin_projects
WHEN EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_intents intent
     WHERE intent.project_id = OLD.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'plugin auto Apply authorization is fenced by a dependency mutation');
END;

CREATE TRIGGER trg_plugin_candidate_imported_provenance_guard
BEFORE INSERT ON plugin_ready_candidates
WHEN NEW.imported_test_provenance_json IS NOT NULL
AND NEW.origin_kind <> 'import'
BEGIN
    SELECT RAISE(ABORT, 'only imported Plugin Candidates can carry source Test provenance');
END;

CREATE TRIGGER trg_plugin_candidate_receipts_exact_insert
BEFORE INSERT ON plugin_candidate_test_receipts
WHEN NOT EXISTS (
    SELECT 1
    FROM plugin_ready_candidates candidate
    WHERE candidate.candidate_id = NEW.candidate_id
      AND candidate.candidate_digest = NEW.candidate_digest
      AND candidate.artifact_id = NEW.artifact_id
      AND candidate.artifact_digest = NEW.artifact_digest
)
BEGIN
    SELECT RAISE(ABORT, 'plugin candidate test receipt must bind one exact candidate');
END;

CREATE TRIGGER trg_plugin_candidate_receipts_immutable
BEFORE UPDATE ON plugin_candidate_test_receipts
BEGIN
    SELECT RAISE(ABORT, 'plugin candidate test receipts are immutable');
END;

CREATE TRIGGER trg_plugin_dependency_commit_cleanup
AFTER UPDATE ON plugin_projects
WHEN EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_commits commit_marker
     WHERE commit_marker.project_id = NEW.project_id
)
BEGIN
    DELETE FROM plugin_dependency_mutation_commits
     WHERE project_id = NEW.project_id;
END;

CREATE TRIGGER trg_plugin_dependency_commit_insert_guard
BEFORE INSERT ON plugin_dependency_mutation_commits
WHEN NOT EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_intents intent
     WHERE intent.intent_id = NEW.intent_id
       AND intent.project_id = NEW.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'plugin dependency commit marker must bind a durable intent');
END;

CREATE TRIGGER trg_plugin_dependency_intent_insert_guard
BEFORE INSERT ON plugin_dependency_mutation_intents
WHEN NOT EXISTS (
    SELECT 1
      FROM plugin_projects project
     WHERE project.project_id = NEW.project_id
       AND project.owner_user_id = NEW.owner_user_id
       AND project.managed_source_path IS NOT NULL
       AND project.updated_at = NEW.expected_project_updated_at
       AND project.build_generation = NEW.expected_build_generation
       AND project.source_head_digest = NEW.expected_source_digest
       AND project.dependency_lock_digest = NEW.expected_lock_digest
)
BEGIN
    SELECT RAISE(ABORT, 'plugin dependency intent must bind the exact managed Project head');
END;

CREATE TRIGGER trg_plugin_dependency_project_delete_guard
BEFORE DELETE ON plugin_projects
WHEN EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_intents intent
     WHERE intent.project_id = OLD.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'plugin Project delete is fenced by a dependency mutation intent');
END;

CREATE TRIGGER trg_plugin_dependency_project_update_guard
BEFORE UPDATE ON plugin_projects
WHEN EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_intents intent
     WHERE intent.project_id = OLD.project_id
)
AND NOT EXISTS (
    SELECT 1
      FROM plugin_dependency_mutation_commits commit_marker
      JOIN plugin_dependency_mutation_intents intent
        ON intent.intent_id = commit_marker.intent_id
       AND intent.project_id = commit_marker.project_id
     WHERE commit_marker.project_id = OLD.project_id
       AND OLD.updated_at = intent.expected_project_updated_at
       AND OLD.build_generation = intent.expected_build_generation
       AND OLD.source_head_digest = intent.expected_source_digest
       AND OLD.dependency_lock_digest = intent.expected_lock_digest
       AND NEW.source_head_digest = intent.next_source_digest
       AND NEW.dependency_lock_digest = intent.next_lock_digest
       AND NEW.build_generation = intent.expected_build_generation + 1
       AND NEW.updated_at > intent.expected_project_updated_at
       AND NEW.id = OLD.id
       AND NEW.project_id = OLD.project_id
       AND NEW.owner_user_id = OLD.owner_user_id
       AND NEW.package_id = OLD.package_id
       AND NEW.managed_source_path IS OLD.managed_source_path
       AND NEW.linked_mount_id IS OLD.linked_mount_id
       AND NEW.ready_candidate_id IS OLD.ready_candidate_id
       AND NEW.created_at = OLD.created_at
       AND NEW.display_name = OLD.display_name
       AND NEW.description = OLD.description
)
BEGIN
    SELECT RAISE(ABORT, 'plugin Project is fenced by a dependency mutation intent');
END;

CREATE TRIGGER trg_plugin_mount_binding_revision_cleanup
AFTER UPDATE OF credential_bindings_revision ON plugin_mounts
WHEN NEW.credential_bindings_revision <> OLD.credential_bindings_revision
BEGIN
    DELETE FROM plugin_credential_binding_mutations
     WHERE mount_id = NEW.mount_id;
END;

CREATE TRIGGER trg_plugin_mount_binding_revision_guard
BEFORE UPDATE OF credential_bindings_revision ON plugin_mounts
WHEN (
    NEW.credential_bindings_revision <> OLD.credential_bindings_revision
    AND NOT EXISTS (
        SELECT 1
        FROM plugin_credential_binding_mutations mutation
        WHERE mutation.mount_id = OLD.mount_id
          AND mutation.expected_mount_revision = OLD.revision
          AND mutation.expected_bindings_revision = OLD.credential_bindings_revision
          AND mutation.target_bindings_revision = NEW.credential_bindings_revision
          AND mutation.updated_at = NEW.updated_at
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin mount credential bindings revision requires a whole-group CAS');
END;

CREATE TRIGGER trg_plugin_mount_binding_revision_shape_guard
BEFORE UPDATE OF credential_bindings_revision ON plugin_mounts
WHEN NEW.credential_bindings_revision <> OLD.credential_bindings_revision
 AND NEW.credential_bindings_revision <> OLD.credential_bindings_revision + 1
BEGIN
    SELECT RAISE(ABORT, 'plugin mount credential bindings revision must advance by one');
END;

CREATE TRIGGER trg_plugin_mount_config_revision_guard
BEFORE UPDATE OF config_json, config_schema_digest, config_revision
ON plugin_mounts
WHEN (
    (
        (NEW.config_json IS NOT OLD.config_json
         OR NEW.config_schema_digest IS NOT OLD.config_schema_digest)
        AND NEW.config_revision <= OLD.config_revision
    )
    OR
    (
        NEW.config_revision <> OLD.config_revision
        AND NEW.config_json IS OLD.config_json
        AND NEW.config_schema_digest IS OLD.config_schema_digest
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin mount config changes require an advancing config revision');
END;

CREATE TRIGGER trg_plugin_mount_config_revision_shape_guard
BEFORE UPDATE OF config_schema_digest, config_revision ON plugin_mounts
WHEN NEW.config_revision = 0 AND NEW.config_schema_digest IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'plugin mount config schema requires a positive config revision');
END;

CREATE TRIGGER trg_plugin_mount_credential_binding_delete_guard
BEFORE DELETE ON plugin_mount_credential_bindings
WHEN NOT EXISTS (
    SELECT 1
    FROM plugin_mounts mount
    LEFT JOIN plugin_credential_binding_mutations mutation
      ON mutation.mount_id = mount.mount_id
     AND mutation.expected_mount_revision = mount.revision
     AND mutation.expected_current_artifact_digest IS mount.current_artifact_digest
     AND mutation.expected_bindings_revision = mount.credential_bindings_revision
     AND mutation.target_bindings_revision = mount.credential_bindings_revision + 1
    WHERE mount.mount_id = OLD.mount_id
      AND (
          (mount.delete_pending = 0 OR mutation.allow_delete_pending = 1)
          AND mutation.mount_id IS NOT NULL
      )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin credential binding delete requires a whole-group CAS');
END;

CREATE TRIGGER trg_plugin_mount_credential_binding_insert_guard
BEFORE INSERT ON plugin_mount_credential_bindings
WHEN NOT EXISTS (
    SELECT 1
    FROM plugin_mounts mount
    LEFT JOIN plugin_credential_binding_mutations mutation
      ON mutation.mount_id = mount.mount_id
     AND mutation.expected_mount_revision = mount.revision
     AND mutation.expected_current_artifact_digest IS mount.current_artifact_digest
     AND mutation.expected_bindings_revision = mount.credential_bindings_revision
     AND mutation.target_bindings_revision = mount.credential_bindings_revision + 1
    WHERE mount.mount_id = NEW.mount_id
      AND (
          (mount.delete_pending = 0 OR mutation.allow_delete_pending = 1)
          AND mutation.mount_id IS NOT NULL
      )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin credential binding insert requires a whole-group CAS');
END;

CREATE TRIGGER trg_plugin_mount_credential_binding_update_guard
BEFORE UPDATE ON plugin_mount_credential_bindings
WHEN NOT EXISTS (
    SELECT 1
    FROM plugin_mounts mount
    LEFT JOIN plugin_credential_binding_mutations mutation
      ON mutation.mount_id = mount.mount_id
     AND mutation.expected_mount_revision = mount.revision
     AND mutation.expected_current_artifact_digest IS mount.current_artifact_digest
     AND mutation.expected_bindings_revision = mount.credential_bindings_revision
     AND mutation.target_bindings_revision = mount.credential_bindings_revision + 1
    WHERE mount.mount_id = OLD.mount_id
      AND (
          (mount.delete_pending = 0 OR mutation.allow_delete_pending = 1)
          AND mutation.mount_id IS NOT NULL
      )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin credential binding update requires a whole-group CAS');
END;

CREATE TRIGGER trg_plugin_mount_credential_binding_updated_at_monotonic
BEFORE UPDATE OF updated_at ON plugin_mount_credential_bindings
WHEN NEW.updated_at < OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'plugin credential binding updated_at cannot move backwards');
END;

CREATE TRIGGER trg_plugin_mount_kv_updated_at_monotonic
BEFORE UPDATE OF updated_at ON plugin_mount_kv
WHEN NEW.updated_at < OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'plugin KV updated_at cannot move backwards');
END;

CREATE TRIGGER trg_plugin_mount_pointer_insert_guard
BEFORE INSERT ON plugin_mounts
WHEN (
    NEW.current_revision_id IS NOT NULL
    OR NEW.previous_revision_id IS NOT NULL
)
BEGIN
    SELECT RAISE(ABORT, 'plugin mount must be created without executable pointers');
END;

CREATE TRIGGER trg_plugin_mount_pointer_update_guard
BEFORE UPDATE OF current_artifact_digest, previous_artifact_digest,
                 current_revision_id, previous_revision_id
ON plugin_mounts
WHEN (
    (
        NEW.current_revision_id IS NOT NULL
        AND NOT EXISTS (
            SELECT 1
            FROM plugin_mount_revisions revision
            WHERE revision.mount_revision_id = NEW.current_revision_id
              AND revision.mount_id = NEW.mount_id
              AND revision.artifact_digest = NEW.current_artifact_digest
        )
    )
    OR
    (
        NEW.previous_revision_id IS NOT NULL
        AND NOT EXISTS (
            SELECT 1
            FROM plugin_mount_revisions revision
            WHERE revision.mount_revision_id = NEW.previous_revision_id
              AND revision.mount_id = NEW.mount_id
              AND revision.artifact_digest = NEW.previous_artifact_digest
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin mount executable pointers require exact mount revisions');
END;

CREATE TRIGGER trg_plugin_mount_revision_authorization_guard
BEFORE INSERT ON plugin_mount_revisions
WHEN NOT (
    (
        NEW.apply_authorization_kind = 'manual_user_confirmation'
        AND NEW.auto_apply_authorization_revision IS NULL
    )
    OR (
        NEW.apply_authorization_kind = 'standing_auto'
        AND NEW.auto_apply_authorization_revision IS NOT NULL
        AND EXISTS (
            SELECT 1
              FROM plugin_ready_candidates candidate
              JOIN plugin_projects project
                ON project.project_id = candidate.project_id
             WHERE candidate.candidate_id = NEW.candidate_key
               AND project.apply_mode = 'auto_compatible_when_idle'
               AND project.auto_apply_mount_id = NEW.mount_id
               AND project.auto_apply_authorization_revision =
                   NEW.auto_apply_authorization_revision
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin Mount revision requires an exact Apply authorization');
END;

CREATE TRIGGER trg_plugin_mount_revision_insert_guard
BEFORE INSERT ON plugin_mount_revisions
WHEN NOT EXISTS (
        SELECT 1
        FROM plugin_mounts mount
        JOIN plugin_ready_candidates candidate
          ON candidate.candidate_id = NEW.candidate_key
        JOIN plugin_artifacts artifact
          ON artifact.artifact_id = NEW.artifact_id
         AND artifact.artifact_digest = NEW.artifact_digest
        WHERE mount.mount_id = NEW.mount_id
          AND candidate.candidate_digest = NEW.candidate_digest
          AND candidate.artifact_id = NEW.artifact_id
          AND candidate.artifact_digest = NEW.artifact_digest
          AND candidate.base_target_digest IS mount.current_artifact_digest
          AND artifact.package_id = mount.package_id
          AND NEW.revision = mount.revision + 1
    )
BEGIN
    SELECT RAISE(ABORT, 'plugin mount revision requires exact candidate, base, artifact, and next revision');
END;

CREATE TRIGGER trg_plugin_mount_revisions_immutable
BEFORE UPDATE ON plugin_mount_revisions
BEGIN
    SELECT RAISE(ABORT, 'plugin mount revisions are immutable');
END;

CREATE TRIGGER trg_plugin_mount_transition_shape_guard
BEFORE UPDATE OF current_artifact_digest, previous_artifact_digest,
                 current_revision_id, previous_revision_id, revision
ON plugin_mounts
WHEN (
    NEW.current_artifact_digest IS NOT OLD.current_artifact_digest
    OR NEW.previous_artifact_digest IS NOT OLD.previous_artifact_digest
    OR NEW.current_revision_id IS NOT OLD.current_revision_id
    OR NEW.previous_revision_id IS NOT OLD.previous_revision_id
)
AND (
    NEW.revision <> OLD.revision + 1
    OR NOT (
        (
            NEW.current_revision_id IS NULL
            AND NEW.previous_revision_id IS NULL
            AND NEW.current_artifact_digest IS NULL
            AND NEW.previous_artifact_digest IS NULL
            AND NEW.enabled = 0
            AND NEW.retained = 1
        )
        OR
        (
            OLD.previous_revision_id IS NOT NULL
            AND NEW.current_revision_id IS OLD.previous_revision_id
            AND NEW.previous_revision_id IS OLD.current_revision_id
            AND NEW.current_artifact_digest IS OLD.previous_artifact_digest
            AND NEW.previous_artifact_digest IS OLD.current_artifact_digest
        )
        OR
        (
            NEW.previous_revision_id IS OLD.current_revision_id
            AND NEW.previous_artifact_digest IS OLD.current_artifact_digest
            AND EXISTS (
                SELECT 1
                FROM plugin_mount_revisions revision
                WHERE revision.mount_revision_id = NEW.current_revision_id
                  AND revision.mount_id = NEW.mount_id
                  AND revision.revision = NEW.revision
                  AND revision.artifact_digest = NEW.current_artifact_digest
            )
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin mount transition must be exact apply, restore, or uninstall');
END;

CREATE TRIGGER trg_plugin_mount_updated_at_monotonic
BEFORE UPDATE OF updated_at ON plugin_mounts
WHEN NEW.updated_at < OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'plugin mount updated_at cannot move backwards');
END;

CREATE TRIGGER trg_plugin_project_auto_apply_insert_guard
BEFORE INSERT ON plugin_projects
WHEN NOT (
    NEW.apply_mode = 'ask_before_apply'
    AND NEW.auto_apply_mount_id IS NULL
    AND NEW.auto_apply_authorization_revision = 0
    AND NEW.auto_apply_authorized_at IS NULL
)
BEGIN
    SELECT RAISE(ABORT, 'plugin Project must begin without standing auto Apply authorization');
END;

CREATE TRIGGER trg_plugin_project_auto_apply_revision_guard
BEFORE UPDATE OF
    apply_mode,
    auto_apply_mount_id,
    auto_apply_authorization_revision,
    auto_apply_authorized_at
ON plugin_projects
WHEN NEW.auto_apply_authorization_revision <> OLD.auto_apply_authorization_revision + 1
BEGIN
    SELECT RAISE(ABORT, 'plugin auto Apply authorization revision must advance exactly once');
END;

CREATE TRIGGER trg_plugin_project_auto_apply_update_guard
BEFORE UPDATE OF
    apply_mode,
    auto_apply_mount_id,
    auto_apply_authorization_revision,
    auto_apply_authorized_at,
    linked_mount_id,
    managed_source_path,
    source_head_digest,
    dependency_lock_digest
ON plugin_projects
WHEN NOT (
    (
        NEW.apply_mode = 'ask_before_apply'
        AND NEW.auto_apply_mount_id IS NULL
        AND NEW.auto_apply_authorized_at IS NULL
    )
    OR (
        NEW.apply_mode = 'auto_compatible_when_idle'
        AND NEW.auto_apply_mount_id IS NOT NULL
        AND NEW.auto_apply_mount_id = NEW.linked_mount_id
        AND NEW.auto_apply_authorization_revision > 0
        AND NEW.auto_apply_authorized_at IS NOT NULL
        AND NEW.managed_source_path IS NOT NULL
        AND NEW.source_head_digest IS NOT NULL
        AND NEW.dependency_lock_digest IS NOT NULL
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin auto Apply authorization has an invalid Project shape');
END;

CREATE TRIGGER trg_plugin_project_initial_pointer_guard
BEFORE INSERT ON plugin_projects
WHEN NEW.linked_mount_id IS NOT NULL OR NEW.ready_candidate_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'plugin project must be created without linked or ready pointers');
END;

CREATE TRIGGER trg_plugin_project_metadata_insert_guard
BEFORE INSERT ON plugin_projects
WHEN NEW.plugin_product_id IS NULL
 AND NEW.display_name = 'Plugin Runtime Project'
BEGIN
    SELECT RAISE(ABORT, 'plugin project display metadata must be explicit');
END;

CREATE TRIGGER trg_plugin_project_metadata_update_guard
BEFORE UPDATE OF display_name, description ON plugin_projects
WHEN NEW.plugin_product_id IS NULL
 AND NEW.display_name = 'Plugin Runtime Project'
BEGIN
    SELECT RAISE(ABORT, 'plugin project display metadata must be explicit');
END;

CREATE TRIGGER trg_plugin_project_ready_candidate_update_guard
BEFORE UPDATE OF ready_candidate_id ON plugin_projects
WHEN NEW.ready_candidate_id IS NOT NULL
 AND NOT EXISTS (
    SELECT 1
    FROM plugin_ready_candidates candidate
    WHERE candidate.candidate_id = NEW.ready_candidate_id
      AND candidate.project_id = NEW.project_id
      AND candidate.build_generation = NEW.build_generation
)
BEGIN
    SELECT RAISE(ABORT, 'plugin project ready pointer requires its exact current-generation candidate');
END;

CREATE TRIGGER trg_plugin_ready_candidate_insert_guard
BEFORE INSERT ON plugin_ready_candidates
WHEN NOT EXISTS (
        SELECT 1
        FROM plugin_projects project
        JOIN plugin_artifacts artifact
          ON artifact.artifact_id = NEW.artifact_id
         AND artifact.artifact_digest = NEW.artifact_digest
        JOIN product_operations operation
          ON operation.operation_id = NEW.origin_operation_id
        WHERE project.project_id = NEW.project_id
          AND project.package_id = artifact.package_id
          AND project.build_generation = NEW.build_generation
          AND operation.kind = NEW.origin_kind
          AND operation.owner_kind = 'plugin_project'
          AND operation.owner_id = project.project_id
          AND operation.kind IN ('build', 'import')
          AND operation.state = 'succeeded'
          AND (
              (
                  NEW.origin_kind = 'import'
                  AND project.managed_source_path IS NULL
                  AND NEW.source_snapshot_digest IS NULL
                  AND NEW.dependency_lock_digest IS NULL
              )
              OR
              (
                  project.managed_source_path IS NOT NULL
                  AND NEW.build_generation > 0
                  AND NEW.source_snapshot_digest = project.source_head_digest
                  AND NEW.dependency_lock_digest = project.dependency_lock_digest
              )
          )
    )
BEGIN
    SELECT RAISE(ABORT, 'plugin ready candidate requires exact project generation, artifact, and successful origin operation');
END;

CREATE TRIGGER trg_plugin_source_build_start_guard
BEFORE INSERT ON product_operations
WHEN NEW.owner_kind = 'plugin'
AND NEW.kind = 'build'
AND NEW.state = 'running'
AND EXISTS (
    SELECT 1 FROM plugin_source_mutation_intents intent
     WHERE intent.plugin_product_id = NEW.owner_id
)
BEGIN
    SELECT RAISE(ABORT, 'Plugin Build is fenced by a Source mutation intent');
END;

CREATE TRIGGER trg_plugin_source_commit_cleanup
AFTER UPDATE ON plugin_projects
WHEN EXISTS (
    SELECT 1 FROM plugin_source_mutation_commits commit_marker
     WHERE commit_marker.project_id = NEW.project_id
)
BEGIN
    DELETE FROM plugin_source_mutation_commits
     WHERE project_id = NEW.project_id;
END;

CREATE TRIGGER trg_plugin_source_commit_insert_guard
BEFORE INSERT ON plugin_source_mutation_commits
WHEN NOT EXISTS (
    SELECT 1
      FROM plugin_source_mutation_intents intent
     WHERE intent.intent_id = NEW.intent_id
       AND intent.project_id = NEW.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'Plugin Source commit marker must bind a durable intent');
END;

CREATE TRIGGER trg_plugin_source_intent_insert_guard
BEFORE INSERT ON plugin_source_mutation_intents
WHEN NOT EXISTS (
    SELECT 1
      FROM plugin_products product
      JOIN plugin_projects project
        ON project.plugin_product_id = product.plugin_product_id
       AND project.owner_user_id = product.owner_user_id
     WHERE product.owner_user_id = NEW.owner_user_id
       AND product.plugin_product_id = NEW.plugin_product_id
       AND product.product_revision = NEW.expected_product_revision
       AND product.lifecycle IN ('enabled', 'disabled')
       AND project.project_id = NEW.project_id
       AND project.project_revision = NEW.expected_project_revision
       AND project.source_state = 'editable'
       AND project.build_generation = NEW.expected_build_generation
       AND project.source_head_digest = NEW.expected_source_digest
       AND NOT EXISTS (
           SELECT 1
             FROM product_operations operation
            WHERE operation.owner_kind = 'plugin'
              AND operation.owner_id = NEW.plugin_product_id
              AND operation.kind = 'build'
              AND operation.state = 'running'
       )
)
BEGIN
    SELECT RAISE(ABORT, 'Plugin Source intent must bind the exact editable Project head');
END;

CREATE TRIGGER trg_plugin_source_product_delete_guard
BEFORE DELETE ON plugin_products
WHEN EXISTS (
    SELECT 1 FROM plugin_source_mutation_intents intent
     WHERE intent.plugin_product_id = OLD.plugin_product_id
)
BEGIN
    SELECT RAISE(ABORT, 'Plugin delete is fenced by a Source mutation intent');
END;

CREATE TRIGGER trg_plugin_source_product_update_guard
BEFORE UPDATE ON plugin_products
WHEN EXISTS (
    SELECT 1 FROM plugin_source_mutation_intents intent
     WHERE intent.plugin_product_id = OLD.plugin_product_id
)
BEGIN
    SELECT RAISE(ABORT, 'Plugin Product is fenced by a Source mutation intent');
END;

CREATE TRIGGER trg_plugin_source_project_delete_guard
BEFORE DELETE ON plugin_projects
WHEN EXISTS (
    SELECT 1 FROM plugin_source_mutation_intents intent
     WHERE intent.project_id = OLD.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'Plugin Project delete is fenced by a Source mutation intent');
END;

CREATE TRIGGER trg_plugin_source_project_update_guard
BEFORE UPDATE ON plugin_projects
WHEN EXISTS (
    SELECT 1
      FROM plugin_source_mutation_intents intent
     WHERE intent.project_id = OLD.project_id
)
AND NOT EXISTS (
    SELECT 1
      FROM plugin_source_mutation_commits commit_marker
      JOIN plugin_source_mutation_intents intent
        ON intent.intent_id = commit_marker.intent_id
       AND intent.project_id = commit_marker.project_id
     WHERE commit_marker.project_id = OLD.project_id
       AND OLD.owner_user_id = intent.owner_user_id
       AND OLD.plugin_product_id = intent.plugin_product_id
       AND OLD.project_revision = intent.expected_project_revision
       AND OLD.build_generation = intent.expected_build_generation
       AND OLD.source_head_digest = intent.expected_source_digest
       AND NEW.project_revision = OLD.project_revision + 1
       AND NEW.build_generation = intent.next_build_generation
       AND NEW.source_head_digest = intent.next_source_digest
       AND NEW.updated_at > OLD.updated_at
       AND NEW.id = OLD.id
       AND NEW.project_id = OLD.project_id
       AND NEW.plugin_product_id = OLD.plugin_product_id
       AND NEW.owner_user_id = OLD.owner_user_id
       AND NEW.source_state = OLD.source_state
       AND NEW.managed_source_path IS OLD.managed_source_path
       AND NEW.dependency_lock_digest IS OLD.dependency_lock_digest
       AND NEW.build_profile_version IS OLD.build_profile_version
       AND NEW.created_at = OLD.created_at
)
BEGIN
    SELECT RAISE(ABORT, 'Plugin Project is fenced by a Source mutation intent');
END;

CREATE TRIGGER trg_product_operation_log_insert_guard
BEFORE INSERT ON product_operations
WHEN EXISTS (
    SELECT 1
    FROM json_each(NEW.bounded_log_tail_json) entry
    WHERE entry.type <> 'text'
       OR length(entry.value) > 4096
       OR instr(entry.value, char(0)) > 0
)
BEGIN
    SELECT RAISE(ABORT, 'product operation log tail lines must be bounded strings');
END;

CREATE TRIGGER trg_product_operation_log_update_guard
BEFORE UPDATE OF bounded_log_tail_json ON product_operations
WHEN EXISTS (
    SELECT 1
    FROM json_each(NEW.bounded_log_tail_json) entry
    WHERE entry.type <> 'text'
       OR length(entry.value) > 4096
       OR instr(entry.value, char(0)) > 0
)
BEGIN
    SELECT RAISE(ABORT, 'product operation log tail lines must be bounded strings');
END;

CREATE TRIGGER trg_product_operation_result_guard
BEFORE UPDATE OF result_artifact_digests_json ON product_operations
WHEN (
    NEW.state <> 'succeeded'
    AND NEW.result_artifact_digests_json <> '{}'
)
OR EXISTS (
    SELECT 1
      FROM json_each(NEW.result_artifact_digests_json) entry
     WHERE entry.type <> 'text'
        OR length(entry.key) NOT BETWEEN 1 AND 64
        OR entry.key GLOB '*[^a-z0-9._-]*'
        OR length(entry.value) <> 64
        OR lower(entry.value) <> entry.value
        OR entry.value GLOB '*[^0-9a-f]*'
)
BEGIN
    SELECT RAISE(ABORT, 'product operation result Artifacts must be bounded SHA-256 facts');
END;

CREATE TRIGGER trg_product_operation_result_insert_guard
BEFORE INSERT ON product_operations
WHEN NEW.result_artifact_digests_json <> '{}'
BEGIN
    SELECT RAISE(ABORT, 'product operation must begin without result Artifacts');
END;

CREATE TRIGGER trg_product_operations_terminal_immutable
BEFORE UPDATE ON product_operations
WHEN OLD.state <> 'running'
BEGIN
    SELECT RAISE(ABORT, 'terminal product operations are immutable');
END;

CREATE TRIGGER trg_requirements_absorb_done_cancelled
BEFORE UPDATE OF status ON requirements
FOR EACH ROW
WHEN OLD.status IN ('done', 'cancelled')
 AND NEW.status IS NOT OLD.status
BEGIN
    SELECT RAISE(ABORT, 'completed or cancelled Requirement status is immutable');
END;

CREATE TRIGGER trg_requirements_active_identity_exit_guard
BEFORE UPDATE ON requirements
FOR EACH ROW
WHEN OLD.status = 'in_progress'
 AND NEW.status IS NOT 'pending'
 AND (
        NEW.claim_generation IS NOT OLD.claim_generation
        OR NEW.claim_token IS NOT OLD.claim_token
        OR NEW.owner_conversation_id IS NOT OLD.owner_conversation_id
        OR NEW.owner_terminal_id IS NOT OLD.owner_terminal_id
        OR NEW.active_turn_started_at IS NOT OLD.active_turn_started_at
        OR NEW.started_at IS NOT OLD.started_at
        OR NEW.attempt_count IS NOT OLD.attempt_count
 )
BEGIN
    SELECT RAISE(ABORT, 'active Requirement identity is immutable until exact requeue');
END;

CREATE TRIGGER trg_requirements_active_to_pending_pre_effect_guard
BEFORE UPDATE ON requirements
FOR EACH ROW
WHEN OLD.status = 'in_progress'
 AND NEW.status = 'pending'
 AND (
     NOT EXISTS (
         SELECT 1 FROM requirement_pre_effect_abandon_guards AS guard
          WHERE guard.requirement_id = OLD.requirement_id
            AND guard.claim_generation = OLD.claim_generation
            AND guard.claim_token = OLD.claim_token
            AND guard.owner_conversation_id IS OLD.owner_conversation_id
            AND guard.owner_terminal_id IS OLD.owner_terminal_id
     )
     OR EXISTS (
         SELECT 1 FROM agent_executions AS execution
          WHERE json_extract(execution.initial_plan_input, '$.mode') = 'automation'
            AND json_extract(execution.initial_plan_input, '$.source.requirement_id') = OLD.requirement_id
            AND json_extract(execution.initial_plan_input, '$.source.claim_generation') = OLD.claim_generation
     )
     OR EXISTS (
         SELECT 1 FROM terminal_turn_admissions AS admission
          WHERE admission.requirement_id = OLD.requirement_id
            AND admission.claim_generation = OLD.claim_generation
     )
     OR NEW.claim_generation IS NOT OLD.claim_generation
     OR NEW.claim_token IS NOT NULL
     OR NEW.completion_note IS NOT NULL
     OR NEW.owner_conversation_id IS NOT NULL
     OR NEW.owner_terminal_id IS NOT NULL
     OR NEW.active_turn_started_at IS NOT NULL
     OR NEW.lease_expires_at IS NOT NULL
     OR NEW.started_at IS NOT OLD.started_at
     OR NEW.attempt_count IS NOT MAX(OLD.attempt_count - 1, 0)
 )
BEGIN
    SELECT RAISE(ABORT, 'active Requirement may become pending only through exact pre-effect abandon');
END;

CREATE TRIGGER trg_requirements_in_progress_insert_guard
BEFORE INSERT ON requirements
FOR EACH ROW
WHEN NEW.status = 'in_progress'
BEGIN
    SELECT RAISE(ABORT, 'in-progress Requirement may only be entered by atomically claiming a pending row');
END;

CREATE TRIGGER trg_requirements_in_progress_update_guard
BEFORE UPDATE ON requirements
FOR EACH ROW
WHEN NEW.status = 'in_progress'
 AND (
        NEW.claim_generation IS NULL
        OR NEW.claim_generation <= 0
        OR NEW.claim_token IS NULL
        OR NEW.active_turn_started_at IS NULL
        OR NEW.lease_expires_at IS NULL
        OR NEW.started_at IS NULL
        OR NEW.lease_expires_at <= NEW.active_turn_started_at
        OR (
            OLD.status = 'in_progress'
            AND (
                NEW.claim_generation IS NOT OLD.claim_generation
                OR NEW.claim_token IS NOT OLD.claim_token
                OR NEW.owner_conversation_id IS NOT OLD.owner_conversation_id
                OR NEW.owner_terminal_id IS NOT OLD.owner_terminal_id
                OR NEW.active_turn_started_at IS NOT OLD.active_turn_started_at
                OR NEW.started_at IS NOT OLD.started_at
                OR NEW.attempt_count IS NOT OLD.attempt_count
            )
        )
        OR (
            OLD.status = 'pending'
            AND (
                OLD.claim_token IS NOT NULL
                OR NEW.claim_generation IS NOT OLD.claim_generation + 1
                OR NEW.attempt_count IS NOT OLD.attempt_count + 1
            )
        )
        OR OLD.status IS NULL
        OR OLD.status NOT IN ('pending', 'in_progress')
        OR NOT (
            (NEW.owner_conversation_id IS NOT NULL AND NEW.owner_terminal_id IS NULL)
            OR
            (NEW.owner_conversation_id IS NULL AND NEW.owner_terminal_id IS NOT NULL)
        )
 )
BEGIN
    SELECT RAISE(ABORT, 'in-progress Requirement requires generation, capability, and exactly one typed owner');
END;

CREATE TRIGGER trg_requirements_pending_insert_guard
BEFORE INSERT ON requirements
FOR EACH ROW
WHEN NEW.status = 'pending'
 AND (
        NEW.claim_token IS NOT NULL
        OR NEW.owner_conversation_id IS NOT NULL
        OR NEW.owner_terminal_id IS NOT NULL
        OR NEW.active_turn_started_at IS NOT NULL
        OR NEW.lease_expires_at IS NOT NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'pending Requirement cannot carry execution authority');
END;

CREATE TRIGGER trg_requirements_pending_update_guard
BEFORE UPDATE ON requirements
FOR EACH ROW
WHEN NEW.status = 'pending'
 AND (
        NEW.claim_token IS NOT NULL
        OR NEW.owner_conversation_id IS NOT NULL
        OR NEW.owner_terminal_id IS NOT NULL
        OR NEW.active_turn_started_at IS NOT NULL
        OR NEW.lease_expires_at IS NOT NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'pending Requirement cannot carry execution authority');
END;

CREATE TRIGGER trg_requirements_pre_effect_abandon_guard_apply
AFTER INSERT ON requirement_pre_effect_abandon_guards
FOR EACH ROW
BEGIN
    UPDATE requirements
       SET status = 'pending',
           completion_note = NULL,
           owner_conversation_id = NULL,
           owner_terminal_id = NULL,
           active_turn_started_at = NULL,
           lease_expires_at = NULL,
           attempt_count = MAX(attempt_count - 1, 0),
           claim_token = NULL,
           updated_at = MAX(updated_at, NEW.created_at)
     WHERE requirement_id = NEW.requirement_id
       AND status = 'in_progress'
       AND claim_generation = NEW.claim_generation
       AND claim_token = NEW.claim_token
       AND owner_conversation_id IS NEW.owner_conversation_id
       AND owner_terminal_id IS NEW.owner_terminal_id;

    SELECT CASE
        WHEN EXISTS (
            SELECT 1
              FROM requirement_pre_effect_abandon_guards AS guard
             WHERE guard.id = NEW.id
        )
        THEN RAISE(
            ABORT,
            'Requirement pre-effect abandon command did not complete its exact transition'
        )
    END;
END;

CREATE TRIGGER trg_requirements_pre_effect_abandon_guard_consume
AFTER UPDATE ON requirements
FOR EACH ROW
WHEN OLD.status = 'in_progress'
 AND NEW.status = 'pending'
BEGIN
    DELETE FROM requirement_pre_effect_abandon_guards
     WHERE requirement_id = OLD.requirement_id
       AND claim_generation = OLD.claim_generation
       AND claim_token = OLD.claim_token
       AND owner_conversation_id IS OLD.owner_conversation_id
       AND owner_terminal_id IS OLD.owner_terminal_id;
END;

CREATE TRIGGER trg_requirements_pre_effect_abandon_guard_delete_guard
BEFORE DELETE ON requirement_pre_effect_abandon_guards
FOR EACH ROW
WHEN EXISTS (
    SELECT 1
      FROM requirements AS requirement
     WHERE requirement.requirement_id = OLD.requirement_id
       AND requirement.status = 'in_progress'
       AND requirement.claim_generation = OLD.claim_generation
       AND requirement.claim_token = OLD.claim_token
       AND requirement.owner_conversation_id IS OLD.owner_conversation_id
       AND requirement.owner_terminal_id IS OLD.owner_terminal_id
)
BEGIN
    SELECT RAISE(
        ABORT,
        'active Requirement pre-effect abandon guard can only be consumed by guarded transition'
    );
END;

CREATE TRIGGER trg_requirements_pre_effect_abandon_guard_immutable
BEFORE UPDATE ON requirement_pre_effect_abandon_guards
BEGIN
    SELECT RAISE(
        ABORT,
        'Requirement pre-effect abandon guards are immutable'
    );
END;

CREATE TRIGGER trg_requirements_pre_effect_abandon_guard_insert
BEFORE INSERT ON requirement_pre_effect_abandon_guards
FOR EACH ROW
WHEN NOT EXISTS (
    SELECT 1 FROM requirements AS requirement
     WHERE requirement.requirement_id = NEW.requirement_id
       AND requirement.status = 'in_progress'
       AND requirement.claim_generation = NEW.claim_generation
       AND requirement.claim_token = NEW.claim_token
       AND requirement.owner_conversation_id IS NEW.owner_conversation_id
       AND requirement.owner_terminal_id IS NEW.owner_terminal_id
       AND NOT EXISTS (
           SELECT 1 FROM agent_executions AS execution
            WHERE json_extract(execution.initial_plan_input, '$.mode') = 'automation'
              AND json_extract(execution.initial_plan_input, '$.source.requirement_id') = requirement.requirement_id
              AND json_extract(execution.initial_plan_input, '$.source.claim_generation') = requirement.claim_generation
       )
       AND NOT EXISTS (
           SELECT 1 FROM terminal_turn_admissions AS admission
            WHERE admission.requirement_id = requirement.requirement_id
              AND admission.claim_generation = requirement.claim_generation
       )
)
BEGIN
    SELECT RAISE(ABORT, 'Requirement pre-effect abandon guard requires exact authority and receiver-admission absence');
END;

CREATE TRIGGER trg_terminal_turn_admissions_open_insert_guard
BEFORE INSERT ON terminal_turn_admissions
FOR EACH ROW
WHEN NEW.phase IS NOT 'settled'
 AND NEW.claim_token IS NULL
BEGIN
    SELECT RAISE(ABORT, 'open terminal turn admission requires a Requirement capability');
END;

CREATE TRIGGER trg_terminal_turn_admissions_open_update_guard
BEFORE UPDATE ON terminal_turn_admissions
FOR EACH ROW
WHEN (
        NEW.phase IS NOT 'settled'
        AND NEW.claim_token IS NULL
     )
 OR NEW.claim_token IS NOT OLD.claim_token
BEGIN
    SELECT RAISE(ABORT, 'terminal turn admission capability is required and immutable');
END;

CREATE TRIGGER validate_creation_task_input_bindings_insert
BEFORE INSERT ON creation_tasks
WHEN NEW.input_bindings IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invalid creation task input binding')
    WHERE EXISTS (
        SELECT 1
        FROM json_each(NEW.input_bindings) AS input
        WHERE json_type(input.value) IS NOT 'object'
           OR json_type(input.value, '$.asset_id') IS NOT 'text'
           OR length(json_extract(input.value, '$.asset_id')) <> 36
           OR lower(json_extract(input.value, '$.asset_id')) <> json_extract(input.value, '$.asset_id')
           OR json_extract(input.value, '$.asset_id') NOT GLOB '????????-????-7???-[89ab]???-????????????'
           OR replace(json_extract(input.value, '$.asset_id'), '-', '') GLOB '*[^0-9a-f]*'
           OR json_type(input.value, '$.kind') IS NOT 'text'
           OR json_extract(input.value, '$.kind') NOT IN ('image', 'video', 'audio', 'text')
           OR json_type(input.value, '$.role') IS NOT 'text'
           OR json_extract(input.value, '$.role') NOT IN (
                'reference', 'mask', 'first_frame', 'last_frame', 'video', 'audio'
           )
           OR (SELECT COUNT(*) FROM json_each(input.value)) <> 3
           OR EXISTS (
                SELECT 1 FROM json_each(input.value) AS field
                WHERE field.key NOT IN ('asset_id', 'kind', 'role')
           )
    );
END;

CREATE TRIGGER validate_creation_task_input_bindings_update
BEFORE UPDATE OF input_bindings ON creation_tasks
WHEN NEW.input_bindings IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invalid creation task input binding')
    WHERE EXISTS (
        SELECT 1
        FROM json_each(NEW.input_bindings) AS input
        WHERE json_type(input.value) IS NOT 'object'
           OR json_type(input.value, '$.asset_id') IS NOT 'text'
           OR length(json_extract(input.value, '$.asset_id')) <> 36
           OR lower(json_extract(input.value, '$.asset_id')) <> json_extract(input.value, '$.asset_id')
           OR json_extract(input.value, '$.asset_id') NOT GLOB '????????-????-7???-[89ab]???-????????????'
           OR replace(json_extract(input.value, '$.asset_id'), '-', '') GLOB '*[^0-9a-f]*'
           OR json_type(input.value, '$.kind') IS NOT 'text'
           OR json_extract(input.value, '$.kind') NOT IN ('image', 'video', 'audio', 'text')
           OR json_type(input.value, '$.role') IS NOT 'text'
           OR json_extract(input.value, '$.role') NOT IN (
                'reference', 'mask', 'first_frame', 'last_frame', 'video', 'audio'
           )
           OR (SELECT COUNT(*) FROM json_each(input.value)) <> 3
           OR EXISTS (
                SELECT 1 FROM json_each(input.value) AS field
                WHERE field.key NOT IN ('asset_id', 'kind', 'role')
           )
    );
END;

CREATE TRIGGER validate_creative_asset_origin_insert
BEFORE INSERT ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'unsupported creative asset origin id key')
    WHERE json_type(NEW.origin, '$.task_id') IS NOT NULL
       OR json_type(NEW.origin, '$.providerId') IS NOT NULL
       OR json_type(NEW.origin, '$.canvasId') IS NOT NULL
       OR json_type(NEW.origin, '$.nodeId') IS NOT NULL
       OR json_type(NEW.origin, '$.creationTaskId') IS NOT NULL
       OR json_type(NEW.origin, '$.projectId') IS NOT NULL
       OR json_type(NEW.origin, '$.workbenchKind') IS NOT NULL
       OR json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
       OR json_type(NEW.origin, '$.templateId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateStepId') IS NOT NULL;

    SELECT RAISE(ABORT, 'invalid creative asset origin conversation_id')
    WHERE json_type(NEW.origin, '$.conversation_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.conversation_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.conversation_id')) = 36
        AND lower(json_extract(NEW.origin, '$.conversation_id')) = json_extract(NEW.origin, '$.conversation_id')
        AND json_extract(NEW.origin, '$.conversation_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.conversation_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin message_id')
    WHERE json_type(NEW.origin, '$.message_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.message_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.message_id')) = 36
        AND lower(json_extract(NEW.origin, '$.message_id')) = json_extract(NEW.origin, '$.message_id')
        AND json_extract(NEW.origin, '$.message_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.message_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin canvas_id')
    WHERE json_type(NEW.origin, '$.canvas_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.canvas_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.canvas_id')) = 36
        AND lower(json_extract(NEW.origin, '$.canvas_id')) = json_extract(NEW.origin, '$.canvas_id')
        AND json_extract(NEW.origin, '$.canvas_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.canvas_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin project_id')
    WHERE json_type(NEW.origin, '$.project_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.project_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.project_id')) = 36
        AND lower(json_extract(NEW.origin, '$.project_id')) = json_extract(NEW.origin, '$.project_id')
        AND json_extract(NEW.origin, '$.project_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.project_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin node_id')
    WHERE json_type(NEW.origin, '$.node_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.node_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.node_id')) = 36
        AND lower(json_extract(NEW.origin, '$.node_id')) = json_extract(NEW.origin, '$.node_id')
        AND json_extract(NEW.origin, '$.node_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.node_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_id')
    WHERE json_type(NEW.origin, '$.template_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_id')) = json_extract(NEW.origin, '$.template_id')
        AND json_extract(NEW.origin, '$.template_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_run_id')
    WHERE json_type(NEW.origin, '$.template_run_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_run_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_run_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_run_id')) = json_extract(NEW.origin, '$.template_run_id')
        AND json_extract(NEW.origin, '$.template_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_step_id')
    WHERE json_type(NEW.origin, '$.template_step_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_step_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_step_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_step_id')) = json_extract(NEW.origin, '$.template_step_id')
        AND json_extract(NEW.origin, '$.template_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset conversation/canvas/template owner branch')
    WHERE (json_type(NEW.origin, '$.conversation_id') IS NOT NULL
        OR json_type(NEW.origin, '$.message_id') IS NOT NULL
        OR json_type(NEW.origin, '$.canvas_id') IS NOT NULL
        OR json_type(NEW.origin, '$.project_id') IS NOT NULL
        OR json_type(NEW.origin, '$.node_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_run_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_step_id') IS NOT NULL)
    AND NOT (
        (json_type(NEW.origin, '$.conversation_id') IS 'text'
            AND json_type(NEW.origin, '$.message_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS 'text'
            AND json_type(NEW.origin, '$.template_run_id') IS 'text'
            AND json_type(NEW.origin, '$.template_step_id') IS 'text')
    );
END;

CREATE TRIGGER validate_creative_asset_origin_update
BEFORE UPDATE OF origin ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'unsupported creative asset origin id key')
    WHERE json_type(NEW.origin, '$.task_id') IS NOT NULL
       OR json_type(NEW.origin, '$.providerId') IS NOT NULL
       OR json_type(NEW.origin, '$.canvasId') IS NOT NULL
       OR json_type(NEW.origin, '$.nodeId') IS NOT NULL
       OR json_type(NEW.origin, '$.creationTaskId') IS NOT NULL
       OR json_type(NEW.origin, '$.projectId') IS NOT NULL
       OR json_type(NEW.origin, '$.workbenchKind') IS NOT NULL
       OR json_type(NEW.origin, '$.workbench_kind') IS NOT NULL
       OR json_type(NEW.origin, '$.templateId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateRunId') IS NOT NULL
       OR json_type(NEW.origin, '$.templateStepId') IS NOT NULL;

    SELECT RAISE(ABORT, 'invalid creative asset origin conversation_id')
    WHERE json_type(NEW.origin, '$.conversation_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.conversation_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.conversation_id')) = 36
        AND lower(json_extract(NEW.origin, '$.conversation_id')) = json_extract(NEW.origin, '$.conversation_id')
        AND json_extract(NEW.origin, '$.conversation_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.conversation_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin message_id')
    WHERE json_type(NEW.origin, '$.message_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.message_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.message_id')) = 36
        AND lower(json_extract(NEW.origin, '$.message_id')) = json_extract(NEW.origin, '$.message_id')
        AND json_extract(NEW.origin, '$.message_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.message_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin canvas_id')
    WHERE json_type(NEW.origin, '$.canvas_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.canvas_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.canvas_id')) = 36
        AND lower(json_extract(NEW.origin, '$.canvas_id')) = json_extract(NEW.origin, '$.canvas_id')
        AND json_extract(NEW.origin, '$.canvas_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.canvas_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin project_id')
    WHERE json_type(NEW.origin, '$.project_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.project_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.project_id')) = 36
        AND lower(json_extract(NEW.origin, '$.project_id')) = json_extract(NEW.origin, '$.project_id')
        AND json_extract(NEW.origin, '$.project_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.project_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin node_id')
    WHERE json_type(NEW.origin, '$.node_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.node_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.node_id')) = 36
        AND lower(json_extract(NEW.origin, '$.node_id')) = json_extract(NEW.origin, '$.node_id')
        AND json_extract(NEW.origin, '$.node_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.node_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_id')
    WHERE json_type(NEW.origin, '$.template_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_id')) = json_extract(NEW.origin, '$.template_id')
        AND json_extract(NEW.origin, '$.template_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_run_id')
    WHERE json_type(NEW.origin, '$.template_run_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_run_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_run_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_run_id')) = json_extract(NEW.origin, '$.template_run_id')
        AND json_extract(NEW.origin, '$.template_run_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_run_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset origin template_step_id')
    WHERE json_type(NEW.origin, '$.template_step_id') IS NOT NULL AND NOT (
        json_type(NEW.origin, '$.template_step_id') IS 'text'
        AND length(json_extract(NEW.origin, '$.template_step_id')) = 36
        AND lower(json_extract(NEW.origin, '$.template_step_id')) = json_extract(NEW.origin, '$.template_step_id')
        AND json_extract(NEW.origin, '$.template_step_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(NEW.origin, '$.template_step_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
    );

    SELECT RAISE(ABORT, 'invalid creative asset conversation/canvas/template owner branch')
    WHERE (json_type(NEW.origin, '$.conversation_id') IS NOT NULL
        OR json_type(NEW.origin, '$.message_id') IS NOT NULL
        OR json_type(NEW.origin, '$.canvas_id') IS NOT NULL
        OR json_type(NEW.origin, '$.project_id') IS NOT NULL
        OR json_type(NEW.origin, '$.node_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_run_id') IS NOT NULL
        OR json_type(NEW.origin, '$.template_step_id') IS NOT NULL)
    AND NOT (
        (json_type(NEW.origin, '$.conversation_id') IS 'text'
            AND json_type(NEW.origin, '$.message_id') IS 'text'
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS 'text'
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS 'text'
            AND json_type(NEW.origin, '$.node_id') IS 'text'
            AND json_type(NEW.origin, '$.template_id') IS NULL
            AND json_type(NEW.origin, '$.template_run_id') IS NULL
            AND json_type(NEW.origin, '$.template_step_id') IS NULL)
        OR (json_type(NEW.origin, '$.conversation_id') IS NULL
            AND json_type(NEW.origin, '$.message_id') IS NULL
            AND json_type(NEW.origin, '$.canvas_id') IS NULL
            AND json_type(NEW.origin, '$.project_id') IS NULL
            AND json_type(NEW.origin, '$.node_id') IS NULL
            AND json_type(NEW.origin, '$.template_id') IS 'text'
            AND json_type(NEW.origin, '$.template_run_id') IS 'text'
            AND json_type(NEW.origin, '$.template_step_id') IS 'text')
    );
END;

CREATE TRIGGER validate_prompt_library_asset_origin_insert
BEFORE INSERT ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invalid prompt library asset origin identity')
    WHERE (
        (json_type(NEW.origin, '$.prompt_library_source') IS NULL)
            <> (json_type(NEW.origin, '$.prompt_library_id') IS NULL)
    ) OR (
        json_type(NEW.origin, '$.prompt_library_source') IS NOT NULL
        AND NOT (
            NEW.kind = 'text'
            AND json_type(NEW.origin, '$.prompt_library_source') = 'text'
            AND json_extract(NEW.origin, '$.prompt_library_source') IN ('catalog', 'preset')
            AND json_type(NEW.origin, '$.prompt_library_id') = 'text'
            AND length(json_extract(NEW.origin, '$.prompt_library_id')) BETWEEN 1 AND 255
            AND trim(json_extract(NEW.origin, '$.prompt_library_id')) =
                json_extract(NEW.origin, '$.prompt_library_id')
        )
    );

    SELECT RAISE(ABORT, 'invalid catalog prompt library asset origin')
    WHERE json_extract(NEW.origin, '$.prompt_library_source') = 'catalog'
      AND NOT (
          json_type(NEW.origin, '$.prompt_catalog_id') = 'text'
          AND json_extract(NEW.origin, '$.prompt_catalog_id') =
              json_extract(NEW.origin, '$.prompt_library_id')
      );

    SELECT RAISE(ABORT, 'invalid preset prompt library asset origin')
    WHERE json_extract(NEW.origin, '$.prompt_library_source') = 'preset'
      AND json_type(NEW.origin, '$.prompt_catalog_id') IS NOT NULL;
END;

CREATE TRIGGER validate_prompt_library_asset_origin_update
BEFORE UPDATE OF origin, kind ON workshop_assets
WHEN NEW.origin IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invalid prompt library asset origin identity')
    WHERE (
        (json_type(NEW.origin, '$.prompt_library_source') IS NULL)
            <> (json_type(NEW.origin, '$.prompt_library_id') IS NULL)
    ) OR (
        json_type(NEW.origin, '$.prompt_library_source') IS NOT NULL
        AND NOT (
            NEW.kind = 'text'
            AND json_type(NEW.origin, '$.prompt_library_source') = 'text'
            AND json_extract(NEW.origin, '$.prompt_library_source') IN ('catalog', 'preset')
            AND json_type(NEW.origin, '$.prompt_library_id') = 'text'
            AND length(json_extract(NEW.origin, '$.prompt_library_id')) BETWEEN 1 AND 255
            AND trim(json_extract(NEW.origin, '$.prompt_library_id')) =
                json_extract(NEW.origin, '$.prompt_library_id')
        )
    );

    SELECT RAISE(ABORT, 'invalid catalog prompt library asset origin')
    WHERE json_extract(NEW.origin, '$.prompt_library_source') = 'catalog'
      AND NOT (
          json_type(NEW.origin, '$.prompt_catalog_id') = 'text'
          AND json_extract(NEW.origin, '$.prompt_catalog_id') =
              json_extract(NEW.origin, '$.prompt_library_id')
      );

    SELECT RAISE(ABORT, 'invalid preset prompt library asset origin')
    WHERE json_extract(NEW.origin, '$.prompt_library_source') = 'preset'
      AND json_type(NEW.origin, '$.prompt_catalog_id') IS NOT NULL;
END;
