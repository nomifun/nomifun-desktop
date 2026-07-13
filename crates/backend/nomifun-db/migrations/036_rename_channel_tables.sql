-- Replace the legacy product-domain "assistant" table names with the Channel
-- domain vocabulary. SQLite rewrites foreign-key targets when tables are
-- renamed, so existing users, sessions, pairings, plugin bindings and their
-- relationships are preserved in place.
-- Existing ids remain opaque and unchanged; runtime creation switches to the
-- canonical `chn_`, `chu_`, and `chs_` prefixes after this migration.

ALTER TABLE assistant_plugins RENAME TO channel_plugins;
ALTER TABLE assistant_users RENAME TO channel_users;
ALTER TABLE assistant_sessions RENAME TO channel_sessions;
ALTER TABLE assistant_pairing_codes RENAME TO channel_pairing_codes;

-- The application migrator deliberately enables `legacy_alter_table` while
-- migrations run, so renaming the parent table does not rewrite FK targets in
-- child CREATE statements. Rebuild each child explicitly with canonical FK
-- targets while preserving every row.
CREATE TABLE _channel_users_new (
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
INSERT INTO _channel_users_new
    (id, platform_user_id, platform_type, channel_id, display_name,
     authorized_at, last_active, session_id)
SELECT id, platform_user_id, platform_type, channel_id, display_name,
       authorized_at, last_active, session_id
FROM channel_users;
DROP TABLE channel_users;
ALTER TABLE _channel_users_new RENAME TO channel_users;

CREATE TABLE _channel_sessions_new (
    id              TEXT PRIMARY KEY NOT NULL,
    user_id         TEXT    NOT NULL,
    agent_type      TEXT    NOT NULL,
    conversation_id INTEGER,
    workspace       TEXT,
    chat_id         TEXT,
    channel_id      TEXT,
    created_at      INTEGER NOT NULL,
    last_activity   INTEGER NOT NULL,
    FOREIGN KEY (user_id) REFERENCES channel_users(id) ON DELETE CASCADE,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE SET NULL,
    FOREIGN KEY (channel_id) REFERENCES channel_plugins(id) ON DELETE SET NULL
);
INSERT INTO _channel_sessions_new
    (id, user_id, agent_type, conversation_id, workspace, chat_id,
     channel_id, created_at, last_activity)
SELECT id, user_id, agent_type, conversation_id, workspace, chat_id,
       channel_id, created_at, last_activity
FROM channel_sessions;
DROP TABLE channel_sessions;
ALTER TABLE _channel_sessions_new RENAME TO channel_sessions;

CREATE TABLE _channel_pairing_codes_new (
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
INSERT INTO _channel_pairing_codes_new
    (code, platform_user_id, platform_type, channel_id, display_name,
     requested_at, expires_at, status)
SELECT code, platform_user_id, platform_type, channel_id, display_name,
       requested_at, expires_at, status
FROM channel_pairing_codes;
DROP TABLE channel_pairing_codes;
ALTER TABLE _channel_pairing_codes_new RENAME TO channel_pairing_codes;

-- Table renames do not rename indexes. Drop the historical names and recreate
-- them with names that match the owning Channel tables.
DROP INDEX IF EXISTS uq_assistant_plugins_type_bot_key;
DROP INDEX IF EXISTS idx_assistant_sessions_user_id;
DROP INDEX IF EXISTS idx_assistant_sessions_user_chat;
DROP INDEX IF EXISTS idx_assistant_sessions_channel;
DROP INDEX IF EXISTS idx_assistant_users_channel;
DROP INDEX IF EXISTS idx_pairing_codes_status;
DROP INDEX IF EXISTS idx_pairing_codes_channel;

CREATE UNIQUE INDEX uq_channel_plugins_type_bot_key
    ON channel_plugins(type, bot_key) WHERE bot_key IS NOT NULL;
CREATE INDEX idx_channel_sessions_user_id ON channel_sessions(user_id);
CREATE INDEX idx_channel_sessions_user_chat ON channel_sessions(user_id, chat_id);
CREATE INDEX idx_channel_sessions_channel ON channel_sessions(channel_id);
CREATE INDEX idx_channel_users_channel ON channel_users(channel_id);
CREATE INDEX idx_channel_pairing_codes_status ON channel_pairing_codes(status);
CREATE INDEX idx_channel_pairing_codes_channel ON channel_pairing_codes(channel_id);

-- Client preferences used the same historical product prefix. Copy into the
-- canonical Channel namespace first so an already-present canonical value wins,
-- then remove the obsolete keys. This covers every current channel platform
-- and every setting suffix without introducing a runtime read alias.
INSERT OR IGNORE INTO client_preferences (key, value, updated_at)
SELECT 'channels.' || substr(key, 11), value, updated_at
FROM client_preferences
WHERE key GLOB 'assistant.telegram.*'
   OR key GLOB 'assistant.lark.*'
   OR key GLOB 'assistant.dingtalk.*'
   OR key GLOB 'assistant.weixin.*'
   OR key GLOB 'assistant.wecom.*'
   OR key GLOB 'assistant.qqbot.*'
   OR key GLOB 'assistant.discord.*'
   OR key GLOB 'assistant.slack.*'
   OR key GLOB 'assistant.matrix.*'
   OR key GLOB 'assistant.mattermost.*'
   OR key GLOB 'assistant.twitch.*'
   OR key GLOB 'assistant.nostr.*';

DELETE FROM client_preferences
WHERE key GLOB 'assistant.telegram.*'
   OR key GLOB 'assistant.lark.*'
   OR key GLOB 'assistant.dingtalk.*'
   OR key GLOB 'assistant.weixin.*'
   OR key GLOB 'assistant.wecom.*'
   OR key GLOB 'assistant.qqbot.*'
   OR key GLOB 'assistant.discord.*'
   OR key GLOB 'assistant.slack.*'
   OR key GLOB 'assistant.matrix.*'
   OR key GLOB 'assistant.mattermost.*'
   OR key GLOB 'assistant.twitch.*'
   OR key GLOB 'assistant.nostr.*';
