-- SSH remote sessions: saved, reusable host connection profiles.
--
-- A user stores a remote Linux host's SSH coordinates and (encrypted)
-- credentials here; an ordinary `type='nomi'` conversation binds to one host
-- via `conversations.extra.$.ssh_host_id`, and the agent then operates that
-- host through the remote tool family. Mirrors the `remote_agents` posture:
-- host/port/username in plaintext, every credential column AES-256-GCM
-- encrypted by the service layer (never the repository), masked on read-back,
-- and omitted entirely from list DTOs.
--
-- Invariants (verified against id_schema_contract on every boot):
--   * `id INTEGER PRIMARY KEY AUTOINCREMENT` + a bare UUIDv7 business id with
--     the standard GLOB/length/lowercase CHECK.
--   * No physical FOREIGN KEY; `user_id` is a logical reference (Cascade) and
--     the extra-JSON reference from `conversations` is Restrict + RequireParent
--     (a host cannot be deleted while a session still binds it).
--   * `user_id` is explicit (unlike `remote_agents`, which relies solely on
--     protect_instance_owner) because SSH sessions stay `type='nomi'` and do
--     not inherit the owner-only construction rule; the service filters by it.
--
-- Verification performed: `cargo test -p nomifun-db` boots an in-memory DB,
-- applies this migration, and runs validate_id_schema_contract; a new e2e in
-- nomifun-app exercises owner-scoped CRUD.

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

CREATE INDEX idx_ssh_hosts_user_id ON ssh_hosts(user_id);
CREATE INDEX idx_ssh_hosts_status ON ssh_hosts(status);

-- Logical reference index for conversations.extra.$.ssh_host_id (Restrict).
CREATE INDEX idx_conversations_extra_ssh_host_id
    ON conversations(json_extract(extra, '$.ssh_host_id'));
