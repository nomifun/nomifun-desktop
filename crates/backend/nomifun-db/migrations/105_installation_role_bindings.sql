-- One default selection per canonical Role. Built-in Mount IDs are opaque
-- Kernel identities, not necessarily rows in plugin_mounts. Keep stale choices
-- after withdrawal so reinstall/repair never silently selects another Provider.
CREATE TABLE installation_role_bindings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    role_id TEXT NOT NULL UNIQUE CHECK (trim(role_id) <> ''),
    role_contract_ref_json TEXT NOT NULL CHECK (
        json_valid(role_contract_ref_json)
        AND json_type(role_contract_ref_json) = 'object'
        AND json_extract(role_contract_ref_json, '$.key.role_id') = role_id
    ),
    provider_mount_id TEXT NOT NULL CHECK (trim(provider_mount_id) <> ''),
    binding_version INTEGER NOT NULL CHECK (binding_version >= 1),
    updated_at INTEGER NOT NULL
);
CREATE INDEX idx_installation_role_bindings_provider_mount
    ON installation_role_bindings(provider_mount_id);
