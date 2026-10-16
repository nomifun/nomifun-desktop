//! One formal input contract for the installed Companion-memory Actions.
//! Both host adapters use these DTOs and schemas; storage and history remain
//! owned by the Companion and canonical Session services.

use std::collections::BTreeSet;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionMemoryRecallInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_kind: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_budget: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_since: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_until: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_ts: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_ts: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}

impl CompanionMemoryRecallInput {
    pub fn validate(&self) -> Result<(), String> {
        if self.per_kind.is_some_and(|v| !(1..=20).contains(&v))
            || self.char_budget.is_some_and(|v| !(1..=65536).contains(&v))
            || self.limit.is_some_and(|v| !(1..=50).contains(&v))
        { return Err("Companion recall limits exceed the bounded schema".into()); }
        let scope = self.scope.as_deref().unwrap_or("facts");
        if !matches!(scope, "facts" | "history") { return Err("unsupported Companion recall scope".into()); }
        if let Some(query) = self.query.as_deref() { bounded_string(query, 1024, "query")?; }
        let filtered = self.source_session_id.is_some() || self.updated_since.is_some() || self.updated_until.is_some()
            || self.limit.is_some() || self.since_ts.is_some() || self.until_ts.is_some() || self.before.is_some();
        if (scope == "history" || filtered) && self.query.is_none() {
            return Err("filtered/history Companion recall requires query".into());
        }
        if let Some(session_id) = self.source_session_id.as_deref() { canonical_id(session_id, "source_session_id")?; }
        if let Some(cursor) = self.before.as_deref() { bounded_string(cursor, 2048, "before")?; }
        for (start, end) in [(self.updated_since, self.updated_until), (self.since_ts, self.until_ts)] {
            if start.is_some_and(|v| v < 0) || end.is_some_and(|v| v < 0)
                || matches!((start, end), (Some(a), Some(b)) if a > b)
            { return Err("invalid Companion recall time range".into()); }
        }
        if scope == "history" {
            if self.updated_since.is_some() || self.updated_until.is_some() {
                return Err("history recall uses since_ts/until_ts".into());
            }
            if self.char_budget.is_some_and(|v| v < 768) {
                return Err("history recall needs at least 768 bytes including metadata".into());
            }
        } else if self.since_ts.is_some() || self.until_ts.is_some() || self.before.is_some() {
            return Err("since_ts/until_ts/before require history scope".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionMemoryWriteInput {
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<i64>,
}

impl CompanionMemoryWriteInput {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.kind.as_str(), "profile" | "preference" | "knowledge" | "episode" | "task" | "affective") {
            return Err("unsupported Companion memory kind".into());
        }
        bounded_string(&self.content, 16384, "content")?;
        if self.memory_id.is_some() != self.expected_revision.is_some() || self.expected_revision.is_some_and(|v| v < 1) {
            return Err("memory correction requires memory_id and positive expected_revision together".into());
        }
        if let Some(memory_id) = self.memory_id.as_deref() { canonical_id(memory_id, "memory_id")?; }
        if self.memory_id.is_some() && !self.tags.is_empty() {
            return Err("memory corrections change content only; preserve existing kind and omit tags".into());
        }
        if self.tags.len() > 32 { return Err("Companion memory tags exceed 32 entries".into()); }
        let mut unique = BTreeSet::new();
        for tag in &self.tags {
            bounded_string(tag, 64, "tag")?;
            if !unique.insert(tag.trim()) { return Err("duplicate Companion memory tag".into()); }
        }
        Ok(())
    }
}

fn bounded_string(value: &str, maximum: usize, field: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > maximum {
        return Err(format!("Companion memory {field} must contain 1 to {maximum} characters"));
    }
    Ok(())
}

fn canonical_id(value: &str, field: &str) -> Result<(), String> {
    match uuid::Uuid::parse_str(value) {
        Ok(id) if id.get_version_num() == 7 && id.get_variant() == uuid::Variant::RFC4122 && id.to_string() == value => Ok(()),
        _ => Err(format!("Companion memory {field} must be a canonical UUIDv7")),
    }
}

pub fn companion_memory_recall_input_schema() -> Value {
    json!({
        "type":"object", "additionalProperties":false,
        "properties": {
            "per_kind":{"type":"integer","minimum":1,"maximum":20},
            "char_budget":{"type":"integer","minimum":1,"maximum":65536},
            "scope":{"type":"string","enum":["facts","history"],"default":"facts"},
            "query":{"type":"string","minLength":1,"maxLength":1024},
            "limit":{"type":"integer","minimum":1,"maximum":50},
            "source_session_id":{"type":"string","minLength":36,"maxLength":36},
            "updated_since":{"type":"integer","minimum":0},
            "updated_until":{"type":"integer","minimum":0},
            "since_ts":{"type":"integer","minimum":0},
            "until_ts":{"type":"integer","minimum":0},
            "before":{"type":"string","minLength":1,"maxLength":2048}
        },
        "dependentRequired": {
            "source_session_id":["query"],"updated_since":["query"],"updated_until":["query"],"limit":["query"],
            "since_ts":["query","scope"],"until_ts":["query","scope"],"before":["query","scope"]
        },
        "allOf":[
            {"if":{"properties":{"scope":{"const":"history"}},"required":["scope"]},"then":{"required":["query"],"properties":{"char_budget":{"minimum":768}}}},
            {"if":{"anyOf":[{"required":["since_ts"]},{"required":["until_ts"]},{"required":["before"]}]},"then":{"properties":{"scope":{"const":"history"}}}}
        ]
    })
}

pub fn companion_memory_write_input_schema() -> Value {
    json!({
        "type":"object","additionalProperties":false,
        "properties":{
            "kind":{"type":"string","enum":["profile","preference","knowledge","episode","task","affective"],"description":"For a correction, repeat the stored kind; corrections update content only."},
            "content":{"type":"string","minLength":1,"maxLength":16384},
            "tags":{"type":"array","maxItems":32,"uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":64},"description":"Tags for a new fact. A correction preserves tags; omit this field or send an empty array."},
            "memory_id":{"type":"string","minLength":36,"maxLength":36},
            "expected_revision":{"type":"integer","minimum":1}
        },
        "required":["kind","content"],
        "dependentRequired":{"memory_id":["expected_revision"],"expected_revision":["memory_id"]},
        "allOf":[{"if":{"required":["memory_id"]},"then":{"properties":{"tags":{"maxItems":0}}}}]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_write_keeps_its_receipt_input_but_corrections_include_identity_and_revision() {
        let prior = json!({"kind":"preference","content":"先给结论","tags":[]});
        let mut request: CompanionMemoryWriteInput = serde_json::from_value(prior.clone()).unwrap();
        request.validate().unwrap();
        let current = serde_json::to_value(&request).unwrap();
        assert_eq!(current, prior, "None fields cannot change the established ordinary-write fingerprint");
        assert_eq!(crate::digest_payload(&current).unwrap(), crate::digest_payload(&prior).unwrap());
        request.memory_id = Some("0199a000-0000-7000-8000-000000000002".into());
        request.expected_revision = Some(3);
        request.validate().unwrap();
        let correction = serde_json::to_value(&request).unwrap();
        assert_ne!(crate::digest_payload(&correction).unwrap(), crate::digest_payload(&prior).unwrap());
        let revision_three = crate::digest_payload(&correction).unwrap();
        request.expected_revision = Some(4);
        assert_ne!(crate::digest_payload(&serde_json::to_value(&request).unwrap()).unwrap(), revision_three);
        request.tags = vec!["must-not-be-silently-replaced".into()];
        assert!(request.validate().is_err(), "corrections preserve metadata instead of ignoring a requested tag replacement");
        assert!(serde_json::from_value::<CompanionMemoryWriteInput>(json!({
            "kind":"preference","content":"x","source_session_id":"forged",
        })).is_err(), "provenance comes only from the actual host invocation");
    }

    #[test]
    fn recall_keeps_old_defaults_and_rejects_invalid_mode_or_time_filters() {
        let old: CompanionMemoryRecallInput = serde_json::from_value(json!({"per_kind":20,"char_budget":49152})).unwrap();
        old.validate().unwrap();
        assert_eq!(serde_json::to_value(&old).unwrap(), json!({"per_kind":20,"char_budget":49152}));
        for input in [json!({"scope":"history"}), json!({"query":"x","before":"cursor"}),
            json!({"query":"x","updated_since":2,"updated_until":1}),
            json!({"scope":"history","query":"x","char_budget":100})] {
            assert!(serde_json::from_value::<CompanionMemoryRecallInput>(input).unwrap().validate().is_err());
        }
    }
}
