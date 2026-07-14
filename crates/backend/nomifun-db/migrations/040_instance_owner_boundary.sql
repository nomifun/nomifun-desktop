-- Migration 040: make the installation owner's singleton domains impossible
-- to bind to another user's runtime rows.
--
-- Requirement and Knowledge are installation-wide control planes. Their HTTP,
-- Gateway and Agent surfaces are owner-gated, but the database must remain the
-- final authority for background workers and raw SQL callers. The canonical
-- owner is the seeded system-user row; usernames and mutable admin labels are
-- deliberately irrelevant.

-- A legacy Knowledge binding to a non-owner (or missing) runtime target cannot
-- be made safe by changing its audience. Delete the binding; junction rows are
-- removed by ON DELETE CASCADE.
DELETE FROM knowledge_bindings
WHERE target_kind = 'conversation'
  AND NOT EXISTS (
      SELECT 1
      FROM conversations conversation
      WHERE conversation.id = knowledge_bindings.target_conv_id
        AND conversation.user_id = 'system_default_user'
  );

DELETE FROM knowledge_bindings
WHERE target_kind = 'terminal'
  AND NOT EXISTS (
      SELECT 1
      FROM terminal_sessions terminal
      WHERE terminal.id = knowledge_bindings.target_term_id
        AND terminal.user_id = 'system_default_user'
  );

-- Requirements have a polymorphic owner without a foreign key. A legacy
-- cross-owner or orphaned in-progress claim is released back to the queue; all
-- other historical statuses are retained while the invalid owner lease is
-- cleared.
UPDATE requirements
SET status = CASE WHEN status = 'in_progress' THEN 'pending' ELSE status END,
    owner_session_id = NULL,
    owner_kind = NULL,
    active_turn_started_at = NULL,
    lease_expires_at = NULL,
    updated_at = MAX(updated_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
WHERE owner_session_id IS NOT NULL
  AND NOT (
      (owner_kind = 'conversation' AND EXISTS (
          SELECT 1
          FROM conversations conversation
          WHERE conversation.id = requirements.owner_session_id
            AND conversation.user_id = 'system_default_user'
      ))
      OR
      (owner_kind = 'terminal' AND EXISTS (
          SELECT 1
          FROM terminal_sessions terminal
          WHERE terminal.id = requirements.owner_session_id
            AND terminal.user_id = 'system_default_user'
      ))
  );

-- Knowledge binding writes must target a runtime row owned by the canonical
-- installation owner. The table CHECK already validates the discriminated
-- columns; these triggers add the ownership invariant.
CREATE TRIGGER knowledge_binding_owner_insert_guard
BEFORE INSERT ON knowledge_bindings
WHEN (NEW.target_kind = 'conversation' AND NOT EXISTS (
          SELECT 1 FROM conversations conversation
          WHERE conversation.id = NEW.target_conv_id
            AND conversation.user_id = 'system_default_user'
      ))
   OR (NEW.target_kind = 'terminal' AND NOT EXISTS (
          SELECT 1 FROM terminal_sessions terminal
          WHERE terminal.id = NEW.target_term_id
            AND terminal.user_id = 'system_default_user'
      ))
BEGIN
    SELECT RAISE(ABORT, 'knowledge binding target must belong to the installation owner');
END;

CREATE TRIGGER knowledge_binding_owner_update_guard
BEFORE UPDATE OF target_kind, target_conv_id, target_term_id ON knowledge_bindings
WHEN (NEW.target_kind = 'conversation' AND NOT EXISTS (
          SELECT 1 FROM conversations conversation
          WHERE conversation.id = NEW.target_conv_id
            AND conversation.user_id = 'system_default_user'
      ))
   OR (NEW.target_kind = 'terminal' AND NOT EXISTS (
          SELECT 1 FROM terminal_sessions terminal
          WHERE terminal.id = NEW.target_term_id
            AND terminal.user_id = 'system_default_user'
      ))
BEGIN
    SELECT RAISE(ABORT, 'knowledge binding target must belong to the installation owner');
END;

-- Close the reverse direction: a bound runtime row cannot be reassigned away
-- from the owner behind the binding trigger's back.
CREATE TRIGGER knowledge_binding_conversation_owner_rewrite_guard
BEFORE UPDATE OF user_id ON conversations
WHEN NEW.user_id <> 'system_default_user'
 AND EXISTS (
     SELECT 1 FROM knowledge_bindings binding
     WHERE binding.target_kind = 'conversation'
       AND binding.target_conv_id = OLD.id
 )
BEGIN
    SELECT RAISE(ABORT, 'knowledge-bound conversation must remain installation-owner owned');
END;

CREATE TRIGGER knowledge_binding_terminal_owner_rewrite_guard
BEFORE UPDATE OF user_id ON terminal_sessions
WHEN NEW.user_id <> 'system_default_user'
 AND EXISTS (
     SELECT 1 FROM knowledge_bindings binding
     WHERE binding.target_kind = 'terminal'
       AND binding.target_term_id = OLD.id
 )
BEGIN
    SELECT RAISE(ABORT, 'knowledge-bound terminal must remain installation-owner owned');
END;

-- Requirement ownership is a dual-domain soft reference, so both INSERT and
-- UPDATE must prove that the discriminated target exists and belongs to the
-- installation owner. NULL/NULL remains the valid unclaimed state.
CREATE TRIGGER requirement_owner_insert_guard
BEFORE INSERT ON requirements
WHEN NEW.owner_session_id IS NOT NULL
 AND NOT (
     (NEW.owner_kind = 'conversation' AND EXISTS (
         SELECT 1 FROM conversations conversation
         WHERE conversation.id = NEW.owner_session_id
           AND conversation.user_id = 'system_default_user'
     ))
     OR
     (NEW.owner_kind = 'terminal' AND EXISTS (
         SELECT 1 FROM terminal_sessions terminal
         WHERE terminal.id = NEW.owner_session_id
           AND terminal.user_id = 'system_default_user'
     ))
 )
BEGIN
    SELECT RAISE(ABORT, 'requirement owner must belong to the installation owner');
END;

CREATE TRIGGER requirement_owner_update_guard
BEFORE UPDATE OF owner_session_id, owner_kind ON requirements
WHEN NEW.owner_session_id IS NOT NULL
 AND NOT (
     (NEW.owner_kind = 'conversation' AND EXISTS (
         SELECT 1 FROM conversations conversation
         WHERE conversation.id = NEW.owner_session_id
           AND conversation.user_id = 'system_default_user'
     ))
     OR
     (NEW.owner_kind = 'terminal' AND EXISTS (
         SELECT 1 FROM terminal_sessions terminal
         WHERE terminal.id = NEW.owner_session_id
           AND terminal.user_id = 'system_default_user'
     ))
 )
BEGIN
    SELECT RAISE(ABORT, 'requirement owner must belong to the installation owner');
END;

CREATE TRIGGER requirement_conversation_owner_rewrite_guard
BEFORE UPDATE OF user_id ON conversations
WHEN NEW.user_id <> 'system_default_user'
 AND EXISTS (
     SELECT 1 FROM requirements requirement
     WHERE requirement.owner_kind = 'conversation'
       AND requirement.owner_session_id = OLD.id
 )
BEGIN
    SELECT RAISE(ABORT, 'requirement-owning conversation must remain installation-owner owned');
END;

CREATE TRIGGER requirement_terminal_owner_rewrite_guard
BEFORE UPDATE OF user_id ON terminal_sessions
WHEN NEW.user_id <> 'system_default_user'
 AND EXISTS (
     SELECT 1 FROM requirements requirement
     WHERE requirement.owner_kind = 'terminal'
       AND requirement.owner_session_id = OLD.id
 )
BEGIN
    SELECT RAISE(ABORT, 'requirement-owning terminal must remain installation-owner owned');
END;

-- Raw deletes receive the same release semantics as the service-level cleanup:
-- an active requirement returns to pending and no dangling polymorphic owner
-- survives. These run before the row disappears so the update guard can still
-- validate the old owner while clearing it.
CREATE TRIGGER requirement_conversation_delete_release
BEFORE DELETE ON conversations
BEGIN
    UPDATE requirements
    SET status = CASE WHEN status = 'in_progress' THEN 'pending' ELSE status END,
        owner_session_id = NULL,
        owner_kind = NULL,
        active_turn_started_at = NULL,
        lease_expires_at = NULL,
        updated_at = MAX(updated_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
    WHERE owner_kind = 'conversation'
      AND owner_session_id = OLD.id;
END;

CREATE TRIGGER requirement_terminal_delete_release
BEFORE DELETE ON terminal_sessions
BEGIN
    UPDATE requirements
    SET status = CASE WHEN status = 'in_progress' THEN 'pending' ELSE status END,
        owner_session_id = NULL,
        owner_kind = NULL,
        active_turn_started_at = NULL,
        lease_expires_at = NULL,
        updated_at = MAX(updated_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
    WHERE owner_kind = 'terminal'
      AND owner_session_id = OLD.id;
END;
