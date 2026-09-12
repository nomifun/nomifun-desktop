-- Durable customer-service handoff queue used by the official Customer Service
-- Agent capability. A handoff is an owned domain fact, not a transient tool
-- acknowledgement: retries converge through idempotency_key and one dialogue
-- can have at most one active (pending/claimed) handoff.

CREATE TABLE cs_handoffs (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    cs_handoff_id     TEXT NOT NULL UNIQUE
                      CHECK (
                          length(cs_handoff_id) = 36
                          AND lower(cs_handoff_id) = cs_handoff_id
                          AND cs_handoff_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(cs_handoff_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    cs_agent_id       TEXT NOT NULL
                      CHECK (
                          length(cs_agent_id) = 36
                          AND lower(cs_agent_id) = cs_agent_id
                          AND cs_agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(cs_agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    cs_dialogue_id    TEXT NOT NULL
                      CHECK (
                          length(cs_dialogue_id) = 36
                          AND lower(cs_dialogue_id) = cs_dialogue_id
                          AND cs_dialogue_id GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(cs_dialogue_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    requested_by      TEXT NOT NULL
                      CHECK (
                          length(requested_by) = 36
                          AND lower(requested_by) = requested_by
                          AND requested_by GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(requested_by, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    idempotency_key   TEXT NOT NULL UNIQUE CHECK (length(idempotency_key) BETWEEN 1 AND 512),
    reason            TEXT NOT NULL DEFAULT '' CHECK (length(reason) <= 4000),
    summary           TEXT NOT NULL DEFAULT '' CHECK (length(summary) <= 12000),
    status            TEXT NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending', 'claimed', 'resolved', 'cancelled')),
    claimed_by        TEXT
                      CHECK (
                          claimed_by IS NULL
                          OR (
                              length(claimed_by) = 36
                              AND lower(claimed_by) = claimed_by
                              AND claimed_by GLOB '????????-????-7???-[89ab]???-????????????'
                              AND replace(claimed_by, '-', '') NOT GLOB '*[^0-9a-f]*'
                          )
                      ),
    updated_by        TEXT NOT NULL
                      CHECK (
                          length(updated_by) = 36
                          AND lower(updated_by) = updated_by
                          AND updated_by GLOB '????????-????-7???-[89ab]???-????????????'
                          AND replace(updated_by, '-', '') NOT GLOB '*[^0-9a-f]*'
                      ),
    resolution        TEXT NOT NULL DEFAULT '' CHECK (length(resolution) <= 12000),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

CREATE INDEX idx_cs_handoffs_agent_status
    ON cs_handoffs(cs_agent_id, status, created_at DESC);
CREATE INDEX idx_cs_handoffs_dialogue
    ON cs_handoffs(cs_dialogue_id, created_at DESC);
CREATE INDEX idx_cs_handoffs_requested_by
    ON cs_handoffs(requested_by, created_at DESC);
CREATE INDEX idx_cs_handoffs_claimed_by
    ON cs_handoffs(claimed_by, updated_at DESC)
    WHERE claimed_by IS NOT NULL;
CREATE INDEX idx_cs_handoffs_updated_by
    ON cs_handoffs(updated_by, updated_at DESC);
CREATE UNIQUE INDEX uq_cs_handoffs_active_dialogue
    ON cs_handoffs(cs_dialogue_id)
    WHERE status IN ('pending', 'claimed');

-- Atomic exactly-once receipts for owner-scoped customer-service Agent
-- actions. The note mutation and its committed result_json are written in one
-- SQLite transaction; there is no success receipt without the effect and no
-- committed effect without a replayable receipt.
CREATE TABLE cs_agent_capability_receipts (
    id                         INTEGER PRIMARY KEY AUTOINCREMENT,
    cs_agent_capability_receipt_id TEXT NOT NULL UNIQUE
                               CHECK (
                                   length(cs_agent_capability_receipt_id) = 36
                                   AND lower(cs_agent_capability_receipt_id) = cs_agent_capability_receipt_id
                                   AND cs_agent_capability_receipt_id GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(cs_agent_capability_receipt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                               ),
    owner_user_id              TEXT NOT NULL
                               CHECK (
                                   length(owner_user_id) = 36
                                   AND lower(owner_user_id) = owner_user_id
                                   AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                               ),
    cs_agent_id                TEXT NOT NULL
                               CHECK (
                                   length(cs_agent_id) = 36
                                   AND lower(cs_agent_id) = cs_agent_id
                                   AND cs_agent_id GLOB '????????-????-7???-[89ab]???-????????????'
                                   AND replace(cs_agent_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                               ),
    capability_id              TEXT NOT NULL CHECK (length(capability_id) BETWEEN 1 AND 256),
    idempotency_key            TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 512),
    request_digest             TEXT NOT NULL
                               CHECK (
                                   length(request_digest) = 64
                                   AND lower(request_digest) = request_digest
                                   AND request_digest NOT GLOB '*[^0-9a-f]*'
                               ),
    result_json                TEXT NOT NULL CHECK (json_valid(result_json)),
    created_at                 INTEGER NOT NULL,
    UNIQUE(owner_user_id, capability_id, idempotency_key)
);

CREATE INDEX idx_cs_agent_capability_receipts_agent
    ON cs_agent_capability_receipts(cs_agent_id, capability_id, created_at DESC);
CREATE INDEX idx_cs_agent_capability_receipts_owner
    ON cs_agent_capability_receipts(owner_user_id, created_at DESC);
