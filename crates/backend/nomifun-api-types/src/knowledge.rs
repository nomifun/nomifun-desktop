use nomifun_common::{KnowledgeBaseId, KnowledgeEntryId};
use serde::{Deserialize, Serialize};

/// Server-enforced mutation policy for a filesystem-backed knowledge base.
/// External directories default to read-only and require explicit user consent
/// before the application may edit or restructure them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeTreeAccess {
    #[default]
    ReadOnly,
    Editable,
}

/// Stable projected kind of a knowledge entry. The file contents and
/// directory structure remain filesystem-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeEntryKind {
    File,
    Directory,
}

/// Provenance controls product policy (for example, URL snapshots are
/// source-managed even though they appear in the same tree).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeEntryOrigin {
    User,
    UrlSnapshot,
    Generated,
}

/// File/directory identity projected from one knowledge base. `entry_id` is
/// stable across rename/move; `rel_path` is its current filesystem locator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeEntry {
    #[ts(type = "string")]
    pub entry_id: KnowledgeEntryId,
    #[ts(type = "string")]
    pub knowledge_base_id: KnowledgeBaseId,
    #[ts(type = "string | null")]
    pub parent_entry_id: Option<KnowledgeEntryId>,
    pub name: String,
    pub kind: KnowledgeEntryKind,
    pub origin: KnowledgeEntryOrigin,
    pub rel_path: String,
    #[ts(type = "number")]
    pub revision: i64,
    #[ts(type = "number | null")]
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum RelocateKnowledgeEntryConflictPolicy {
    Reject,
}

/// One idempotent move/rename command inside the knowledge base identified by
/// the route. Stable IDs are authoritative when present; paths are required as
/// locators for path-only clients and projection-degraded compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct RelocateKnowledgeEntryRequest {
    pub request_id: String,
    pub source_path: String,
    pub destination_parent_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "string")]
    pub entry_id: Option<KnowledgeEntryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "string")]
    pub destination_parent_id: Option<KnowledgeEntryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub new_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub expected_revision: Option<i64>,
    pub conflict_policy: RelocateKnowledgeEntryConflictPolicy,
}

/// Durable receipt returned by drag/drop, "Move to…", and rename. One path
/// prefix pair is sufficient to update all descendants in UI consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct RelocateKnowledgeEntryResponse {
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "string")]
    pub entry_id: Option<KnowledgeEntryId>,
    pub old_path: String,
    pub new_path: String,
    pub kind: KnowledgeEntryKind,
    #[ts(type = "number")]
    pub moved_descendant_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub revision: Option<i64>,
    #[ts(type = "number")]
    pub tree_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub undo_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub warnings: Option<Vec<String>>,
}

/// Idempotently reverse a previously committed relocate operation. The token
/// is opaque to clients and remains valid across process restarts because it
/// resolves through the durable mutation journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct UndoKnowledgeEntryRelocationRequest {
    pub request_id: String,
    pub undo_token: String,
}

/// Candidate-generation backend used by knowledge retrieval.
///
/// `local` is the explicit, no-network keyword implementation. `remote`
/// identifies one exact provider model whose `embedding` capability must be
/// enabled at save time and every time it is invoked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum KnowledgeEmbeddingConfig {
    Local {},
    Remote {
        #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
        provider_id: String,
        #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
        model: String,
    },
}

impl Default for KnowledgeEmbeddingConfig {
    fn default() -> Self {
        Self::Local {}
    }
}

impl KnowledgeEmbeddingConfig {
    pub fn remote_model(&self) -> Option<(&str, &str)> {
        match self {
            Self::Local {} => None,
            Self::Remote { provider_id, model } => Some((provider_id, model)),
        }
    }
}

/// Result-ordering backend used after knowledge candidates are collected.
///
/// The stages are independent: local keyword candidates may be remotely
/// reranked, and remote embedding candidates may keep their local cosine
/// order. A configured remote stage is never treated as optional at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum KnowledgeRerankConfig {
    Local {},
    Remote {
        #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
        provider_id: String,
        #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
        model: String,
    },
}

impl Default for KnowledgeRerankConfig {
    fn default() -> Self {
        Self::Local {}
    }
}

impl KnowledgeRerankConfig {
    pub fn remote_model(&self) -> Option<(&str, &str)> {
        match self {
            Self::Local {} => None,
            Self::Remote { provider_id, model } => Some((provider_id, model)),
        }
    }
}

/// Install-wide knowledge retrieval policy persisted under the single
/// `client_preferences` key `knowledge.retrieval`.
///
/// Both stages default explicitly to `local`; there is no implicit "first
/// available model" lookup and no runtime fallback from a configured remote
/// stage.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct KnowledgeRetrievalConfig {
    pub embedding: KnowledgeEmbeddingConfig,
    pub rerank: KnowledgeRerankConfig,
}

/// A live (URL-backed) source attached to a knowledge base. Snapshots of
/// such sources can go stale between syncs, so the knowledge context builder
/// surfaces the URLs in a dedicated "Realtime sources" section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeSourceEntry {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// **P3-K3**: route this URL through the rendering backend (`BrowserFetcher`,
    /// a real headless browser) instead of the default HTTP fetcher. Set for
    /// JS-heavy SPAs whose content a plain HTTP GET cannot see. `#[serde(default)]`
    /// keeps old persisted `extra.source` rows (which lack the key) deserializing
    /// to `false` ⇒ HTTP — full backward compatibility. When `true` but no render
    /// backend is wired (`browser-use` feature off / not injected), the fetch
    /// gracefully falls back to HTTP at the dispatch site (`prepare_snapshot_body`).
    #[serde(default)]
    pub rendered: bool,
}

/// How a URL source feeds the knowledge base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KnowledgeSourceMode {
    /// URLs are surfaced to the agent as realtime sources (rendered into the
    /// knowledge context); nothing is fetched at create time.
    Live,
    /// URLs are fetched and persisted as markdown snapshots under
    /// `{kb_root}/snapshots/` (re-fetchable via the refresh endpoint).
    Snapshot,
}

/// URL source configuration of a knowledge base. Persisted as JSON in the
/// registry row's forward-compatible `extra` column under the `source` key —
/// every field added later MUST be `#[serde(default)]`-compatible.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeSource {
    /// Source kind discriminator; `"url"` is the only kind today.
    #[serde(default = "default_source_kind")]
    pub kind: String,
    pub mode: KnowledgeSourceMode,
    #[serde(default)]
    pub entries: Vec<KnowledgeSourceEntry>,
    /// Last successful snapshot fetch (epoch ms); `None` until the first
    /// fetch. Live-mode sources are never fetched at create time so they
    /// start as `None`, but the refresh-source endpoint snapshots live
    /// sources too and stamps this field once an entry succeeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fetched_at: Option<i64>,
}

fn default_source_kind() -> String {
    "url".to_owned()
}

/// A knowledge base mounted into a session workspace. Carried in
/// `NomiBuildExtra` (and future build extras) so the
/// shared context builder (`nomifun_knowledge::context`) can tell the agent
/// what extended knowledge is available and where.
///
/// Serialized into `extra.knowledge_mounts`; every field added after the
/// initial shape MUST be `#[serde(default)]`-compatible so old persisted
/// extras keep deserializing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeMountInfo {
    pub knowledge_base_id: KnowledgeBaseId,
    pub name: String,
    pub description: String,
    /// Workspace-relative mount path, e.g. `.nomi/knowledge/领域知识`.
    pub rel_path: String,
    /// Lightweight table of contents — one line per document
    /// (`rel/path.md — first heading`), budgeted at mount time so the prompt
    /// stays bounded; overflow is aggregated into `dir/ — N files` lines.
    /// Lets the agent target the right file instead of crawling the
    /// directory.
    #[serde(default)]
    pub toc: Vec<String>,
    /// First non-heading paragraph of the base's root `README.md`, truncated
    /// to ≤400 chars at mount time. `None` when the base has no README (the
    /// AI-autogen README task fills these in over time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Live URL sources backing (parts of) this base. Rendered as a
    /// "Realtime sources" context section when non-empty. Populated from
    /// `extra.source` when the base has a live-mode URL source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub live_sources: Vec<KnowledgeSourceEntry>,
}

/// A user-defined tag that can be assigned to knowledge bases for
/// categorization / filtering in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeTag {
    pub key: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub sort_order: i64,
}

/// Request body for creating a new knowledge tag.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct CreateKnowledgeTagRequest {
    pub label: String,
    #[serde(default)]
    pub color: Option<String>,
}

/// Request body for partially updating an existing knowledge tag.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct UpdateKnowledgeTagRequest {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub sort_order: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ts_rs::{Config, TS};

    const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";

    #[test]
    fn knowledge_entry_and_relocate_contracts_are_typed_and_snake_case() {
        let entry_id = KnowledgeEntryId::new();
        let destination_parent_id = KnowledgeEntryId::new();
        let request_id = nomifun_common::generate_id();
        let request: RelocateKnowledgeEntryRequest = serde_json::from_value(serde_json::json!({
            "request_id": request_id,
            "source_path": "drafts/topic.md",
            "destination_parent_path": "archive",
            "entry_id": entry_id,
            "destination_parent_id": destination_parent_id,
            "expected_revision": 3,
            "conflict_policy": "reject"
        }))
        .unwrap();
        assert_eq!(request.entry_id.as_ref(), Some(&entry_id));
        assert_eq!(
            request.destination_parent_id.as_ref(),
            Some(&destination_parent_id)
        );
        assert_eq!(request.conflict_policy, RelocateKnowledgeEntryConflictPolicy::Reject);
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["expected_revision"], 3);
        assert!(wire.get("new_name").is_none());
        assert!(wire.get("expectedRevision").is_none());

        let unsupported_policy = serde_json::json!({
            "request_id": "retry-me",
            "source_path": "drafts/topic.md",
            "destination_parent_path": "archive",
            "conflict_policy": "keep_both"
        });
        assert!(
            serde_json::from_value::<RelocateKnowledgeEntryRequest>(unsupported_policy).is_err(),
            "the shared contract must not advertise an unimplemented conflict policy"
        );

        let unsupported_field = serde_json::json!({
            "request_id": "retry-me",
            "source_path": "drafts/topic.md",
            "destination_parent_path": "archive",
            "conflict_policy": "reject",
            "update_links": true
        });
        assert!(
            serde_json::from_value::<RelocateKnowledgeEntryRequest>(unsupported_field).is_err(),
            "unsupported capabilities must be rejected instead of silently ignored"
        );

        let generated = RelocateKnowledgeEntryRequest::export_to_string(&Config::default())
            .expect("relocate request must generate TypeScript");
        assert!(generated.contains("source_path: string"), "got: {generated}");
        assert!(generated.contains("entry_id?: string"), "got: {generated}");
        assert!(
            generated.contains("destination_parent_id?: string"),
            "got: {generated}"
        );
        assert!(!generated.contains("keep_both"), "got: {generated}");
        assert!(!generated.contains("update_links"), "got: {generated}");

        let generated_response =
            RelocateKnowledgeEntryResponse::export_to_string(&Config::default())
                .expect("relocate response must generate TypeScript");
        assert!(
            generated_response.contains("entry_id?: string"),
            "got: {generated_response}"
        );
        assert!(
            generated_response.contains("tree_revision: number"),
            "got: {generated_response}"
        );
        assert!(
            !generated_response.contains("references_updated"),
            "got: {generated_response}"
        );
    }

    #[test]
    fn retrieval_config_is_two_independent_tagged_stages() {
        let config: KnowledgeRetrievalConfig = serde_json::from_value(serde_json::json!({
            "embedding": {"mode": "local"},
            "rerank": {
                "mode": "remote",
                "provider_id": PROVIDER_ID,
                "model": "rerank-v3"
            }
        }))
        .unwrap();
        assert_eq!(config.embedding, KnowledgeEmbeddingConfig::Local {});
        assert_eq!(
            config.rerank.remote_model(),
            Some((PROVIDER_ID, "rerank-v3"))
        );
        assert_eq!(
            serde_json::to_value(config).unwrap(),
            serde_json::json!({
                "embedding": {"mode": "local"},
                "rerank": {
                    "mode": "remote",
                    "provider_id": PROVIDER_ID,
                    "model": "rerank-v3"
                }
            })
        );
    }

    #[test]
    fn retrieval_config_default_serializes_both_stages_as_explicit_local() {
        let config = KnowledgeRetrievalConfig::default();
        assert_eq!(
            serde_json::to_value(config).unwrap(),
            serde_json::json!({
                "embedding": {"mode": "local"},
                "rerank": {"mode": "local"}
            })
        );
    }

    #[test]
    fn retrieval_config_rejects_unknown_fields_blank_models_and_bad_provider_ids() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"embedding": {"mode": "local"}}),
            serde_json::json!({"rerank": {"mode": "local"}}),
            serde_json::json!({
                "embedding": {"mode": "local", "provider_id": PROVIDER_ID},
                "rerank": {"mode": "local"}
            }),
            serde_json::json!({
                "embedding": {"mode": "remote", "provider_id": PROVIDER_ID, "model": " "},
                "rerank": {"mode": "local"}
            }),
            serde_json::json!({
                "embedding": {"mode": "local"},
                "rerank": {"mode": "remote", "provider_id": "legacy", "model": "r"}
            }),
        ] {
            assert!(
                serde_json::from_value::<KnowledgeRetrievalConfig>(value.clone()).is_err(),
                "malformed retrieval config was accepted: {value}"
            );
        }
    }

    #[test]
    fn knowledge_mount_info_uses_named_id_and_rejects_legacy_id() {
        let knowledge_base_id = KnowledgeBaseId::new();
        let mount = serde_json::json!({
            "knowledge_base_id": knowledge_base_id,
            "name": "docs",
            "description": "",
            "rel_path": ".nomi/knowledge/docs"
        });
        let info: KnowledgeMountInfo =
            serde_json::from_value(mount).expect("named knowledge base id should deserialize");
        assert_eq!(info.knowledge_base_id, knowledge_base_id);
        let wire = serde_json::to_value(info).unwrap();
        assert_eq!(wire["knowledge_base_id"], knowledge_base_id.as_str());
        assert!(
            wire.get("id").is_none(),
            "legacy generic id must stay off the wire: {wire}"
        );

        let legacy = serde_json::json!({
            "id": knowledge_base_id,
            "name": "docs",
            "description": "",
            "rel_path": ".nomi/knowledge/docs"
        });
        assert!(serde_json::from_value::<KnowledgeMountInfo>(legacy).is_err());
    }

    /// The `extra.source` wire shape (camelCase + lowercase mode) is a
    /// frontend/gateway contract — pin it.
    #[test]
    fn knowledge_source_serde_shape() {
        let source = KnowledgeSource {
            kind: "url".into(),
            mode: KnowledgeSourceMode::Snapshot,
            entries: vec![KnowledgeSourceEntry {
                url: "https://example.com/docs".into(),
                title: Some("Docs".into()),
                rendered: false,
            }],
            last_fetched_at: Some(1_770_000_000_000),
        };
        let v = serde_json::to_value(&source).unwrap();
        assert_eq!(v["kind"], "url");
        assert_eq!(v["mode"], "snapshot");
        assert_eq!(v["entries"][0]["url"], "https://example.com/docs");
        assert_eq!(v["lastFetchedAt"], 1_770_000_000_000_i64);
        assert!(v.get("last_fetched_at").is_none(), "must be camelCase: {v}");

        let live = serde_json::json!({"mode": "live", "entries": [{"url": "https://e.com"}]});
        let parsed: KnowledgeSource = serde_json::from_value(live).unwrap();
        assert_eq!(parsed.mode, KnowledgeSourceMode::Live);
        assert_eq!(parsed.kind, "url", "kind defaults to url");
        assert_eq!(parsed.last_fetched_at, None);
        let round = serde_json::to_value(&parsed).unwrap();
        assert!(
            round.get("lastFetchedAt").is_none(),
            "None stays off the wire: {round}"
        );
    }

    /// **P3-K3**: the `rendered` flag must be additive/backward-compatible —
    /// old persisted `extra.source` rows have no `rendered` key and MUST
    /// deserialize to `false` (= HTTP fetcher). A present flag round-trips.
    #[test]
    fn knowledge_source_entry_rendered_is_backward_compatible() {
        // Old wire shape (no `rendered` key) → defaults to false.
        let legacy: KnowledgeSourceEntry =
            serde_json::from_value(serde_json::json!({"url": "https://old.example.com"})).unwrap();
        assert!(
            !legacy.rendered,
            "missing `rendered` key must default to false (HTTP)"
        );

        // Present-and-true round-trips through the wire.
        let entry = KnowledgeSourceEntry {
            url: "https://spa.example.com".into(),
            title: None,
            rendered: true,
        };
        let v = serde_json::to_value(&entry).unwrap();
        assert_eq!(v["rendered"], true);
        let back: KnowledgeSourceEntry = serde_json::from_value(v).unwrap();
        assert!(back.rendered);

        // A whole source with one legacy entry and one rendered entry survives.
        let mixed = serde_json::json!({
            "kind": "url",
            "mode": "snapshot",
            "entries": [
                {"url": "https://plain.example.com"},
                {"url": "https://spa.example.com", "rendered": true}
            ]
        });
        let parsed: KnowledgeSource = serde_json::from_value(mixed).unwrap();
        assert!(!parsed.entries[0].rendered, "legacy entry defaults to HTTP");
        assert!(
            parsed.entries[1].rendered,
            "explicit rendered entry preserved"
        );
    }

    /// Tag DTO wire shape: camelCase keys, optional fields omit-when-None,
    /// request DTOs accept partial payloads.
    #[test]
    fn knowledge_tag_serde_shape() {
        // Full KnowledgeTag serializes in camelCase with color present.
        let tag = KnowledgeTag {
            key: "k1".into(),
            label: "Research".into(),
            color: Some("#ff0000".into()),
            sort_order: 2,
        };
        let v = serde_json::to_value(&tag).unwrap();
        assert_eq!(v["key"], "k1");
        assert_eq!(v["label"], "Research");
        assert_eq!(v["color"], "#ff0000");
        assert_eq!(v["sortOrder"], 2);
        assert!(v.get("sort_order").is_none(), "must be camelCase: {v}");

        // color=None stays off the wire.
        let tag_no_color = KnowledgeTag {
            key: "k2".into(),
            label: "Archive".into(),
            color: None,
            sort_order: 0,
        };
        let v2 = serde_json::to_value(&tag_no_color).unwrap();
        assert!(
            v2.get("color").is_none(),
            "None color must be omitted: {v2}"
        );

        // CreateKnowledgeTagRequest — minimal (color defaults to None).
        let create: CreateKnowledgeTagRequest =
            serde_json::from_value(serde_json::json!({"label": "New"})).unwrap();
        assert_eq!(create.label, "New");
        assert_eq!(create.color, None);

        // UpdateKnowledgeTagRequest — all-None (empty patch).
        let update: UpdateKnowledgeTagRequest =
            serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(update.label, None);
        assert_eq!(update.color, None);
        assert_eq!(update.sort_order, None);

        // UpdateKnowledgeTagRequest — partial patch.
        let update2: UpdateKnowledgeTagRequest =
            serde_json::from_value(serde_json::json!({"sortOrder": 5})).unwrap();
        assert_eq!(update2.sort_order, Some(5));
    }
}
