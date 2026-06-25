-- 004_channel_scoped_pairing.sql
-- 配对/授权域从全局 (platform_user_id, platform_type) 收敛到 per-bot:
-- 两张表加 channel_id + FK→assistant_plugins(id) ON DELETE CASCADE,
-- 唯一约束改为含 channel_id。回填到对应平台"最可能在用"的那一行。

-- ── 1) assistant_users 重建（子表 assistant_sessions 需备份/恢复）──
CREATE TEMP TABLE _sessions_backup AS SELECT * FROM assistant_sessions;

CREATE TABLE assistant_users_new (
    id               TEXT PRIMARY KEY NOT NULL,
    platform_user_id TEXT    NOT NULL,
    platform_type    TEXT    NOT NULL,
    channel_id       TEXT,
    display_name     TEXT,
    authorized_at    INTEGER NOT NULL,
    last_active      INTEGER,
    session_id       TEXT,
    UNIQUE (platform_user_id, platform_type, channel_id),
    FOREIGN KEY (channel_id) REFERENCES assistant_plugins(id) ON DELETE CASCADE
);

INSERT INTO assistant_users_new
    (id, platform_user_id, platform_type, channel_id, display_name, authorized_at, last_active, session_id)
SELECT u.id, u.platform_user_id, u.platform_type,
       (SELECT p.id FROM assistant_plugins p
         WHERE p.type = u.platform_type
         ORDER BY (p.companion_id IS NOT NULL) DESC, p.created_at ASC
         LIMIT 1),
       u.display_name, u.authorized_at, u.last_active, u.session_id
FROM assistant_users u;

DROP TABLE assistant_users;
ALTER TABLE assistant_users_new RENAME TO assistant_users;

DELETE FROM assistant_sessions;
INSERT INTO assistant_sessions SELECT * FROM _sessions_backup;
DROP TABLE _sessions_backup;

CREATE INDEX IF NOT EXISTS idx_assistant_sessions_user_id ON assistant_sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_assistant_sessions_user_chat ON assistant_sessions(user_id, chat_id);
CREATE INDEX IF NOT EXISTS idx_assistant_sessions_channel ON assistant_sessions(channel_id);
CREATE INDEX IF NOT EXISTS idx_assistant_users_channel ON assistant_users(channel_id);

-- ── 2) assistant_pairing_codes 重建（无子表）──
CREATE TABLE assistant_pairing_codes_new (
    code             TEXT PRIMARY KEY NOT NULL,
    platform_user_id TEXT    NOT NULL,
    platform_type    TEXT    NOT NULL,
    channel_id       TEXT,
    display_name     TEXT,
    requested_at     INTEGER NOT NULL,
    expires_at       INTEGER NOT NULL,
    status           TEXT    NOT NULL DEFAULT 'pending'
                             CHECK (status IN ('pending', 'approved', 'rejected', 'expired')),
    FOREIGN KEY (channel_id) REFERENCES assistant_plugins(id) ON DELETE CASCADE
);

INSERT INTO assistant_pairing_codes_new
    (code, platform_user_id, platform_type, channel_id, display_name, requested_at, expires_at, status)
SELECT c.code, c.platform_user_id, c.platform_type,
       (SELECT p.id FROM assistant_plugins p
         WHERE p.type = c.platform_type
         ORDER BY (p.companion_id IS NOT NULL) DESC, p.created_at ASC
         LIMIT 1),
       c.display_name, c.requested_at, c.expires_at, c.status
FROM assistant_pairing_codes c;

DROP TABLE assistant_pairing_codes;
ALTER TABLE assistant_pairing_codes_new RENAME TO assistant_pairing_codes;

CREATE INDEX IF NOT EXISTS idx_pairing_codes_status ON assistant_pairing_codes(status);
CREATE INDEX IF NOT EXISTS idx_pairing_codes_channel ON assistant_pairing_codes(channel_id);
