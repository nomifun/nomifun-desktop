-- Phase M1-2 durable MiniApp deletion intent.
--
-- The intent is deliberately separate from product_operations so a failed
-- delete keeps one owner-scoped recovery pointer while the non-cancelable
-- operation history remains append-only. No physical foreign keys or triggers
-- are used; ownership and aggregate scope are executable logical references.

CREATE TABLE miniapp_deletion_intents (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    miniapp_id      TEXT NOT NULL UNIQUE CHECK (
        length(miniapp_id) = 36
        AND lower(miniapp_id) = miniapp_id
        AND miniapp_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(miniapp_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id   TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    operation_id    TEXT NOT NULL UNIQUE CHECK (
        length(operation_id) = 36
        AND lower(operation_id) = operation_id
        AND operation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    started_at_ms   INTEGER NOT NULL CHECK (started_at_ms > 0),
    last_error_code TEXT CHECK (
        last_error_code IS NULL OR (
            length(last_error_code) BETWEEN 1 AND 256
            AND last_error_code NOT GLOB '*[^!-~]*'
        )
    ),
    UNIQUE (owner_user_id, miniapp_id)
);

CREATE INDEX idx_miniapp_deletion_intents_owner_user_id
    ON miniapp_deletion_intents(owner_user_id);
CREATE INDEX idx_miniapp_deletion_intents_miniapp_id
    ON miniapp_deletion_intents(miniapp_id);
CREATE INDEX idx_miniapp_deletion_intents_operation_id
    ON miniapp_deletion_intents(operation_id);
