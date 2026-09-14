-- Extend retained hosted receipts without changing migration 102/checksums.
-- The migrator executes this rebuild transactionally. No old receipt is lost;
-- Git's workspace key is a platform-computed digest, never a model parameter.
CREATE TABLE conversation_hosted_effects_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    turn_operation_id TEXT NOT NULL,
    admission_epoch INTEGER NOT NULL,
    owner_domain TEXT NOT NULL CHECK(owner_domain IN ('miniapp', 'robot', 'git')),
    capability_id TEXT NOT NULL,
    action_name TEXT NOT NULL CHECK(length(CAST(action_name AS BLOB)) BETWEEN 1 AND 1024),
    input_sha256 TEXT NOT NULL CHECK(length(input_sha256) = 64 AND input_sha256 NOT GLOB '*[^0-9a-f]*'),
    resource_key TEXT,
    state TEXT NOT NULL CHECK(state IN ('pending', 'returned', 'rejected')),
    created_at INTEGER NOT NULL,
    settled_at INTEGER,
    observation_json TEXT CHECK(observation_json IS NULL OR
        (json_valid(observation_json) AND length(CAST(observation_json AS BLOB)) <= 8192)),
    UNIQUE(user_id, conversation_id, operation_id),
    CHECK((owner_domain = 'git' AND resource_key IS NOT NULL AND length(resource_key) = 64
            AND resource_key NOT GLOB '*[^0-9a-f]*' AND capability_id = 'vcs.push' AND action_name = 'vcs.push.invoke') OR
          (owner_domain != 'git' AND resource_key IS NULL)),
    CHECK((state = 'pending' AND settled_at IS NULL AND observation_json IS NULL) OR
          (state != 'pending' AND settled_at IS NOT NULL AND observation_json IS NOT NULL))
);
INSERT INTO conversation_hosted_effects_next
    (id, user_id, conversation_id, operation_id, turn_operation_id, admission_epoch,
     owner_domain, capability_id, action_name, input_sha256, state, created_at, settled_at, observation_json)
SELECT id, user_id, conversation_id, operation_id, turn_operation_id, admission_epoch,
       owner_domain, capability_id, action_name, input_sha256, state, created_at, settled_at, observation_json
FROM conversation_hosted_effects;
DROP TRIGGER trg_conversation_hosted_effect_admission;
DROP TRIGGER trg_conversation_hosted_effect_update;
DROP TRIGGER trg_conversation_hosted_effect_no_delete;
DROP TABLE conversation_hosted_effects;
ALTER TABLE conversation_hosted_effects_next RENAME TO conversation_hosted_effects;
CREATE UNIQUE INDEX idx_conversation_hosted_pending
    ON conversation_hosted_effects(user_id, conversation_id) WHERE state = 'pending';
-- An unknown push remains fenced even in a different Session after restart.
CREATE UNIQUE INDEX idx_conversation_git_pending
    ON conversation_hosted_effects(resource_key) WHERE owner_domain = 'git' AND state = 'pending';
CREATE INDEX idx_conversation_hosted_turn
    ON conversation_hosted_effects(turn_operation_id, conversation_id);
CREATE INDEX idx_conversation_hosted_history
    ON conversation_hosted_effects(user_id, conversation_id, id DESC);
CREATE TRIGGER trg_conversation_hosted_effect_admission
BEFORE INSERT ON conversation_hosted_effects
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
    SELECT RAISE(ABORT, 'Hosted effect requires an exact live Conversation turn');
END;
CREATE TRIGGER trg_conversation_hosted_effect_update
BEFORE UPDATE ON conversation_hosted_effects
WHEN NEW.id IS NOT OLD.id OR NEW.user_id IS NOT OLD.user_id
  OR NEW.conversation_id IS NOT OLD.conversation_id OR NEW.operation_id IS NOT OLD.operation_id
  OR NEW.turn_operation_id IS NOT OLD.turn_operation_id OR NEW.admission_epoch IS NOT OLD.admission_epoch
  OR NEW.owner_domain IS NOT OLD.owner_domain OR NEW.capability_id IS NOT OLD.capability_id
  OR NEW.action_name IS NOT OLD.action_name OR NEW.input_sha256 IS NOT OLD.input_sha256
  OR NEW.resource_key IS NOT OLD.resource_key
  OR NEW.created_at IS NOT OLD.created_at OR OLD.state != 'pending'
  OR NEW.state NOT IN ('returned', 'rejected') OR NEW.settled_at IS NULL
BEGIN
    SELECT RAISE(ABORT, 'Hosted effect permits only exact pending to terminal transition');
END;
CREATE TRIGGER trg_conversation_hosted_effect_no_delete
BEFORE DELETE ON conversation_hosted_effects
BEGIN
    SELECT RAISE(ABORT, 'Hosted effect receipts are retained indefinitely');
END;
