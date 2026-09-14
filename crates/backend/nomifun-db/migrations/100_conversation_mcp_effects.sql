-- Owner receipts, not a second Session/turn authority. A pending row means
-- the remote transaction may have started; restart MUST NOT replay it.
-- No arguments, credentials, response bodies or transport URLs are persisted.
CREATE TABLE conversation_mcp_effects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    turn_operation_id TEXT NOT NULL,
    admission_epoch INTEGER NOT NULL,
    capability_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending', 'settled')),
    created_at INTEGER NOT NULL,
    settled_at INTEGER,
    UNIQUE(user_id, conversation_id, operation_id),
    CHECK((state = 'pending' AND settled_at IS NULL) OR
          (state = 'settled' AND settled_at IS NOT NULL))
);
CREATE UNIQUE INDEX idx_conversation_mcp_pending
    ON conversation_mcp_effects(user_id, conversation_id) WHERE state = 'pending';
CREATE INDEX idx_conversation_mcp_turn
    ON conversation_mcp_effects(turn_operation_id, conversation_id);
CREATE TRIGGER trg_conversation_mcp_effect_admission
BEFORE INSERT ON conversation_mcp_effects
WHEN NEW.state != 'pending' OR NOT EXISTS (
    SELECT 1 FROM conversations c
    JOIN conversation_delivery_receipts r ON r.operation_id = c.active_turn_operation_id
    WHERE c.conversation_id = NEW.conversation_id AND c.user_id = NEW.user_id
      AND c.status = 'running' AND c.admission_epoch = NEW.admission_epoch
      AND c.active_turn_operation_id = NEW.turn_operation_id
      AND r.conversation_id = c.conversation_id AND r.user_id = c.user_id
      AND r.kind = 'turn' AND r.status = 'accepted'
)
BEGIN
    SELECT RAISE(ABORT, 'MCP effect requires an exact live Conversation turn');
END;
CREATE TRIGGER trg_conversation_mcp_effect_update
BEFORE UPDATE ON conversation_mcp_effects
WHEN NEW.id IS NOT OLD.id OR NEW.user_id IS NOT OLD.user_id
  OR NEW.conversation_id IS NOT OLD.conversation_id OR NEW.operation_id IS NOT OLD.operation_id
  OR NEW.turn_operation_id IS NOT OLD.turn_operation_id OR NEW.admission_epoch IS NOT OLD.admission_epoch
  OR NEW.capability_id IS NOT OLD.capability_id OR NEW.created_at IS NOT OLD.created_at
  OR OLD.state != 'pending' OR NEW.state != 'settled' OR NEW.settled_at IS NULL
BEGIN
    SELECT RAISE(ABORT, 'MCP effect permits only exact pending to settled transition');
END;
CREATE TRIGGER trg_conversation_mcp_effect_no_delete
BEFORE DELETE ON conversation_mcp_effects
BEGIN
    SELECT RAISE(ABORT, 'MCP effect receipts are retained indefinitely');
END;
