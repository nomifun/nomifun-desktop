PRAGMA foreign_keys = ON;

CREATE TABLE schema_metadata (
    singleton_key TEXT PRIMARY KEY CHECK (singleton_key = 'canonical'),
    data_generation INTEGER NOT NULL CHECK (data_generation = 4),
    root_instance_id TEXT NOT NULL,
    migration_head INTEGER NOT NULL CHECK (migration_head >= 1),
    seed_manifest_digest TEXT NOT NULL CHECK (length(seed_manifest_digest) = 64),
    canonical_schema_manifest_digest TEXT NOT NULL
        CHECK (length(canonical_schema_manifest_digest) = 64),
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version >= 1)
) STRICT;

CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version >= 1),
    name TEXT NOT NULL UNIQUE,
    checksum TEXT NOT NULL CHECK (length(checksum) = 64),
    applied_at INTEGER NOT NULL
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

CREATE TABLE capability_definitions (
    capability_id TEXT NOT NULL,
    capability_version TEXT NOT NULL,
    package_id TEXT NOT NULL,
    package_version TEXT NOT NULL,
    manifest_json TEXT NOT NULL CHECK (json_valid(manifest_json)),
    manifest_digest TEXT NOT NULL CHECK (length(manifest_digest) = 64),
    PRIMARY KEY (capability_id, capability_version),
    FOREIGN KEY (package_id, package_version)
        REFERENCES plugin_packages (package_id, package_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE capability_packs (
    pack_id TEXT NOT NULL,
    pack_version TEXT NOT NULL,
    package_id TEXT NOT NULL,
    package_version TEXT NOT NULL,
    manifest_json TEXT NOT NULL CHECK (json_valid(manifest_json)),
    manifest_digest TEXT NOT NULL CHECK (length(manifest_digest) = 64),
    PRIMARY KEY (pack_id, pack_version),
    FOREIGN KEY (package_id, package_version)
        REFERENCES plugin_packages (package_id, package_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE capability_pack_items (
    pack_id TEXT NOT NULL,
    pack_version TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    capability_id TEXT NOT NULL,
    capability_version TEXT NOT NULL,
    PRIMARY KEY (pack_id, pack_version, ordinal),
    UNIQUE (pack_id, pack_version, capability_id),
    FOREIGN KEY (pack_id, pack_version)
        REFERENCES capability_packs (pack_id, pack_version)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    FOREIGN KEY (capability_id, capability_version)
        REFERENCES capability_definitions (capability_id, capability_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
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
    capability_version TEXT NOT NULL,
    materialization_revision INTEGER NOT NULL CHECK (materialization_revision >= 1),
    package_id TEXT NOT NULL,
    package_version TEXT NOT NULL,
    PRIMARY KEY (server_id, canonical_tool_key),
    FOREIGN KEY (server_id) REFERENCES mcp_servers (server_id)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    FOREIGN KEY (capability_id, capability_version)
        REFERENCES capability_definitions (capability_id, capability_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (package_id, package_version)
        REFERENCES plugin_packages (package_id, package_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE agent_preset_templates (
    template_key TEXT PRIMARY KEY,
    source_package_id TEXT NOT NULL,
    source_package_version TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('official', 'package')),
    template_json TEXT NOT NULL CHECK (json_valid(template_json)),
    template_digest TEXT NOT NULL CHECK (length(template_digest) = 64),
    FOREIGN KEY (source_package_id, source_package_version)
        REFERENCES plugin_packages (package_id, package_version)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE agent_presets (
    preset_id TEXT PRIMARY KEY,
    owner_ref_json TEXT NOT NULL CHECK (json_valid(owner_ref_json)),
    source_json TEXT NOT NULL CHECK (json_valid(source_json)),
    display_json TEXT NOT NULL CHECK (json_valid(display_json)),
    current_stable_revision INTEGER,
    created_at INTEGER NOT NULL,
    CHECK (current_stable_revision IS NULL OR current_stable_revision >= 1)
) STRICT;

CREATE TABLE agent_preset_revisions (
    revision_id TEXT PRIMARY KEY,
    preset_id TEXT NOT NULL,
    revision_no INTEGER NOT NULL CHECK (revision_no >= 1),
    schema_version TEXT NOT NULL,
    editor_document_json TEXT NOT NULL CHECK (json_valid(editor_document_json)),
    revision_digest TEXT NOT NULL CHECK (length(revision_digest) = 64),
    created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    reason TEXT NOT NULL,
    UNIQUE (preset_id, revision_no),
    UNIQUE (preset_id, revision_digest),
    FOREIGN KEY (preset_id) REFERENCES agent_presets (preset_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE agent_preset_model_routes (
    revision_id TEXT NOT NULL,
    model_task TEXT NOT NULL,
    route_json TEXT NOT NULL CHECK (json_valid(route_json)),
    PRIMARY KEY (revision_id, model_task),
    FOREIGN KEY (revision_id) REFERENCES agent_preset_revisions (revision_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE preset_initial_capabilities (
    revision_id TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    capability_version TEXT NOT NULL,
    selection_json TEXT NOT NULL CHECK (json_valid(selection_json)),
    PRIMARY KEY (revision_id, capability_id),
    FOREIGN KEY (revision_id) REFERENCES agent_preset_revisions (revision_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE preset_on_demand_capabilities (
    revision_id TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    capability_version TEXT NOT NULL,
    selection_json TEXT NOT NULL CHECK (json_valid(selection_json)),
    PRIMARY KEY (revision_id, capability_id),
    FOREIGN KEY (revision_id) REFERENCES agent_preset_revisions (revision_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TRIGGER preset_initial_capability_disjoint_insert
BEFORE INSERT ON preset_initial_capabilities
WHEN EXISTS (
    SELECT 1 FROM preset_on_demand_capabilities
    WHERE revision_id = NEW.revision_id AND capability_id = NEW.capability_id
)
BEGIN
    SELECT RAISE(ABORT, 'capability already belongs to on-demand set');
END;

CREATE TRIGGER preset_on_demand_capability_disjoint_insert
BEFORE INSERT ON preset_on_demand_capabilities
WHEN EXISTS (
    SELECT 1 FROM preset_initial_capabilities
    WHERE revision_id = NEW.revision_id AND capability_id = NEW.capability_id
)
BEGIN
    SELECT RAISE(ABORT, 'capability already belongs to initial set');
END;

CREATE TABLE preset_skill_bindings (
    revision_id TEXT NOT NULL,
    skill_id TEXT NOT NULL,
    skill_version TEXT NOT NULL,
    PRIMARY KEY (revision_id, skill_id),
    FOREIGN KEY (revision_id) REFERENCES agent_preset_revisions (revision_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE preset_resource_bindings (
    revision_id TEXT NOT NULL,
    resource_binding_id TEXT NOT NULL,
    binding_json TEXT NOT NULL CHECK (json_valid(binding_json)),
    PRIMARY KEY (revision_id, resource_binding_id),
    FOREIGN KEY (revision_id) REFERENCES agent_preset_revisions (revision_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE agent_bindings (
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    agent_binding_json TEXT NOT NULL CHECK (json_valid(agent_binding_json)),
    PRIMARY KEY (target_kind, target_id)
) STRICT;

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

-- Provider configuration is part of the Fresh-v4 root.  A clean start does
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
    preset_id TEXT NOT NULL,
    revision_no INTEGER NOT NULL,
    revision_digest TEXT NOT NULL CHECK (length(revision_digest) = 64),
    content_json TEXT NOT NULL CHECK (json_valid(content_json)),
    envelope_json TEXT NOT NULL CHECK (json_valid(envelope_json)),
    created_at INTEGER NOT NULL,
    FOREIGN KEY (preset_id, revision_no)
        REFERENCES agent_preset_revisions (preset_id, revision_no)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE agent_runtime_snapshot_capabilities (
    snapshot_id TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    capability_version TEXT NOT NULL,
    set_kind TEXT NOT NULL CHECK (set_kind IN ('initial', 'on_demand')),
    activation_plan_json TEXT CHECK (
        activation_plan_json IS NULL OR json_valid(activation_plan_json)
    ),
    PRIMARY KEY (snapshot_id, capability_id),
    FOREIGN KEY (snapshot_id) REFERENCES agent_runtime_snapshots (snapshot_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE agent_runtime_profiles (
    snapshot_id TEXT PRIMARY KEY,
    profile_kind TEXT NOT NULL CHECK (profile_kind IN ('coding_native', 'managed_minimal')),
    profile_json TEXT NOT NULL CHECK (json_valid(profile_json)),
    profile_digest TEXT NOT NULL CHECK (length(profile_digest) = 64),
    FOREIGN KEY (snapshot_id) REFERENCES agent_runtime_snapshots (snapshot_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE agent_preset_audit_events (
    audit_event_id TEXT PRIMARY KEY,
    preset_id TEXT NOT NULL,
    revision_no INTEGER,
    actor_ref_json TEXT NOT NULL CHECK (json_valid(actor_ref_json)),
    action TEXT NOT NULL,
    reason TEXT NOT NULL,
    revision_digest TEXT CHECK (revision_digest IS NULL OR length(revision_digest) = 64),
    created_at INTEGER NOT NULL,
    FOREIGN KEY (preset_id) REFERENCES agent_presets (preset_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

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
            fork_base_payload_id IS NULL AND next_seq IS NULL AND
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

CREATE TABLE session_payloads (
    payload_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    media_type TEXT NOT NULL,
    byte_len INTEGER NOT NULL CHECK (byte_len >= 0),
    digest TEXT NOT NULL CHECK (length(digest) = 64),
    body BLOB NOT NULL,
    FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT;

CREATE TABLE session_events (
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
    FOREIGN KEY (payload_id) REFERENCES session_payloads (payload_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (causation_event_id) REFERENCES session_events (event_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT;

CREATE TABLE session_heads (
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
    FOREIGN KEY (runtime_bound_event_id) REFERENCES session_events (event_id)
        ON UPDATE RESTRICT ON DELETE SET NULL
) STRICT;

CREATE TABLE message_projection (
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
CREATE INDEX idx_provider_connections_provider
    ON provider_connections (provider_id, role);
CREATE INDEX idx_provider_model_capabilities_provider_model
    ON provider_model_capabilities (provider_id, model);
CREATE INDEX idx_provider_model_capabilities_task
    ON provider_model_capabilities (task, provider_id, model);
CREATE INDEX idx_client_preferences_key
    ON client_preferences (key);
CREATE INDEX idx_agent_preset_revisions_preset
    ON agent_preset_revisions (preset_id, revision_no);
CREATE INDEX idx_agent_sessions_owner_state
    ON agent_sessions (owner_ref_json, state);
CREATE INDEX idx_session_events_correlation
    ON session_events (session_id, correlation_id, seq);
CREATE INDEX idx_session_payloads_session
    ON session_payloads (session_id);
CREATE INDEX idx_message_projection_sequence
    ON message_projection (session_id, first_seq, last_seq);
