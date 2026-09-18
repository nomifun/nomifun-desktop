-- One official Runtime remains. Replace the retired selector-era trigger with
-- a fence over the host-written exact build identity. The uppercase identifier
-- below names only the pre-existing object that must be removed on upgrades.
DROP TRIGGER IF EXISTS TRG_CONVERSATION_RUNTIME_ENGINE_IMMUTABLE;

DROP TRIGGER IF EXISTS trg_conversation_runtime_build_immutable;
CREATE TRIGGER trg_conversation_runtime_build_immutable
BEFORE UPDATE OF extra ON conversations
WHEN json_extract(OLD.extra, '$.runtime_build_binding') IS NOT
     json_extract(NEW.extra, '$.runtime_build_binding')
BEGIN
    SELECT RAISE(ABORT, 'Conversation Runtime build is immutable; fork explicitly');
END;
