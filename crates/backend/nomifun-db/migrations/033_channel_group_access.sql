-- Group-chat authorization is explicit and fail-closed by default. Existing
-- customer-service bots retain their historical stranger-facing behavior.
ALTER TABLE channel_plugins
    ADD COLUMN group_access_mode TEXT NOT NULL DEFAULT 'allowlist'
        CHECK (group_access_mode IN ('all_members', 'allowlist', 'disabled'));

UPDATE channel_plugins
SET group_access_mode = 'all_members'
WHERE owner_domain = 'customer_service';

-- `approved` identities were explicitly paired/authorized and may be used in
-- direct messages. `auto_group` is the non-approved guest identity used for
-- automatic admission (open groups and customer-service direct messages); it
-- can later be promoted without changing its stable user id.
ALTER TABLE channel_users
    ADD COLUMN authorization_kind TEXT NOT NULL DEFAULT 'approved'
        CHECK (authorization_kind IN ('approved', 'auto_group'));

-- Before authorization kinds existed, customer-service bots automatically
-- created a channel user for every stranger they served. Those rows were not
-- explicit approvals: classifying them as `approved` would silently turn the
-- legacy guest population into an allowlist if the operator later tightened
-- the bot from `all_members` to `allowlist`.
UPDATE channel_users
SET authorization_kind = 'auto_group'
WHERE channel_plugin_id IN (
    SELECT channel_plugin_id
    FROM channel_plugins
    WHERE owner_domain = 'customer_service'
);

-- Legacy sessions did not persist whether their chat was direct or grouped.
-- They stay unknown until a subsequently classified inbound event reuses them.
ALTER TABLE channel_sessions
    ADD COLUMN chat_kind TEXT NOT NULL DEFAULT 'unknown'
        CHECK (chat_kind IN ('unknown', 'direct', 'group'));
