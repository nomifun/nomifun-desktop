-- Structured, machine-readable terminal error facts for conversation
-- delivery receipts (spec D4).
--
-- `result_error_code` is a stable snake_case token (AgentErrorCode serde name
-- lowercased, or one of the fixed lifecycle codes such as `empty_final_text`,
-- `turn_cancelled`, `channel_closed`). `result_error_retryable` records
-- whether the failure is safe to retry automatically. Both columns are part
-- of the absorbing terminal outcome: once a receipt is completed they are as
-- immutable as `result_error`. The legacy `result_error` free-text column is
-- retained and still written for existing diagnostics tooling.

ALTER TABLE conversation_delivery_receipts
    ADD COLUMN result_error_code TEXT
        CHECK (result_error_code IS NULL OR trim(result_error_code) <> '');

ALTER TABLE conversation_delivery_receipts
    ADD COLUMN result_error_retryable INTEGER
        CHECK (result_error_retryable IS NULL OR result_error_retryable IN (0, 1));

-- The 012 lifecycle guards enumerate the terminal outcome columns by name, so
-- they are rebuilt here verbatim with the two new columns folded into both the
-- accepted-shape checks and the completed-immutability comparisons.

DROP TRIGGER trg_conversation_delivery_receipts_lifecycle_insert_guard;
DROP TRIGGER trg_conversation_delivery_receipts_lifecycle_update_guard;

CREATE TRIGGER trg_conversation_delivery_receipts_lifecycle_insert_guard
BEFORE INSERT ON conversation_delivery_receipts
WHEN (
    NEW.status = 'accepted'
    AND (
        NEW.result_ok IS NOT NULL
        OR NEW.result_text IS NOT NULL
        OR NEW.result_error IS NOT NULL
        OR NEW.result_error_code IS NOT NULL
        OR NEW.result_error_retryable IS NOT NULL
        OR NEW.completed_at IS NOT NULL
    )
) OR (
    NEW.status = 'completed'
    AND (
        typeof(NEW.completed_at) <> 'integer'
        OR NEW.completed_at < NEW.created_at
        OR typeof(NEW.result_ok) <> 'integer'
        OR NEW.result_ok NOT IN (0, 1)
    )
)
BEGIN
    SELECT RAISE(
        ABORT,
        'Conversation delivery receipt has an invalid lifecycle shape'
    );
END;

CREATE TRIGGER trg_conversation_delivery_receipts_lifecycle_update_guard
BEFORE UPDATE OF status, result_ok, result_text, result_error, result_error_code, result_error_retryable, completed_at
ON conversation_delivery_receipts
WHEN (
    OLD.status = 'completed'
    AND (
        NEW.status IS NOT OLD.status
        OR NEW.result_ok IS NOT OLD.result_ok
        OR NEW.result_text IS NOT OLD.result_text
        OR NEW.result_error IS NOT OLD.result_error
        OR NEW.result_error_code IS NOT OLD.result_error_code
        OR NEW.result_error_retryable IS NOT OLD.result_error_retryable
        OR NEW.completed_at IS NOT OLD.completed_at
    )
) OR (
    NEW.status = 'accepted'
    AND (
        NEW.result_ok IS NOT NULL
        OR NEW.result_text IS NOT NULL
        OR NEW.result_error IS NOT NULL
        OR NEW.result_error_code IS NOT NULL
        OR NEW.result_error_retryable IS NOT NULL
        OR NEW.completed_at IS NOT NULL
    )
) OR (
    NEW.status = 'completed'
    AND (
        typeof(NEW.completed_at) <> 'integer'
        OR NEW.completed_at < NEW.created_at
        OR typeof(NEW.result_ok) <> 'integer'
        OR NEW.result_ok NOT IN (0, 1)
    )
)
BEGIN
    SELECT RAISE(
        ABORT,
        'Conversation delivery receipt lifecycle is absorbing and terminal outcomes are immutable'
    );
END;
