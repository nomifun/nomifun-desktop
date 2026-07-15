-- NomiFun clean baseline (ID contract v2).
--
-- This product is still pre-release. The identifier redesign intentionally
-- starts a new database lineage instead of carrying integer entity keys into
-- the future. Existing databases from the former migration chain are
-- quarantined and a new database is created; there is no row-by-row legacy
-- compatibility layer.
--
-- Persisted, referencable entities use application-minted, canonical
-- `{registered-prefix}_{lowercase-hyphenated-uuidv7}` values. Their PK/FK
-- columns are TEXT and their JSON representation is always a string.
-- Revisions, sequences, timestamps, counters, sort positions, ports and other
-- non-entity numbers remain INTEGER.
--
-- Polymorphic requirement ownership is represented by two typed nullable FKs
-- (`owner_conversation_id`, `owner_terminal_id`) with at most one set. No
-- untyped `(kind, numeric-id)` pair or sentinel value is permitted.
-- table: acp_session
CREATE TABLE acp_session (
    conversation_id TEXT PRIMARY KEY,
    agent_backend   TEXT    NOT NULL,
    agent_source    TEXT    NOT NULL,
    -- Nullable: an ACP conversation can be created before a concrete catalog
    -- agent is picked (legacy clients post only `backend`, or nothing at all).
    -- A non-NULL value is RESTRICT-bound to agent_metadata; NULL means
    -- "no agent chosen yet" and is exempt from FK enforcement (SQLite does not
    -- check FKs on NULL child columns). Replaces the old empty-string sentinel,
    -- which the new RESTRICT FK would reject.
    agent_id        TEXT,
    session_id      TEXT,
    session_status  TEXT    NOT NULL DEFAULT 'idle',
    session_config  TEXT    NOT NULL DEFAULT '{}',
    last_active_at  INTEGER,
    suspended_at    INTEGER,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
    FOREIGN KEY (agent_id) REFERENCES agent_metadata(id) ON DELETE RESTRICT
);

-- table: agent_execution_attempts
CREATE TABLE agent_execution_attempts (
    id                   TEXT NOT NULL,
    execution_id         TEXT NOT NULL,
    step_id              TEXT NOT NULL,
    attempt_no           INTEGER NOT NULL CHECK (attempt_no >= 0),
    participant_id       TEXT,
    status               TEXT NOT NULL CHECK (status IN (
                             'queued', 'running', 'waiting_input', 'completed',
                             'failed', 'cancelled', 'interrupted'
                         )),
    trigger_reason       TEXT NOT NULL CHECK (trim(trigger_reason) <> ''),
    effective_config     TEXT NOT NULL DEFAULT '{}'
                             CHECK (json_valid(effective_config) AND json_type(effective_config) = 'object'),
    question             TEXT,
    error                TEXT,
    output_summary       TEXT,
    output_files         TEXT NOT NULL DEFAULT '[]'
                             CHECK (json_valid(output_files) AND json_type(output_files) = 'array'),
    tokens               INTEGER CHECK (tokens IS NULL OR tokens >= 0),
    retry_after          INTEGER CHECK (retry_after IS NULL OR retry_after >= 0),
    runtime_state        TEXT CHECK (
                             runtime_state IS NULL
                             OR (json_valid(runtime_state) AND json_type(runtime_state) = 'object')
                         ),
    started_at           INTEGER,
    finished_at          INTEGER,
    version              INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    PRIMARY KEY (execution_id, step_id, id),
    UNIQUE (execution_id, step_id, attempt_no),
    FOREIGN KEY (execution_id, step_id)
        REFERENCES agent_execution_steps(execution_id, id) ON DELETE CASCADE,
    FOREIGN KEY (execution_id, participant_id)
        REFERENCES agent_execution_participants(execution_id, id),
    CHECK (
        (status IN ('completed', 'failed', 'cancelled', 'interrupted') AND finished_at IS NOT NULL)
        OR (status NOT IN ('completed', 'failed', 'cancelled', 'interrupted') AND finished_at IS NULL)
    ),
    CHECK (
        (status = 'queued' AND started_at IS NULL)
        OR (status IN ('running', 'waiting_input', 'completed', 'failed', 'interrupted')
            AND started_at IS NOT NULL)
        OR status = 'cancelled'
    ),
    CHECK (
        (status = 'waiting_input' AND trim(COALESCE(question, '')) <> '')
        OR (status <> 'waiting_input' AND question IS NULL)
    )
);

-- table: agent_execution_events
CREATE TABLE agent_execution_events (
    id               TEXT PRIMARY KEY NOT NULL,
    execution_id     TEXT NOT NULL REFERENCES agent_executions(id) ON DELETE CASCADE,
    sequence         INTEGER NOT NULL CHECK (sequence > 0),
    event_type       TEXT NOT NULL CHECK (event_type IN (
        'created', 'migrated', 'status_changed', 'plan_changed',
        'step_changed', 'attempt_changed', 'decision_requested',
        'decision_answered', 'deleted'
    )),
    step_id          TEXT,
    attempt_id       TEXT,
    actor_type       TEXT NOT NULL CHECK (actor_type IN ('system', 'user', 'agent')),
    actor_id         TEXT,
    actor_conversation_id TEXT,
    actor_attempt_id TEXT,
    on_behalf_of_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    payload          TEXT NOT NULL CHECK (json_valid(payload)),
    created_at       INTEGER NOT NULL,
    published_at     INTEGER,
    UNIQUE (execution_id, sequence),
    FOREIGN KEY (execution_id, step_id)
        REFERENCES agent_execution_steps(execution_id, id) ON DELETE CASCADE,
    FOREIGN KEY (execution_id, step_id, attempt_id)
        REFERENCES agent_execution_attempts(execution_id, step_id, id) ON DELETE CASCADE,
    CHECK (attempt_id IS NULL OR step_id IS NOT NULL),
    CHECK (actor_attempt_id IS NULL OR trim(actor_attempt_id) <> ''),
    CHECK (
        (actor_type = 'system' AND actor_id IS NULL)
        OR (actor_type IN ('user', 'agent') AND trim(COALESCE(actor_id, '')) <> '')
    ),
    CHECK (
        (actor_type = 'system'
            AND actor_id IS NULL
            AND actor_conversation_id IS NULL
            AND actor_attempt_id IS NULL)
        OR (actor_type = 'user'
            AND actor_id = on_behalf_of_user_id
            AND actor_conversation_id IS NULL
            AND actor_attempt_id IS NULL)
        OR (actor_type = 'agent'
            AND trim(actor_id) <> ''
            AND (
                (actor_conversation_id IS NULL AND actor_attempt_id IS NULL)
                OR (trim(actor_conversation_id) <> ''
                    AND actor_id = actor_conversation_id)
            ))
    )
);

-- table: agent_execution_participants
CREATE TABLE agent_execution_participants (
    id                      TEXT NOT NULL,
    execution_id            TEXT NOT NULL REFERENCES agent_executions(id) ON DELETE CASCADE,
    source_agent_id         TEXT NOT NULL CHECK (trim(source_agent_id) <> ''),
    preset_id               TEXT,
    preset_revision         INTEGER CHECK (preset_revision IS NULL OR preset_revision > 0),
    preset_snapshot         TEXT CHECK (
                                preset_snapshot IS NULL
                                OR (json_valid(preset_snapshot) AND json_type(preset_snapshot) = 'object')
                            ),
    provider_id             TEXT,
    model                   TEXT,
    role                    TEXT,
    capability              TEXT CHECK (
                                capability IS NULL
                                OR (json_valid(capability) AND json_type(capability) = 'object')
                            ),
    constraints             TEXT CHECK (
                                CASE WHEN constraints IS NULL THEN 1
                                     WHEN NOT json_valid(constraints) THEN 0
                                     ELSE json_type(constraints) = 'object'
                                          AND (
                                              json_type(constraints, '$.max_concurrency') IS NULL
                                              OR json_type(constraints, '$.max_concurrency') = 'null'
                                              OR (
                                                  json_type(constraints, '$.max_concurrency') = 'integer'
                                                  AND json_extract(constraints, '$.max_concurrency') BETWEEN 1 AND 64
                                              )
                                          )
                                     END
                            ),
    description             TEXT,
    system_prompt           TEXT,
    enabled_skills          TEXT NOT NULL DEFAULT '[]'
                                CHECK (json_valid(enabled_skills) AND json_type(enabled_skills) = 'array'),
    disabled_builtin_skills TEXT NOT NULL DEFAULT '[]'
                                CHECK (json_valid(disabled_builtin_skills)
                                       AND json_type(disabled_builtin_skills) = 'array'),
    sort_order              INTEGER NOT NULL DEFAULT 0,
    introduced_in_revision  INTEGER NOT NULL CHECK (introduced_in_revision >= 0),
    retired_in_revision     INTEGER CHECK (
                                retired_in_revision IS NULL
                                OR retired_in_revision > introduced_in_revision
                            ),
    created_at              INTEGER NOT NULL,
    PRIMARY KEY (execution_id, id),
    CHECK (
        (provider_id IS NULL AND model IS NULL)
        OR (provider_id IS NOT NULL AND model IS NOT NULL
            AND trim(provider_id) <> '' AND trim(provider_id) = provider_id
            AND trim(model) <> '' AND trim(model) = model)
    )
);

-- table: agent_execution_step_dependencies
CREATE TABLE agent_execution_step_dependencies (
    execution_id   TEXT NOT NULL,
    blocker_step_id TEXT NOT NULL,
    blocked_step_id TEXT NOT NULL,
    introduced_in_revision INTEGER NOT NULL CHECK (introduced_in_revision >= 0),
    superseded_in_revision INTEGER CHECK (
        superseded_in_revision IS NULL OR superseded_in_revision > introduced_in_revision
    ),
    PRIMARY KEY (execution_id, blocker_step_id, blocked_step_id, introduced_in_revision),
    FOREIGN KEY (execution_id, blocker_step_id)
        REFERENCES agent_execution_steps(execution_id, id) ON DELETE CASCADE,
    FOREIGN KEY (execution_id, blocked_step_id)
        REFERENCES agent_execution_steps(execution_id, id) ON DELETE CASCADE,
    CHECK (blocker_step_id <> blocked_step_id)
);

-- table: agent_execution_steps
CREATE TABLE agent_execution_steps (
    id                       TEXT NOT NULL,
    execution_id             TEXT NOT NULL REFERENCES agent_executions(id) ON DELETE CASCADE,
    title                    TEXT NOT NULL CHECK (trim(title) <> ''),
    spec                     TEXT NOT NULL,
    role                     TEXT,
    tool_policy              TEXT NOT NULL DEFAULT 'full'
                                 CHECK (tool_policy IN ('full', 'read_only', 'read_shell')),
    kind                     TEXT NOT NULL CHECK (kind IN ('agent', 'verify', 'judge', 'loop')),
    agent_mode               TEXT CHECK (agent_mode IS NULL OR agent_mode IN ('normal', 'synthesis')),
    profile                  TEXT CHECK (
                                 profile IS NULL
                                 OR (json_valid(profile) AND json_type(profile) = 'object')
                             ),
    fanout_group             TEXT CHECK (fanout_group IS NULL OR trim(fanout_group) <> ''),
    control_policy           TEXT CHECK (
                                 control_policy IS NULL
                                 OR (json_valid(control_policy) AND json_type(control_policy) = 'object')
                             ),
    delegation_depth         INTEGER NOT NULL DEFAULT 0
                                 CHECK (delegation_depth BETWEEN 0 AND 4),
    status                   TEXT NOT NULL CHECK (status IN (
                               'pending', 'running', 'waiting_input', 'completed',
                               'failed', 'skipped', 'cancelled'
                           )),
    assigned_participant_id  TEXT,
    assignment_score         REAL,
    assignment_rationale     TEXT,
    assignment_source        TEXT CHECK (
                                 assignment_source IS NULL
                                 OR assignment_source IN ('planner', 'automatic', 'manual')
                             ),
    assignment_locked        INTEGER NOT NULL DEFAULT 0 CHECK (assignment_locked IN (0, 1)),
    failure_policy           TEXT NOT NULL DEFAULT 'fail_execution'
                                 CHECK (failure_policy IN ('fail_execution', 'skip_dependents')),
    preset_prompt            TEXT,
    graph_x                  REAL,
    graph_y                  REAL,
    dispatch_after           INTEGER CHECK (
                                 dispatch_after IS NULL
                                 OR (dispatch_after >= 0 AND status = 'pending')
                             ),
    version                  INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    introduced_in_revision   INTEGER NOT NULL CHECK (introduced_in_revision >= 0),
    superseded_in_revision   INTEGER CHECK (
                                 superseded_in_revision IS NULL
                                 OR superseded_in_revision > introduced_in_revision
                             ),
    created_at               INTEGER NOT NULL,
    updated_at               INTEGER NOT NULL,
    PRIMARY KEY (execution_id, id),
    FOREIGN KEY (execution_id, assigned_participant_id)
        REFERENCES agent_execution_participants(execution_id, id),
    CHECK (
        (kind = 'agent' AND agent_mode IS NOT NULL AND control_policy IS NULL
            AND assigned_participant_id IS NOT NULL AND assignment_source IS NOT NULL)
        OR
        (kind IN ('verify', 'judge', 'loop') AND agent_mode IS NULL
            AND control_policy IS NOT NULL
            AND json_extract(control_policy, '$.kind') = kind
            AND assigned_participant_id IS NULL
            AND assignment_score IS NULL AND assignment_rationale IS NULL
            AND assignment_source IS NULL AND assignment_locked = 0)
    ),
    CHECK (kind = 'agent' OR tool_policy = 'full'),
    CHECK (kind = 'agent' OR fanout_group IS NULL)
);

-- table: agent_execution_template_participants
CREATE TABLE agent_execution_template_participants (
    id                      TEXT NOT NULL CHECK (trim(id) <> ''),
    template_id             TEXT NOT NULL
                                REFERENCES agent_execution_templates(id) ON DELETE CASCADE,
    source_agent_id         TEXT NOT NULL CHECK (trim(source_agent_id) <> ''),
    preset_id               TEXT,
    preset_revision         INTEGER,
    preset_snapshot         TEXT,
    provider_id             TEXT,
    model                   TEXT,
    role                    TEXT,
    capability              TEXT CHECK (
                                CASE WHEN capability IS NULL THEN 1
                                     WHEN NOT json_valid(capability) THEN 0
                                     ELSE json_type(capability) = 'object' END
                            ),
    constraints             TEXT CHECK (
                                CASE WHEN constraints IS NULL THEN 1
                                     WHEN NOT json_valid(constraints) THEN 0
                                     ELSE json_type(constraints) = 'object'
                                          AND (
                                              json_type(constraints, '$.max_concurrency') IS NULL
                                              OR json_type(constraints, '$.max_concurrency') = 'null'
                                              OR (
                                                  json_type(constraints, '$.max_concurrency') = 'integer'
                                                  AND json_extract(constraints, '$.max_concurrency') BETWEEN 1 AND 64
                                              )
                                          )
                                     END
                            ),
    description             TEXT,
    system_prompt           TEXT,
    enabled_skills          TEXT NOT NULL DEFAULT '[]' CHECK (
                                json_valid(enabled_skills)
                                AND json_type(enabled_skills) = 'array'
                            ),
    disabled_builtin_skills TEXT NOT NULL DEFAULT '[]' CHECK (
                                json_valid(disabled_builtin_skills)
                                AND json_type(disabled_builtin_skills) = 'array'
                            ),
    sort_order              INTEGER NOT NULL DEFAULT 0,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL CHECK (updated_at >= created_at),
    PRIMARY KEY (template_id, id),
    CHECK (
        provider_id IS NOT NULL AND model IS NOT NULL
        AND trim(provider_id) <> '' AND trim(provider_id) = provider_id
        AND trim(model) <> '' AND trim(model) = model
    ),
    CHECK (
        CASE WHEN preset_id IS NULL THEN
            preset_revision IS NULL AND preset_snapshot IS NULL
        WHEN trim(preset_id) = '' OR preset_revision IS NULL OR preset_revision <= 0
             OR preset_snapshot IS NULL OR NOT json_valid(preset_snapshot) THEN 0
        ELSE json_type(preset_snapshot) = 'object'
             AND json_extract(preset_snapshot, '$.preset_id') = preset_id
             AND json_extract(preset_snapshot, '$.preset_revision') = preset_revision
             AND json_extract(preset_snapshot, '$.target') = 'execution_step'
        END
    )
);

-- table: agent_execution_templates
CREATE TABLE agent_execution_templates (
    id              TEXT PRIMARY KEY NOT NULL CHECK (trim(id) <> ''),
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name            TEXT NOT NULL CHECK (trim(name) <> ''),
    description     TEXT,
    max_parallel    INTEGER CHECK (max_parallel IS NULL OR max_parallel BETWEEN 1 AND 64),
    work_dir        TEXT,
    context         TEXT CHECK (context IS NULL OR json_valid(context)),
    -- A Template has no hidden draft/status state: every persisted aggregate
    -- names at least one concrete Participant. The circular FK is deferred so
    -- parent and children can be inserted/replaced atomically in one
    -- transaction, while commit can never leave an empty Template.
    primary_participant_id TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL CHECK (updated_at >= created_at),
    FOREIGN KEY (id, primary_participant_id)
        REFERENCES agent_execution_template_participants(template_id, id)
        DEFERRABLE INITIALLY DEFERRED
);

-- table: agent_executions
CREATE TABLE agent_executions (
    id                    TEXT PRIMARY KEY NOT NULL,
    user_id               TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    goal                  TEXT NOT NULL CHECK (trim(goal) <> ''),
    status                TEXT NOT NULL CHECK (status IN (
                              'planning', 'awaiting_approval', 'running', 'paused',
                              'waiting_input', 'completed', 'completed_with_failures',
                              'failed', 'cancelled'
                          )),
    plan_gate             TEXT NOT NULL CHECK (plan_gate IN ('automatic', 'require_approval')),
    adaptation_policy     TEXT NOT NULL CHECK (adaptation_policy IN ('fixed', 'adaptive')),
    decision_policy       TEXT NOT NULL CHECK (decision_policy IN ('automatic', 'ask_user')),
    delegation_policy     TEXT NOT NULL CHECK (delegation_policy IN ('disabled', 'automatic', 'prefer_parallel')),
    max_parallel          INTEGER NOT NULL DEFAULT 4 CHECK (max_parallel BETWEEN 1 AND 64),
    work_dir              TEXT,
    initial_plan_input    TEXT NOT NULL CHECK (
                              json_valid(initial_plan_input)
                              AND json_type(initial_plan_input) = 'object'
                              AND json_extract(initial_plan_input, '$.mode') IN ('automatic', 'explicit')
                              AND (
                                  (json_extract(initial_plan_input, '$.mode') = 'automatic'
                                   AND json_type(initial_plan_input, '$.plan') IS NULL)
                                  OR
                                  (json_extract(initial_plan_input, '$.mode') = 'explicit'
                                   AND json_type(initial_plan_input, '$.plan') = 'object'
                                   AND json_type(initial_plan_input, '$.plan.steps') = 'array'
                                   AND json_array_length(initial_plan_input, '$.plan.steps') > 0)
                              )
                          ),
    summary               TEXT,
    total_tokens          INTEGER CHECK (total_tokens IS NULL OR total_tokens >= 0),
    version               INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    plan_revision         INTEGER NOT NULL DEFAULT 0 CHECK (plan_revision >= 0),
    event_sequence        INTEGER NOT NULL DEFAULT 0 CHECK (event_sequence >= 0),
    lease_owner           TEXT,
    lease_expires_at      INTEGER,
    deleted_at            INTEGER CHECK (deleted_at IS NULL OR deleted_at >= created_at),
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL,
    CHECK (
        (lease_owner IS NULL AND lease_expires_at IS NULL)
        OR (trim(lease_owner) <> '' AND lease_expires_at IS NOT NULL)
    ),
    -- WaitingInput is an aggregate attention signal, not a global execution
    -- mutex: independent ready steps and durable decision continuations may
    -- still run while another attempt waits for the user.
    CHECK (status IN ('running', 'waiting_input')
           OR (lease_owner IS NULL AND lease_expires_at IS NULL)),
    CHECK (updated_at >= created_at)
);

-- table: agent_metadata
CREATE TABLE agent_metadata (
    id                  TEXT PRIMARY KEY NOT NULL,
    icon                TEXT,
    name                TEXT NOT NULL,
    name_i18n           TEXT,
    description         TEXT,
    description_i18n    TEXT,
    backend             TEXT,
    agent_type          TEXT NOT NULL,
    agent_source        TEXT NOT NULL,
    agent_source_info   TEXT,
    enabled             INTEGER NOT NULL DEFAULT 1,
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

-- table: attachments
CREATE TABLE attachments (
    id             TEXT    PRIMARY KEY,              -- att_{uuidv7}
    requirement_id TEXT NOT NULL,
    file_name      TEXT    NOT NULL,                 -- original display name (deduped per requirement)
    rel_path       TEXT    NOT NULL,                 -- relative to data_dir
    mime           TEXT    NOT NULL,
    size_bytes     INTEGER NOT NULL,
    created_by     TEXT,
    created_at     INTEGER NOT NULL,
    FOREIGN KEY (requirement_id) REFERENCES requirements(id) ON DELETE CASCADE
);

-- table: channel_pairing_codes
CREATE TABLE "channel_pairing_codes" (
    code             TEXT PRIMARY KEY NOT NULL,
    platform_user_id TEXT    NOT NULL,
    platform_type    TEXT    NOT NULL,
    channel_id       TEXT,
    display_name     TEXT,
    requested_at     INTEGER NOT NULL,
    expires_at       INTEGER NOT NULL,
    status           TEXT    NOT NULL DEFAULT 'pending'
                             CHECK (status IN ('pending', 'approved', 'rejected', 'expired')),
    FOREIGN KEY (channel_id) REFERENCES channel_plugins(id) ON DELETE CASCADE
);

-- table: channel_plugins
CREATE TABLE "channel_plugins" (
    id             TEXT PRIMARY KEY NOT NULL,
    type           TEXT    NOT NULL,
    name           TEXT    NOT NULL,
    enabled        INTEGER NOT NULL DEFAULT 0,
    config         TEXT    NOT NULL,
    status         TEXT,
    last_connected INTEGER,
    companion_id         TEXT,
    bot_key        TEXT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
, public_agent_id TEXT);

-- table: channel_sessions
CREATE TABLE "channel_sessions" (
    id              TEXT PRIMARY KEY NOT NULL,
    user_id         TEXT    NOT NULL,
    agent_type      TEXT    NOT NULL,
    conversation_id TEXT,
    workspace       TEXT,
    chat_id         TEXT,
    channel_id      TEXT,
    created_at      INTEGER NOT NULL,
    last_activity   INTEGER NOT NULL,
    FOREIGN KEY (user_id) REFERENCES channel_users(id) ON DELETE CASCADE,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE SET NULL,
    FOREIGN KEY (channel_id) REFERENCES channel_plugins(id) ON DELETE SET NULL
);

-- table: channel_users
CREATE TABLE "channel_users" (
    id               TEXT PRIMARY KEY NOT NULL,
    platform_user_id TEXT    NOT NULL,
    platform_type    TEXT    NOT NULL,
    channel_id       TEXT,
    display_name     TEXT,
    authorized_at    INTEGER NOT NULL,
    last_active      INTEGER,
    session_id       TEXT,
    UNIQUE (platform_user_id, platform_type, channel_id),
    FOREIGN KEY (channel_id) REFERENCES channel_plugins(id) ON DELETE CASCADE
);

-- table: client_preferences
CREATE TABLE client_preferences (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
);

-- table: companion_access_token
CREATE TABLE companion_access_token (
    companion_id TEXT PRIMARY KEY,
    token_hash   TEXT NOT NULL,
    created_at   INTEGER NOT NULL
);

-- table: connector_credentials
CREATE TABLE connector_credentials (
    id                TEXT PRIMARY KEY,
    kind              TEXT NOT NULL,
    name              TEXT NOT NULL,
    payload_encrypted TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

-- table: conversation_artifacts
CREATE TABLE conversation_artifacts (
    id              TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL,
    cron_job_id     TEXT,
    kind            TEXT    NOT NULL
                            CHECK(kind IN ('cron_trigger', 'skill_suggest')),
    status          TEXT    NOT NULL DEFAULT 'active'
                            CHECK(status IN ('active', 'pending', 'dismissed', 'saved')),
    payload         TEXT    NOT NULL DEFAULT '{}',
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
    FOREIGN KEY (cron_job_id) REFERENCES cron_jobs(id) ON DELETE SET NULL
);

-- table: conversation_creation_keys
CREATE TABLE conversation_creation_keys (
    creation_key   TEXT PRIMARY KEY NOT NULL CHECK (trim(creation_key) <> ''),
    user_id        TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL UNIQUE REFERENCES conversations(id) ON DELETE CASCADE,
    created_at     INTEGER NOT NULL
);

-- table: conversation_delivery_receipts
CREATE TABLE conversation_delivery_receipts (
    operation_id    TEXT PRIMARY KEY NOT NULL CHECK (trim(operation_id) <> ''),
    message_id      TEXT NOT NULL UNIQUE CHECK (
        length(message_id) = 40
        AND substr(message_id, 1, 4) = 'msg_'
        AND lower(message_id) = message_id
    ),
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL CHECK (kind IN ('turn', 'steer', 'projection')),
    request_payload TEXT NOT NULL CHECK (json_valid(request_payload)),
    status          TEXT NOT NULL CHECK (status IN ('accepted', 'completed')),
    result_ok       INTEGER CHECK (result_ok IS NULL OR result_ok IN (0, 1)),
    result_text     TEXT,
    result_error    TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    completed_at    INTEGER,
    CHECK (updated_at >= created_at),
    CHECK (completed_at IS NULL OR (completed_at >= created_at AND completed_at <= updated_at)),
    CHECK (
        (status = 'accepted' AND result_ok IS NULL AND completed_at IS NULL)
        OR (status = 'completed' AND result_ok IS NOT NULL AND completed_at IS NOT NULL)
    )
);

-- table: conversation_execution_links
CREATE TABLE conversation_execution_links (
    id              TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    execution_id    TEXT NOT NULL REFERENCES agent_executions(id) ON DELETE CASCADE,
    relation        TEXT NOT NULL CHECK (relation IN ('lead', 'attempt')),
    step_id         TEXT,
    attempt_id      TEXT,
    active          INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    -- NULL is a durable cancellation/termination intent.  The execution
    -- engine marks this only after the Conversation runtime acknowledges the
    -- idempotent cleanup, so a process crash simply retries it at boot.
    cleanup_completed_at INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY (execution_id, step_id)
        REFERENCES agent_execution_steps(execution_id, id) ON DELETE CASCADE,
    FOREIGN KEY (execution_id, step_id, attempt_id)
        REFERENCES agent_execution_attempts(execution_id, step_id, id) ON DELETE CASCADE,
    CHECK (
        (relation = 'lead' AND step_id IS NULL AND attempt_id IS NULL)
        OR (relation = 'attempt' AND step_id IS NOT NULL AND attempt_id IS NOT NULL)
    ),
    CHECK (cleanup_completed_at IS NULL OR (relation = 'attempt' AND active = 0))
);

-- table: conversation_mcp_servers
CREATE TABLE conversation_mcp_servers (
    conversation_id TEXT NOT NULL,
    mcp_server_id   TEXT NOT NULL,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (conversation_id, mcp_server_id),
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
    FOREIGN KEY (mcp_server_id)   REFERENCES mcp_servers(id)   ON DELETE CASCADE
);

-- table: conversations
CREATE TABLE conversations (
    id              TEXT PRIMARY KEY NOT NULL,
    user_id         TEXT    NOT NULL,
    name            TEXT    NOT NULL,
    type            TEXT    NOT NULL,
    extra           TEXT    NOT NULL DEFAULT '{}',
    model           TEXT,
    status          TEXT    NOT NULL DEFAULT 'pending'
                            CHECK(status IN ('pending', 'running', 'finished')),
    source          TEXT,
    channel_chat_id TEXT,
    pinned          INTEGER NOT NULL DEFAULT 0,
    pinned_at       INTEGER,
    cron_job_id     TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL, preset_id TEXT, preset_revision INTEGER, preset_snapshot TEXT, delegation_policy TEXT NOT NULL DEFAULT 'automatic'
    CHECK (delegation_policy IN ('disabled', 'automatic', 'prefer_parallel')), execution_model_pool TEXT
    CHECK (
        CASE
            WHEN execution_model_pool IS NULL THEN 1
            WHEN NOT json_valid(execution_model_pool) THEN 0
            WHEN json_type(execution_model_pool) <> 'object' THEN 0
            WHEN json_type(execution_model_pool, '$.mode') <> 'text' THEN 0
            WHEN json_extract(execution_model_pool, '$.mode') = 'automatic'
                THEN 1
            WHEN json_extract(execution_model_pool, '$.mode') = 'single'
                THEN json_type(execution_model_pool, '$.model') = 'object'
                 AND json_type(execution_model_pool, '$.model.provider_id') = 'text'
                 AND trim(json_extract(execution_model_pool, '$.model.provider_id')) <> ''
                 AND trim(json_extract(execution_model_pool, '$.model.provider_id'))
                        = json_extract(execution_model_pool, '$.model.provider_id')
                 AND json_type(execution_model_pool, '$.model.model') = 'text'
                 AND trim(json_extract(execution_model_pool, '$.model.model')) <> ''
                 AND trim(json_extract(execution_model_pool, '$.model.model'))
                        = json_extract(execution_model_pool, '$.model.model')
            WHEN json_extract(execution_model_pool, '$.mode') = 'range'
                THEN json_type(execution_model_pool, '$.models') = 'array'
                 AND json_array_length(execution_model_pool, '$.models') BETWEEN 1 AND 16
            ELSE 0
        END
    ), decision_policy TEXT NOT NULL DEFAULT 'automatic'
    CHECK (decision_policy IN ('automatic', 'ask_user')), execution_template_id TEXT
    REFERENCES agent_execution_templates(id) ON DELETE SET NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (cron_job_id) REFERENCES cron_jobs(id) ON DELETE SET NULL
);

-- table: creation_tasks
CREATE TABLE creation_tasks (
    id               TEXT PRIMARY KEY,
    canvas_id        TEXT,
    node_id          TEXT,
    provider_id      TEXT NOT NULL,
    model            TEXT NOT NULL,
    capability       TEXT NOT NULL,
    params           TEXT NOT NULL,
    status           TEXT NOT NULL,
    error            TEXT,
    result_asset_ids TEXT NOT NULL DEFAULT '[]',
    remote_task_id   TEXT,
    attempt          INTEGER NOT NULL DEFAULT 0,
    submitted_at     INTEGER NOT NULL,
    started_at       INTEGER,
    finished_at      INTEGER,
    FOREIGN KEY (canvas_id) REFERENCES workshop_canvases(id) ON DELETE SET NULL,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE RESTRICT
);

-- table: cron_job_runs
CREATE TABLE cron_job_runs (
    id             TEXT    PRIMARY KEY NOT NULL,
    job_id         TEXT    NOT NULL,
    executed_at_ms INTEGER NOT NULL,
    status         TEXT    NOT NULL CHECK(status IN ('ok', 'error', 'skipped', 'missed')),
    created_at_ms  INTEGER NOT NULL,
    FOREIGN KEY (job_id) REFERENCES cron_jobs(id) ON DELETE CASCADE
);

-- table: cron_jobs
CREATE TABLE "cron_jobs" (
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
    conversation_id      TEXT,
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

-- table: idmm_interventions
CREATE TABLE "idmm_interventions" (
    id           TEXT PRIMARY KEY NOT NULL,
    user_id      TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE
                         CHECK (length(trim(user_id)) > 0),
    target_kind  TEXT    NOT NULL CHECK (target_kind IN ('conversation', 'terminal')),
    target_id    TEXT    NOT NULL CHECK (length(trim(target_id)) > 0),
    watch        TEXT    NOT NULL,
    at           INTEGER NOT NULL,
    signal       TEXT    NOT NULL,
    tier_used    TEXT    NOT NULL,
    category     TEXT,
    action       TEXT    NOT NULL,
    detail       TEXT,
    reason       TEXT,
    confidence   REAL,
    bypass_model TEXT,
    outcome      TEXT    NOT NULL
);

-- table: knowledge_bases
CREATE TABLE knowledge_bases (
    id          TEXT    PRIMARY KEY,            -- kb_{uuidv7}
    name        TEXT    NOT NULL,
    description TEXT    NOT NULL DEFAULT '',
    root_path   TEXT    NOT NULL,
    managed     INTEGER NOT NULL DEFAULT 1,
    extra       TEXT    NOT NULL DEFAULT '{}',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
, tags TEXT);

-- table: knowledge_binding_bases
CREATE TABLE knowledge_binding_bases (
    binding_id TEXT NOT NULL,
    kb_id      TEXT    NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (binding_id, kb_id),
    FOREIGN KEY (binding_id) REFERENCES knowledge_bindings(binding_id) ON DELETE CASCADE,
    FOREIGN KEY (kb_id)      REFERENCES knowledge_bases(id)            ON DELETE CASCADE
);

-- table: knowledge_bindings
CREATE TABLE "knowledge_bindings" (
    binding_id          TEXT PRIMARY KEY NOT NULL,
    target_kind         TEXT    NOT NULL,
    target_workpath     TEXT,
    target_conv_id      TEXT,
    target_term_id      TEXT,
    target_companion_id TEXT,
    enabled             INTEGER NOT NULL DEFAULT 0,
    writeback           INTEGER NOT NULL DEFAULT 0,
    writeback_mode      TEXT    NOT NULL DEFAULT 'staged'
                                CHECK(writeback_mode IN ('staged', 'direct')),
    writeback_eagerness TEXT    NOT NULL DEFAULT 'conservative'
                                CHECK(writeback_eagerness IN ('conservative', 'aggressive')),
    updated_at          INTEGER NOT NULL, channel_write_enabled INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (target_conv_id) REFERENCES conversations(id) ON DELETE CASCADE,
    FOREIGN KEY (target_term_id) REFERENCES terminal_sessions(id) ON DELETE CASCADE,
    CHECK (
        (target_kind = 'workpath'     AND target_workpath IS NOT NULL
            AND target_conv_id IS NULL AND target_term_id IS NULL AND target_companion_id IS NULL)
     OR (target_kind = 'conversation' AND target_conv_id  IS NOT NULL
            AND target_workpath IS NULL AND target_term_id IS NULL AND target_companion_id IS NULL)
     OR (target_kind = 'terminal'     AND target_term_id  IS NOT NULL
            AND target_workpath IS NULL AND target_conv_id IS NULL AND target_companion_id IS NULL)
     OR (target_kind = 'companion'    AND target_companion_id IS NOT NULL
            AND target_workpath IS NULL AND target_conv_id IS NULL AND target_term_id IS NULL)
    )
);

-- table: knowledge_tags
CREATE TABLE knowledge_tags (
  key        TEXT PRIMARY KEY,
  label      TEXT NOT NULL,
  color      TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

-- table: mcp_servers
CREATE TABLE mcp_servers (
    id               TEXT PRIMARY KEY NOT NULL,
    name             TEXT    NOT NULL UNIQUE,
    description      TEXT,
    enabled          INTEGER NOT NULL DEFAULT 0,
    transport_type   TEXT    NOT NULL,
    transport_config TEXT    NOT NULL,
    tools            TEXT,
    last_test_status TEXT    NOT NULL DEFAULT 'disconnected',
    last_connected   INTEGER,
    original_json    TEXT,
    builtin          INTEGER NOT NULL DEFAULT 0,
    deleted_at       INTEGER,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

-- table: messages
CREATE TABLE messages (
    id              TEXT    PRIMARY KEY NOT NULL,   -- msg_{uuidv7}
    conversation_id TEXT NOT NULL,
    msg_id          TEXT,
    type            TEXT    NOT NULL,
    content         TEXT    NOT NULL DEFAULT '{}',
    position        TEXT    CHECK(position IN ('left', 'right', 'center', 'pop')),
    status          TEXT    CHECK(status IN ('finish', 'pending', 'error', 'work')),
    hidden          INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
);

-- Durable protocol-correlation registry. Provider/session/call keys remain
-- opaque correlation data; only message_id is an entity identity.
CREATE TABLE message_correlations (
    conversation_id TEXT NOT NULL,
    turn_message_id TEXT NOT NULL
        CHECK(turn_message_id GLOB 'msg_????????-????-7???-[89ab]???-????????????'),
    message_type    TEXT NOT NULL CHECK(length(trim(message_type)) > 0),
    correlation_key TEXT NOT NULL CHECK(length(trim(correlation_key)) > 0),
    message_id      TEXT NOT NULL UNIQUE
        CHECK(message_id GLOB 'msg_????????-????-7???-[89ab]???-????????????'),
    PRIMARY KEY (conversation_id, turn_message_id, message_type, correlation_key),
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
);

-- table: model_profiles
CREATE TABLE model_profiles (
    provider_id TEXT    NOT NULL,
    model       TEXT    NOT NULL,
    tasks       TEXT    NOT NULL DEFAULT '[]',
    traits      TEXT    NOT NULL DEFAULT '[]',
    params      TEXT    NOT NULL DEFAULT '{}',
    source      TEXT    NOT NULL DEFAULT 'inferred',
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (provider_id, model),
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE
);

-- table: oauth_tokens
CREATE TABLE oauth_tokens (
    server_url    TEXT PRIMARY KEY NOT NULL,
    access_token  TEXT    NOT NULL,
    refresh_token TEXT,
    token_type    TEXT    NOT NULL DEFAULT 'bearer',
    expires_at    INTEGER,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

-- table: preset_agent_preferences
CREATE TABLE preset_agent_preferences (
    preset_id   TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    agent_id    TEXT NOT NULL,
    rank        INTEGER NOT NULL DEFAULT 0,
    required    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (preset_id, agent_id)
);

-- table: preset_examples
CREATE TABLE preset_examples (
    preset_id   TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    locale      TEXT NOT NULL DEFAULT '',
    sort_order  INTEGER NOT NULL DEFAULT 0,
    prompt      TEXT NOT NULL,
    PRIMARY KEY (preset_id, locale, sort_order)
);

-- table: preset_knowledge_bases
CREATE TABLE preset_knowledge_bases (
    preset_id          TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    knowledge_base_id  TEXT NOT NULL,
    sort_order         INTEGER NOT NULL DEFAULT 0,
    required           INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (preset_id, knowledge_base_id)
);

-- table: preset_knowledge_policy
CREATE TABLE preset_knowledge_policy (
    preset_id   TEXT PRIMARY KEY NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    enabled     INTEGER NOT NULL DEFAULT 0,
    mode        TEXT NOT NULL DEFAULT 'inherit',
    writeback   INTEGER NOT NULL DEFAULT 0,
    eagerness   TEXT CHECK (eagerness IS NULL OR eagerness IN ('conservative','aggressive')),
    grounded    INTEGER NOT NULL DEFAULT 0
);

-- table: preset_localizations
CREATE TABLE preset_localizations (
    preset_id            TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    locale               TEXT NOT NULL,
    name                 TEXT,
    description          TEXT,
    routing_description  TEXT,
    instructions         TEXT,
    PRIMARY KEY (preset_id, locale)
);

-- table: preset_model_preferences
CREATE TABLE preset_model_preferences (
    preset_id   TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    provider_id TEXT,
    model       TEXT NOT NULL,
    rank        INTEGER NOT NULL DEFAULT 0,
    required    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (preset_id, rank)
);

-- table: preset_skill_bindings
CREATE TABLE preset_skill_bindings (
    preset_id   TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    skill_name  TEXT NOT NULL,
    binding     TEXT NOT NULL CHECK (binding IN ('include','exclude_auto')),
    required    INTEGER NOT NULL DEFAULT 0,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (preset_id, skill_name, binding)
);

-- table: preset_tag_bindings
CREATE TABLE preset_tag_bindings (
    preset_id  TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    -- Explicit string union (not a single-domain FK): built-in tags use stable
    -- manifest natural keys (`office`, `coding`, …), while user-created tags
    -- use canonical `presettag_{uuidv7}` entity IDs. Consumers must branch on
    -- the reserved `presettag_` namespace before parsing; a builtin key must
    -- never be coerced to PresetTagId, and a presettag value must resolve to a
    -- user row with the same dimension.
    tag_key    TEXT NOT NULL CHECK (trim(tag_key) <> ''),
    dimension  TEXT NOT NULL CHECK (dimension IN ('audience','scenario')),
    PRIMARY KEY (preset_id, tag_key, dimension)
);

-- table: preset_tags
CREATE TABLE preset_tags (
    -- Only user-authored rows live here. Built-in tags are manifest natural
    -- keys and are intentionally not duplicated into the database.
    key         TEXT PRIMARY KEY NOT NULL CHECK (trim(key) <> ''),
    dimension   TEXT NOT NULL CHECK (dimension IN ('audience','scenario')),
    label       TEXT NOT NULL,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    CHECK (key GLOB 'presettag_*')
);

-- table: preset_targets
CREATE TABLE "preset_targets" (
    preset_id   TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL CHECK (target_kind IN
        ('conversation', 'execution_step', 'companion', 'public_companion', 'cron')),
    PRIMARY KEY (preset_id, target_kind)
);

-- table: preset_user_state
CREATE TABLE preset_user_state (
    preset_id        TEXT PRIMARY KEY NOT NULL,
    enabled          INTEGER NOT NULL DEFAULT 1,
    auto_selectable  INTEGER NOT NULL DEFAULT 0,
    preferred_agent_id TEXT,
    sort_order       INTEGER NOT NULL DEFAULT 0,
    last_used_at     INTEGER,
    updated_at       INTEGER NOT NULL
);

-- table: presets
CREATE TABLE presets (
    id                  TEXT PRIMARY KEY NOT NULL,
    source_kind         TEXT NOT NULL DEFAULT 'user'
                            CHECK (source_kind IN ('builtin','user','extension')),
    source_key          TEXT,
    revision            INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    name                TEXT NOT NULL,
    description         TEXT,
    routing_description TEXT,
    instructions        TEXT NOT NULL DEFAULT '',
    avatar              TEXT,
    fallback_allowed    INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

-- table: providers
CREATE TABLE providers (
    id                TEXT PRIMARY KEY NOT NULL,   -- prov_{uuidv7}
    platform          TEXT    NOT NULL,
    name              TEXT    NOT NULL,
    base_url          TEXT    NOT NULL,
    api_key_encrypted TEXT    NOT NULL,
    models            TEXT    NOT NULL DEFAULT '[]',
    enabled           INTEGER NOT NULL DEFAULT 1,
    capabilities      TEXT    NOT NULL DEFAULT '[]',
    context_limit     INTEGER,
    model_protocols   TEXT,
    model_enabled     TEXT,
    model_health      TEXT,
    bedrock_config    TEXT,
    is_full_url       INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
, model_descriptions TEXT NOT NULL DEFAULT '{}', model_context_limits TEXT NOT NULL DEFAULT '{}', sort_order INTEGER NOT NULL DEFAULT 0);

-- table: remote_agents
CREATE TABLE remote_agents (
    id                 TEXT PRIMARY KEY NOT NULL,
    name               TEXT    NOT NULL,
    protocol           TEXT    NOT NULL,
    url                TEXT    NOT NULL,
    auth_type          TEXT    NOT NULL,
    auth_token         TEXT,
    allow_insecure     INTEGER NOT NULL DEFAULT 0,
    avatar             TEXT,
    description        TEXT,
    device_id          TEXT,
    device_public_key  TEXT,
    device_private_key TEXT,
    device_token       TEXT,
    status             TEXT    NOT NULL DEFAULT 'unknown',
    last_connected_at  INTEGER,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

-- table: requirement_tags
CREATE TABLE requirement_tags (
    tag           TEXT    PRIMARY KEY,
    paused        INTEGER NOT NULL DEFAULT 0,
    paused_reason TEXT,
    paused_req_id TEXT,
    paused_at     INTEGER,
    FOREIGN KEY (paused_req_id) REFERENCES requirements(id) ON DELETE SET NULL
);

-- table: requirements
CREATE TABLE requirements (
    id               TEXT PRIMARY KEY NOT NULL,
    title            TEXT    NOT NULL,
    content          TEXT    NOT NULL DEFAULT '',
    tag              TEXT    NOT NULL,
    order_key        TEXT    NOT NULL DEFAULT '',
    sort_seq         TEXT    NOT NULL DEFAULT '',           -- normalized sortable form of order_key (NOT a display seq)
    status           TEXT    NOT NULL DEFAULT 'pending',
    priority         INTEGER NOT NULL DEFAULT 0,
    completion_note  TEXT,
    owner_conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
    owner_terminal_id     TEXT REFERENCES terminal_sessions(id) ON DELETE SET NULL,
    active_turn_started_at       INTEGER,
    lease_expires_at INTEGER,
    started_at       INTEGER,
    completed_at     INTEGER,
    attempt_count    INTEGER NOT NULL DEFAULT 0,
    created_by       TEXT    NOT NULL DEFAULT 'user',
    extra            TEXT    NOT NULL DEFAULT '{}',
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    CHECK (owner_conversation_id IS NULL OR owner_terminal_id IS NULL)
);

-- table: skill_tags
CREATE TABLE skill_tags (
    skill_name    TEXT PRIMARY KEY,
    audience_tags TEXT,
    scenario_tags TEXT,
    updated_at    INTEGER NOT NULL
);

-- table: system_settings
CREATE TABLE system_settings (
    id                        INTEGER PRIMARY KEY CHECK (id = 1),
    language                  TEXT    NOT NULL DEFAULT 'en-US',
    notification_enabled      INTEGER NOT NULL DEFAULT 1,
    cron_notification_enabled INTEGER NOT NULL DEFAULT 0,
    command_queue_enabled     INTEGER NOT NULL DEFAULT 0,
    save_upload_to_workspace  INTEGER NOT NULL DEFAULT 0,
    updated_at                INTEGER NOT NULL
);

-- table: tag_settings
CREATE TABLE tag_settings (
    tag         TEXT PRIMARY KEY,
    webhook_id  TEXT,
    description TEXT NOT NULL DEFAULT '',
    updated_at  INTEGER NOT NULL, notify_events TEXT NOT NULL DEFAULT 'done,failed,needs_review',
    FOREIGN KEY (webhook_id) REFERENCES webhooks(id) ON DELETE SET NULL
);

-- table: terminal_scrollback
CREATE TABLE terminal_scrollback (
    session_id  TEXT PRIMARY KEY NOT NULL,
    data        BLOB    NOT NULL,
    updated_at  INTEGER NOT NULL,
    FOREIGN KEY (session_id) REFERENCES terminal_sessions(id) ON DELETE CASCADE
);

-- table: terminal_sessions
CREATE TABLE terminal_sessions (
    id            TEXT PRIMARY KEY NOT NULL,
    name          TEXT    NOT NULL,
    cwd           TEXT    NOT NULL,
    command       TEXT    NOT NULL,
    args          TEXT    NOT NULL DEFAULT '[]',
    env           TEXT,
    backend       TEXT,
    mode          TEXT,
    cols          INTEGER NOT NULL DEFAULT 80,
    rows          INTEGER NOT NULL DEFAULT 24,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    last_status   TEXT    NOT NULL DEFAULT 'running'
                  CHECK(last_status IN ('running', 'exited', 'error')),
    exit_code     INTEGER,
    user_id       TEXT    NOT NULL,
    pinned        INTEGER NOT NULL DEFAULT 0,
    pinned_at     INTEGER,
    autowork      TEXT,
    idmm          TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

-- table: users
CREATE TABLE users (
    id            TEXT PRIMARY KEY NOT NULL,
    username      TEXT NOT NULL UNIQUE,
    email         TEXT UNIQUE,
    password_hash TEXT NOT NULL,
    avatar_path   TEXT,
    jwt_secret    TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    last_login    INTEGER
);

-- Non-entity singleton that identifies the owner of this installation.
-- `key` is a natural singleton discriminator; the referencable user remains a
-- normal canonical `user_<uuidv7>` entity. Backups/restores preserve this row,
-- so the installation owner is stable for the lifetime of a dataset.
CREATE TABLE installation_identity (
    key           TEXT PRIMARY KEY NOT NULL CHECK (key = 'installation'),
    owner_user_id TEXT NOT NULL UNIQUE,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE RESTRICT
);

CREATE TRIGGER installation_identity_delete_guard
BEFORE DELETE ON installation_identity
BEGIN
    SELECT RAISE(ABORT, 'installation identity is immutable');
END;
CREATE TRIGGER installation_identity_update_guard
BEFORE UPDATE ON installation_identity
BEGIN
    SELECT RAISE(ABORT, 'installation identity is immutable');
END;

-- table: webhooks
CREATE TABLE webhooks (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    platform    TEXT NOT NULL DEFAULT 'lark',
    url         TEXT NOT NULL,
    secret      TEXT,
    description TEXT NOT NULL DEFAULT '',
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- table: workshop_assets
CREATE TABLE workshop_assets (
    id             TEXT PRIMARY KEY,
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
    in_library     INTEGER NOT NULL DEFAULT 1,
    origin         TEXT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

-- table: workshop_canvases
CREATE TABLE workshop_canvases (
    id                 TEXT PRIMARY KEY,
    title              TEXT NOT NULL,
    thumbnail_rel_path TEXT,
    node_count         INTEGER NOT NULL DEFAULT 0,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

-- Stable builtin agent catalog. These are natural installation keys, not
-- application-minted user entities; runtime lookup is by backend.
INSERT OR IGNORE INTO agent_metadata
    (id, icon, name, backend, agent_type, agent_source, agent_source_info,
     enabled, command, args, env, native_skills_dirs, behavior_policy, yolo_id,
     agent_capabilities, auth_methods,
     sort_order, created_at, updated_at)
VALUES
    -- ACP builtin agents
    ('agent_builtin_claude', '/api/assets/logos/ai-major/claude.svg', 'Claude Code',
     'claude', 'acp', 'builtin', '{"binary_name":"claude","bridge_binary":"bun"}',
     1, 'bun', '["x","--bun","@agentclientprotocol/claude-agent-acp@0.33.1"]', '[]',
     '[".claude/skills"]',
     '{"supports_side_question":true,"self_identity_sticky":true,"session_load_via_meta_field":true,"supports_team":true}',
     'bypassPermissions',
     NULL, NULL,
     3100,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_codex', '/api/assets/logos/tools/coding/codex.svg', 'Codex CLI',
     'codex', 'acp', 'builtin', '{"binary_name":"codex","bridge_binary":"bun"}',
     1, 'bun', '["x","--bun","@zed-industries/codex-acp@0.14.0"]', '[]',
     '[".codex/skills"]',
     '{"supports_side_question":false,"supports_team":true}',
     'full-access',
     NULL, NULL,
     3110,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_gemini', '/api/assets/logos/ai-major/gemini.svg', 'Gemini CLI',
     'gemini', 'acp', 'builtin', '{"binary_name":"gemini"}',
     1, 'gemini', '["--experimental-acp"]', '[]',
     '[".gemini/skills"]',
     '{"supports_side_question":false,"supports_team":true}',
     'yolo',
     NULL, NULL,
     3120,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_qwen', '/api/assets/logos/ai-china/qwen.svg', 'Qwen',
     'qwen', 'acp', 'builtin', '{"binary_name":"qwen"}',
     1, 'qwen', '["--acp"]', '[]',
     '[".qwen/skills"]',
     '{"supports_side_question":false}',
     'yolo',
     '{"load_session":true,"prompt_capabilities":{"image":true,"audio":true,"embedded_context":true},"session_capabilities":{"list":{},"resume":{}},"mcp_capabilities":{"sse":true,"http":true}}',
     '[{"id":"openai","name":"Use OpenAI API key","description":"Requires setting the `OPENAI_API_KEY` environment variable","_meta":{"type":"terminal","args":["--auth-type=openai"]}},{"id":"qwen-oauth","name":"Qwen OAuth","description":"Qwen OAuth (free tier discontinued 2026-04-15)","_meta":{"type":"terminal","args":["--auth-type=qwen-oauth"]}}]',
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_codebuddy', '/api/assets/logos/tools/coding/codebuddy.svg', 'CodeBuddy',
     'codebuddy', 'acp', 'builtin', '{"binary_name":"codebuddy","bridge_binary":"bun"}',
     1, 'bun', '["x","--bun","@tencent-ai/codebuddy-code@2.97.0","--acp"]', '[]',
     '[".codebuddy/skills"]',
     '{"supports_side_question":false,"supports_team":true}',
     'bypassPermissions',
     '{"prompt_capabilities":{"image":true,"embedded_context":true},"mcp_capabilities":{"http":true,"sse":true},"load_session":true,"delegate_tools_support":true}',
     '[{"id":"iOA","name":"Login with iOA","description":null},{"id":"external","name":"Login with Google/Github","description":null},{"id":"internal","name":"Login with WeChat","description":null},{"id":"selfhosted","name":"Login with Enterprise Domain","description":null}]',
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_droid', '/api/assets/logos/brand/droid.svg', 'Droid',
     'droid', 'acp', 'builtin', '{"binary_name":"droid"}',
     1, 'droid', '["exec","--output-format","acp"]', '[]',
     '[".factory/skills"]',
     '{"supports_side_question":false}',
     'yolo',
     '{"load_session":true,"session_capabilities":{"list":{},"resume":{}},"prompt_capabilities":{"image":true,"embedded_context":true},"_meta":{"terminal_output":true,"terminal-auth":true}}',
     '[{"id":"device-pairing","name":"Login","description":"Authenticate with Factory using a device pairing code in your browser."},{"id":"factory-api-key","name":"Factory API Key","description":"Authenticate using a Factory API key set in the FACTORY_API_KEY environment variable."}]',
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_goose', '/api/assets/logos/tools/goose.svg', 'Goose',
     'goose', 'acp', 'builtin', '{"binary_name":"goose"}',
     1, 'goose', '["acp"]', '[]',
     '[".goose/skills"]',
     '{"supports_side_question":false}',
     'yolo',
     '{"load_session":true,"prompt_capabilities":{"image":true,"audio":false,"embedded_context":true},"mcp_capabilities":{"http":true,"sse":false},"session_capabilities":{"list":{},"close":{}},"auth":{}}',
     '[{"id":"goose-provider","name":"Configure Provider","description":"Run `goose configure` to set up your AI provider and API key"}]',
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_auggie', '/api/assets/logos/brand/auggie.svg', 'Auggie',
     'auggie', 'acp', 'builtin', '{"binary_name":"auggie"}',
     1, 'auggie', '["--acp"]', '[]',
     NULL,
     '{"supports_side_question":false}',
     'yolo',
     '{"load_session":true,"prompt_capabilities":{"image":true},"session_capabilities":{"list":{}}}',
     '[]',
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_kimi', '/api/assets/logos/ai-china/kimi.svg', 'Kimi',
     'kimi', 'acp', 'builtin', '{"binary_name":"kimi"}',
     1, 'kimi', '["acp"]', '[]',
     '[".kimi/skills"]',
     '{"supports_side_question":false}',
     'yolo',
     '{"load_session":true,"mcp_capabilities":{"http":true,"sse":false},"prompt_capabilities":{"audio":false,"embedded_context":true,"image":true},"session_capabilities":{"list":{},"resume":{}}}',
     '[{"_meta":{"terminal-auth":{"command":"kimi","args":["login"],"label":"Kimi Code Login","env":{},"type":"terminal"}},"description":"Run `kimi login` command in the terminal, then follow the instructions to finish login.","id":"login","name":"Login with Kimi account"}]',
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_opencode', '/api/assets/logos/tools/coding/opencode-light.svg', 'OpenCode',
     'opencode', 'acp', 'builtin', '{"binary_name":"opencode"}',
     1, 'opencode', '["acp"]', '[]',
     '[".opencode/skills"]',
     '{"supports_side_question":false}',
     'build',
     NULL, NULL,
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_copilot', '/api/assets/logos/tools/github.svg', 'Copilot',
     'copilot', 'acp', 'builtin', '{"binary_name":"copilot"}',
     1, 'copilot', '["--acp","--stdio"]', '[]',
     NULL,
     '{"supports_side_question":false}',
     'yolo',
     '{"load_session":true,"mcp_capabilities":{"http":true,"sse":true},"prompt_capabilities":{"image":true,"audio":false,"embedded_context":true},"session_capabilities":{"list":{}}}',
     '[{"id":"copilot-login","name":"Log in with Copilot CLI","description":"Run `copilot login` in the terminal","_meta":{"terminal-auth":{"command":"copilot","args":["login"],"label":"Copilot Login"}}}]',
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_qoder', '/api/assets/logos/tools/coding/qoder.png', 'Qoder',
     'qoder', 'acp', 'builtin', '{"binary_name":"qodercli"}',
     1, 'qodercli', '["--acp"]', '[]',
     NULL,
     '{"supports_side_question":false}',
     'yolo',
     '{"load_session":true,"session_capabilities":{"list":{}},"prompt_capabilities":{"image":true,"audio":true,"embedded_context":true},"mcp_capabilities":{"http":true,"sse":true}}',
     '[{"id":"qodercli-login","name":"Use qodercli login","description":"Use your existing qodercli login for this agent. If needed, sign in from qodercli first."},{"type":"env_var","id":"qoder-personal-access-token","name":"Use QODER_PERSONAL_ACCESS_TOKEN","description":"Requires `QODER_PERSONAL_ACCESS_TOKEN` in the agent environment.","vars":[{"name":"QODER_PERSONAL_ACCESS_TOKEN"}]}]',
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_vibe', '/api/assets/logos/ai-major/mistral.svg', 'Vibe',
     'vibe', 'acp', 'builtin', '{"binary_name":"vibe-acp"}',
     1, 'vibe-acp', '[]', '[]',
     '[".vibe/skills"]',
     '{"supports_side_question":false}',
     'yolo',
     '{"load_session":true,"prompt_capabilities":{"audio":false,"embedded_context":true,"image":false},"session_capabilities":{"close":{},"fork":{},"list":{}}}',
     '[]',
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_cursor', '/api/assets/logos/tools/coding/cursor.png', 'Cursor',
     'cursor', 'acp', 'builtin', '{"binary_name":"agent"}',
     1, 'agent', '["acp"]', '[]',
     '[".cursor/skills"]',
     '{"supports_side_question":false}',
     'agent',
     NULL, NULL,
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_kiro', NULL, 'Kiro',
     'kiro', 'acp', 'builtin', '{"binary_name":"kiro-cli"}',
     1, 'kiro-cli', '["acp"]', '[]',
     NULL,
     '{"supports_side_question":false}',
     'yolo',
     '{"load_session":true,"prompt_capabilities":{"image":true,"audio":false,"embedded_context":false},"mcp_capabilities":{"http":true,"sse":false},"session_capabilities":{}}',
     '[{"id":"kiro-login","name":"Kiro Login","description":"Run ''kiro-cli login'' in terminal to authenticate. See https://kiro.dev/docs/cli/authentication/"}]',
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_hermes', '/api/assets/logos/brand/hermes.svg', 'Hermes',
     'hermes', 'acp', 'builtin', '{"binary_name":"hermes"}',
     1, 'hermes', '["acp"]', '[]',
     NULL,
     '{"supports_side_question":false}',
     'yolo',
     NULL, NULL,
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_snow', '/api/assets/logos/tools/coding/snow.png', 'Snow',
     'snow', 'acp', 'builtin', '{"binary_name":"snow"}',
     1, 'snow', '["--acp"]', '[]',
     NULL,
     '{"supports_side_question":false}',
     'yolo',
     NULL, NULL,
     3130,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    -- Non-ACP builtins
    ('agent_builtin_nanobot', '/api/assets/logos/tools/nanobot.svg', 'Nanobot',
     NULL, 'nanobot', 'builtin', '{"binary_name":"nanobot"}',
     1, 'nanobot', '["--experimental-acp"]', '[]',
     NULL,
     '{}',
     'yolo',
     NULL, NULL,
     3990,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    ('agent_builtin_openclaw', '/api/assets/logos/tools/openclaw.svg', 'OpenClaw',
     NULL, 'openclaw-gateway', 'builtin', '{"binary_name":"openclaw"}',
     1, 'openclaw', '[]', '[]',
     NULL,
     '{}',
     'yolo',
     NULL, NULL,
     3900,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000),

    -- Internal
    ('agent_builtin_nomi', '/api/assets/logos/brand/nomi.svg', 'Nomi',
     NULL, 'nomi', 'internal', '{}',
     1, NULL, '[]', '[]',
     '[".nomi/skills"]',
     '{"supports_team":true}',
     'yolo',
     NULL, NULL,
     100,
     unixepoch('now','subsec')*1000, unixepoch('now','subsec')*1000);



-- index: idx_acp_session_agent_id
CREATE INDEX idx_acp_session_agent_id ON acp_session(agent_id);

-- index: idx_acp_session_status
CREATE INDEX idx_acp_session_status ON acp_session(session_status);

-- index: idx_acp_session_suspended
CREATE INDEX idx_acp_session_suspended ON acp_session(session_status, suspended_at) WHERE session_status = 'suspended';

-- index: idx_agent_execution_attempts_one_active
CREATE UNIQUE INDEX idx_agent_execution_attempts_one_active
    ON agent_execution_attempts(execution_id, step_id)
    WHERE status IN ('queued', 'running', 'waiting_input');

-- index: idx_agent_execution_attempts_step
CREATE INDEX idx_agent_execution_attempts_step
    ON agent_execution_attempts(execution_id, step_id, attempt_no DESC);

-- index: idx_agent_execution_delegation_operation
CREATE UNIQUE INDEX idx_agent_execution_delegation_operation
    ON agent_execution_events(
        execution_id,
        json_extract(payload, '$.operation_id')
    )
    WHERE event_type = 'plan_changed'
      AND json_type(payload, '$.operation_id') = 'text';

-- index: idx_agent_execution_dependencies_active
CREATE UNIQUE INDEX idx_agent_execution_dependencies_active
    ON agent_execution_step_dependencies(execution_id, blocker_step_id, blocked_step_id)
    WHERE superseded_in_revision IS NULL;

-- index: idx_agent_execution_dependencies_active_blocked
CREATE INDEX idx_agent_execution_dependencies_active_blocked
    ON agent_execution_step_dependencies(execution_id, blocked_step_id)
    WHERE superseded_in_revision IS NULL;

-- index: idx_agent_execution_dependencies_blocked
CREATE INDEX idx_agent_execution_dependencies_blocked
    ON agent_execution_step_dependencies(execution_id, blocked_step_id);

-- index: idx_agent_execution_events_unpublished
CREATE INDEX idx_agent_execution_events_unpublished
    ON agent_execution_events(execution_id, sequence)
    WHERE published_at IS NULL;

-- index: idx_agent_execution_participants_active
CREATE INDEX idx_agent_execution_participants_active
    ON agent_execution_participants(execution_id, sort_order, id)
    WHERE retired_in_revision IS NULL;

-- index: idx_agent_execution_participants_order
CREATE INDEX idx_agent_execution_participants_order
    ON agent_execution_participants(execution_id, sort_order, id);

-- index: idx_agent_execution_steps_active_status
CREATE INDEX idx_agent_execution_steps_active_status
    ON agent_execution_steps(execution_id, status, updated_at)
    WHERE superseded_in_revision IS NULL;

-- index: idx_agent_execution_steps_status
CREATE INDEX idx_agent_execution_steps_status
    ON agent_execution_steps(execution_id, status, updated_at);

-- index: idx_agent_execution_template_participants_order
CREATE INDEX idx_agent_execution_template_participants_order
    ON agent_execution_template_participants(template_id, sort_order, id);

-- index: idx_agent_execution_template_participants_provider
CREATE INDEX idx_agent_execution_template_participants_provider
    ON agent_execution_template_participants(provider_id, template_id)
    WHERE provider_id IS NOT NULL;

-- index: idx_agent_execution_templates_owner_updated
CREATE INDEX idx_agent_execution_templates_owner_updated
    ON agent_execution_templates(user_id, updated_at DESC, id);

-- index: idx_agent_executions_owner_updated
CREATE INDEX idx_agent_executions_owner_updated
    ON agent_executions(user_id, updated_at DESC) WHERE deleted_at IS NULL;

-- index: idx_agent_executions_status_lease
CREATE INDEX idx_agent_executions_status_lease
    ON agent_executions(status, lease_expires_at) WHERE deleted_at IS NULL;

-- index: idx_agent_metadata_agent_type
CREATE INDEX idx_agent_metadata_agent_type ON agent_metadata(agent_type);

-- index: idx_agent_metadata_backend
CREATE INDEX idx_agent_metadata_backend ON agent_metadata(backend);

-- index: idx_agent_metadata_sort_order
CREATE INDEX idx_agent_metadata_sort_order ON agent_metadata(sort_order);

-- index: idx_attachments_requirement
CREATE INDEX idx_attachments_requirement ON attachments(requirement_id);

-- index: idx_channel_pairing_codes_channel
CREATE INDEX idx_channel_pairing_codes_channel ON channel_pairing_codes(channel_id);

-- index: idx_channel_pairing_codes_status
CREATE INDEX idx_channel_pairing_codes_status ON channel_pairing_codes(status);

-- index: idx_channel_sessions_channel
CREATE INDEX idx_channel_sessions_channel ON channel_sessions(channel_id);

-- index: idx_channel_sessions_user_chat
CREATE INDEX idx_channel_sessions_user_chat ON channel_sessions(user_id, chat_id);

-- index: idx_channel_sessions_user_id
CREATE INDEX idx_channel_sessions_user_id ON channel_sessions(user_id);

-- index: idx_channel_users_channel
CREATE INDEX idx_channel_users_channel ON channel_users(channel_id);

-- index: idx_connector_credentials_kind
CREATE INDEX idx_connector_credentials_kind ON connector_credentials(kind);

-- index: idx_conversation_artifacts_conversation_created
CREATE INDEX idx_conversation_artifacts_conversation_created ON conversation_artifacts(conversation_id, created_at);

-- index: idx_conversation_artifacts_conversation_id
CREATE INDEX idx_conversation_artifacts_conversation_id ON conversation_artifacts(conversation_id);

-- index: idx_conversation_artifacts_created_at
CREATE INDEX idx_conversation_artifacts_created_at ON conversation_artifacts(created_at);

-- index: idx_conversation_artifacts_cron_job
CREATE INDEX idx_conversation_artifacts_cron_job ON conversation_artifacts(cron_job_id);

-- index: idx_conversation_artifacts_kind_status
CREATE INDEX idx_conversation_artifacts_kind_status ON conversation_artifacts(kind, status);

-- index: idx_conversation_delivery_receipts_conversation
CREATE INDEX idx_conversation_delivery_receipts_conversation
    ON conversation_delivery_receipts(conversation_id, created_at);

-- index: idx_conversation_execution_active_attempt
CREATE UNIQUE INDEX idx_conversation_execution_active_attempt
    ON conversation_execution_links(execution_id, step_id, attempt_id)
    WHERE relation = 'attempt' AND active = 1;

-- index: idx_conversation_execution_active_attempt_conversation
CREATE UNIQUE INDEX idx_conversation_execution_active_attempt_conversation
    ON conversation_execution_links(conversation_id)
    WHERE relation = 'attempt' AND active = 1;

-- index: idx_conversation_execution_active_lead
CREATE UNIQUE INDEX idx_conversation_execution_active_lead
    ON conversation_execution_links(execution_id) WHERE relation = 'lead' AND active = 1;

-- index: idx_conversation_execution_current_lead
CREATE UNIQUE INDEX idx_conversation_execution_current_lead
    ON conversation_execution_links(conversation_id)
    WHERE relation = 'lead' AND active = 1;

-- index: idx_conversation_execution_execution
CREATE INDEX idx_conversation_execution_execution
    ON conversation_execution_links(execution_id, relation, active);

-- index: idx_conversation_execution_lookup
CREATE INDEX idx_conversation_execution_lookup
    ON conversation_execution_links(conversation_id, active DESC, created_at DESC);

-- index: idx_conversation_execution_pending_cleanup
CREATE INDEX idx_conversation_execution_pending_cleanup
    ON conversation_execution_links(execution_id, conversation_id)
    WHERE relation = 'attempt' AND active = 0 AND cleanup_completed_at IS NULL;

-- index: idx_conversation_mcp_servers_mcp
CREATE INDEX idx_conversation_mcp_servers_mcp ON conversation_mcp_servers(mcp_server_id);

-- index: idx_conversations_cron_job_id
CREATE INDEX idx_conversations_cron_job_id ON conversations(cron_job_id);

-- index: idx_conversations_preset_id
CREATE INDEX idx_conversations_preset_id ON conversations(preset_id);

-- index: idx_conversations_source
CREATE INDEX idx_conversations_source ON conversations(source);

-- index: idx_conversations_source_chat
CREATE INDEX idx_conversations_source_chat ON conversations(source, channel_chat_id, updated_at DESC);

-- index: idx_conversations_source_updated
CREATE INDEX idx_conversations_source_updated ON conversations(source, updated_at DESC);

-- index: idx_conversations_type
CREATE INDEX idx_conversations_type ON conversations(type);

-- index: idx_conversations_updated_at
CREATE INDEX idx_conversations_updated_at ON conversations(updated_at);

-- index: idx_conversations_user_id
CREATE INDEX idx_conversations_user_id ON conversations(user_id);

-- index: idx_conversations_user_updated
CREATE INDEX idx_conversations_user_updated ON conversations(user_id, updated_at DESC);

-- index: idx_creation_tasks_canvas
CREATE INDEX idx_creation_tasks_canvas ON creation_tasks(canvas_id);

-- index: idx_creation_tasks_status
CREATE INDEX idx_creation_tasks_status ON creation_tasks(status);

-- index: idx_cron_job_runs_job_time
CREATE INDEX idx_cron_job_runs_job_time
    ON cron_job_runs(job_id, executed_at_ms DESC, created_at_ms DESC);

-- index: idx_cron_jobs_agent_type
CREATE INDEX idx_cron_jobs_agent_type
    ON cron_jobs(agent_type);

-- index: idx_cron_jobs_conversation
CREATE INDEX idx_cron_jobs_conversation
    ON cron_jobs(conversation_id);

-- index: idx_cron_jobs_next_run
CREATE INDEX idx_cron_jobs_next_run
    ON cron_jobs(next_run_at) WHERE enabled = 1;

-- index: idx_cron_jobs_preset_id
CREATE INDEX idx_cron_jobs_preset_id
    ON cron_jobs(preset_id);

-- index: idx_cron_jobs_user
CREATE INDEX idx_cron_jobs_user
    ON cron_jobs(user_id, created_at);

-- index: idx_cron_jobs_user_conversation
CREATE INDEX idx_cron_jobs_user_conversation
    ON cron_jobs(user_id, conversation_id, created_at);

-- index: idx_idmm_interventions_at
CREATE INDEX idx_idmm_interventions_at
    ON idmm_interventions(at);

-- index: idx_idmm_interventions_owner_activity
CREATE INDEX idx_idmm_interventions_owner_activity
    ON idmm_interventions(user_id, at DESC, id DESC);

-- index: idx_idmm_interventions_owner_target
CREATE INDEX idx_idmm_interventions_owner_target
    ON idmm_interventions(user_id, target_kind, target_id, at DESC, id DESC);

-- index: idx_kb_binding_bases_kb
CREATE INDEX idx_kb_binding_bases_kb ON knowledge_binding_bases(kb_id);

-- index: idx_mcp_servers_deleted_at
CREATE INDEX idx_mcp_servers_deleted_at ON mcp_servers(deleted_at);

-- index: idx_mcp_servers_enabled
CREATE INDEX idx_mcp_servers_enabled ON mcp_servers(enabled);

-- index: idx_mcp_servers_name
CREATE INDEX idx_mcp_servers_name ON mcp_servers(name);

-- index: idx_messages_conv_created
CREATE INDEX idx_messages_conv_created ON messages(conversation_id, created_at);

-- index: idx_messages_conv_created_desc
CREATE INDEX idx_messages_conv_created_desc ON messages(conversation_id, created_at DESC);

-- index: idx_messages_conv_created_id
CREATE INDEX idx_messages_conv_created_id
    ON messages (conversation_id, created_at DESC, id DESC);

-- index: idx_messages_conversation_id
CREATE INDEX idx_messages_conversation_id ON messages(conversation_id);

-- index: idx_messages_created_at
CREATE INDEX idx_messages_created_at ON messages(created_at);

-- index: idx_messages_msg_id
CREATE INDEX idx_messages_msg_id ON messages(msg_id);

-- index: idx_messages_type
CREATE INDEX idx_messages_type ON messages(type);

-- index: idx_messages_type_created
CREATE INDEX idx_messages_type_created ON messages(type, created_at DESC);

-- index: idx_message_correlations_message_id
CREATE INDEX idx_message_correlations_message_id ON message_correlations(message_id);

-- index: idx_preset_agent_rank
CREATE INDEX idx_preset_agent_rank ON preset_agent_preferences(preset_id, rank);

-- index: idx_preset_model_lookup
CREATE INDEX idx_preset_model_lookup ON preset_model_preferences(provider_id, model);

-- index: idx_preset_tags_dimension
CREATE INDEX idx_preset_tags_dimension ON preset_tags(dimension, sort_order);

-- index: idx_presets_source
CREATE UNIQUE INDEX idx_presets_source ON presets(source_kind, source_key)
    WHERE source_key IS NOT NULL;

-- index: idx_presets_updated_at
CREATE INDEX idx_presets_updated_at ON presets(updated_at DESC);

-- index: idx_providers_platform
CREATE INDEX idx_providers_platform ON providers(platform);

-- index: idx_providers_sort_order
CREATE INDEX idx_providers_sort_order ON providers(sort_order, created_at);

-- index: idx_remote_agents_status
CREATE INDEX idx_remote_agents_status ON remote_agents(status);

-- index: idx_requirements_owner
CREATE INDEX idx_requirements_owner_conversation ON requirements(owner_conversation_id);
CREATE INDEX idx_requirements_owner_terminal ON requirements(owner_terminal_id);

-- index: idx_requirements_status
CREATE INDEX idx_requirements_status     ON requirements(status);

-- index: idx_requirements_tag_order
CREATE INDEX idx_requirements_tag_order  ON requirements(tag, sort_seq);

-- index: idx_requirements_tag_status
CREATE INDEX idx_requirements_tag_status ON requirements(tag, status);

-- index: idx_terminal_sessions_user
CREATE INDEX idx_terminal_sessions_user ON terminal_sessions(user_id);

-- index: idx_users_email
CREATE INDEX idx_users_email ON users(email);

-- index: idx_users_username
CREATE INDEX idx_users_username ON users(username);

-- index: idx_workshop_assets_kind
CREATE INDEX idx_workshop_assets_kind ON workshop_assets(kind);

-- index: idx_workshop_assets_library
CREATE INDEX idx_workshop_assets_library ON workshop_assets(in_library);

-- index: idx_workshop_canvases_updated
CREATE INDEX idx_workshop_canvases_updated ON workshop_canvases(updated_at);

-- index: uq_channel_plugins_type_bot_key
CREATE UNIQUE INDEX uq_channel_plugins_type_bot_key
    ON channel_plugins(type, bot_key) WHERE bot_key IS NOT NULL;

-- index: uq_conversation_artifacts_skill_suggest
CREATE UNIQUE INDEX uq_conversation_artifacts_skill_suggest
    ON conversation_artifacts(conversation_id, cron_job_id) WHERE kind = 'skill_suggest';

-- index: uq_kb_binding_companion
CREATE UNIQUE INDEX uq_kb_binding_companion ON knowledge_bindings(target_companion_id) WHERE target_companion_id IS NOT NULL;

-- index: uq_kb_binding_conv
CREATE UNIQUE INDEX uq_kb_binding_conv      ON knowledge_bindings(target_conv_id)      WHERE target_conv_id      IS NOT NULL;

-- index: uq_kb_binding_term
CREATE UNIQUE INDEX uq_kb_binding_term      ON knowledge_bindings(target_term_id)      WHERE target_term_id      IS NOT NULL;

-- index: uq_kb_binding_workpath
CREATE UNIQUE INDEX uq_kb_binding_workpath  ON knowledge_bindings(target_workpath)     WHERE target_workpath     IS NOT NULL;

-- trigger: agent_execution_active_lead_conversation_retained
CREATE TRIGGER agent_execution_active_lead_conversation_retained
BEFORE DELETE ON conversations
WHEN EXISTS (
    SELECT 1
    FROM conversation_execution_links link
    JOIN agent_executions execution ON execution.id = link.execution_id
    JOIN users owner ON owner.id = execution.user_id
    WHERE link.conversation_id = OLD.id
      AND link.relation = 'lead' AND link.active = 1
      AND execution.deleted_at IS NULL
      AND execution.status NOT IN (
          'completed', 'completed_with_failures', 'failed', 'cancelled'
      )
)
BEGIN
    SELECT RAISE(ABORT, 'active Agent Execution lead conversation cannot be deleted');
END;

-- trigger: agent_execution_active_participant_limit
CREATE TRIGGER agent_execution_active_participant_limit
BEFORE INSERT ON agent_execution_participants
WHEN NEW.retired_in_revision IS NULL
 AND (SELECT COUNT(*) FROM agent_execution_participants
      WHERE execution_id = NEW.execution_id
        AND retired_in_revision IS NULL) >= 64
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution exceeds 64 active participants');
END;

-- trigger: agent_execution_active_step_limit
CREATE TRIGGER agent_execution_active_step_limit
BEFORE INSERT ON agent_execution_steps
WHEN (SELECT COUNT(*) FROM agent_execution_steps
      WHERE execution_id = NEW.execution_id
        AND superseded_in_revision IS NULL) >= 128
BEGIN
    SELECT RAISE(ABORT, 'active Agent Execution DAG exceeds 128 steps');
END;

-- trigger: agent_execution_attempt_conversation_retained
CREATE TRIGGER agent_execution_attempt_conversation_retained
BEFORE DELETE ON conversations
WHEN EXISTS (
        SELECT 1 FROM conversation_execution_links link
        WHERE link.conversation_id = OLD.id AND link.relation = 'attempt'
     )
 AND EXISTS (SELECT 1 FROM users WHERE id = OLD.user_id)
BEGIN
    SELECT RAISE(ABORT, 'execution attempt conversation is retained for audit');
END;

-- trigger: agent_execution_attempt_delete_guard
CREATE TRIGGER agent_execution_attempt_delete_guard
BEFORE DELETE ON agent_execution_attempts
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    JOIN users owner ON owner.id = execution.user_id
    WHERE execution.id = OLD.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'agent execution attempts cannot be deleted directly');
END;

-- trigger: agent_execution_attempt_insert_guard
CREATE TRIGGER agent_execution_attempt_insert_guard
BEFORE INSERT ON agent_execution_attempts
WHEN NOT EXISTS (
         SELECT 1 FROM agent_execution_steps step
         WHERE step.execution_id = NEW.execution_id
           AND step.id = NEW.step_id
           AND step.superseded_in_revision IS NULL
     )
  OR NEW.attempt_no IS NOT COALESCE((
         SELECT max(attempt.attempt_no) + 1
         FROM agent_execution_attempts attempt
         WHERE attempt.execution_id = NEW.execution_id
           AND attempt.step_id = NEW.step_id
     ), 0)
BEGIN
    SELECT RAISE(ABORT, 'attempt must append contiguously to the current step');
END;

-- trigger: agent_execution_attempt_kind_guard_insert
CREATE TRIGGER agent_execution_attempt_kind_guard_insert
BEFORE INSERT ON agent_execution_attempts
WHEN EXISTS (
    SELECT 1
    FROM agent_execution_steps step
    WHERE step.execution_id = NEW.execution_id AND step.id = NEW.step_id
      AND (
          step.superseded_in_revision IS NOT NULL
          OR (step.kind = 'agent' AND NEW.participant_id IS NULL)
          OR (step.kind IN ('verify', 'judge', 'loop') AND (
              NEW.participant_id IS NOT NULL OR NEW.status = 'queued'
          ))
      )
)
BEGIN
    SELECT RAISE(ABORT, 'attempt kind/participant does not match its active step');
END;

-- trigger: agent_execution_attempt_kind_guard_update
CREATE TRIGGER agent_execution_attempt_kind_guard_update
BEFORE UPDATE ON agent_execution_attempts
WHEN EXISTS (
    SELECT 1
    FROM agent_execution_steps step
    WHERE step.execution_id = NEW.execution_id AND step.id = NEW.step_id
      AND (
          step.superseded_in_revision IS NOT NULL
          OR (step.kind = 'agent' AND NEW.participant_id IS NULL)
          OR (step.kind IN ('verify', 'judge', 'loop') AND (
              NEW.participant_id IS NOT NULL OR NEW.status = 'queued'
          ))
      )
)
BEGIN
    SELECT RAISE(ABORT, 'attempt kind/participant does not match its active step');
END;

-- trigger: agent_execution_attempt_message_retained
CREATE TRIGGER agent_execution_attempt_message_retained
BEFORE DELETE ON messages
WHEN EXISTS (
    SELECT 1
    FROM conversation_execution_links link
    JOIN agent_executions execution ON execution.id = link.execution_id
    JOIN users owner ON owner.id = execution.user_id
    WHERE link.conversation_id = OLD.conversation_id
      AND link.relation = 'attempt'
)
BEGIN
    SELECT RAISE(ABORT, 'execution attempt messages are retained for audit');
END;

-- trigger: agent_execution_attempt_runtime_snapshot_immutable
CREATE TRIGGER agent_execution_attempt_runtime_snapshot_immutable
BEFORE UPDATE ON agent_execution_attempts
WHEN NEW.id IS NOT OLD.id
    OR NEW.execution_id IS NOT OLD.execution_id
    OR NEW.step_id IS NOT OLD.step_id
    OR NEW.attempt_no IS NOT OLD.attempt_no
    OR NEW.participant_id IS NOT OLD.participant_id
    OR NEW.trigger_reason <> OLD.trigger_reason
    OR NEW.effective_config <> OLD.effective_config
    OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'attempt identity and runtime snapshot are immutable');
END;

-- trigger: agent_execution_attempt_status_transition
CREATE TRIGGER agent_execution_attempt_status_transition
BEFORE UPDATE OF status ON agent_execution_attempts
WHEN NEW.status <> OLD.status AND NOT (
       (OLD.status = 'queued' AND NEW.status IN ('running', 'cancelled'))
    OR (OLD.status = 'running'
        AND NEW.status IN ('waiting_input', 'completed', 'failed', 'cancelled', 'interrupted'))
    OR (OLD.status = 'waiting_input'
        AND NEW.status IN ('running', 'completed', 'failed', 'cancelled', 'interrupted'))
)
BEGIN
    SELECT RAISE(ABORT, 'invalid agent execution attempt status transition');
END;

-- trigger: agent_execution_deleted_immutable
CREATE TRIGGER agent_execution_deleted_immutable
BEFORE UPDATE ON agent_executions
WHEN OLD.deleted_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'deleted agent execution is immutable');
END;

-- trigger: agent_execution_dependency_delete_guard
CREATE TRIGGER agent_execution_dependency_delete_guard
BEFORE DELETE ON agent_execution_step_dependencies
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    JOIN users owner ON owner.id = execution.user_id
    WHERE execution.id = OLD.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'agent execution dependencies cannot be deleted directly');
END;

-- trigger: agent_execution_dependency_lifecycle
CREATE TRIGGER agent_execution_dependency_lifecycle
BEFORE UPDATE ON agent_execution_step_dependencies
WHEN OLD.superseded_in_revision IS NOT NULL
  OR NEW.superseded_in_revision IS NULL
  OR NEW.superseded_in_revision <= OLD.introduced_in_revision
  OR NEW.execution_id IS NOT OLD.execution_id
  OR NEW.blocker_step_id IS NOT OLD.blocker_step_id
  OR NEW.blocked_step_id IS NOT OLD.blocked_step_id
  OR NEW.introduced_in_revision IS NOT OLD.introduced_in_revision
BEGIN
    SELECT RAISE(ABORT, 'dependency revision is immutable; only first supersession is allowed');
END;

-- trigger: agent_execution_dependency_revision_insert_guard
CREATE TRIGGER agent_execution_dependency_revision_insert_guard
BEFORE INSERT ON agent_execution_step_dependencies
WHEN NEW.introduced_in_revision IS NOT (
         SELECT execution.plan_revision FROM agent_executions execution
         WHERE execution.id = NEW.execution_id
     )
  OR NEW.superseded_in_revision IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'dependency must be introduced in the current plan revision');
END;

-- trigger: agent_execution_dependency_revision_supersede_guard
CREATE TRIGGER agent_execution_dependency_revision_supersede_guard
BEFORE UPDATE OF superseded_in_revision ON agent_execution_step_dependencies
WHEN NEW.superseded_in_revision IS NOT (
    SELECT execution.plan_revision FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'dependency must be superseded in the current plan revision');
END;

-- trigger: agent_execution_event_agent_link_guard
CREATE TRIGGER agent_execution_event_agent_link_guard
BEFORE INSERT ON agent_execution_events
WHEN NEW.actor_type = 'agent'
 AND NEW.actor_conversation_id IS NOT NULL
 AND (
    (SELECT COUNT(*) FROM conversation_execution_links link
     JOIN agent_executions execution ON execution.id = link.execution_id
     WHERE link.execution_id = NEW.execution_id
       AND link.conversation_id = NEW.actor_conversation_id
       AND link.active = 1
       AND execution.user_id = NEW.on_behalf_of_user_id
       AND execution.deleted_at IS NULL) <> 1
    OR EXISTS (
        SELECT 1 FROM conversation_execution_links link
        WHERE link.execution_id = NEW.execution_id
          AND link.conversation_id = NEW.actor_conversation_id
          AND link.active = 1
          AND link.relation = 'attempt'
          AND link.attempt_id IS NOT NEW.actor_attempt_id
    )
    OR (NEW.actor_attempt_id IS NOT NULL AND
        (SELECT COUNT(*) FROM conversation_execution_links link
         JOIN agent_executions execution ON execution.id = link.execution_id
         WHERE link.conversation_id = NEW.actor_conversation_id
           AND link.attempt_id = NEW.actor_attempt_id
           AND link.relation = 'attempt'
           AND link.active = 1
           AND execution.user_id = NEW.on_behalf_of_user_id
           AND execution.deleted_at IS NULL) <> 1)
)
BEGIN
    SELECT RAISE(ABORT, 'execution event Agent actor requires one active caller link');
END;

-- trigger: agent_execution_event_delete_guard
CREATE TRIGGER agent_execution_event_delete_guard
BEFORE DELETE ON agent_execution_events
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    JOIN users owner ON owner.id = execution.user_id
    WHERE execution.id = OLD.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'agent execution events cannot be deleted directly');
END;

-- trigger: agent_execution_event_external_actor_guard
CREATE TRIGGER agent_execution_event_external_actor_guard
BEFORE INSERT ON agent_execution_events
WHEN NEW.actor_type = 'agent'
 AND NEW.actor_conversation_id IS NULL
 AND NOT (
     (NEW.sequence = 1
      AND NEW.event_type = 'created')
     OR EXISTS (
         SELECT 1
         FROM agent_execution_events baseline
         WHERE baseline.execution_id = NEW.execution_id
          AND baseline.sequence = 1
          AND baseline.event_type = 'created'
          AND baseline.actor_type = 'agent'
          AND baseline.actor_conversation_id IS NULL
          AND baseline.actor_attempt_id IS NULL
          AND baseline.actor_id = NEW.actor_id
     )
 )
BEGIN
    SELECT RAISE(ABORT, 'external Agent actor must match execution initiator');
END;

-- trigger: agent_execution_event_fact_immutable
CREATE TRIGGER agent_execution_event_fact_immutable
BEFORE UPDATE ON agent_execution_events
WHEN NEW.id IS NOT OLD.id
  OR NEW.execution_id IS NOT OLD.execution_id
  OR NEW.sequence IS NOT OLD.sequence
  OR NEW.event_type IS NOT OLD.event_type
  OR NEW.step_id IS NOT OLD.step_id
  OR NEW.attempt_id IS NOT OLD.attempt_id
  OR NEW.actor_type IS NOT OLD.actor_type
  OR NEW.actor_id IS NOT OLD.actor_id
  OR NEW.actor_conversation_id IS NOT OLD.actor_conversation_id
  OR NEW.actor_attempt_id IS NOT OLD.actor_attempt_id
  OR NEW.on_behalf_of_user_id IS NOT OLD.on_behalf_of_user_id
  OR NEW.payload IS NOT OLD.payload
  OR NEW.created_at IS NOT OLD.created_at
  OR (OLD.published_at IS NOT NULL AND NEW.published_at IS NOT OLD.published_at)
BEGIN
    SELECT RAISE(ABORT, 'agent execution event fact is immutable');
END;

-- trigger: agent_execution_event_owner_guard
CREATE TRIGGER agent_execution_event_owner_guard
BEFORE INSERT ON agent_execution_events
WHEN NEW.on_behalf_of_user_id IS NOT (
    SELECT execution.user_id FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'execution event on-behalf user must match execution owner');
END;

-- trigger: agent_execution_event_sequence_guard
CREATE TRIGGER agent_execution_event_sequence_guard
BEFORE INSERT ON agent_execution_events
WHEN NEW.sequence IS NOT (
         SELECT execution.event_sequence FROM agent_executions execution
         WHERE execution.id = NEW.execution_id
     )
  OR NEW.sequence <> COALESCE((
         SELECT max(event.sequence) FROM agent_execution_events event
         WHERE event.execution_id = NEW.execution_id
     ), 0) + 1
  OR (NEW.sequence = 1 AND NEW.event_type NOT IN ('created', 'migrated'))
  OR (NEW.sequence <> 1 AND NEW.event_type IN ('created', 'migrated'))
BEGIN
    SELECT RAISE(ABORT, 'execution event sequence or baseline kind is invalid');
END;

-- trigger: agent_execution_identity_monotonic
CREATE TRIGGER agent_execution_identity_monotonic
BEFORE UPDATE ON agent_executions
WHEN NEW.id IS NOT OLD.id
  OR NEW.user_id IS NOT OLD.user_id
  OR NEW.initial_plan_input IS NOT OLD.initial_plan_input
  OR NEW.created_at IS NOT OLD.created_at
  OR NEW.version < OLD.version
  OR NEW.plan_revision < OLD.plan_revision
  OR NEW.event_sequence < OLD.event_sequence
BEGIN
    SELECT RAISE(ABORT, 'agent execution identity and revisions are immutable');
END;

-- trigger: agent_execution_owner_insert_guard
CREATE TRIGGER agent_execution_owner_insert_guard
BEFORE INSERT ON agent_executions
WHEN NEW.user_id IS NOT (
    SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
)
BEGIN
    SELECT RAISE(ABORT, 'AgentExecution requires the installation owner');
END;

-- trigger: agent_execution_participant_delete_guard
CREATE TRIGGER agent_execution_participant_delete_guard
BEFORE DELETE ON agent_execution_participants
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    JOIN users owner ON owner.id = execution.user_id
    WHERE execution.id = OLD.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'agent execution participants cannot be deleted directly');
END;

-- trigger: agent_execution_participant_provider_insert_guard
CREATE TRIGGER agent_execution_participant_provider_insert_guard
BEFORE INSERT ON agent_execution_participants
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
      AND execution.deleted_at IS NULL
      AND execution.status <> 'cancelled'
)
 AND NOT EXISTS (SELECT 1 FROM providers WHERE id = NEW.provider_id)
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution participant references a missing provider');
END;

-- trigger: agent_execution_participant_revision_insert_guard
CREATE TRIGGER agent_execution_participant_revision_insert_guard
BEFORE INSERT ON agent_execution_participants
WHEN NEW.introduced_in_revision IS NOT (
         SELECT execution.plan_revision FROM agent_executions execution
         WHERE execution.id = NEW.execution_id
     )
  OR NEW.retired_in_revision IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'participant must be introduced in the current plan revision');
END;

-- trigger: agent_execution_participant_revision_retire_guard
CREATE TRIGGER agent_execution_participant_revision_retire_guard
BEFORE UPDATE OF retired_in_revision ON agent_execution_participants
WHEN NEW.retired_in_revision IS NOT (
    SELECT execution.plan_revision FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'participant must retire in the current plan revision');
END;

-- trigger: agent_execution_participants_immutable
CREATE TRIGGER agent_execution_participants_immutable
BEFORE UPDATE ON agent_execution_participants
WHEN OLD.retired_in_revision IS NOT NULL
  OR NEW.retired_in_revision IS NULL
  OR NEW.retired_in_revision <= OLD.introduced_in_revision
  OR NEW.id IS NOT OLD.id
  OR NEW.execution_id IS NOT OLD.execution_id
  OR NEW.source_agent_id IS NOT OLD.source_agent_id
  OR NEW.preset_id IS NOT OLD.preset_id
  OR NEW.preset_revision IS NOT OLD.preset_revision
  OR NEW.preset_snapshot IS NOT OLD.preset_snapshot
  OR NEW.provider_id IS NOT OLD.provider_id
  OR NEW.model IS NOT OLD.model
  OR NEW.role IS NOT OLD.role
  OR NEW.capability IS NOT OLD.capability
  OR NEW.constraints IS NOT OLD.constraints
  OR NEW.description IS NOT OLD.description
  OR NEW.system_prompt IS NOT OLD.system_prompt
  OR NEW.enabled_skills IS NOT OLD.enabled_skills
  OR NEW.disabled_builtin_skills IS NOT OLD.disabled_builtin_skills
  OR NEW.sort_order IS NOT OLD.sort_order
  OR NEW.introduced_in_revision IS NOT OLD.introduced_in_revision
  OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'participant snapshot is immutable; only first retirement is allowed');
END;

-- trigger: agent_execution_reopen_participant_model_guard
CREATE TRIGGER agent_execution_reopen_participant_model_guard
BEFORE UPDATE OF status ON agent_executions
WHEN NEW.status = 'running'
 AND OLD.status IN ('completed', 'completed_with_failures', 'failed')
 AND EXISTS (
     SELECT 1 FROM agent_execution_participants participant
     WHERE participant.execution_id = OLD.id
       AND participant.retired_in_revision IS NULL
       AND (participant.provider_id IS NULL OR participant.model IS NULL)
 )
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution cannot reopen with an unresolved participant');
END;

-- trigger: agent_execution_reopenable_participant_model_guard
CREATE TRIGGER agent_execution_reopenable_participant_model_guard
BEFORE INSERT ON agent_execution_participants
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
      AND execution.status <> 'cancelled'
)
 AND (NEW.provider_id IS NULL OR NEW.model IS NULL)
BEGIN
    SELECT RAISE(ABORT, 'non-terminal Agent Execution participant requires a concrete model');
END;

-- trigger: agent_execution_requires_tombstone
CREATE TRIGGER agent_execution_requires_tombstone
BEFORE DELETE ON agent_executions
WHEN EXISTS (SELECT 1 FROM users WHERE id = OLD.user_id)
BEGIN
    SELECT RAISE(ABORT, 'agent execution must be tombstoned, not physically deleted');
END;

-- trigger: agent_execution_settled_attempt_immutable
CREATE TRIGGER agent_execution_settled_attempt_immutable
BEFORE UPDATE ON agent_execution_attempts
WHEN OLD.status IN ('completed', 'failed', 'cancelled', 'interrupted')
BEGIN
    SELECT RAISE(ABORT, 'settled execution attempt is immutable');
END;

-- trigger: agent_execution_status_transition
CREATE TRIGGER agent_execution_status_transition
BEFORE UPDATE OF status ON agent_executions
WHEN NEW.status <> OLD.status AND NOT (
       (OLD.status = 'planning'
        AND NEW.status IN ('awaiting_approval', 'running', 'failed', 'cancelled'))
    OR (OLD.status = 'awaiting_approval'
        AND NEW.status IN ('running', 'failed', 'cancelled'))
    OR (OLD.status = 'running'
        AND NEW.status IN ('awaiting_approval', 'paused', 'waiting_input', 'completed',
                           'completed_with_failures', 'failed', 'cancelled'))
    OR (OLD.status = 'paused'
        AND NEW.status IN (
            'awaiting_approval', 'running', 'waiting_input', 'failed', 'cancelled'
        ))
    OR (OLD.status = 'waiting_input'
        AND NEW.status IN ('awaiting_approval', 'running', 'paused', 'failed', 'cancelled'))
    OR (OLD.status IN ('completed', 'completed_with_failures', 'failed')
        AND NEW.status = 'running')
)
BEGIN
    SELECT RAISE(ABORT, 'invalid agent execution status transition');
END;

-- trigger: agent_execution_step_delete_guard
CREATE TRIGGER agent_execution_step_delete_guard
BEFORE DELETE ON agent_execution_steps
WHEN EXISTS (
    SELECT 1 FROM agent_executions execution
    JOIN users owner ON owner.id = execution.user_id
    WHERE execution.id = OLD.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'agent execution steps cannot be deleted directly');
END;

-- trigger: agent_execution_step_revision_insert_guard
CREATE TRIGGER agent_execution_step_revision_insert_guard
BEFORE INSERT ON agent_execution_steps
WHEN NEW.introduced_in_revision IS NOT (
         SELECT execution.plan_revision FROM agent_executions execution
         WHERE execution.id = NEW.execution_id
     )
  OR NEW.superseded_in_revision IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'step must be introduced in the current plan revision');
END;

-- trigger: agent_execution_step_revision_supersede_guard
CREATE TRIGGER agent_execution_step_revision_supersede_guard
BEFORE UPDATE OF superseded_in_revision ON agent_execution_steps
WHEN NEW.superseded_in_revision IS NOT (
    SELECT execution.plan_revision FROM agent_executions execution
    WHERE execution.id = NEW.execution_id
)
BEGIN
    SELECT RAISE(ABORT, 'step must be superseded in the current plan revision');
END;

-- trigger: agent_execution_step_snapshot_immutable
CREATE TRIGGER agent_execution_step_snapshot_immutable
BEFORE UPDATE ON agent_execution_steps
WHEN NEW.id IS NOT OLD.id
  OR NEW.execution_id IS NOT OLD.execution_id
  OR NEW.title IS NOT OLD.title
  OR NEW.spec IS NOT OLD.spec
  OR NEW.role IS NOT OLD.role
  OR NEW.tool_policy IS NOT OLD.tool_policy
  OR NEW.kind IS NOT OLD.kind
  OR NEW.agent_mode IS NOT OLD.agent_mode
  OR NEW.profile IS NOT OLD.profile
  OR NEW.fanout_group IS NOT OLD.fanout_group
  OR NEW.control_policy IS NOT OLD.control_policy
  OR NEW.delegation_depth IS NOT OLD.delegation_depth
  OR NEW.assigned_participant_id IS NOT OLD.assigned_participant_id
  OR NEW.assignment_score IS NOT OLD.assignment_score
  OR NEW.assignment_rationale IS NOT OLD.assignment_rationale
  OR NEW.assignment_source IS NOT OLD.assignment_source
  OR NEW.assignment_locked IS NOT OLD.assignment_locked
  OR NEW.failure_policy IS NOT OLD.failure_policy
  OR NEW.preset_prompt IS NOT OLD.preset_prompt
  OR NEW.graph_x IS NOT OLD.graph_x
  OR NEW.graph_y IS NOT OLD.graph_y
  OR NEW.introduced_in_revision IS NOT OLD.introduced_in_revision
  OR NEW.created_at IS NOT OLD.created_at
  OR NEW.version < OLD.version
  OR NEW.updated_at < OLD.updated_at
  OR (
       NEW.superseded_in_revision IS NOT OLD.superseded_in_revision
       AND (
            OLD.superseded_in_revision IS NOT NULL
            OR NEW.superseded_in_revision IS NULL
            OR NEW.superseded_in_revision <= OLD.introduced_in_revision
       )
     )
  OR (OLD.superseded_in_revision IS NOT NULL
      AND (NEW.status IS NOT OLD.status
           OR NEW.dispatch_after IS NOT OLD.dispatch_after
           OR NEW.version IS NOT OLD.version
           OR NEW.updated_at IS NOT OLD.updated_at))
BEGIN
    SELECT RAISE(ABORT, 'execution step semantics are immutable; create a new plan revision');
END;

-- trigger: agent_execution_step_status_transition
CREATE TRIGGER agent_execution_step_status_transition
BEFORE UPDATE OF status ON agent_execution_steps
WHEN NEW.status <> OLD.status AND NOT (
       (OLD.status = 'pending'
        AND NEW.status IN ('running', 'completed', 'failed', 'skipped', 'cancelled'))
    OR (OLD.status = 'running'
        AND NEW.status IN ('pending', 'waiting_input', 'completed', 'failed', 'cancelled'))
    OR (OLD.status = 'waiting_input'
        AND NEW.status IN ('pending', 'running', 'completed', 'failed', 'cancelled'))
    OR (OLD.status = 'completed' AND NEW.status = 'pending')
    OR (OLD.status IN ('failed', 'skipped') AND NEW.status IN ('pending', 'completed'))
)
BEGIN
    SELECT RAISE(ABORT, 'invalid agent execution step status transition');
END;

-- trigger: agent_execution_template_identity_guard
CREATE TRIGGER agent_execution_template_identity_guard
BEFORE UPDATE ON agent_execution_templates
WHEN NEW.id IS NOT OLD.id
  OR NEW.user_id IS NOT OLD.user_id
  OR NEW.created_at IS NOT OLD.created_at
  OR NEW.version < OLD.version
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template identity is immutable');
END;

-- trigger: agent_execution_template_model_limit
CREATE TRIGGER agent_execution_template_model_limit
BEFORE INSERT ON agent_execution_template_participants
WHEN NOT EXISTS (
         SELECT 1 FROM agent_execution_template_participants participant
         WHERE participant.template_id = NEW.template_id
           AND participant.provider_id = NEW.provider_id
           AND participant.model = NEW.model
     )
 AND (SELECT COUNT(*) FROM (
          SELECT participant.provider_id, participant.model
          FROM agent_execution_template_participants participant
          WHERE participant.template_id = NEW.template_id
          GROUP BY participant.provider_id, participant.model
     )) >= 16
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template exceeds 16 distinct models');
END;

-- trigger: agent_execution_template_owner_insert_guard
CREATE TRIGGER agent_execution_template_owner_insert_guard
BEFORE INSERT ON agent_execution_templates
WHEN NEW.user_id IS NOT (
    SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
)
BEGIN
    SELECT RAISE(ABORT, 'AgentExecutionTemplate requires the installation owner');
END;

-- trigger: agent_execution_template_participant_identity_guard
CREATE TRIGGER agent_execution_template_participant_identity_guard
BEFORE UPDATE ON agent_execution_template_participants
WHEN NEW.id IS NOT OLD.id
  OR NEW.template_id IS NOT OLD.template_id
  OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template Participant identity is immutable');
END;

-- trigger: agent_execution_template_participant_limit
CREATE TRIGGER agent_execution_template_participant_limit
BEFORE INSERT ON agent_execution_template_participants
WHEN (SELECT COUNT(*) FROM agent_execution_template_participants participant
      WHERE participant.template_id = NEW.template_id) >= 64
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template exceeds 64 participants');
END;

-- trigger: agent_execution_template_provider_insert_guard
CREATE TRIGGER agent_execution_template_provider_insert_guard
BEFORE INSERT ON agent_execution_template_participants
WHEN NOT EXISTS (SELECT 1 FROM providers WHERE id = NEW.provider_id)
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template references a missing provider');
END;

-- trigger: agent_execution_template_provider_update_guard
CREATE TRIGGER agent_execution_template_provider_update_guard
BEFORE UPDATE OF provider_id, model ON agent_execution_template_participants
WHEN NOT EXISTS (SELECT 1 FROM providers WHERE id = NEW.provider_id)
BEGIN
    SELECT RAISE(ABORT, 'Agent Execution Template references a missing provider');
END;

-- trigger: conversation_artifact_cron_job_owner_insert
CREATE TRIGGER conversation_artifact_cron_job_owner_insert
BEFORE INSERT ON conversation_artifacts
WHEN NEW.cron_job_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1
     FROM conversations conversation
     JOIN cron_jobs job ON job.id = NEW.cron_job_id
     WHERE conversation.id = NEW.conversation_id
       AND conversation.user_id = job.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'conversation artifact cron job owner mismatch');
END;

-- trigger: conversation_artifact_cron_job_owner_update
CREATE TRIGGER conversation_artifact_cron_job_owner_update
BEFORE UPDATE OF conversation_id, cron_job_id ON conversation_artifacts
WHEN NEW.cron_job_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1
     FROM conversations conversation
     JOIN cron_jobs job ON job.id = NEW.cron_job_id
     WHERE conversation.id = NEW.conversation_id
       AND conversation.user_id = job.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'conversation artifact cron job owner mismatch');
END;

-- trigger: conversation_creation_key_delete_guard
CREATE TRIGGER conversation_creation_key_delete_guard
BEFORE DELETE ON conversation_creation_keys
WHEN EXISTS (SELECT 1 FROM users WHERE id = OLD.user_id)
 AND EXISTS (SELECT 1 FROM conversations WHERE id = OLD.conversation_id)
BEGIN
    SELECT RAISE(ABORT, 'conversation creation key may only be cascade deleted');
END;

-- trigger: conversation_creation_key_immutable
CREATE TRIGGER conversation_creation_key_immutable
BEFORE UPDATE ON conversation_creation_keys
BEGIN
    SELECT RAISE(ABORT, 'conversation creation key is immutable');
END;

-- trigger: conversation_creation_key_owner_guard
CREATE TRIGGER conversation_creation_key_owner_guard
BEFORE INSERT ON conversation_creation_keys
WHEN (SELECT COUNT(*) FROM conversations conversation
      WHERE conversation.id = NEW.conversation_id
        AND conversation.user_id = NEW.user_id) <> 1
BEGIN
    SELECT RAISE(ABORT, 'conversation creation key owner must match conversation owner');
END;

-- trigger: conversation_cron_artifact_owner_immutable
CREATE TRIGGER conversation_cron_artifact_owner_immutable
BEFORE UPDATE OF user_id ON conversations
WHEN NEW.user_id IS NOT OLD.user_id
 AND EXISTS (
     SELECT 1 FROM conversation_artifacts artifact
     WHERE artifact.conversation_id = OLD.id
       AND artifact.cron_job_id IS NOT NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'cron artifact conversation owner is immutable');
END;

-- trigger: conversation_cron_job_owner_insert
CREATE TRIGGER conversation_cron_job_owner_insert
BEFORE INSERT ON conversations
WHEN NEW.cron_job_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM cron_jobs job
     WHERE job.id = NEW.cron_job_id
       AND job.user_id = NEW.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'conversation cron job owner mismatch');
END;

-- trigger: conversation_cron_job_owner_update
CREATE TRIGGER conversation_cron_job_owner_update
BEFORE UPDATE OF cron_job_id, user_id ON conversations
WHEN NEW.cron_job_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM cron_jobs job
     WHERE job.id = NEW.cron_job_id
       AND job.user_id = NEW.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'conversation cron job owner mismatch');
END;

-- trigger: conversation_delivery_receipt_delete_guard
CREATE TRIGGER conversation_delivery_receipt_delete_guard
BEFORE DELETE ON conversation_delivery_receipts
FOR EACH ROW
WHEN EXISTS (SELECT 1 FROM users WHERE id = OLD.user_id)
 AND EXISTS (SELECT 1 FROM conversations WHERE id = OLD.conversation_id)
BEGIN
    SELECT RAISE(ABORT, 'conversation delivery receipt may only be cascade deleted');
END;

-- trigger: conversation_delivery_receipt_owner_guard
CREATE TRIGGER conversation_delivery_receipt_owner_guard
BEFORE INSERT ON conversation_delivery_receipts
WHEN (SELECT COUNT(*) FROM conversations conversation
      WHERE conversation.id = NEW.conversation_id
        AND conversation.user_id = NEW.user_id) <> 1
BEGIN
    SELECT RAISE(ABORT, 'conversation delivery receipt owner must match conversation owner');
END;

-- trigger: conversation_delivery_receipt_update_guard
CREATE TRIGGER conversation_delivery_receipt_update_guard
BEFORE UPDATE ON conversation_delivery_receipts
FOR EACH ROW
BEGIN
    SELECT CASE WHEN
        NEW.operation_id IS NOT OLD.operation_id
        OR NEW.message_id IS NOT OLD.message_id
        OR NEW.conversation_id IS NOT OLD.conversation_id
        OR NEW.user_id IS NOT OLD.user_id
        OR NEW.kind IS NOT OLD.kind
        OR NEW.request_payload IS NOT OLD.request_payload
        OR NEW.created_at IS NOT OLD.created_at
    THEN RAISE(ABORT, 'conversation delivery receipt identity is immutable') END;

    SELECT CASE WHEN NEW.updated_at < OLD.updated_at
        THEN RAISE(ABORT, 'conversation delivery receipt time must be monotonic') END;

    SELECT CASE WHEN NOT (
        (
            OLD.status = 'accepted'
            AND NEW.status = 'completed'
            AND NEW.result_ok IS NOT NULL
            AND NEW.completed_at IS NOT NULL
        )
        OR (
            NEW.status IS OLD.status
            AND NEW.result_ok IS OLD.result_ok
            AND NEW.result_text IS OLD.result_text
            AND NEW.result_error IS OLD.result_error
            AND NEW.completed_at IS OLD.completed_at
            AND NEW.updated_at IS OLD.updated_at
        )
    ) THEN RAISE(ABORT, 'conversation delivery receipt has one terminal transition') END;
END;

-- trigger: conversation_execution_attempt_cannot_lead_another_execution
CREATE TRIGGER conversation_execution_attempt_cannot_lead_another_execution
BEFORE INSERT ON conversation_execution_links
WHEN NEW.relation = 'lead'
 AND EXISTS (
     SELECT 1
     FROM conversation_execution_links attempt_link
     JOIN agent_executions current_execution
       ON current_execution.id = attempt_link.execution_id
     JOIN agent_executions incoming_execution
       ON incoming_execution.id = NEW.execution_id
     WHERE attempt_link.conversation_id = NEW.conversation_id
       AND attempt_link.relation = 'attempt'
       AND current_execution.user_id = incoming_execution.user_id
 )
BEGIN
    SELECT RAISE(ABORT, 'Attempt Conversation permanently belongs to its Agent Execution');
END;

-- trigger: conversation_execution_authority_insert_guard
CREATE TRIGGER conversation_execution_authority_insert_guard
BEFORE INSERT ON conversations
WHEN NEW.user_id IS NOT (
    SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
)
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

-- trigger: conversation_execution_authority_update_guard
CREATE TRIGGER conversation_execution_authority_update_guard
BEFORE UPDATE OF user_id, type, delegation_policy, execution_model_pool,
                 execution_template_id, channel_chat_id, preset_id,
                 preset_revision, preset_snapshot
ON conversations
WHEN NEW.user_id IS NOT (
    SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
)
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

-- trigger: conversation_execution_link_delete_guard
CREATE TRIGGER conversation_execution_link_delete_guard
BEFORE DELETE ON conversation_execution_links
WHEN EXISTS (
        SELECT 1 FROM agent_executions execution
        JOIN users owner ON owner.id = execution.user_id
        WHERE execution.id = OLD.execution_id
     )
 AND EXISTS (SELECT 1 FROM conversations WHERE id = OLD.conversation_id)
BEGIN
    SELECT RAISE(ABORT, 'conversation execution links cannot be deleted directly');
END;

-- trigger: conversation_execution_link_identity_immutable
CREATE TRIGGER conversation_execution_link_identity_immutable
BEFORE UPDATE ON conversation_execution_links
WHEN NEW.id IS NOT OLD.id
  OR NEW.conversation_id IS NOT OLD.conversation_id
  OR NEW.execution_id IS NOT OLD.execution_id
  OR NEW.relation IS NOT OLD.relation
  OR NEW.step_id IS NOT OLD.step_id
  OR NEW.attempt_id IS NOT OLD.attempt_id
  OR NEW.created_at IS NOT OLD.created_at
  OR NEW.active > OLD.active
  OR (OLD.cleanup_completed_at IS NOT NULL
      AND NEW.cleanup_completed_at IS NOT OLD.cleanup_completed_at)
  OR NEW.updated_at < OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'conversation execution link identity is immutable');
END;

-- trigger: conversation_execution_link_owner_guard
CREATE TRIGGER conversation_execution_link_owner_guard
BEFORE INSERT ON conversation_execution_links
WHEN (SELECT COUNT(*)
      FROM agent_executions execution
      JOIN conversations conversation ON conversation.id = NEW.conversation_id
      WHERE execution.id = NEW.execution_id
        AND execution.user_id = conversation.user_id) <> 1
BEGIN
    SELECT RAISE(ABORT, 'conversation execution link owner mismatch');
END;

-- trigger: conversation_execution_model_authority_insert_guard
CREATE TRIGGER conversation_model_contract_insert_guard
BEFORE INSERT ON conversations
WHEN NEW.model IS NOT NULL
 AND (
      NOT json_valid(NEW.model)
      OR json_type(NEW.model) <> 'object'
      OR EXISTS (
          SELECT 1 FROM json_each(NEW.model)
          WHERE key NOT IN ('provider_id', 'model', 'use_model')
      )
      OR typeof(json_extract(NEW.model, '$.provider_id')) <> 'text'
      OR json_extract(NEW.model, '$.provider_id') = ''
      OR trim(json_extract(NEW.model, '$.provider_id'))
           <> json_extract(NEW.model, '$.provider_id')
      OR typeof(json_extract(NEW.model, '$.model')) <> 'text'
      OR json_extract(NEW.model, '$.model') = ''
      OR trim(json_extract(NEW.model, '$.model'))
           <> json_extract(NEW.model, '$.model')
      OR (
          json_type(NEW.model, '$.use_model') IS NOT NULL
          AND (
              typeof(json_extract(NEW.model, '$.use_model')) <> 'text'
              OR json_extract(NEW.model, '$.use_model') = ''
              OR trim(json_extract(NEW.model, '$.use_model'))
                   <> json_extract(NEW.model, '$.use_model')
          )
      )
 )
BEGIN
    SELECT RAISE(ABORT, 'Conversation model must use the canonical ProviderWithModel contract');
END;

CREATE TRIGGER conversation_model_contract_update_guard
BEFORE UPDATE OF model ON conversations
WHEN NEW.model IS NOT NULL
 AND (
      NOT json_valid(NEW.model)
      OR json_type(NEW.model) <> 'object'
      OR EXISTS (
          SELECT 1 FROM json_each(NEW.model)
          WHERE key NOT IN ('provider_id', 'model', 'use_model')
      )
      OR typeof(json_extract(NEW.model, '$.provider_id')) <> 'text'
      OR json_extract(NEW.model, '$.provider_id') = ''
      OR trim(json_extract(NEW.model, '$.provider_id'))
           <> json_extract(NEW.model, '$.provider_id')
      OR typeof(json_extract(NEW.model, '$.model')) <> 'text'
      OR json_extract(NEW.model, '$.model') = ''
      OR trim(json_extract(NEW.model, '$.model'))
           <> json_extract(NEW.model, '$.model')
      OR (
          json_type(NEW.model, '$.use_model') IS NOT NULL
          AND (
              typeof(json_extract(NEW.model, '$.use_model')) <> 'text'
              OR json_extract(NEW.model, '$.use_model') = ''
              OR trim(json_extract(NEW.model, '$.use_model'))
                   <> json_extract(NEW.model, '$.use_model')
          )
      )
 )
BEGIN
    SELECT RAISE(ABORT, 'Conversation model must use the canonical ProviderWithModel contract');
END;

-- trigger: conversation_execution_model_authority_insert_guard
CREATE TRIGGER conversation_execution_model_authority_insert_guard
BEFORE INSERT ON conversations
WHEN NEW.execution_model_pool IS NOT NULL
 AND json_extract(NEW.execution_model_pool, '$.mode') IN ('single', 'range')
 AND (
      NEW.model IS NULL
      OR NOT json_valid(NEW.model)
      OR typeof(json_extract(NEW.model, '$.provider_id')) <> 'text'
      OR trim(json_extract(NEW.model, '$.provider_id')) = ''
      OR typeof(CASE
             WHEN trim(COALESCE(json_extract(NEW.model, '$.use_model'), '')) = ''
             THEN json_extract(NEW.model, '$.model')
             ELSE json_extract(NEW.model, '$.use_model')
         END) <> 'text'
      OR trim(CASE
             WHEN trim(COALESCE(json_extract(NEW.model, '$.use_model'), '')) = ''
             THEN json_extract(NEW.model, '$.model')
             ELSE json_extract(NEW.model, '$.use_model')
         END) = ''
      OR (
          json_extract(NEW.execution_model_pool, '$.mode') = 'single'
          AND (
              json_extract(NEW.execution_model_pool, '$.model.provider_id')
                  <> json_extract(NEW.model, '$.provider_id')
              OR json_extract(NEW.execution_model_pool, '$.model.model')
                  <> CASE
                      WHEN trim(COALESCE(json_extract(NEW.model, '$.use_model'), '')) = ''
                      THEN json_extract(NEW.model, '$.model')
                      ELSE json_extract(NEW.model, '$.use_model')
                  END
          )
      )
      OR (
          json_extract(NEW.execution_model_pool, '$.mode') = 'range'
          AND NOT EXISTS (
              SELECT 1
              FROM json_each(NEW.execution_model_pool, '$.models') AS allowed
              WHERE json_extract(allowed.value, '$.provider_id') = json_extract(NEW.model, '$.provider_id')
                AND json_extract(allowed.value, '$.model') = CASE
                        WHEN trim(COALESCE(json_extract(NEW.model, '$.use_model'), '')) = ''
                        THEN json_extract(NEW.model, '$.model')
                        ELSE json_extract(NEW.model, '$.use_model')
                    END
          )
      )
 )
BEGIN
    SELECT RAISE(ABORT, 'Conversation lead model must belong to execution model pool');
END;

-- trigger: conversation_execution_model_authority_update_guard
CREATE TRIGGER conversation_execution_model_authority_update_guard
BEFORE UPDATE OF model, execution_model_pool ON conversations
WHEN NEW.execution_model_pool IS NOT NULL
 AND json_extract(NEW.execution_model_pool, '$.mode') IN ('single', 'range')
 AND (
      NEW.model IS NULL
      OR NOT json_valid(NEW.model)
      OR typeof(json_extract(NEW.model, '$.provider_id')) <> 'text'
      OR trim(json_extract(NEW.model, '$.provider_id')) = ''
      OR typeof(CASE
             WHEN trim(COALESCE(json_extract(NEW.model, '$.use_model'), '')) = ''
             THEN json_extract(NEW.model, '$.model')
             ELSE json_extract(NEW.model, '$.use_model')
         END) <> 'text'
      OR trim(CASE
             WHEN trim(COALESCE(json_extract(NEW.model, '$.use_model'), '')) = ''
             THEN json_extract(NEW.model, '$.model')
             ELSE json_extract(NEW.model, '$.use_model')
         END) = ''
      OR (
          json_extract(NEW.execution_model_pool, '$.mode') = 'single'
          AND (
              json_extract(NEW.execution_model_pool, '$.model.provider_id')
                  <> json_extract(NEW.model, '$.provider_id')
              OR json_extract(NEW.execution_model_pool, '$.model.model')
                  <> CASE
                      WHEN trim(COALESCE(json_extract(NEW.model, '$.use_model'), '')) = ''
                      THEN json_extract(NEW.model, '$.model')
                      ELSE json_extract(NEW.model, '$.use_model')
                  END
          )
      )
      OR (
          json_extract(NEW.execution_model_pool, '$.mode') = 'range'
          AND NOT EXISTS (
              SELECT 1
              FROM json_each(NEW.execution_model_pool, '$.models') AS allowed
              WHERE json_extract(allowed.value, '$.provider_id') = json_extract(NEW.model, '$.provider_id')
                AND json_extract(allowed.value, '$.model') = CASE
                        WHEN trim(COALESCE(json_extract(NEW.model, '$.use_model'), '')) = ''
                        THEN json_extract(NEW.model, '$.model')
                        ELSE json_extract(NEW.model, '$.use_model')
                    END
          )
      )
 )
BEGIN
    SELECT RAISE(ABORT, 'Conversation lead model must belong to execution model pool');
END;

-- trigger: conversation_execution_model_pool_insert_guard
CREATE TRIGGER conversation_execution_model_pool_insert_guard
BEFORE INSERT ON conversations
WHEN NEW.execution_model_pool IS NOT NULL
 AND (
      (json_extract(NEW.execution_model_pool, '$.mode') = 'automatic'
       AND (SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 1)
      OR (json_extract(NEW.execution_model_pool, '$.mode') = 'single'
          AND ((SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 2
               OR (SELECT COUNT(*) FROM json_each(NEW.execution_model_pool, '$.model')) <> 2))
      OR (json_extract(NEW.execution_model_pool, '$.mode') = 'range'
          AND ((SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 2
               OR EXISTS (
          SELECT 1 FROM json_each(NEW.execution_model_pool, '$.models') AS model
          WHERE json_type(model.value) <> 'object'
             OR json_type(model.value, '$.provider_id') <> 'text'
             OR trim(json_extract(model.value, '$.provider_id')) = ''
             OR trim(json_extract(model.value, '$.provider_id'))
                    <> json_extract(model.value, '$.provider_id')
             OR json_type(model.value, '$.model') <> 'text'
             OR trim(json_extract(model.value, '$.model')) = ''
             OR trim(json_extract(model.value, '$.model'))
                    <> json_extract(model.value, '$.model')
             OR (SELECT COUNT(*) FROM json_each(model.value)) <> 2
               )
               OR (
          SELECT COUNT(*)
          FROM (
              SELECT json_extract(model.value, '$.provider_id'),
                     json_extract(model.value, '$.model')
              FROM json_each(NEW.execution_model_pool, '$.models') AS model
              GROUP BY 1, 2
          )
               ) <> json_array_length(NEW.execution_model_pool, '$.models')))
 )
BEGIN
    SELECT RAISE(ABORT, 'invalid conversation execution model pool');
END;

-- trigger: conversation_execution_model_pool_update_guard
CREATE TRIGGER conversation_execution_model_pool_update_guard
BEFORE UPDATE OF execution_model_pool ON conversations
WHEN NEW.execution_model_pool IS NOT NULL
 AND (
      (json_extract(NEW.execution_model_pool, '$.mode') = 'automatic'
       AND (SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 1)
      OR (json_extract(NEW.execution_model_pool, '$.mode') = 'single'
          AND ((SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 2
               OR (SELECT COUNT(*) FROM json_each(NEW.execution_model_pool, '$.model')) <> 2))
      OR (json_extract(NEW.execution_model_pool, '$.mode') = 'range'
          AND ((SELECT COUNT(*) FROM json_each(NEW.execution_model_pool)) <> 2
               OR EXISTS (
          SELECT 1 FROM json_each(NEW.execution_model_pool, '$.models') AS model
          WHERE json_type(model.value) <> 'object'
             OR json_type(model.value, '$.provider_id') <> 'text'
             OR trim(json_extract(model.value, '$.provider_id')) = ''
             OR trim(json_extract(model.value, '$.provider_id'))
                    <> json_extract(model.value, '$.provider_id')
             OR json_type(model.value, '$.model') <> 'text'
             OR trim(json_extract(model.value, '$.model')) = ''
             OR trim(json_extract(model.value, '$.model'))
                    <> json_extract(model.value, '$.model')
             OR (SELECT COUNT(*) FROM json_each(model.value)) <> 2
               )
               OR (
          SELECT COUNT(*)
          FROM (
              SELECT json_extract(model.value, '$.provider_id'),
                     json_extract(model.value, '$.model')
              FROM json_each(NEW.execution_model_pool, '$.models') AS model
              GROUP BY 1, 2
          )
               ) <> json_array_length(NEW.execution_model_pool, '$.models')))
 )
BEGIN
    SELECT RAISE(ABORT, 'invalid conversation execution model pool');
END;

-- trigger: conversation_execution_single_unfinished_lead_insert
CREATE TRIGGER conversation_execution_single_unfinished_lead_insert
BEFORE INSERT ON conversation_execution_links
WHEN NEW.relation = 'lead' AND NEW.active = 1
 AND EXISTS (
     SELECT 1 FROM agent_executions incoming
     WHERE incoming.id = NEW.execution_id
       AND incoming.status NOT IN ('completed', 'completed_with_failures', 'failed', 'cancelled')
 )
 AND EXISTS (
     SELECT 1
     FROM conversation_execution_links existing_link
     JOIN agent_executions existing_execution
       ON existing_execution.id = existing_link.execution_id
     WHERE existing_link.conversation_id = NEW.conversation_id
       AND existing_link.relation = 'lead' AND existing_link.active = 1
       AND existing_link.execution_id <> NEW.execution_id
       AND existing_execution.deleted_at IS NULL
       AND existing_execution.status NOT IN (
           'completed', 'completed_with_failures', 'failed', 'cancelled'
       )
 )
BEGIN
    SELECT RAISE(ABORT, 'conversation already has an unfinished Agent Execution');
END;

-- trigger: conversation_execution_template_owner_guard_insert
CREATE TRIGGER conversation_execution_template_owner_guard_insert
BEFORE INSERT ON conversations
WHEN NEW.execution_template_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1
     FROM agent_execution_templates template
     JOIN agent_execution_template_participants participant
       ON participant.template_id = template.id
     WHERE template.id = NEW.execution_template_id
       AND template.user_id = NEW.user_id
       AND participant.provider_id = json_extract(NEW.model, '$.provider_id')
       AND participant.model = CASE
           WHEN typeof(json_extract(NEW.model, '$.use_model')) = 'text'
            AND trim(json_extract(NEW.model, '$.use_model')) <> ''
           THEN json_extract(NEW.model, '$.use_model')
           ELSE json_extract(NEW.model, '$.model')
       END
 )
BEGIN
    SELECT RAISE(ABORT, 'Conversation execution template must be executable, owner-scoped, and contain the lead model');
END;

-- trigger: conversation_execution_template_owner_guard_update
CREATE TRIGGER conversation_execution_template_owner_guard_update
BEFORE UPDATE OF user_id, execution_template_id, model ON conversations
WHEN NEW.execution_template_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1
     FROM agent_execution_templates template
     JOIN agent_execution_template_participants participant
       ON participant.template_id = template.id
     WHERE template.id = NEW.execution_template_id
       AND template.user_id = NEW.user_id
       AND participant.provider_id = json_extract(NEW.model, '$.provider_id')
       AND participant.model = CASE
           WHEN typeof(json_extract(NEW.model, '$.use_model')) = 'text'
            AND trim(json_extract(NEW.model, '$.use_model')) <> ''
           THEN json_extract(NEW.model, '$.use_model')
           ELSE json_extract(NEW.model, '$.model')
       END
 )
BEGIN
    SELECT RAISE(ABORT, 'Conversation execution template must be executable, owner-scoped, and contain the lead model');
END;

-- trigger: conversation_owner_immutable
CREATE TRIGGER conversation_owner_immutable
BEFORE UPDATE OF user_id ON conversations
WHEN NEW.user_id IS NOT OLD.user_id
BEGIN
    SELECT RAISE(ABORT, 'conversation owner is immutable');
END;

-- trigger: conversation_provider_binding_insert_guard
CREATE TRIGGER conversation_provider_binding_insert_guard
BEFORE INSERT ON conversations
WHEN (
    NEW.model IS NOT NULL
    AND NOT EXISTS (
        SELECT 1 FROM providers provider
        WHERE provider.id = json_extract(NEW.model, '$.provider_id')
    )
) OR (
    NEW.execution_model_pool IS NOT NULL
    AND json_extract(NEW.execution_model_pool, '$.mode') = 'single'
    AND NOT EXISTS (
        SELECT 1 FROM providers provider
        WHERE provider.id = json_extract(NEW.execution_model_pool, '$.model.provider_id')
    )
) OR (
    NEW.execution_model_pool IS NOT NULL
    AND json_extract(NEW.execution_model_pool, '$.mode') = 'range'
    AND EXISTS (
        SELECT 1 FROM json_each(NEW.execution_model_pool, '$.models') model_ref
        WHERE NOT EXISTS (
            SELECT 1 FROM providers provider
            WHERE provider.id = json_extract(model_ref.value, '$.provider_id')
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'Conversation model authority references a missing provider');
END;

-- trigger: conversation_provider_binding_update_guard
CREATE TRIGGER conversation_provider_binding_update_guard
BEFORE UPDATE OF model, execution_model_pool ON conversations
WHEN (
    NEW.model IS NOT NULL
    AND NOT EXISTS (
        SELECT 1 FROM providers provider
        WHERE provider.id = json_extract(NEW.model, '$.provider_id')
    )
) OR (
    NEW.execution_model_pool IS NOT NULL
    AND json_extract(NEW.execution_model_pool, '$.mode') = 'single'
    AND NOT EXISTS (
        SELECT 1 FROM providers provider
        WHERE provider.id = json_extract(NEW.execution_model_pool, '$.model.provider_id')
    )
) OR (
    NEW.execution_model_pool IS NOT NULL
    AND json_extract(NEW.execution_model_pool, '$.mode') = 'range'
    AND EXISTS (
        SELECT 1 FROM json_each(NEW.execution_model_pool, '$.models') model_ref
        WHERE NOT EXISTS (
            SELECT 1 FROM providers provider
            WHERE provider.id = json_extract(model_ref.value, '$.provider_id')
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'Conversation model authority references a missing provider');
END;

-- trigger: cron_execution_authority_insert_guard
CREATE TRIGGER cron_execution_authority_insert_guard
BEFORE INSERT ON cron_jobs
WHEN NEW.user_id IS NOT (
    SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
)
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

-- trigger: cron_execution_authority_update_guard
CREATE TRIGGER cron_execution_authority_update_guard
BEFORE UPDATE OF user_id, enabled, execution_mode, conversation_id, agent_type,
                 agent_config, preset_id, preset_revision, preset_snapshot,
                 skill_content
ON cron_jobs
WHEN NEW.user_id IS NOT (
    SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
)
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

-- trigger: cron_job_conversation_owner_insert
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

-- trigger: cron_job_conversation_owner_update
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

-- trigger: cron_job_owner_immutable
CREATE TRIGGER cron_job_owner_immutable
BEFORE UPDATE OF user_id ON cron_jobs
WHEN NEW.user_id IS NOT OLD.user_id
BEGIN
    SELECT RAISE(ABORT, 'cron job owner is immutable');
END;

-- trigger: execution_conversation_owner_immutable
CREATE TRIGGER execution_conversation_owner_immutable
BEFORE UPDATE OF user_id ON conversations
WHEN NEW.user_id IS NOT OLD.user_id
 AND (
     EXISTS (SELECT 1 FROM conversation_creation_keys key
             WHERE key.conversation_id = OLD.id)
     OR EXISTS (SELECT 1 FROM conversation_delivery_receipts receipt
                WHERE receipt.conversation_id = OLD.id)
     OR EXISTS (SELECT 1 FROM conversation_execution_links link
                WHERE link.conversation_id = OLD.id)
 )
BEGIN
    SELECT RAISE(ABORT, 'execution conversation owner is immutable');
END;

-- trigger: idmm_backup_provider_insert_guard
CREATE TRIGGER idmm_backup_provider_insert_guard
BEFORE INSERT ON client_preferences
WHEN NEW.key = 'idmm_backup_provider_id'
 AND NOT EXISTS (SELECT 1 FROM providers WHERE id = NEW.value)
BEGIN
    SELECT RAISE(ABORT, 'IDMM backup references a missing provider');
END;

-- trigger: idmm_backup_provider_update_guard
CREATE TRIGGER idmm_backup_provider_update_guard
BEFORE UPDATE OF key, value ON client_preferences
WHEN NEW.key = 'idmm_backup_provider_id'
 AND NOT EXISTS (SELECT 1 FROM providers WHERE id = NEW.value)
BEGIN
    SELECT RAISE(ABORT, 'IDMM backup references a missing provider');
END;

-- trigger: idmm_intervention_audit_immutable
CREATE TRIGGER idmm_intervention_audit_immutable
BEFORE UPDATE ON idmm_interventions
BEGIN
    SELECT RAISE(ABORT, 'IDMM intervention audit rows are immutable');
END;

-- trigger: idmm_intervention_target_owner_insert_guard
CREATE TRIGGER idmm_intervention_target_owner_insert_guard
BEFORE INSERT ON idmm_interventions
BEGIN
    SELECT CASE
        WHEN NEW.target_kind = 'conversation'
         AND NOT EXISTS (
             SELECT 1
             FROM conversations AS conversation
             WHERE conversation.id = NEW.target_id
               AND conversation.user_id = NEW.user_id
         )
        THEN RAISE(ABORT, 'IDMM conversation target owner mismatch')
        WHEN NEW.target_kind = 'terminal'
         AND NOT EXISTS (
             SELECT 1
             FROM terminal_sessions AS terminal
             WHERE terminal.id = NEW.target_id
               AND terminal.user_id = NEW.user_id
         )
        THEN RAISE(ABORT, 'IDMM terminal target owner mismatch')
    END;
END;

-- trigger: knowledge_binding_conversation_owner_rewrite_guard
CREATE TRIGGER knowledge_binding_conversation_owner_rewrite_guard
BEFORE UPDATE OF user_id ON conversations
WHEN NEW.user_id IS NOT (
    SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
)
 AND EXISTS (
     SELECT 1 FROM knowledge_bindings binding
     WHERE binding.target_kind = 'conversation'
       AND binding.target_conv_id = OLD.id
 )
BEGIN
    SELECT RAISE(ABORT, 'knowledge-bound conversation must remain installation-owner owned');
END;

-- trigger: knowledge_binding_owner_insert_guard
CREATE TRIGGER knowledge_binding_owner_insert_guard
BEFORE INSERT ON knowledge_bindings
WHEN (NEW.target_kind = 'conversation' AND NOT EXISTS (
          SELECT 1 FROM conversations conversation
          WHERE conversation.id = NEW.target_conv_id
            AND conversation.user_id = (
                SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
            )
      ))
   OR (NEW.target_kind = 'terminal' AND NOT EXISTS (
          SELECT 1 FROM terminal_sessions terminal
          WHERE terminal.id = NEW.target_term_id
            AND terminal.user_id = (
                SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
            )
      ))
BEGIN
    SELECT RAISE(ABORT, 'knowledge binding target must belong to the installation owner');
END;

-- trigger: knowledge_binding_owner_update_guard
CREATE TRIGGER knowledge_binding_owner_update_guard
BEFORE UPDATE OF target_kind, target_conv_id, target_term_id ON knowledge_bindings
WHEN (NEW.target_kind = 'conversation' AND NOT EXISTS (
          SELECT 1 FROM conversations conversation
          WHERE conversation.id = NEW.target_conv_id
            AND conversation.user_id = (
                SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
            )
      ))
   OR (NEW.target_kind = 'terminal' AND NOT EXISTS (
          SELECT 1 FROM terminal_sessions terminal
          WHERE terminal.id = NEW.target_term_id
            AND terminal.user_id = (
                SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
            )
      ))
BEGIN
    SELECT RAISE(ABORT, 'knowledge binding target must belong to the installation owner');
END;

-- trigger: knowledge_binding_terminal_owner_rewrite_guard
CREATE TRIGGER knowledge_binding_terminal_owner_rewrite_guard
BEFORE UPDATE OF user_id ON terminal_sessions
WHEN NEW.user_id IS NOT (
    SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
)
 AND EXISTS (
     SELECT 1 FROM knowledge_bindings binding
     WHERE binding.target_kind = 'terminal'
       AND binding.target_term_id = OLD.id
 )
BEGIN
    SELECT RAISE(ABORT, 'knowledge-bound terminal must remain installation-owner owned');
END;

-- trigger: provider_hard_binding_delete_guard
CREATE TRIGGER provider_hard_binding_delete_guard
BEFORE DELETE ON providers
WHEN EXISTS (
    SELECT 1 FROM conversations conversation
    WHERE json_extract(conversation.model, '$.provider_id') = OLD.id
) OR EXISTS (
    SELECT 1 FROM agent_execution_template_participants participant
    WHERE participant.provider_id = OLD.id
) OR EXISTS (
    SELECT 1
    FROM agent_execution_participants participant
    JOIN agent_executions execution ON execution.id = participant.execution_id
    WHERE participant.provider_id = OLD.id
      AND participant.retired_in_revision IS NULL
      AND execution.deleted_at IS NULL
      AND execution.status <> 'cancelled'
) OR EXISTS (
    SELECT 1 FROM client_preferences preference
    WHERE preference.key = 'idmm_backup_provider_id'
      AND preference.value = OLD.id
)
BEGIN
    SELECT RAISE(ABORT, 'provider is still referenced by an executable Agent binding');
END;

-- trigger: provider_soft_reference_cleanup
CREATE TRIGGER provider_soft_reference_cleanup
AFTER DELETE ON providers
BEGIN
    UPDATE conversations
    SET execution_model_pool = CASE
        WHEN json_extract(conversations.execution_model_pool, '$.mode') = 'single'
            THEN NULL
        ELSE COALESCE(
            (
                SELECT CASE WHEN COUNT(*) = 0 THEN NULL ELSE
                    json_object(
                        'mode', 'range',
                        'models', json(json_group_array(json(item.value)))
                    )
                END
                FROM json_each(conversations.execution_model_pool, '$.models') item
                WHERE json_extract(item.value, '$.provider_id') <> OLD.id
                  AND EXISTS (
                      SELECT 1 FROM providers provider
                      WHERE provider.id = json_extract(item.value, '$.provider_id')
                  )
            ),
            NULL
        )
    END
    WHERE conversations.execution_model_pool IS NOT NULL
      AND (
          (json_extract(conversations.execution_model_pool, '$.mode') = 'single'
           AND json_extract(conversations.execution_model_pool, '$.model.provider_id') = OLD.id)
          OR
          (json_extract(conversations.execution_model_pool, '$.mode') = 'range'
           AND EXISTS (
               SELECT 1
               FROM json_each(conversations.execution_model_pool, '$.models') target
               WHERE json_extract(target.value, '$.provider_id') = OLD.id
           ))
      );

    -- The collaboration picker persists a canonical, ordered array of concrete
    -- provider/model value objects. Keep the preference itself when every
    -- candidate disappears: [] means "lead model only" at the product boundary.
    UPDATE client_preferences
    SET value = json(COALESCE(
            (
                SELECT json_group_array(json(retained.value))
                FROM (
                    SELECT candidate.value
                    FROM json_each(client_preferences.value) candidate
                    WHERE candidate.type = 'object'
                      AND (SELECT COUNT(*) FROM json_each(candidate.value)) = 2
                      AND json_type(candidate.value, '$.provider_id') = 'text'
                      AND trim(json_extract(candidate.value, '$.provider_id')) <> ''
                      AND trim(json_extract(candidate.value, '$.provider_id'))
                            = json_extract(candidate.value, '$.provider_id')
                      AND json_type(candidate.value, '$.model') = 'text'
                      AND trim(json_extract(candidate.value, '$.model')) <> ''
                      AND trim(json_extract(candidate.value, '$.model'))
                            = json_extract(candidate.value, '$.model')
                      AND EXISTS (
                          SELECT 1 FROM providers provider
                          WHERE provider.id = json_extract(candidate.value, '$.provider_id')
                      )
                    ORDER BY CAST(candidate.key AS INTEGER)
                ) retained
            ),
            '[]'
        )),
        updated_at = MAX(
            client_preferences.updated_at,
            CAST(strftime('%s', 'now') AS INTEGER) * 1000
        )
    WHERE client_preferences.key = 'nomi.collaborationModels'
      AND json_valid(client_preferences.value)
      AND json_type(client_preferences.value) = 'array'
      AND EXISTS (
          SELECT 1 FROM json_each(client_preferences.value) candidate
          WHERE candidate.type <> 'object'
             OR (SELECT COUNT(*) FROM json_each(candidate.value)) <> 2
             OR json_type(candidate.value, '$.provider_id') IS NOT 'text'
             OR trim(COALESCE(json_extract(candidate.value, '$.provider_id'), '')) = ''
             OR trim(json_extract(candidate.value, '$.provider_id'))
                   <> json_extract(candidate.value, '$.provider_id')
             OR json_type(candidate.value, '$.model') IS NOT 'text'
             OR trim(COALESCE(json_extract(candidate.value, '$.model'), '')) = ''
             OR trim(json_extract(candidate.value, '$.model'))
                   <> json_extract(candidate.value, '$.model')
             OR NOT EXISTS (
                 SELECT 1 FROM providers provider
                 WHERE provider.id = json_extract(candidate.value, '$.provider_id')
             )
      );

    UPDATE client_preferences
    SET value = json_set(
            client_preferences.value,
            '$.queue',
            json(COALESCE((
                SELECT json_group_array(json(candidate.value))
                FROM json_each(client_preferences.value, '$.queue') candidate
                WHERE json_extract(candidate.value, '$.provider_id') <> OLD.id
                  AND EXISTS (
                      SELECT 1 FROM providers provider
                      WHERE provider.id = json_extract(candidate.value, '$.provider_id')
                  )
            ), '[]'))
        ),
        updated_at = MAX(
            client_preferences.updated_at,
            CAST(strftime('%s', 'now') AS INTEGER) * 1000
        )
    WHERE client_preferences.key = 'agent.model_failover'
      AND json_valid(client_preferences.value)
      AND json_type(client_preferences.value, '$.queue') = 'array'
      AND EXISTS (
          SELECT 1 FROM json_each(client_preferences.value, '$.queue') candidate
          WHERE json_extract(candidate.value, '$.provider_id') = OLD.id
      );
END;

-- trigger: requirement_conversation_delete_release
CREATE TRIGGER requirement_conversation_delete_release
BEFORE DELETE ON conversations
BEGIN
    UPDATE requirements
    SET status = CASE WHEN status = 'in_progress' THEN 'pending' ELSE status END,
        owner_conversation_id = NULL,
        active_turn_started_at = NULL,
        lease_expires_at = NULL,
        updated_at = MAX(updated_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
    WHERE owner_conversation_id = OLD.id;
END;

-- trigger: requirement_conversation_owner_rewrite_guard
CREATE TRIGGER requirement_conversation_owner_rewrite_guard
BEFORE UPDATE OF user_id ON conversations
WHEN NEW.user_id IS NOT (
    SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
)
 AND EXISTS (
     SELECT 1 FROM requirements requirement
     WHERE requirement.owner_conversation_id = OLD.id
 )
BEGIN
    SELECT RAISE(ABORT, 'requirement-owning conversation must remain installation-owner owned');
END;

-- trigger: requirement_owner_insert_guard
CREATE TRIGGER requirement_owner_insert_guard
BEFORE INSERT ON requirements
WHEN (NEW.owner_conversation_id IS NOT NULL AND NEW.owner_terminal_id IS NOT NULL)
  OR (NEW.owner_conversation_id IS NOT NULL AND NOT EXISTS (
         SELECT 1 FROM conversations conversation
         WHERE conversation.id = NEW.owner_conversation_id
           AND conversation.user_id = (
               SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
           )
     ))
  OR (NEW.owner_terminal_id IS NOT NULL AND NOT EXISTS (
         SELECT 1 FROM terminal_sessions terminal
         WHERE terminal.id = NEW.owner_terminal_id
           AND terminal.user_id = (
               SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
           )
     ))
BEGIN
    SELECT RAISE(ABORT, 'requirement owner must be one typed installation-owner session');
END;

-- trigger: requirement_owner_update_guard
CREATE TRIGGER requirement_owner_update_guard
BEFORE UPDATE OF owner_conversation_id, owner_terminal_id ON requirements
WHEN (NEW.owner_conversation_id IS NOT NULL AND NEW.owner_terminal_id IS NOT NULL)
  OR (NEW.owner_conversation_id IS NOT NULL AND NOT EXISTS (
         SELECT 1 FROM conversations conversation
         WHERE conversation.id = NEW.owner_conversation_id
           AND conversation.user_id = (
               SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
           )
     ))
  OR (NEW.owner_terminal_id IS NOT NULL AND NOT EXISTS (
         SELECT 1 FROM terminal_sessions terminal
         WHERE terminal.id = NEW.owner_terminal_id
           AND terminal.user_id = (
               SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
           )
     ))
BEGIN
    SELECT RAISE(ABORT, 'requirement owner must be one typed installation-owner session');
END;

-- trigger: requirement_terminal_delete_release
CREATE TRIGGER requirement_terminal_delete_release
BEFORE DELETE ON terminal_sessions
BEGIN
    UPDATE requirements
    SET status = CASE WHEN status = 'in_progress' THEN 'pending' ELSE status END,
        owner_terminal_id = NULL,
        active_turn_started_at = NULL,
        lease_expires_at = NULL,
        updated_at = MAX(updated_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
    WHERE owner_terminal_id = OLD.id;
END;

-- trigger: requirement_terminal_owner_rewrite_guard
CREATE TRIGGER requirement_terminal_owner_rewrite_guard
BEFORE UPDATE OF user_id ON terminal_sessions
WHEN NEW.user_id IS NOT (
    SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
)
 AND EXISTS (
     SELECT 1 FROM requirements requirement
     WHERE requirement.owner_terminal_id = OLD.id
 )
BEGIN
    SELECT RAISE(ABORT, 'requirement-owning terminal must remain installation-owner owned');
END;
-- trigger: terminal_execution_authority_insert_guard
CREATE TRIGGER terminal_execution_authority_insert_guard
BEFORE INSERT ON terminal_sessions
WHEN NEW.user_id IS NOT (
    SELECT owner_user_id FROM installation_identity WHERE key = 'installation'
)
BEGIN
    SELECT RAISE(ABORT, 'terminal execution requires the installation owner');
END;

-- trigger: terminal_session_owner_immutable
CREATE TRIGGER terminal_session_owner_immutable
BEFORE UPDATE OF user_id ON terminal_sessions
WHEN NEW.user_id IS NOT OLD.user_id
BEGIN
    SELECT RAISE(ABORT, 'terminal session owner is immutable');
END;
