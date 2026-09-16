-- Append after the e617feb2b migration head (103). Historical 060/080 SQL
-- remains immutable so canonical and authenticated displaced prefixes upgrade.
-- Existing rows gain no UI consent or conversation access implicitly.
ALTER TABLE nomi_agent_presets
    ADD COLUMN ui_binding_json TEXT NOT NULL DEFAULT '{"binding_version":0,"selection":null}' CHECK (
        json_valid(ui_binding_json)
        AND json_type(ui_binding_json) = 'object'
        AND json_type(ui_binding_json, '$.binding_version') IS 'integer'
        AND json_extract(ui_binding_json, '$.binding_version') >= 0
        AND coalesce(json_type(ui_binding_json, '$.selection') IN ('object', 'null'), 0)
    );

CREATE INDEX idx_nomi_agent_presets_ui_plugin
    ON nomi_agent_presets(json_extract(ui_binding_json, '$.selection.plugin_id'));

-- Optional explicit UI access grant, not an Agent execution owner.
ALTER TABLE plugin_surface_sessions
    ADD COLUMN conversation_id TEXT CHECK (
        conversation_id IS NULL OR (
            length(conversation_id) = 36
            AND lower(conversation_id) = conversation_id
            AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    );

CREATE INDEX idx_plugin_surface_sessions_conversation_id
    ON plugin_surface_sessions(conversation_id);
