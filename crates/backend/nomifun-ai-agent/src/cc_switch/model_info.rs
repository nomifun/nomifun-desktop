use std::collections::HashMap;
use std::fs;
use std::path::Path;

use nomifun_api_types::{ModelInfoEntry, ModelInfoPayload};
use rusqlite::Connection;
use tracing::warn;

use super::CcSwitchPaths;

pub(crate) fn normalize_claude_model_slot(value: &str) -> Option<&'static str> {
    match value.trim().to_lowercase().as_str() {
        "sonnet" | "default" => Some("default"),
        "opus" => Some("opus"),
        "haiku" => Some("haiku"),
        _ => None,
    }
}

fn read_claude_selected_slot(claude_settings_path: &Path) -> Option<&'static str> {
    let content = fs::read_to_string(claude_settings_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    let model_str = parsed.get("model")?.as_str()?;
    normalize_claude_model_slot(model_str)
}

pub fn build_model_info_from_env(
    env: &HashMap<String, String>,
    labels: &HashMap<String, String>,
    active_slot: Option<&str>,
) -> Option<ModelInfoPayload> {
    let default_model = env
        .get("ANTHROPIC_DEFAULT_SONNET_MODEL")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| env.get("ANTHROPIC_MODEL").filter(|s| !s.trim().is_empty()));
    let opus_model = env.get("ANTHROPIC_DEFAULT_OPUS_MODEL").filter(|s| !s.trim().is_empty());
    let haiku_model = env
        .get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
        .filter(|s| !s.trim().is_empty());

    let mut available = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let candidates = [("default", default_model), ("opus", opus_model), ("haiku", haiku_model)];

    for (slot, model_id_opt) in &candidates {
        if let Some(model_id) = model_id_opt
            && seen.insert(model_id.as_str())
        {
            let label = labels
                .get(model_id.as_str())
                .cloned()
                .unwrap_or_else(|| (*model_id).clone());
            available.push(ModelInfoEntry {
                id: slot.to_string(),
                label,
            });
        }
    }

    if available.is_empty() {
        return None;
    }

    let preferred_slot = active_slot.and_then(normalize_claude_model_slot).unwrap_or("default");
    let current_model_id = available
        .iter()
        .find(|m| m.id == preferred_slot)
        .map(|m| m.id.clone())
        .unwrap_or_else(|| available[0].id.clone());
    let current_model_label = available
        .iter()
        .find(|m| m.id == current_model_id)
        .map(|m| m.label.clone());

    Some(ModelInfoPayload {
        current_model_id: Some(current_model_id),
        current_model_label,
        available_models: available,
    })
}

fn read_model_labels(conn: &Connection) -> HashMap<String, String> {
    let mut stmt = match conn.prepare("SELECT model_id, display_name FROM model_pricing") {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };
    let rows = stmt
        .query_map([], |row| {
            let model_id: String = row.get(0)?;
            let display_name: Option<String> = row.get(1)?;
            Ok((model_id, display_name))
        })
        .ok();

    let Some(rows) = rows else {
        return HashMap::new();
    };

    rows.filter_map(|r| r.ok())
        .filter(|(id, _)| !id.trim().is_empty())
        .map(|(id, name)| {
            let label = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| id.clone());
            (id, label)
        })
        .collect()
}

pub fn read_claude_model_info_with_paths(paths: &CcSwitchPaths) -> Option<ModelInfoPayload> {
    let settings_content = fs::read_to_string(&paths.settings_path).ok()?;
    let settings: serde_json::Value = serde_json::from_str(&settings_content).ok()?;
    let provider_id = settings
        .get("currentProviderClaude")?
        .as_str()
        .filter(|s| !s.trim().is_empty())?;

    if !paths.database_path.exists() {
        return None;
    }

    let conn = Connection::open_with_flags(&paths.database_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| warn!(error = %e, "cc-switch: failed to open database for model info"))
        .ok()?;

    let settings_config_json: String = conn
        .query_row(
            "SELECT settings_config FROM providers WHERE id = ?1 AND app_type = 'claude' LIMIT 1",
            [provider_id],
            |row| row.get(0),
        )
        .ok()?;

    let config: serde_json::Value = serde_json::from_str(&settings_config_json).ok()?;
    let env_obj = config.get("env")?.as_object()?;

    let env: HashMap<String, String> = env_obj
        .iter()
        .filter_map(|(k, v)| {
            v.as_str()
                .filter(|s| !s.trim().is_empty())
                .map(|s| (k.clone(), s.to_owned()))
        })
        .collect();

    let labels = read_model_labels(&conn);
    let active_slot = read_claude_selected_slot(&paths.claude_settings_path)
        .or_else(|| config.get("model")?.as_str().and_then(normalize_claude_model_slot));

    build_model_info_from_env(&env, &labels, active_slot)
}

pub fn read_claude_model_info() -> Option<ModelInfoPayload> {
    let paths = CcSwitchPaths::system()?;
    read_claude_model_info_with_paths(&paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_model_info_from_env_with_all_slots() {
        let mut env = HashMap::new();
        env.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), "deepseek-v4-pro".into());
        env.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), "deepseek-v4-max".into());
        env.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), "deepseek-v4-lite".into());

        let labels = HashMap::from([
            ("deepseek-v4-pro".to_owned(), "DeepSeek V4 Pro".to_owned()),
            ("deepseek-v4-max".to_owned(), "DeepSeek V4 Max".to_owned()),
            ("deepseek-v4-lite".to_owned(), "DeepSeek V4 Lite".to_owned()),
        ]);

        let info = build_model_info_from_env(&env, &labels, None);
        assert!(info.is_some());

        let payload = info.unwrap();
        assert_eq!(payload.available_models.len(), 3);
        assert_eq!(payload.current_model_id.as_deref(), Some("default"));
        assert_eq!(payload.current_model_label.as_deref(), Some("DeepSeek V4 Pro"));
    }

    #[test]
    fn builds_model_info_single_model() {
        let mut env = HashMap::new();
        env.insert("ANTHROPIC_MODEL".into(), "glm-5.1x".into());

        let info = build_model_info_from_env(&env, &HashMap::new(), None);
        assert!(info.is_some());

        let payload = info.unwrap();
        assert_eq!(payload.available_models.len(), 1);
        assert_eq!(payload.available_models[0].id, "default");
        assert_eq!(payload.available_models[0].label, "glm-5.1x");
    }

    #[test]
    fn returns_none_when_no_model_env_vars() {
        let env = HashMap::new();
        let info = build_model_info_from_env(&env, &HashMap::new(), None);
        assert!(info.is_none());
    }

    #[test]
    fn respects_active_slot_override() {
        let mut env = HashMap::new();
        env.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), "model-a".into());
        env.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), "model-b".into());

        let info = build_model_info_from_env(&env, &HashMap::new(), Some("opus"));
        assert!(info.is_some());
        assert_eq!(info.unwrap().current_model_id.as_deref(), Some("opus"));
    }

    #[test]
    fn normalize_slot_maps_sonnet_to_default() {
        assert_eq!(normalize_claude_model_slot("sonnet"), Some("default"));
        assert_eq!(normalize_claude_model_slot("default"), Some("default"));
        assert_eq!(normalize_claude_model_slot("opus"), Some("opus"));
        assert_eq!(normalize_claude_model_slot("haiku"), Some("haiku"));
        assert_eq!(normalize_claude_model_slot("unknown"), None);
        assert_eq!(normalize_claude_model_slot(""), None);
    }
}
