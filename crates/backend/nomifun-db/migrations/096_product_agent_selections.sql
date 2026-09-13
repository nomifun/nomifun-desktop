-- Authoring intent is independent of a runnable model-bound AgentBinding.
-- Existing immutable bindings remain intact; new intent is materialized only
-- after a compatible model has been selected.
CREATE TABLE product_agent_selections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36 AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    target_kind TEXT NOT NULL CHECK (target_kind IN ('companion', 'robot', 'customer', 'creative_studio_canvas')),
    target_id TEXT NOT NULL CHECK (length(trim(target_id)) > 0),
    selection_json TEXT NOT NULL CHECK (json_valid(selection_json) AND json_type(selection_json) = 'object'),
    UNIQUE (owner_user_id, target_kind, target_id)
);
CREATE INDEX idx_product_agent_selections_owner_user_id ON product_agent_selections(owner_user_id);
