-- Customer-service domain (设计 C, C1 batch).
--
-- Six new `cs_` tables owned by the nomifun-customer-service crate, plus the
-- removal of the retired public-agent binding surfaces. The customer-service
-- domain deliberately does NOT touch the Conversation/turn system: dialogues
-- and messages are its own aggregate, keyed by v3 business IDs.
--
-- v3 rules: local INTEGER PRIMARY KEY AUTOINCREMENT technical keys, named
-- UUIDv7 business IDs with the baseline GLOB CHECK, no physical foreign keys,
-- logical references + named indexes only.

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
CREATE INDEX idx_cs_agents_provider_id ON cs_agents(provider_id);
CREATE INDEX idx_cs_agents_knowledge_base_ids_json ON cs_agents(knowledge_base_ids);

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
CREATE INDEX idx_cs_channel_bindings_agent ON cs_channel_bindings(cs_agent_id);
-- One bot serves at most one customer-service agent.
CREATE UNIQUE INDEX idx_cs_channel_bindings_plugin ON cs_channel_bindings(channel_plugin_id);

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
-- 一人一线: one dialogue per (bot, visitor, chat) triple.
CREATE UNIQUE INDEX idx_cs_dialogues_identity
    ON cs_dialogues(channel_plugin_id, channel_user_id, chat_id);
CREATE INDEX idx_cs_dialogues_agent ON cs_dialogues(cs_agent_id, last_activity);
CREATE INDEX idx_cs_dialogues_channel_user ON cs_dialogues(channel_user_id);

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
CREATE INDEX idx_cs_messages_dialogue ON cs_messages(cs_dialogue_id, id);

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
);
CREATE INDEX idx_cs_notes_agent ON cs_notes(cs_agent_id);

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
CREATE INDEX idx_cs_audit_agent_time ON cs_audit_events(cs_agent_id, created_at);

-- ── Retire the public-agent binding surfaces ────────────────────────────────

DROP INDEX IF EXISTS idx_channel_plugins_public_agent_id;
DROP INDEX IF EXISTS idx_conversations_extra_public_agent_id;

-- `ALTER TABLE channel_plugins DROP COLUMN public_agent_id` is rejected by
-- SQLite because the baseline table-level CHECK
-- `(companion_id IS NULL OR public_agent_id IS NULL)` still references the
-- column, so the table is rebuilt without the column and without that CHECK.
-- The `id` values are copied verbatim; any stored public-agent binding value
-- is intentionally dropped (breaking change recorded in the CHANGELOG).
CREATE TABLE channel_plugins_new (
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
    updated_at        INTEGER NOT NULL,
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
INSERT INTO channel_plugins_new (
    id, channel_plugin_id, type, name, enabled, config, status,
    last_connected, companion_id, bot_key, created_at, updated_at
)
SELECT
    id, channel_plugin_id, type, name, enabled, config, status,
    last_connected, companion_id, bot_key, created_at, updated_at
FROM channel_plugins;
DROP TABLE channel_plugins;
ALTER TABLE channel_plugins_new RENAME TO channel_plugins;
CREATE INDEX idx_channel_plugins_companion_id ON channel_plugins(companion_id);
CREATE UNIQUE INDEX uq_channel_plugins_type_bot_key
    ON channel_plugins(type, bot_key) WHERE bot_key IS NOT NULL;
