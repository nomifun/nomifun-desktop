-- Channel bot ownership domains (2026-07-30 customer-service correction).
--
-- Customer-service bots must be a self-contained pool, fully separate from
-- the desktop-companion channel pool. `owner_domain` records which domain
-- owns each `channel_plugins` row:
--   'companion'        — desktop companion channel bots (default, legacy)
--   'customer_service' — bots created and managed by the customer-service
--                        domain; they may never carry a companion binding.
--
-- ADD COLUMN accepts a column-level CHECK; the cross-column mutual-exclusion
-- invariant (cs-domain bot must not carry companion_id) cannot be a table
-- CHECK on ADD COLUMN, so it is enforced by a pair of guard triggers.

ALTER TABLE channel_plugins ADD COLUMN owner_domain TEXT NOT NULL DEFAULT 'companion'
    CHECK (owner_domain IN ('companion', 'customer_service'));

-- Backfill: a bot already bound by customer service and not claimed by a
-- companion moves to the customer-service domain.
UPDATE channel_plugins SET owner_domain = 'customer_service'
 WHERE companion_id IS NULL
   AND channel_plugin_id IN (SELECT channel_plugin_id FROM cs_channel_bindings);

-- Repair invalid state: a bot bound on both sides stays with the companion;
-- its customer-service binding is dropped.
DELETE FROM cs_channel_bindings
 WHERE channel_plugin_id IN (
    SELECT channel_plugin_id FROM channel_plugins WHERE companion_id IS NOT NULL);

-- Mutual-exclusion guards: customer-service bots never carry a companion
-- binding (application-layer validation is the second line of defence).
CREATE TRIGGER trg_channel_plugins_owner_domain_insert_guard
BEFORE INSERT ON channel_plugins
WHEN NEW.owner_domain = 'customer_service' AND NEW.companion_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'customer-service channel bots cannot carry a companion binding');
END;

CREATE TRIGGER trg_channel_plugins_owner_domain_update_guard
BEFORE UPDATE OF owner_domain, companion_id ON channel_plugins
WHEN NEW.owner_domain = 'customer_service' AND NEW.companion_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'customer-service channel bots cannot carry a companion binding');
END;
