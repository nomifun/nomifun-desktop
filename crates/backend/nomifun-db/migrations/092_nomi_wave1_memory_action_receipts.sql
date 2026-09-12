-- Durable exactly-once receipts for Nomi Wave 1 Companion-memory mutations.
--
-- The receipt key is scoped by the installation owner, Nomi Session,
-- capability and Kernel-issued idempotency key. `request_digest` also covers
-- the exact selected Companion resource, so an idempotency key cannot be
-- replayed against a different target after a Session binding change.
--
-- An abandoned `in_flight` receipt is deliberately not retried by another
-- process. The next process promotes it to `outcome_unknown`, fencing a
-- mutation that may have committed to the Companion store before the process
-- stopped. Terminal results are retained for exact replay.

CREATE TABLE IF NOT EXISTS nomi_wave1_memory_action_receipts (
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
    capability_id     TEXT NOT NULL CHECK (
        length(capability_id) BETWEEN 1 AND 256
        AND capability_id NOT GLOB '*[^!-~]*'
    ),
    idempotency_key   TEXT NOT NULL CHECK (
        length(idempotency_key) BETWEEN 1 AND 256
        AND idempotency_key NOT GLOB '*[^!-~]*'
    ),
    request_digest    TEXT NOT NULL CHECK (
        length(request_digest) = 64
        AND lower(request_digest) = request_digest
        AND request_digest NOT GLOB '*[^0-9a-f]*'
    ),
    state             TEXT NOT NULL CHECK (
        state IN ('in_flight', 'completed', 'failed', 'outcome_unknown')
    ),
    process_lease_id  TEXT CHECK (
        process_lease_id IS NULL OR (
            length(process_lease_id) = 36
            AND lower(process_lease_id) = process_lease_id
            AND process_lease_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(process_lease_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    output_json       TEXT CHECK (
        output_json IS NULL OR json_valid(output_json)
    ),
    error_code        TEXT CHECK (
        error_code IS NULL OR (
            length(error_code) BETWEEN 1 AND 256
            AND error_code NOT GLOB '*[^!-~]*'
        )
    ),
    error_message     TEXT CHECK (
        error_message IS NULL OR length(error_message) BETWEEN 1 AND 4096
    ),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL CHECK (updated_at >= created_at),
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

CREATE INDEX IF NOT EXISTS idx_nomi_wave1_memory_receipts_owner_user_id
    ON nomi_wave1_memory_action_receipts(owner_user_id);

CREATE INDEX IF NOT EXISTS idx_nomi_wave1_memory_receipts_agent_session_id
    ON nomi_wave1_memory_action_receipts(agent_session_id);

CREATE INDEX IF NOT EXISTS idx_nomi_wave1_memory_receipts_orphan_sweep
    ON nomi_wave1_memory_action_receipts(updated_at, agent_session_id);
