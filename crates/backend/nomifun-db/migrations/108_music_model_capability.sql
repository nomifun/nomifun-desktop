-- Music generation is distinct from speech synthesis. Preserve all catalog
-- settings, health reports and connection bindings while widening the task enum.
CREATE TABLE provider_model_capabilities_next (
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

INSERT INTO provider_model_capabilities_next (id, provider_id, model, task, traits, protocol, connection_role, base_url_override, endpoint, poll_endpoint, content_endpoint, realtime_endpoint, allow_cross_origin_credentials, provider_params, context_limit, health, health_checked_at, created_at, updated_at, output_limit)
SELECT id, provider_id, model, task, traits, protocol, connection_role, base_url_override, endpoint, poll_endpoint, content_endpoint, realtime_endpoint, allow_cross_origin_credentials, provider_params, context_limit, health, health_checked_at, created_at, updated_at, output_limit FROM provider_model_capabilities;
DROP TABLE provider_model_capabilities;
ALTER TABLE provider_model_capabilities_next RENAME TO provider_model_capabilities;

CREATE INDEX idx_provider_model_capabilities_provider_model
    ON provider_model_capabilities(provider_id, model);

CREATE INDEX idx_provider_model_capabilities_task
    ON provider_model_capabilities(task, provider_id, model);
