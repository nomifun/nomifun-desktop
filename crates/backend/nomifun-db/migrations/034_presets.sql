-- Replace the former assistant template model with first-class reusable presets.
-- The old tables are read only by this migration and removed at the end; all
-- runtime code after 034 reads the preset catalog exclusively.

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
CREATE UNIQUE INDEX idx_presets_source ON presets(source_kind, source_key)
    WHERE source_key IS NOT NULL;
CREATE INDEX idx_presets_updated_at ON presets(updated_at DESC);

CREATE TABLE preset_localizations (
    preset_id            TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    locale               TEXT NOT NULL,
    name                 TEXT,
    description          TEXT,
    routing_description  TEXT,
    instructions         TEXT,
    PRIMARY KEY (preset_id, locale)
);

CREATE TABLE preset_targets (
    preset_id    TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    target_kind  TEXT NOT NULL CHECK (target_kind IN
        ('conversation','cluster_member','companion','public_companion','cron')),
    PRIMARY KEY (preset_id, target_kind)
);

CREATE TABLE preset_agent_preferences (
    preset_id   TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    agent_id    TEXT NOT NULL,
    rank        INTEGER NOT NULL DEFAULT 0,
    required    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (preset_id, agent_id)
);
CREATE INDEX idx_preset_agent_rank ON preset_agent_preferences(preset_id, rank);

CREATE TABLE preset_model_preferences (
    preset_id   TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    provider_id TEXT,
    model       TEXT NOT NULL,
    rank        INTEGER NOT NULL DEFAULT 0,
    required    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (preset_id, rank)
);
CREATE INDEX idx_preset_model_lookup ON preset_model_preferences(provider_id, model);

CREATE TABLE preset_skill_bindings (
    preset_id   TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    skill_name  TEXT NOT NULL,
    binding     TEXT NOT NULL CHECK (binding IN ('include','exclude_auto')),
    required    INTEGER NOT NULL DEFAULT 0,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (preset_id, skill_name, binding)
);

CREATE TABLE preset_knowledge_policy (
    preset_id   TEXT PRIMARY KEY NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    enabled     INTEGER NOT NULL DEFAULT 0,
    mode        TEXT NOT NULL DEFAULT 'inherit',
    writeback   INTEGER NOT NULL DEFAULT 0,
    eagerness   TEXT CHECK (eagerness IS NULL OR eagerness IN ('conservative','aggressive')),
    grounded    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE preset_knowledge_bases (
    preset_id          TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    knowledge_base_id  TEXT NOT NULL,
    sort_order         INTEGER NOT NULL DEFAULT 0,
    required           INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (preset_id, knowledge_base_id)
);

CREATE TABLE preset_examples (
    preset_id   TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    locale      TEXT NOT NULL DEFAULT '',
    sort_order  INTEGER NOT NULL DEFAULT 0,
    prompt      TEXT NOT NULL,
    PRIMARY KEY (preset_id, locale, sort_order)
);

CREATE TABLE preset_tags (
    key         TEXT PRIMARY KEY NOT NULL,
    dimension   TEXT NOT NULL CHECK (dimension IN ('audience','scenario')),
    label       TEXT NOT NULL,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);
CREATE INDEX idx_preset_tags_dimension ON preset_tags(dimension, sort_order);

CREATE TABLE preset_tag_bindings (
    preset_id  TEXT NOT NULL REFERENCES presets(id) ON DELETE CASCADE,
    tag_key    TEXT NOT NULL,
    dimension  TEXT NOT NULL CHECK (dimension IN ('audience','scenario')),
    PRIMARY KEY (preset_id, tag_key, dimension)
);

-- Deliberately no FK: builtin/extension preset content lives in signed bundles
-- and is merged into the catalog, but user state still belongs in SQLite.
CREATE TABLE preset_user_state (
    preset_id        TEXT PRIMARY KEY NOT NULL,
    enabled          INTEGER NOT NULL DEFAULT 1,
    auto_selectable  INTEGER NOT NULL DEFAULT 0,
    preferred_agent_id TEXT,
    sort_order       INTEGER NOT NULL DEFAULT 0,
    last_used_at     INTEGER,
    updated_at       INTEGER NOT NULL
);

-- Migrate user-authored content.
INSERT INTO presets (
    id, source_kind, source_key, revision, name, description,
    routing_description, instructions, avatar, fallback_allowed,
    created_at, updated_at
)
SELECT id, 'user', id, 1, name, description, description, '', avatar, 0,
       created_at, updated_at
FROM assistants;

INSERT INTO preset_targets (preset_id, target_kind)
SELECT id, target_kind
FROM assistants
CROSS JOIN (
    SELECT 'conversation' AS target_kind UNION ALL
    SELECT 'cluster_member' UNION ALL SELECT 'companion' UNION ALL SELECT 'cron'
);

INSERT INTO preset_agent_preferences (preset_id, agent_id, rank, required)
SELECT id, preset_agent_type, 0, 0 FROM assistants
WHERE trim(COALESCE(preset_agent_type, '')) <> '';

INSERT OR IGNORE INTO preset_skill_bindings (preset_id, skill_name, binding, required, sort_order)
SELECT a.id, j.value, 'include', 0, CAST(j.key AS INTEGER)
FROM assistants a, json_each(CASE WHEN json_valid(a.enabled_skills) THEN a.enabled_skills ELSE '[]' END) j
WHERE j.type = 'text' AND trim(j.value) <> '';

INSERT OR IGNORE INTO preset_skill_bindings (preset_id, skill_name, binding, required, sort_order)
SELECT a.id, j.value, 'include', 0, 1000 + CAST(j.key AS INTEGER)
FROM assistants a, json_each(CASE WHEN json_valid(a.custom_skill_names) THEN a.custom_skill_names ELSE '[]' END) j
WHERE j.type = 'text' AND trim(j.value) <> '';

INSERT OR IGNORE INTO preset_skill_bindings (preset_id, skill_name, binding, required, sort_order)
SELECT a.id, j.value, 'exclude_auto', 0, CAST(j.key AS INTEGER)
FROM assistants a, json_each(CASE WHEN json_valid(a.disabled_builtin_skills) THEN a.disabled_builtin_skills ELSE '[]' END) j
WHERE j.type = 'text' AND trim(j.value) <> '';

INSERT INTO preset_model_preferences (preset_id, provider_id, model, rank, required)
SELECT a.id, NULL, j.value, CAST(j.key AS INTEGER), 0
FROM assistants a, json_each(CASE WHEN json_valid(a.models) THEN a.models ELSE '[]' END) j
WHERE j.type = 'text' AND trim(j.value) <> '';

INSERT INTO preset_examples (preset_id, locale, sort_order, prompt)
SELECT a.id, '', CAST(j.key AS INTEGER), j.value
FROM assistants a, json_each(CASE WHEN json_valid(a.prompts) THEN a.prompts ELSE '[]' END) j
WHERE j.type = 'text';

-- Localized examples were stored as `{ locale: string[] }`. Keep them as
-- first-class localized preset examples instead of silently dropping every
-- non-default prompt during the domain migration.
INSERT INTO preset_examples (preset_id, locale, sort_order, prompt)
SELECT a.id, localized.key, CAST(prompt.key AS INTEGER), prompt.value
FROM assistants a,
     json_each(CASE WHEN json_valid(a.prompts_i18n) THEN a.prompts_i18n ELSE '{}' END) localized,
     json_each(
         CASE WHEN json_valid(localized.value) AND json_type(localized.value) = 'array'
              THEN localized.value ELSE '[]' END
     ) prompt
WHERE localized.type = 'array' AND prompt.type = 'text';

INSERT OR IGNORE INTO preset_localizations (preset_id, locale, name)
SELECT a.id, n.key, n.value
FROM assistants a, json_each(CASE WHEN json_valid(a.name_i18n) THEN a.name_i18n ELSE '{}' END) n
WHERE n.type = 'text';

-- A locale may have a translated description without a translated name.
-- Materialize that locale before the update below so description-only
-- translations survive.
INSERT OR IGNORE INTO preset_localizations (preset_id, locale, description)
SELECT a.id, d.key, d.value
FROM assistants a,
     json_each(CASE WHEN json_valid(a.description_i18n) THEN a.description_i18n ELSE '{}' END) d
WHERE d.type = 'text';
UPDATE preset_localizations
SET description = (
    SELECT d.value FROM assistants a,
      json_each(CASE WHEN json_valid(a.description_i18n) THEN a.description_i18n ELSE '{}' END) d
    WHERE a.id = preset_localizations.preset_id AND d.key = preset_localizations.locale
)
WHERE EXISTS (
    SELECT 1 FROM assistants a,
      json_each(CASE WHEN json_valid(a.description_i18n) THEN a.description_i18n ELSE '{}' END) d
    WHERE a.id = preset_localizations.preset_id AND d.key = preset_localizations.locale
);

INSERT INTO preset_knowledge_policy (preset_id)
SELECT id FROM assistants;

INSERT INTO preset_tags (key, dimension, label, sort_order, created_at)
SELECT key, dimension, label, sort_order, created_at FROM assistant_tags;

INSERT OR IGNORE INTO preset_tag_bindings (preset_id, tag_key, dimension)
SELECT a.id, j.value, 'audience'
FROM assistants a, json_each(CASE WHEN json_valid(a.audience_tags) THEN a.audience_tags ELSE '[]' END) j
WHERE j.type = 'text';
INSERT OR IGNORE INTO preset_tag_bindings (preset_id, tag_key, dimension)
SELECT a.id, j.value, 'scenario'
FROM assistants a, json_each(CASE WHEN json_valid(a.scenario_tags) THEN a.scenario_tags ELSE '[]' END) j
WHERE j.type = 'text';

INSERT INTO preset_user_state (
    preset_id, enabled, auto_selectable, preferred_agent_id,
    sort_order, last_used_at, updated_at
)
SELECT p.id, COALESCE(o.enabled, 1), COALESCE(o.enabled, 1), o.preset_agent_type,
       COALESCE(o.sort_order, 0), o.last_used_at, COALESCE(o.updated_at, p.updated_at)
FROM presets p LEFT JOIN assistant_overrides o ON o.assistant_id = p.id;

-- Preserve builtin overrides. Their content is reconciled from the embedded
-- preset catalog, so a state row may intentionally have no `presets` row.
INSERT OR REPLACE INTO preset_user_state (
    preset_id, enabled, auto_selectable, preferred_agent_id,
    sort_order, last_used_at, updated_at
)
SELECT assistant_id, enabled, enabled, preset_agent_type,
       sort_order, last_used_at, updated_at
FROM assistant_overrides;

-- First-class preset lineage and immutable snapshots for long-lived targets.
ALTER TABLE conversations ADD COLUMN preset_id TEXT;
ALTER TABLE conversations ADD COLUMN preset_revision INTEGER;
ALTER TABLE conversations ADD COLUMN preset_snapshot TEXT;
CREATE INDEX idx_conversations_preset_id ON conversations(preset_id);

UPDATE conversations
SET preset_id = COALESCE(
        json_extract(extra, '$.preset_id'),
        json_extract(extra, '$.presetId'),
        json_extract(extra, '$.preset_assistant_id'),
        json_extract(extra, '$.presetAssistantId')
    ),
    preset_revision = 1
WHERE json_valid(extra) AND COALESCE(
        json_extract(extra, '$.preset_id'),
        json_extract(extra, '$.presetId'),
        json_extract(extra, '$.preset_assistant_id'),
        json_extract(extra, '$.presetAssistantId')
    ) IS NOT NULL;

UPDATE conversations
SET preset_snapshot = json_object(
    'preset_id', preset_id,
    'preset_revision', preset_revision,
    'preset_name', COALESCE((SELECT name FROM presets WHERE id = preset_id), preset_id),
    'target', 'conversation',
    'instructions', COALESCE(json_extract(extra, '$.preset_context'), ''),
    'included_skills', COALESCE(json_extract(extra, '$.skills'), json('[]')),
    'excluded_auto_skills', json('[]'),
    'knowledge_policy', json_object(
        'enabled', json('false'),
        'mode', 'inherit',
        'writeback', json('false'),
        'grounded', json('false')
    ),
    'knowledge_base_ids', json('[]'),
    'warnings', json_array('Migrated from legacy assistant context')
)
WHERE preset_id IS NOT NULL;

ALTER TABLE cron_jobs ADD COLUMN preset_id TEXT;
ALTER TABLE cron_jobs ADD COLUMN preset_revision INTEGER;
ALTER TABLE cron_jobs ADD COLUMN preset_snapshot TEXT;
CREATE INDEX idx_cron_jobs_preset_id ON cron_jobs(preset_id);

UPDATE cron_jobs
SET preset_id = COALESCE(
        json_extract(agent_config, '$.preset_id'),
        json_extract(agent_config, '$.presetId'),
        json_extract(agent_config, '$.preset_assistant_id'),
        json_extract(agent_config, '$.presetAssistantId')
    ),
    preset_revision = 1
WHERE json_valid(agent_config) AND COALESCE(
        json_extract(agent_config, '$.preset_id'),
        json_extract(agent_config, '$.presetId'),
        json_extract(agent_config, '$.preset_assistant_id'),
        json_extract(agent_config, '$.presetAssistantId')
    ) IS NOT NULL;

UPDATE cron_jobs
SET preset_snapshot = json_object(
    'preset_id', preset_id,
    'preset_revision', preset_revision,
    'preset_name', COALESCE((SELECT name FROM presets WHERE id = preset_id), preset_id),
    'target', 'cron',
    'instructions', '',
    'included_skills', json(COALESCE((
        SELECT json_group_array(skill_name)
        FROM (
            SELECT skill_name
            FROM preset_skill_bindings
            WHERE preset_id = cron_jobs.preset_id AND binding = 'include'
            ORDER BY sort_order, skill_name
            LIMIT -1
        )
    ), '[]')),
    'excluded_auto_skills', json(COALESCE((
        SELECT json_group_array(skill_name)
        FROM (
            SELECT skill_name
            FROM preset_skill_bindings
            WHERE preset_id = cron_jobs.preset_id AND binding = 'exclude_auto'
            ORDER BY sort_order, skill_name
            LIMIT -1
        )
    ), '[]')),
    'knowledge_policy', json_object(
        'enabled', json('false'),
        'mode', 'inherit',
        'writeback', json('false'),
        'grounded', json('false')
    ),
    'knowledge_base_ids', json('[]'),
    'warnings', json_array('Migrated from legacy scheduled assistant selection')
)
WHERE preset_id IS NOT NULL;

-- Fleet members formerly overloaded `agent_id` with an assistant id. Preserve
-- that template lineage separately and restore `agent_id` to executor meaning.
ALTER TABLE fleet_members ADD COLUMN preset_id TEXT;
ALTER TABLE fleet_members ADD COLUMN preset_revision INTEGER;
ALTER TABLE fleet_members ADD COLUMN preset_snapshot TEXT;
CREATE INDEX idx_fleet_members_preset_id ON fleet_members(preset_id);

UPDATE fleet_members
SET preset_id = agent_id,
    preset_revision = 1,
    preset_snapshot = json_object(
        'preset_id', agent_id,
        'preset_revision', 1,
        'preset_name', COALESCE((SELECT name FROM presets WHERE id = fleet_members.agent_id), agent_id),
        'target', 'cluster_member',
        'instructions', '',
        'resolved_agent_id', (
            SELECT a.id
            FROM preset_agent_preferences p
            JOIN agent_metadata a ON a.id = p.agent_id OR a.backend = p.agent_id
            WHERE p.preset_id = fleet_members.agent_id
            ORDER BY p.rank, a.sort_order
            LIMIT 1
        ),
        'included_skills', json(COALESCE((
            SELECT json_group_array(skill_name)
            FROM (
                SELECT skill_name
                FROM preset_skill_bindings
                WHERE preset_id = fleet_members.agent_id AND binding = 'include'
                ORDER BY sort_order, skill_name
                LIMIT -1
            )
        ), '[]')),
        'excluded_auto_skills', json(COALESCE((
            SELECT json_group_array(skill_name)
            FROM (
                SELECT skill_name
                FROM preset_skill_bindings
                WHERE preset_id = fleet_members.agent_id AND binding = 'exclude_auto'
                ORDER BY sort_order, skill_name
                LIMIT -1
            )
        ), '[]')),
        'knowledge_policy', json_object(
            'enabled', json('false'),
            'mode', 'inherit',
            'writeback', json('false'),
            'grounded', json('false')
        ),
        'knowledge_base_ids', json('[]'),
        'warnings', json_array('Migrated from overloaded fleet member agent id')
    ),
    agent_id = COALESCE(
        (SELECT a.id
         FROM preset_agent_preferences p
         JOIN agent_metadata a ON a.id = p.agent_id OR a.backend = p.agent_id
         WHERE p.preset_id = fleet_members.agent_id
         ORDER BY p.rank, a.sort_order
         LIMIT 1),
        ''
    )
WHERE EXISTS (SELECT 1 FROM presets p WHERE p.id = fleet_members.agent_id);

-- Runs freeze fleet members as a JSON array. Rewriting only `fleet_members`
-- would leave every historical run with the former preset id overloaded in
-- `agent_id`, causing resume/replan paths to treat a template as an executor.
-- Materialize the same lineage and immutable snapshot inside each matching
-- frozen member while keeping unrelated/bare-agent members byte-equivalent at
-- the JSON value level.
UPDATE orch_runs
SET fleet_snapshot = (
    SELECT COALESCE(
        json_group_array(json(
            CASE
                WHEN EXISTS (
                    SELECT 1 FROM presets p
                    WHERE p.id = json_extract(member.value, '$.agent_id')
                )
                THEN json_set(
                    member.value,
                    '$.preset_id', json_extract(member.value, '$.agent_id'),
                    '$.preset_revision', 1,
                    '$.preset_snapshot', json_object(
                        'preset_id', json_extract(member.value, '$.agent_id'),
                        'preset_revision', 1,
                        'preset_name', COALESCE(
                            (SELECT name FROM presets
                             WHERE id = json_extract(member.value, '$.agent_id')),
                            json_extract(member.value, '$.agent_id')
                        ),
                        'target', 'cluster_member',
                        'instructions', '',
                        'resolved_agent_id', (
                            SELECT a.id
                            FROM preset_agent_preferences p
                            JOIN agent_metadata a ON a.id = p.agent_id OR a.backend = p.agent_id
                            WHERE p.preset_id = json_extract(member.value, '$.agent_id')
                            ORDER BY p.rank, a.sort_order
                            LIMIT 1
                        ),
                        'included_skills', json(COALESCE((
                            SELECT json_group_array(skill_name)
                            FROM (
                                SELECT skill_name
                                FROM preset_skill_bindings
                                WHERE preset_id = json_extract(member.value, '$.agent_id')
                                  AND binding = 'include'
                                ORDER BY sort_order, skill_name
                                LIMIT -1
                            )
                        ), '[]')),
                        'excluded_auto_skills', json(COALESCE((
                            SELECT json_group_array(skill_name)
                            FROM (
                                SELECT skill_name
                                FROM preset_skill_bindings
                                WHERE preset_id = json_extract(member.value, '$.agent_id')
                                  AND binding = 'exclude_auto'
                                ORDER BY sort_order, skill_name
                                LIMIT -1
                            )
                        ), '[]')),
                        'knowledge_policy', json_object(
                            'enabled', json('false'),
                            'mode', 'inherit',
                            'writeback', json('false'),
                            'grounded', json('false')
                        ),
                        'knowledge_base_ids', json('[]'),
                        'warnings', json_array('Migrated from overloaded fleet snapshot agent id')
                    ),
                    '$.agent_id', COALESCE((
                        SELECT a.id
                        FROM preset_agent_preferences p
                        JOIN agent_metadata a ON a.id = p.agent_id OR a.backend = p.agent_id
                        WHERE p.preset_id = json_extract(member.value, '$.agent_id')
                        ORDER BY p.rank, a.sort_order
                        LIMIT 1
                    ), ''),
                    '$.enabled_skills', json(COALESCE((
                        SELECT json_group_array(skill_name)
                        FROM (
                            SELECT skill_name
                            FROM preset_skill_bindings
                            WHERE preset_id = json_extract(member.value, '$.agent_id')
                              AND binding = 'include'
                            ORDER BY sort_order, skill_name
                            LIMIT -1
                        )
                    ), '[]')),
                    '$.disabled_builtin_skills', json(COALESCE((
                        SELECT json_group_array(skill_name)
                        FROM (
                            SELECT skill_name
                            FROM preset_skill_bindings
                            WHERE preset_id = json_extract(member.value, '$.agent_id')
                              AND binding = 'exclude_auto'
                            ORDER BY sort_order, skill_name
                            LIMIT -1
                        )
                    ), '[]'))
                )
                ELSE member.value
            END
        )),
        json('[]')
    )
    FROM json_each(orch_runs.fleet_snapshot) member
)
WHERE json_valid(fleet_snapshot)
  AND json_type(fleet_snapshot) = 'array'
  AND EXISTS (
      SELECT 1
      FROM json_each(orch_runs.fleet_snapshot) member
      JOIN presets p ON p.id = json_extract(member.value, '$.agent_id')
  );

DROP TABLE assistant_overrides;
DROP TABLE assistant_tags;
DROP TABLE assistants;
