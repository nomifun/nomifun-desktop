-- Remote hardening batch 2 (spec D1/D2).
--
-- `channel_pending_prompts` persists IM prompts that arrived while their
-- bound conversation was busy. The channel queue drain delivers them strictly
-- FIFO per conversation once the running turn completes; rows are settled
-- (delivered / expired / cancelled / failed) rather than deleted so a crash
-- between delivery and settlement cannot resurrect an absorbed prompt.
--
-- `conversation_delivery_notify` registers a requester conversation that
-- asked for a completion receipt of one keyed turn operation
-- (`nomi_send_to_conversation` with `notify_back=true`). It is deliberately a
-- separate small table: `conversation_delivery_receipts` keeps its
-- identity-immutable trigger red line untouched.

CREATE TABLE channel_pending_prompts (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    prompt_id           TEXT NOT NULL UNIQUE
                        CHECK (
                            length(prompt_id) = 36
                            AND lower(prompt_id) = prompt_id
                            AND prompt_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(prompt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    channel_plugin_id   TEXT NOT NULL
                        CHECK (
                            length(channel_plugin_id) = 36
                            AND lower(channel_plugin_id) = channel_plugin_id
                            AND channel_plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(channel_plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    chat_id             TEXT NOT NULL CHECK (length(chat_id) BETWEEN 1 AND 512),
    channel_session_id  TEXT NOT NULL
                        CHECK (
                            length(channel_session_id) = 36
                            AND lower(channel_session_id) = channel_session_id
                            AND channel_session_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(channel_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    conversation_id     TEXT NOT NULL
                        CHECK (
                            length(conversation_id) = 36
                            AND lower(conversation_id) = conversation_id
                            AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    text                TEXT NOT NULL,
    idempotency_key     TEXT NOT NULL,
    state               TEXT NOT NULL DEFAULT 'queued'
                        CHECK (state IN ('queued','delivered','expired','cancelled','failed')),
    attempts            INTEGER NOT NULL DEFAULT 0,
    queued_at           INTEGER NOT NULL,
    settled_at          INTEGER
);

CREATE INDEX idx_cpp_conversation_state
    ON channel_pending_prompts(conversation_id, state, id);
CREATE INDEX idx_cpp_plugin_chat
    ON channel_pending_prompts(channel_plugin_id, chat_id, state);
CREATE INDEX idx_cpp_session
    ON channel_pending_prompts(channel_session_id);

CREATE TABLE conversation_delivery_notify (
    id                          INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id                TEXT NOT NULL UNIQUE,
    requester_conversation_id   TEXT NOT NULL
                                CHECK (
                                    length(requester_conversation_id) = 36
                                    AND lower(requester_conversation_id) = requester_conversation_id
                                    AND requester_conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
                                    AND replace(requester_conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                                ),
    state                       TEXT NOT NULL DEFAULT 'pending'
                                CHECK (state IN ('pending','notified','failed')),
    created_at                  INTEGER NOT NULL,
    settled_at                  INTEGER
);

CREATE INDEX idx_cdn_requester
    ON conversation_delivery_notify(requester_conversation_id);
