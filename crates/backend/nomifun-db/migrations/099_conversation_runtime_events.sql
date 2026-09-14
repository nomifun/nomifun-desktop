-- Runtime-neutral semantic records subordinate to the existing Conversation
-- and its permanent delivery receipts. This is not another Session owner:
-- admission, cancellation and terminal state remain in conversations/receipts.
CREATE TABLE conversation_runtime_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL,
    turn_operation_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    event_json TEXT NOT NULL CHECK(json_valid(event_json)),
    model_operation_id TEXT UNIQUE,
    model_claimed INTEGER NOT NULL DEFAULT 0 CHECK(model_claimed IN (0, 1)),
    created_at INTEGER NOT NULL,
    UNIQUE(conversation_id, turn_operation_id, sequence)
);
CREATE INDEX idx_conversation_runtime_events_history
    ON conversation_runtime_events(conversation_id, id);
CREATE INDEX idx_conversation_runtime_events_turn
    ON conversation_runtime_events(turn_operation_id);

CREATE TRIGGER trg_conversation_runtime_engine_immutable
BEFORE UPDATE OF extra ON conversations
WHEN json_extract(OLD.extra, '$.runtime_engine_binding') IS NOT
     json_extract(NEW.extra, '$.runtime_engine_binding')
BEGIN
    SELECT RAISE(ABORT, 'Conversation runtime engine is immutable; fork explicitly');
END;

CREATE TRIGGER trg_conversation_runtime_event_owner
BEFORE INSERT ON conversation_runtime_events
WHEN NOT EXISTS (
    SELECT 1 FROM conversation_delivery_receipts r
    WHERE r.operation_id = NEW.turn_operation_id
      AND r.conversation_id = NEW.conversation_id AND r.kind = 'turn'
)
BEGIN
    SELECT RAISE(ABORT, 'Runtime event requires its Conversation turn receipt');
END;
