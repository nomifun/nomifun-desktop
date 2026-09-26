PRAGMA foreign_keys = ON;

CREATE TABLE schema_metadata (
    singleton_key TEXT PRIMARY KEY CHECK (singleton_key = 'canonical'),
    data_generation INTEGER NOT NULL CHECK (data_generation = 6),
    root_instance_id TEXT NOT NULL,
    migration_head INTEGER NOT NULL CHECK (migration_head >= 1),
    seed_manifest_digest TEXT NOT NULL CHECK (length(seed_manifest_digest) = 64),
    canonical_schema_manifest_digest TEXT NOT NULL
        CHECK (length(canonical_schema_manifest_digest) = 64),
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version >= 1)
) STRICT;

CREATE TABLE plugin_packages (
    package_id TEXT NOT NULL,
    package_version TEXT NOT NULL,
    manifest_json TEXT NOT NULL CHECK (json_valid(manifest_json)),
    manifest_digest TEXT NOT NULL CHECK (length(manifest_digest) = 64),
    display_json TEXT NOT NULL CHECK (json_valid(display_json)),
    PRIMARY KEY (package_id, package_version)
) STRICT;

CREATE TABLE plugin_mounts (
    mount_id TEXT PRIMARY KEY,
    package_id TEXT NOT NULL,
    package_version TEXT NOT NULL,
    source_json TEXT NOT NULL CHECK (json_valid(source_json)),
    desired_state TEXT NOT NULL CHECK (desired_state IN ('enabled', 'disabled')),
    effective_state TEXT NOT NULL
        CHECK (effective_state IN ('disabled', 'blocked', 'failed', 'active')),
    criticality TEXT NOT NULL CHECK (criticality IN ('required', 'optional')),
    UNIQUE (package_id, mount_id),
    FOREIGN KEY (package_id, package_version)
        REFERENCES plugin_packages (package_id, package_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE plugin_configs (
    package_id TEXT NOT NULL,
    mount_id TEXT NOT NULL,
    config_json TEXT NOT NULL CHECK (json_valid(config_json)),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    PRIMARY KEY (package_id, mount_id),
    FOREIGN KEY (package_id, mount_id)
        REFERENCES plugin_mounts (package_id, mount_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE plugin_states (
    package_id TEXT NOT NULL,
    mount_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    state_key TEXT NOT NULL,
    value_json TEXT CHECK (value_json IS NULL OR json_valid(value_json)),
    cas_revision INTEGER NOT NULL CHECK (cas_revision >= 1),
    state_format_version TEXT NOT NULL,
    writer_package_version TEXT NOT NULL,
    PRIMARY KEY (package_id, mount_id, scope_key, state_key),
    FOREIGN KEY (package_id, mount_id)
        REFERENCES plugin_mounts (package_id, mount_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE installation_role_bindings (
    role_id TEXT PRIMARY KEY CHECK (trim(role_id) <> ''),
    role_contract_ref_json TEXT NOT NULL CHECK (
        json_valid(role_contract_ref_json)
        AND json_type(role_contract_ref_json) = 'object'
        AND json_type(role_contract_ref_json, '$.key') = 'object'
        AND json_extract(role_contract_ref_json, '$.key.role_id') = role_id
        AND trim(json_extract(role_contract_ref_json, '$.key.contract_version')) <> ''
        AND length(json_extract(role_contract_ref_json, '$.contract_digest')) = 64
    ),
    provider_mount_id TEXT NOT NULL,
    binding_version INTEGER NOT NULL CHECK (binding_version >= 1),
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE capability_definitions (
    capability_id TEXT PRIMARY KEY,
    package_id TEXT NOT NULL,
    package_version TEXT NOT NULL,
    manifest_json TEXT NOT NULL CHECK (json_valid(manifest_json)),
    manifest_digest TEXT NOT NULL CHECK (length(manifest_digest) = 64),
    FOREIGN KEY (package_id, package_version)
        REFERENCES plugin_packages (package_id, package_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE capability_catalog_entries (
    capability_id TEXT NOT NULL CHECK (trim(capability_id) <> ''),
    contribution_id TEXT NOT NULL CHECK (trim(contribution_id) <> ''),
    entry_json TEXT NOT NULL CHECK (json_valid(entry_json)),
    entry_digest TEXT NOT NULL CHECK (length(entry_digest) = 64),
    PRIMARY KEY (capability_id, contribution_id)
) STRICT;

CREATE TABLE skill_instructions (
    skill_id TEXT NOT NULL,
    skill_version TEXT NOT NULL,
    package_id TEXT NOT NULL,
    package_version TEXT NOT NULL,
    definition_json TEXT NOT NULL CHECK (json_valid(definition_json)),
    definition_digest TEXT NOT NULL CHECK (length(definition_digest) = 64),
    PRIMARY KEY (skill_id, skill_version),
    FOREIGN KEY (package_id, package_version)
        REFERENCES plugin_packages (package_id, package_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE mcp_servers (
    server_id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    connection_config_ref TEXT NOT NULL,
    catalog_revision INTEGER NOT NULL CHECK (catalog_revision >= 0)
) STRICT;

CREATE TABLE mcp_tool_materializations (
    server_id TEXT NOT NULL,
    canonical_tool_key TEXT NOT NULL,
    schema_hash TEXT NOT NULL CHECK (length(schema_hash) = 64),
    capability_id TEXT NOT NULL,
    materialization_revision INTEGER NOT NULL CHECK (materialization_revision >= 1),
    package_id TEXT NOT NULL,
    package_version TEXT NOT NULL,
    PRIMARY KEY (server_id, canonical_tool_key),
    FOREIGN KEY (server_id) REFERENCES mcp_servers (server_id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    FOREIGN KEY (capability_id)
        REFERENCES capability_definitions (capability_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (package_id, package_version)
        REFERENCES plugin_packages (package_id, package_version)
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

CREATE INDEX idx_agent_presets_owner_active
    ON agent_presets(json_extract(owner_ref_json, '$.user_id'), preset_id)
    WHERE retired_at_ms IS NULL;

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

CREATE INDEX idx_agent_bindings_preset
    ON agent_bindings(
        json_extract(agent_binding_json, '$.preset_revision_ref.preset_id')
    );

CREATE TABLE remote_bindings (
    remote_binding_id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    agent_binding_json TEXT NOT NULL CHECK (json_valid(agent_binding_json))
) STRICT;

CREATE TABLE installation_auth (
    singleton_key TEXT PRIMARY KEY CHECK (singleton_key = 'installation'),
    owner_user_id TEXT NOT NULL,
    current_verifier_hash TEXT,
    auth_revision INTEGER NOT NULL CHECK (auth_revision >= 1),
    status TEXT NOT NULL CHECK (status IN ('active', 'revoked')),
    updated_at INTEGER NOT NULL,
    CHECK (
        (status = 'active' AND current_verifier_hash IS NOT NULL) OR
        (status = 'revoked' AND current_verifier_hash IS NULL)
    )
) STRICT;

-- Provider configuration is part of the canonical Agent Store root.  A clean start does
-- not import legacy rows, but the new host must have one canonical place for
-- user-entered provider routes and encrypted credential material.
CREATE TABLE providers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL UNIQUE,
    platform TEXT NOT NULL,
    name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    auth_scheme TEXT NOT NULL CHECK (trim(auth_scheme) <> ''),
    credentials_encrypted TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    bedrock_config TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0 CHECK (sort_order >= 0),
    config_revision INTEGER NOT NULL DEFAULT 0 CHECK (config_revision >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (
        length(provider_id) = 36 AND lower(provider_id) = provider_id
        AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    CHECK (bedrock_config IS NULL OR json_valid(bedrock_config))
) STRICT;

CREATE TABLE provider_models (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL,
    model TEXT NOT NULL,
    display_name TEXT,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    sort_order INTEGER NOT NULL DEFAULT 0 CHECK (sort_order >= 0),
    description TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (provider_id, model),
    FOREIGN KEY (provider_id) REFERENCES providers (provider_id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    CHECK (
        length(provider_id) = 36 AND lower(provider_id) = provider_id
        AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    CHECK (trim(model) <> '')
) STRICT;

CREATE TABLE provider_connections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_id TEXT NOT NULL UNIQUE,
    provider_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (trim(role) <> '' AND role <> 'default'),
    label TEXT,
    base_url TEXT NOT NULL,
    auth_scheme TEXT NOT NULL CHECK (trim(auth_scheme) <> ''),
    credentials_encrypted TEXT NOT NULL,
    extra TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (provider_id, role),
    FOREIGN KEY (provider_id) REFERENCES providers (provider_id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    CHECK (
        length(connection_id) = 36 AND lower(connection_id) = connection_id
        AND connection_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(connection_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    CHECK (
        length(provider_id) = 36 AND lower(provider_id) = provider_id
        AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    CHECK (json_valid(extra) AND json_type(extra) = 'object')
) STRICT;

CREATE TABLE provider_model_capabilities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL,
    model TEXT NOT NULL,
    task TEXT NOT NULL,
    traits TEXT NOT NULL DEFAULT '[]',
    protocol TEXT NOT NULL CHECK (trim(protocol) <> ''),
    connection_role TEXT NOT NULL DEFAULT 'default'
        CHECK (trim(connection_role) <> ''),
    base_url_override TEXT,
    endpoint TEXT,
    poll_endpoint TEXT,
    content_endpoint TEXT,
    realtime_endpoint TEXT,
    allow_cross_origin_credentials INTEGER NOT NULL DEFAULT 0
        CHECK (allow_cross_origin_credentials IN (0, 1)),
    provider_params TEXT NOT NULL DEFAULT '{}',
    context_limit INTEGER CHECK (context_limit IS NULL OR context_limit > 0),
    output_limit INTEGER CHECK (output_limit IS NULL OR output_limit > 0),
    health TEXT,
    health_checked_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (provider_id, model, task),
    FOREIGN KEY (provider_id, model)
        REFERENCES provider_models (provider_id, model)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    CHECK (json_valid(traits) AND json_type(traits) = 'array'),
    CHECK (json_valid(provider_params) AND json_type(provider_params) = 'object'),
    CHECK (health IS NULL OR json_valid(health)),
    CHECK (
        task IN (
            'chat', 'realtime_conversation', 'image_generation', 'image_edit',
            'video_generation', 'speech_synthesis', 'speech_recognition',
            'embedding', 'rerank'
        )
    ),
    CHECK (
        length(provider_id) = 36 AND lower(provider_id) = provider_id
        AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
) STRICT;

CREATE TABLE client_preferences (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE CHECK (trim(key) <> ''),
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE system_settings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    singleton_key TEXT NOT NULL UNIQUE CHECK (singleton_key = 'system'),
    language TEXT NOT NULL DEFAULT 'en-US',
    notification_enabled INTEGER NOT NULL DEFAULT 1
        CHECK (notification_enabled IN (0, 1)),
    cron_notification_enabled INTEGER NOT NULL DEFAULT 0
        CHECK (cron_notification_enabled IN (0, 1)),
    command_queue_enabled INTEGER NOT NULL DEFAULT 0
        CHECK (command_queue_enabled IN (0, 1)),
    save_upload_to_workspace INTEGER NOT NULL DEFAULT 0
        CHECK (save_upload_to_workspace IN (0, 1)),
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE agent_runtime_snapshots (
    snapshot_id TEXT PRIMARY KEY,
    snapshot_digest TEXT NOT NULL UNIQUE CHECK (length(snapshot_digest) = 64),
    content_json TEXT NOT NULL CHECK (json_valid(content_json)),
    envelope_json TEXT NOT NULL CHECK (json_valid(envelope_json))
) STRICT;

CREATE INDEX idx_agent_runtime_snapshots_revision
    ON agent_runtime_snapshots(
        json_extract(content_json, '$.preset_revision_ref.preset_id'),
        json_extract(content_json, '$.preset_revision_ref.revision'),
        json_extract(content_json, '$.preset_revision_ref.revision_digest')
    );

CREATE TABLE agent_sessions (
    agent_session_id TEXT PRIMARY KEY,
    owner_ref_json TEXT NOT NULL CHECK (json_valid(owner_ref_json)),
    state TEXT NOT NULL CHECK (state IN ('live', 'deleting', 'deleted')),
    title TEXT,
    archived INTEGER CHECK (archived IN (0, 1)),
    pinned INTEGER CHECK (pinned IN (0, 1)),
    agent_binding_json TEXT CHECK (
        agent_binding_json IS NULL OR json_valid(agent_binding_json)
    ),
    remote_binding_id TEXT,
    remote_binding_version INTEGER,
    parent_agent_session_id TEXT,
    fork_base_payload_id TEXT,
    next_seq INTEGER,
    created_at INTEGER,
    deleted_at INTEGER,
    reasoning_effort TEXT CHECK (
        reasoning_effort IS NULL OR reasoning_effort IN ('low', 'medium', 'high')
    ),
    reasoning_effort_v2 TEXT CHECK (
        reasoning_effort_v2 IS NULL OR
        reasoning_effort_v2 IN ('low', 'medium', 'high', 'xhigh', 'max', 'ultra')
    ),
    CHECK (
        (
            state IN ('live', 'deleting') AND
            agent_binding_json IS NOT NULL AND
            archived IS NOT NULL AND
            pinned IS NOT NULL AND
            next_seq IS NOT NULL AND next_seq >= 1 AND
            created_at IS NOT NULL AND
            deleted_at IS NULL
        ) OR (
            state = 'deleted' AND
            title IS NULL AND archived IS NULL AND pinned IS NULL AND
            agent_binding_json IS NULL AND remote_binding_id IS NULL AND
            remote_binding_version IS NULL AND parent_agent_session_id IS NULL AND
            fork_base_payload_id IS NULL AND reasoning_effort IS NULL AND
            reasoning_effort_v2 IS NULL AND next_seq IS NULL AND
            created_at IS NULL AND deleted_at IS NOT NULL
        )
    ),
    -- A live/deleting Session owns its Remote provenance until the session
    -- deletion transaction clears the reference before creating the tombstone.
    -- Never silently erase that provenance when a RemoteBinding is deleted.
    -- RemoteBinding provenance is an immutable Session fact. It is
    -- intentionally not a foreign key: deleting a Binding prevents new
    -- opens, but must not rewrite or invalidate an existing Session.
    FOREIGN KEY (parent_agent_session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

-- Non-private append-only audit facts for an explicit installation-owner
-- override of a deletion quarantine. Session transcripts/events are still
-- purged completely; this ledger retains only bounded identity and digests.
CREATE TABLE agent_deletion_audits (
    audit_id TEXT PRIMARY KEY CHECK (trim(audit_id) <> ''),
    agent_session_id TEXT NOT NULL,
    owner_ref_json TEXT NOT NULL CHECK (json_valid(owner_ref_json)),
    target_kind TEXT NOT NULL CHECK (
        target_kind IN ('effect', 'resource_cleanup')
    ),
    target_id TEXT NOT NULL CHECK (trim(target_id) <> ''),
    authority TEXT NOT NULL CHECK (
        authority = 'installation_owner_manual_override'
    ),
    risk_acknowledged INTEGER NOT NULL CHECK (risk_acknowledged = 1),
    reason_digest TEXT NOT NULL CHECK (length(reason_digest) = 64),
    recorded_at INTEGER NOT NULL CHECK (recorded_at >= 0),
    UNIQUE (agent_session_id, target_kind, target_id, reason_digest),
    FOREIGN KEY (agent_session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE agent_turns (
    session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL CHECK (trim(turn_id) <> ''),
    operation_id TEXT NOT NULL CHECK (trim(operation_id) <> ''),
    idempotency_key TEXT NOT NULL CHECK (trim(idempotency_key) <> ''),
    source_message_id TEXT,
    admission_json TEXT CHECK (admission_json IS NULL OR json_valid(admission_json)),
    state TEXT NOT NULL CHECK (
        state IN ('accepted', 'running', 'completed', 'failed', 'cancelled', 'interrupted')
    ),
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    error_json TEXT CHECK (error_json IS NULL OR json_valid(error_json)),
    started_event_id TEXT,
    terminal_event_id TEXT,
    accepted_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    native_checkpoint_json TEXT CHECK (native_checkpoint_json IS NULL OR json_valid(native_checkpoint_json)),
    native_checkpoint_digest TEXT CHECK (native_checkpoint_digest IS NULL OR length(native_checkpoint_digest) = 64),
    native_checkpoint_revision INTEGER NOT NULL DEFAULT 0 CHECK (native_checkpoint_revision >= 0),
    native_checkpoint_seq INTEGER CHECK (native_checkpoint_seq IS NULL OR native_checkpoint_seq >= 0),
    execution_fence INTEGER NOT NULL DEFAULT 0 CHECK (execution_fence >= 0),
    execution_owner TEXT,
    execution_generation INTEGER NOT NULL DEFAULT 0 CHECK (execution_generation >= 0),
    execution_lease_until INTEGER NOT NULL DEFAULT 0 CHECK (execution_lease_until >= 0),
    native_pause_revision INTEGER NOT NULL DEFAULT 0 CHECK (native_pause_revision >= 0),
    native_pause_json TEXT CHECK (native_pause_json IS NULL OR json_valid(native_pause_json)),
    native_pause_requested_json TEXT CHECK (native_pause_requested_json IS NULL OR json_valid(native_pause_requested_json)),
    native_budget_json TEXT CHECK (native_budget_json IS NULL OR json_valid(native_budget_json)),
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
        (state IN ('accepted', 'running') AND terminal_event_id IS NULL AND finished_at IS NULL) OR
        (state IN ('completed', 'failed', 'cancelled', 'interrupted')
            AND terminal_event_id IS NOT NULL AND finished_at IS NOT NULL)
    )
) STRICT;

CREATE TABLE agent_session_resources (
    binding_id TEXT NOT NULL CHECK (trim(binding_id) <> ''),
    session_id TEXT NOT NULL,
    resource_kind TEXT NOT NULL CHECK (trim(resource_kind) <> ''),
    resource_id TEXT NOT NULL CHECK (trim(resource_id) <> ''),
    owner_id TEXT NOT NULL CHECK (trim(owner_id) <> ''),
    operations_json TEXT NOT NULL CHECK (
        json_valid(operations_json) AND json_type(operations_json) = 'array'
    ),
    connection_config_ref TEXT,
    typed_parameters_json TEXT NOT NULL CHECK (
        json_valid(typed_parameters_json) AND json_type(typed_parameters_json) = 'object'
    ),
    binding_digest TEXT NOT NULL CHECK (length(binding_digest) = 64),
    PRIMARY KEY (session_id, binding_id),
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
        (storage_kind = 'inline' AND body IS NOT NULL AND object_ref IS NULL) OR
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
    runtime_producer_seq INTEGER CHECK (
        runtime_producer_seq IS NULL OR runtime_producer_seq >= 1
    ),
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
    strategy TEXT NOT NULL CHECK (
        strategy IN ('managed_effect', 'external_uncertain_effect')
    ),
    state TEXT NOT NULL CHECK (
        state IN ('pending', 'returned', 'rejected', 'cancelled', 'unknown')
    ),
    bounded_observation_json TEXT CHECK (
        bounded_observation_json IS NULL OR json_valid(bounded_observation_json)
    ),
    started_event_id TEXT NOT NULL,
    terminal_event_id TEXT,
    created_at INTEGER NOT NULL,
    settled_at INTEGER,
    FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    FOREIGN KEY (session_id, turn_id) REFERENCES agent_turns (session_id, turn_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (session_id, resource_binding_id)
        REFERENCES agent_session_resources (session_id, binding_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (started_event_id) REFERENCES agent_events (event_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (terminal_event_id) REFERENCES agent_events (event_id)
        ON UPDATE RESTRICT ON DELETE SET NULL,
    CHECK (
        (state = 'pending' AND terminal_event_id IS NULL AND settled_at IS NULL) OR
        (state <> 'pending' AND terminal_event_id IS NOT NULL AND settled_at IS NOT NULL)
    )
) STRICT;

CREATE TABLE agent_session_heads (
    session_id TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    active_turn_id TEXT,
    active_set_generation INTEGER NOT NULL CHECK (active_set_generation >= 0),
    runtime_checkpoint_locator TEXT,
    runtime_checkpoint_digest TEXT CHECK (
        runtime_checkpoint_digest IS NULL OR length(runtime_checkpoint_digest) = 64
    ),
    runtime_bound_event_id TEXT,
    runtime_protocol_version TEXT,
    snapshot_digest TEXT CHECK (
        snapshot_digest IS NULL OR length(snapshot_digest) = 64
    ),
    checkpoint_through_seq INTEGER CHECK (
        checkpoint_through_seq IS NULL OR checkpoint_through_seq >= 0
    ),
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

CREATE INDEX idx_plugin_mounts_package
    ON plugin_mounts (package_id, package_version);
CREATE INDEX idx_capability_definitions_package
    ON capability_definitions (package_id, package_version);
CREATE INDEX idx_providers_platform
    ON providers (platform, sort_order, created_at, id);
CREATE INDEX idx_provider_models_provider
    ON provider_models (provider_id, sort_order, id);
CREATE INDEX idx_provider_model_capabilities_task
    ON provider_model_capabilities (task, provider_id, model);
CREATE INDEX idx_agent_sessions_owner_state
    ON agent_sessions (owner_ref_json, state);
CREATE INDEX idx_agent_deletion_audits_session_time
    ON agent_deletion_audits (agent_session_id, recorded_at, audit_id);
CREATE INDEX idx_agent_turns_session_state
    ON agent_turns (session_id, state, accepted_at);
CREATE INDEX idx_agent_session_resources_session_kind
    ON agent_session_resources (session_id, resource_kind, binding_id);
CREATE INDEX idx_agent_events_correlation
    ON agent_events (session_id, correlation_id, seq);
CREATE INDEX idx_agent_payloads_session
    ON agent_payloads (session_id);
CREATE INDEX idx_agent_messages_sequence
    ON agent_messages (session_id, first_seq, last_seq);
CREATE INDEX idx_agent_effects_session_turn
    ON agent_effects (session_id, turn_id, created_at);
CREATE UNIQUE INDEX idx_agent_effects_resource_unsettled
    ON agent_effects (owner_domain, resource_key)
    WHERE state IN ('pending', 'unknown') AND resource_key IS NOT NULL;
