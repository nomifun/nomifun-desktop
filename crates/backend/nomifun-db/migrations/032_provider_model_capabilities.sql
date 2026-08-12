-- Replace the model-level transport/profile compatibility shape with one
-- authoritative row per provider/model/task. This migration is intentionally
-- one-way: runtime code reads only provider_model_capabilities afterwards.

-- Preserve the already-verified provider lifecycle endpoint corrections from
-- the discarded unpublished migration lineage.
UPDATE providers
SET base_url = 'https://api.ppio.com/openai/v1',
    updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE platform = 'ppio'
  AND lower(rtrim(base_url, '/')) = 'https://api.ppinfra.com/v3/openai';

UPDATE providers
SET base_url = 'https://ai.ctaigw.cn/v1',
    updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE platform = 'ctyun'
  AND rtrim(base_url, '/') = 'https://wishub-x6.ctyun.cn/v1';

-- Official Gemini now uses its native protocol/auth contract. Normalize only
-- the retired official OpenAI-compatible default; custom URLs are preserved.
UPDATE providers
SET base_url = 'https://generativelanguage.googleapis.com',
    updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE platform = 'gemini'
  AND lower(rtrim(base_url, '/')) =
      'https://generativelanguage.googleapis.com/v1beta/openai';

-- Preserve the exact realtime reclassification. User-authored rows are never
-- rewritten and no wildcard model-name inference is performed.
UPDATE provider_models
SET tasks = '["realtime_conversation"]',
    traits = '["audio_input","audio_output","realtime","streaming"]',
    updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE source = 'inferred'
  AND lower(model) = 'stepaudio-2.5-realtime'
  AND EXISTS (
      SELECT 1 FROM providers p
      WHERE p.provider_id = provider_models.provider_id
        AND p.platform IN ('stepfun', 'stepfun-plan')
  );

UPDATE provider_models
SET tasks = '["realtime_conversation"]',
    traits = '["audio_input","audio_output","realtime","streaming"]',
    updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE source = 'inferred'
  AND lower(model) = 'glm-realtime'
  AND EXISTS (
      SELECT 1 FROM providers p
      WHERE p.provider_id = provider_models.provider_id
        AND p.platform = 'zhipu'
  );

CREATE TABLE provider_model_capabilities (
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
    updated_at                     INTEGER NOT NULL,
    UNIQUE (provider_id, model, task),
    CHECK (task IN (
        'chat', 'realtime_conversation', 'image_generation', 'image_edit',
        'video_generation', 'speech_synthesis', 'speech_recognition',
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

CREATE INDEX idx_provider_model_capabilities_provider_model
    ON provider_model_capabilities(provider_id, model);
CREATE INDEX idx_provider_model_capabilities_task
    ON provider_model_capabilities(task, provider_id, model);

-- Expand only declared old tasks. An explicit task override wins, then the
-- old dedicated protocol column, then a verified preset route. Unknown or
-- unverified combinations deliberately produce no capability row; the model
-- remains visible after the table rebuild so the user can configure it.
WITH known_protocol_tasks(protocol, task) AS (
    VALUES
        ('openai.chat_text', 'chat'),
        ('anthropic.messages', 'chat'),
        ('bedrock.anthropic_messages', 'chat'),
        ('gemini.generate_text', 'chat'),
        ('stepfun.realtime_s2s', 'realtime_conversation'),
        ('openai.images', 'image_generation'),
        ('openai.images', 'image_edit'),
        ('gemini.generate_content', 'image_generation'),
        ('gemini.generate_content', 'image_edit'),
        ('ark.images', 'image_generation'),
        ('dashscope.images', 'image_generation'),
        ('siliconflow.images', 'image_generation'),
        ('siliconflow.images', 'image_edit'),
        ('xai.images_json', 'image_generation'),
        ('xai.images_json', 'image_edit'),
        ('stepfun.images', 'image_generation'),
        ('stepfun.images', 'image_edit'),
        ('openai.videos', 'video_generation'),
        ('ark.video_jobs', 'video_generation'),
        ('siliconflow.video_jobs', 'video_generation'),
        ('xai.video_jobs', 'video_generation'),
        ('zhipu.video_jobs', 'video_generation'),
        ('openai.audio_speech', 'speech_synthesis'),
        ('deepgram.speak_rest', 'speech_synthesis'),
        ('volc.tts_v3', 'speech_synthesis'),
        ('minimax.t2a', 'speech_synthesis'),
        ('mimo.chat_tts', 'speech_synthesis'),
        ('siliconflow.audio_speech', 'speech_synthesis'),
        ('xai.tts', 'speech_synthesis'),
        ('stepfun.audio_speech', 'speech_synthesis'),
        ('openai.audio_transcriptions', 'speech_recognition'),
        ('deepgram.listen', 'speech_recognition'),
        ('volc.asr_file', 'speech_recognition'),
        ('mimo.chat_asr', 'speech_recognition'),
        ('xai.stt', 'speech_recognition'),
        ('stepfun.asr_sse', 'speech_recognition'),
        ('openai.embeddings', 'embedding'),
        ('dashscope.embeddings', 'embedding'),
        ('generic.rerank', 'rerank')
), expanded AS (
    SELECT
        pm.provider_id,
        pm.model,
        t.value AS task,
        CASE WHEN json_valid(pm.traits) AND json_type(pm.traits) = 'array'
             THEN pm.traits ELSE '[]' END AS traits,
        CASE WHEN json_valid(pm.params) AND json_type(pm.params) = 'object'
             THEN pm.params ELSE '{}' END AS root_params,
        CASE
            WHEN json_valid(pm.params)
             AND json_type(pm.params, '$.task_overrides.' || t.value) = 'object'
            THEN json_extract(pm.params, '$.task_overrides.' || t.value)
            ELSE '{}'
        END AS task_params,
        pm.protocol AS model_protocol,
        pm.connection_role AS model_connection_role,
        pm.context_limit,
        pm.health,
        pm.health_checked_at,
        pm.created_at,
        pm.updated_at,
        p.platform,
        p.base_url AS provider_base_url,
        p.is_full_url AS provider_is_full_url
    FROM provider_models pm
    JOIN providers p ON p.provider_id = pm.provider_id
    JOIN json_each(
        CASE WHEN json_valid(pm.tasks) AND json_type(pm.tasks) = 'array'
             THEN pm.tasks ELSE '[]' END
    ) t
    WHERE t.type = 'text'
), routed_raw AS (
    SELECT *,
        NULLIF(trim(json_extract(task_params, '$.protocol')), '') AS task_protocol,
        NULLIF(trim(model_protocol), '') AS row_protocol,
        CASE
                WHEN task = 'chat' AND platform = 'anthropic' THEN 'anthropic.messages'
                WHEN task = 'chat' AND platform = 'bedrock' THEN 'bedrock.anthropic_messages'
                WHEN task = 'chat' AND platform = 'gemini' THEN 'gemini.generate_text'
                WHEN task = 'chat' AND platform IN (
                    'openai', 'deepseek', 'mimo', 'mimo-token-plan-cn',
                    'mimo-token-plan-sgp', 'mimo-token-plan-ams', 'minimax',
                    'minimax-code', 'minimax-coding-plan', 'novita', 'openrouter',
                    'dashscope', 'alibaba', 'dashscope-coding', 'siliconflow',
                    'zhipu', 'glm-coding-plan', 'moonshot-cn', 'moonshot-global',
                    'xai', 'ark', 'volcengine', 'ark-coding-plan', 'ark-agent-plan',
                    'qianfan', 'qianfan-coding-plan', 'hunyuan', 'hunyuan-global',
                    'lingyi', 'poe', 'ppio', 'modelscope', 'infiniai', 'ctyun',
                    'stepfun', 'stepfun-plan'
                ) THEN 'openai.chat_text'
                WHEN task = 'realtime_conversation'
                 AND platform IN ('stepfun', 'stepfun-plan') THEN 'stepfun.realtime_s2s'
                WHEN task IN ('image_generation', 'image_edit') AND platform = 'openai'
                    THEN 'openai.images'
                WHEN task IN ('image_generation', 'image_edit') AND platform = 'gemini'
                    THEN 'gemini.generate_content'
                WHEN task = 'image_generation' AND platform IN ('ark', 'volcengine')
                    THEN 'ark.images'
                WHEN task = 'image_generation' AND platform IN ('dashscope', 'alibaba')
                    THEN 'dashscope.images'
                WHEN task IN ('image_generation', 'image_edit') AND platform = 'siliconflow'
                    THEN 'siliconflow.images'
                WHEN task IN ('image_generation', 'image_edit') AND platform = 'xai'
                    THEN 'xai.images_json'
                WHEN task IN ('image_generation', 'image_edit')
                 AND platform IN ('stepfun', 'stepfun-plan') THEN 'stepfun.images'
                WHEN task = 'image_generation' AND platform = 'ctyun' THEN 'openai.images'
                WHEN task = 'video_generation' AND platform = 'openai' THEN 'openai.videos'
                WHEN task = 'video_generation' AND platform IN ('ark', 'volcengine')
                    THEN 'ark.video_jobs'
                WHEN task = 'video_generation' AND platform = 'siliconflow'
                    THEN 'siliconflow.video_jobs'
                WHEN task = 'video_generation' AND platform = 'xai' THEN 'xai.video_jobs'
                WHEN task = 'video_generation' AND platform = 'zhipu' THEN 'zhipu.video_jobs'
                WHEN task = 'speech_synthesis' AND platform = 'openai'
                    THEN 'openai.audio_speech'
                WHEN task = 'speech_synthesis' AND platform = 'deepgram'
                    THEN 'deepgram.speak_rest'
                WHEN task = 'speech_synthesis' AND platform IN ('ark', 'volcengine')
                    THEN 'volc.tts_v3'
                WHEN task = 'speech_synthesis' AND platform = 'minimax' THEN 'minimax.t2a'
                WHEN task = 'speech_synthesis' AND platform = 'mimo' THEN 'mimo.chat_tts'
                WHEN task = 'speech_synthesis' AND platform = 'siliconflow'
                    THEN 'siliconflow.audio_speech'
                WHEN task = 'speech_synthesis' AND platform = 'xai' THEN 'xai.tts'
                WHEN task = 'speech_synthesis' AND platform IN ('stepfun', 'stepfun-plan')
                    THEN 'stepfun.audio_speech'
                WHEN task = 'speech_recognition' AND platform = 'openai'
                    THEN 'openai.audio_transcriptions'
                WHEN task = 'speech_recognition' AND platform = 'deepgram'
                    THEN 'deepgram.listen'
                WHEN task = 'speech_recognition' AND platform IN ('ark', 'volcengine')
                    THEN 'volc.asr_file'
                WHEN task = 'speech_recognition' AND platform = 'mimo' THEN 'mimo.chat_asr'
                WHEN task = 'speech_recognition' AND platform = 'siliconflow'
                    THEN 'openai.audio_transcriptions'
                WHEN task = 'speech_recognition' AND platform = 'xai' THEN 'xai.stt'
                WHEN task = 'speech_recognition' AND platform IN ('stepfun', 'stepfun-plan')
                    THEN 'stepfun.asr_sse'
                WHEN task = 'embedding' AND platform = 'openai' THEN 'openai.embeddings'
                WHEN task = 'embedding' AND platform IN ('dashscope', 'alibaba')
                    THEN 'dashscope.embeddings'
                WHEN task = 'embedding' AND platform IN (
                    'novita', 'openrouter', 'siliconflow', 'ppio', 'infiniai',
                    'qianfan', 'hunyuan', 'hunyuan-global', 'ctyun', 'zhipu'
                ) THEN 'openai.embeddings'
                WHEN task = 'rerank' AND platform IN (
                    'siliconflow', 'ppio', 'qianfan', 'ctyun', 'zhipu'
                ) THEN 'generic.rerank'
        END AS preset_protocol,
        COALESCE(
            NULLIF(trim(json_extract(task_params, '$.connection_role')), ''),
            NULLIF(trim(model_connection_role), ''),
            CASE WHEN platform IN ('ark', 'volcengine')
                       AND task IN ('speech_synthesis', 'speech_recognition')
                 THEN 'voice' END,
            'default'
        ) AS resolved_connection_role
    FROM expanded
), normalized AS (
    SELECT routed_raw.*,
        CASE
            WHEN task = 'chat' AND lower(task_protocol) = 'openai'
                THEN 'openai.chat_text'
            WHEN task = 'chat' AND lower(task_protocol) = 'anthropic'
                THEN 'anthropic.messages'
            WHEN task = 'chat' AND lower(task_protocol) = 'gemini'
             AND platform = 'gemini' THEN 'gemini.generate_text'
            WHEN task = 'chat' AND lower(task_protocol) = 'gemini'
                THEN 'openai.chat_text'
            ELSE task_protocol
        END AS normalized_task_protocol,
        CASE
            WHEN task = 'chat' AND lower(row_protocol) = 'openai'
                THEN 'openai.chat_text'
            WHEN task = 'chat' AND lower(row_protocol) = 'anthropic'
                THEN 'anthropic.messages'
            WHEN task = 'chat' AND lower(row_protocol) = 'gemini'
             AND platform = 'gemini' THEN 'gemini.generate_text'
            WHEN task = 'chat' AND lower(row_protocol) = 'gemini'
                THEN 'openai.chat_text'
            ELSE row_protocol
        END AS normalized_row_protocol
    FROM routed_raw
), routed AS (
    SELECT normalized.*,
        COALESCE(
            CASE WHEN EXISTS (
                SELECT 1 FROM known_protocol_tasks known
                WHERE known.protocol = normalized_task_protocol
                  AND known.task = normalized.task
            ) THEN normalized_task_protocol END,
            CASE WHEN EXISTS (
                SELECT 1 FROM known_protocol_tasks known
                WHERE known.protocol = normalized_row_protocol
                  AND known.task = normalized.task
            ) THEN normalized_row_protocol END,
            preset_protocol
        ) AS resolved_protocol
    FROM normalized
), effective AS (
    SELECT routed.*, pc.base_url AS named_base_url, pc.is_full_url AS named_is_full_url,
        COALESCE(
            json_extract(task_params, '$.base_url_override'),
            json_extract(root_params, '$.base_url_override'),
            json_extract(task_params, '$.base_url'),
            json_extract(root_params, '$.base_url')
        ) AS configured_base_url,
        COALESCE(
            json_extract(task_params, '$.base_url_is_full'),
            json_extract(root_params, '$.base_url_is_full'),
            0
        ) AS configured_base_url_is_full
    FROM routed
    LEFT JOIN provider_connections pc
      ON pc.provider_id = routed.provider_id
     AND pc.role = routed.resolved_connection_role
)
INSERT INTO provider_model_capabilities (
    provider_id, model, task, traits, protocol, connection_role,
    base_url_override, endpoint, poll_endpoint, content_endpoint, realtime_endpoint,
    allow_cross_origin_credentials, provider_params, context_limit,
    health, health_checked_at, created_at, updated_at
)
SELECT
    provider_id,
    model,
    task,
    traits,
    resolved_protocol,
    resolved_connection_role,
    CASE
        WHEN configured_base_url IS NOT NULL AND NOT configured_base_url_is_full
            THEN configured_base_url
        WHEN platform IN ('dashscope', 'alibaba')
         AND task IN ('image_generation', 'embedding')
            THEN 'https://dashscope.aliyuncs.com'
    END,
    COALESCE(
        json_extract(task_params, '$.endpoint'),
        json_extract(root_params, '$.endpoint'),
        CASE WHEN configured_base_url_is_full THEN configured_base_url END,
        CASE WHEN named_is_full_url THEN named_base_url END,
        CASE WHEN provider_is_full_url THEN provider_base_url END
    ),
    COALESCE(json_extract(task_params, '$.poll_endpoint'),
             json_extract(root_params, '$.poll_endpoint'),
             json_extract(task_params, '$.status_endpoint'),
             json_extract(root_params, '$.status_endpoint')),
    COALESCE(json_extract(task_params, '$.content_endpoint'),
             json_extract(root_params, '$.content_endpoint')),
    COALESCE(json_extract(task_params, '$.realtime_endpoint'),
             json_extract(root_params, '$.realtime_endpoint')),
    CASE WHEN COALESCE(
        json_extract(task_params, '$.allow_cross_origin_credentials'),
        json_extract(root_params, '$.allow_cross_origin_credentials'), 0
    ) THEN 1 ELSE 0 END,
    json_patch(
        json_remove(
            root_params, '$.task_overrides', '$.protocol', '$.connection_role',
            '$.connection', '$.connection_id', '$.base_url', '$.base_url_override', '$.base_url_is_full',
            '$.is_full_url', '$.endpoint', '$.poll_endpoint', '$.status_endpoint',
            '$.content_endpoint', '$.realtime_endpoint', '$.allow_cross_origin_credentials',
            '$.request_shape', '$.request_defaults', '$.request_body', '$.auth',
            '$.auth_scheme', '$.credentials', '$.api_key', '$.api_keys', '$.headers'
        ),
        json_remove(
            task_params, '$.protocol', '$.connection_role', '$.connection',
            '$.connection_id', '$.base_url', '$.base_url_override', '$.base_url_is_full', '$.is_full_url',
            '$.endpoint', '$.poll_endpoint', '$.status_endpoint', '$.content_endpoint',
            '$.realtime_endpoint', '$.allow_cross_origin_credentials',
            '$.request_shape', '$.request_defaults', '$.request_body', '$.auth',
            '$.auth_scheme', '$.credentials', '$.api_key', '$.api_keys', '$.headers',
            '$.task_overrides'
        )
    ),
    context_limit,
    CASE
        WHEN json_valid(health)
         AND json_extract(health, '$.task') = task
         AND json_extract(health, '$.status') IN ('unknown', 'healthy', 'unhealthy')
        THEN json_object(
            'status', json_extract(health, '$.status'),
            'latency', json_extract(health, '$.latency'),
            'error', json_extract(health, '$.error')
        )
    END,
    CASE WHEN json_valid(health) AND json_extract(health, '$.task') = task
         THEN health_checked_at END,
    created_at,
    updated_at
FROM effective
WHERE resolved_protocol IS NOT NULL
  AND trim(resolved_protocol) <> ''
  AND (resolved_connection_role = 'default' OR named_base_url IS NOT NULL)
ON CONFLICT(provider_id, model, task) DO NOTHING;

-- Provider default connections are always roots and carry explicit auth.
CREATE TABLE providers_new (
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

INSERT INTO providers_new (
    id, provider_id, platform, name, base_url, auth_scheme, credentials_encrypted,
    enabled, bedrock_config, sort_order, config_revision, created_at, updated_at
)
SELECT
    id, provider_id, platform, name,
    CASE
        WHEN is_full_url = 1
         AND instr(base_url, '://') > 0
         AND instr(substr(base_url, instr(base_url, '://') + 3), '/') > 0
        THEN substr(
            base_url, 1,
            instr(base_url, '://') + 1
            + instr(substr(base_url, instr(base_url, '://') + 3), '/')
        )
        ELSE rtrim(base_url, '/')
    END,
    CASE
        WHEN platform = 'anthropic' THEN 'header_key:x-api-key'
        WHEN platform = 'gemini' THEN 'header_key:x-goog-api-key'
        WHEN platform = 'deepgram' THEN 'token'
        WHEN platform = 'bedrock' THEN 'bedrock'
        ELSE 'bearer'
    END,
    '',
    enabled,
    CASE
        WHEN json_valid(bedrock_config) AND json_type(bedrock_config) = 'object'
        THEN json_remove(
            bedrock_config,
            '$.access_key_id', '$.secret_access_key', '$.session_token',
            '$.accessKeyId', '$.secretAccessKey', '$.sessionToken'
        )
        ELSE NULL
    END,
    sort_order, 0, created_at, updated_at
FROM providers;

DROP TABLE providers;
ALTER TABLE providers_new RENAME TO providers;
CREATE INDEX idx_providers_platform ON providers(platform);

-- Models now contain identity/display fields only.
CREATE TABLE provider_models_new (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL,
    model       TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    sort_order  INTEGER NOT NULL DEFAULT 0,
    description TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    UNIQUE (provider_id, model),
    CHECK (length(provider_id) = 36 AND lower(provider_id) = provider_id
           AND provider_id GLOB '????????-????-7???-[89ab]???-????????????'
           AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

INSERT INTO provider_models_new (
    id, provider_id, model, enabled, sort_order, description, created_at, updated_at
)
SELECT id, provider_id, model, enabled, sort_order, description, created_at, updated_at
FROM provider_models;

DROP TABLE provider_models;
ALTER TABLE provider_models_new RENAME TO provider_models;
CREATE INDEX idx_provider_models_provider_id ON provider_models(provider_id);

-- Named connections follow the same root-only rule; complete task endpoints
-- were materialized on the matching capability above.
CREATE TABLE provider_connections_new (
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

INSERT INTO provider_connections_new (
    id, connection_id, provider_id, role, label, base_url, auth_scheme,
    credentials_encrypted, extra, created_at, updated_at
)
SELECT
    id, connection_id, provider_id, role, label,
    CASE
        WHEN is_full_url = 1
         AND instr(base_url, '://') > 0
         AND instr(substr(base_url, instr(base_url, '://') + 3), '/') > 0
        THEN substr(
            base_url, 1,
            instr(base_url, '://') + 1
            + instr(substr(base_url, instr(base_url, '://') + 3), '/')
        )
        ELSE rtrim(base_url, '/')
    END,
    CASE WHEN lower(trim(auth_scheme)) = 'api_key'
         THEN 'header_key:x-api-key'
         ELSE trim(auth_scheme) END,
    '',
    CASE
        WHEN json_valid(extra) AND json_type(extra) = 'object'
        THEN json_remove(
            extra,
            '$.auth', '$.auth_scheme', '$.authorization', '$.credentials',
            '$.api_key', '$.api_keys', '$.headers', '$.password', '$.token',
            '$.access_key_id', '$.secret_access_key', '$.session_token',
            '$.accessKeyId', '$.secretAccessKey', '$.sessionToken'
        )
        ELSE '{}'
    END,
    created_at, updated_at
FROM provider_connections
WHERE trim(role) <> '' AND role <> 'default';

DROP TABLE provider_connections;
ALTER TABLE provider_connections_new RENAME TO provider_connections;
CREATE INDEX idx_provider_connections_provider_id ON provider_connections(provider_id);
