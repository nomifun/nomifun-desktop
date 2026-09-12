-- Durable idempotency ledger for Nomi-core Wave 4 Companion/Channel actions.
--
-- `in_flight` rows carry the process lease that admitted the effect. A later
-- process never retries such a row: it promotes it to `outcome_unknown`.
-- Terminal rows retain the original result/error for exact replays. Rows are
-- owner/session scoped and are removed only with their owning Nomi Session.

CREATE TABLE IF NOT EXISTS nomi_wave4_action_receipts (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_user_id     TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    agent_session_id  TEXT NOT NULL CHECK (
        length(agent_session_id) = 36
        AND lower(agent_session_id) = agent_session_id
        AND agent_session_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(agent_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    capability_id     TEXT NOT NULL,
    idempotency_key   TEXT NOT NULL,
    request_digest    TEXT NOT NULL,
    state             TEXT NOT NULL CHECK (
        state IN ('in_flight', 'completed', 'failed', 'outcome_unknown')
    ),
    process_lease_id  TEXT,
    output_json       TEXT,
    error_code        TEXT,
    error_message     TEXT,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    UNIQUE (
        owner_user_id,
        agent_session_id,
        capability_id,
        idempotency_key
    ),
    CHECK (
        (state = 'in_flight'
            AND process_lease_id IS NOT NULL
            AND output_json IS NULL
            AND error_code IS NULL
            AND error_message IS NULL)
        OR
        (state = 'completed'
            AND process_lease_id IS NULL
            AND output_json IS NOT NULL
            AND error_code IS NULL
            AND error_message IS NULL)
        OR
        (state = 'failed'
            AND process_lease_id IS NULL
            AND output_json IS NULL
            AND error_code IS NOT NULL
            AND error_message IS NOT NULL)
        OR
        (state = 'outcome_unknown'
            AND process_lease_id IS NULL
            AND output_json IS NULL
            AND error_code IS NULL
            AND error_message IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_nomi_wave4_receipts_owner_user_id
    ON nomi_wave4_action_receipts(owner_user_id);

CREATE INDEX IF NOT EXISTS idx_nomi_wave4_receipts_agent_session_id
    ON nomi_wave4_action_receipts(agent_session_id);

CREATE INDEX IF NOT EXISTS idx_nomi_wave4_receipts_orphan_sweep
    ON nomi_wave4_action_receipts(updated_at, agent_session_id);
