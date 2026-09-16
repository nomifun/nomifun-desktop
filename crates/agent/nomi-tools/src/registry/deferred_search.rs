//! Shared built-in ranking and host-owned discovery activation.

use super::*;
use crate::tool_search::{ToolDiscoveryCandidate, ToolDiscoveryInput};

/// The same default algorithm is used locally and by the bundled Kernel
/// policy. It does not read or mutate Session state.
pub fn rank_tool_discovery(input: &ToolDiscoveryInput) -> Vec<String> {
    rank_candidates(
        &input.query,
        input.candidates.iter().map(|candidate| SearchCandidate {
            name: &candidate.name,
            description: &candidate.description,
            aliases: &candidate.aliases,
        }),
    )
    .into_iter()
    .take(input.limit.min(MAX_DEFERRED_SEARCH_MATCHES))
    .collect()
}

/// Search information only: no input schemas, activation identities, or
/// mutable Session/registry handles are needed by the ranking algorithm.
struct SearchCandidate<'a> {
    name: &'a str,
    description: &'a str,
    aliases: &'a [String],
}

fn rank_candidates<'a>(
    query: &str,
    candidates: impl Iterator<Item = SearchCandidate<'a>>,
) -> Vec<String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(u8, &str)> = candidates
        .filter_map(|candidate| {
            let name = candidate.name.to_lowercase();
            let rank = if name == query {
                0
            } else if candidate.aliases.iter().any(|alias| alias == &query) {
                1
            } else if name.starts_with(&query) {
                2
            } else if candidate
                .aliases
                .iter()
                .any(|alias| alias.starts_with(&query))
            {
                3
            } else if name.contains(&query) {
                4
            } else if candidate.aliases.iter().any(|alias| alias.contains(&query)) {
                5
            } else if candidate.description.to_lowercase().contains(&query) {
                6
            } else {
                return None;
            };
            Some((rank, candidate.name))
        })
        .collect();
    ranked.sort_unstable();
    let exact = ranked
        .first()
        .map(|(rank, _)| *rank)
        .filter(|rank| *rank <= 1);
    ranked
        .into_iter()
        .take_while(|(rank, _)| exact.is_none_or(|exact| *rank == exact))
        .take(MAX_DEFERRED_SEARCH_MATCHES)
        .map(|(_, name)| name.to_owned())
        .collect()
}

pub(super) struct DeferredSearchSnapshot {
    // Arc identity detects replacement, including remove/re-register with an
    // identical display name or schema. Capturing does not copy large schemas.
    entries: BTreeMap<String, Arc<DeferredCatalogEntry>>,
}

impl DeferredSearchSnapshot {
    pub(super) fn candidates(&self) -> Vec<ToolDiscoveryCandidate> {
        self.entries
            .values()
            .map(|entry| ToolDiscoveryCandidate {
                name: entry.definition.name.clone(),
                description: entry.definition.description.clone(),
                aliases: entry.search_aliases.clone(),
            })
            .collect()
    }
    pub(super) fn capture(state: &DeferredToolStateInner) -> Self {
        Self {
            entries: state.catalog.clone(),
        }
    }

    pub(super) fn rank(&self, query: &str) -> Vec<String> {
        rank_candidates(
            query,
            self.entries.values().map(|entry| SearchCandidate {
                name: &entry.definition.name,
                description: &entry.definition.description,
                aliases: &entry.search_aliases,
            }),
        )
    }

    /// Validate first, commit second. A stale/invalid selection cannot activate
    /// a valid prefix, route through an alias, or admit a later registration.
    /// Only the host's current definitions supply schemas and stable identities.
    pub(super) fn activate(
        &self,
        state: &mut DeferredToolStateInner,
        names: &[String],
    ) -> Result<Vec<ToolDef>, &'static str> {
        if names.len() > MAX_DEFERRED_SEARCH_MATCHES {
            return Err("discovery selection exceeds the activation limit");
        }
        let mut unique = BTreeSet::new();
        let mut selected = Vec::with_capacity(names.len());
        for name in names {
            if !unique.insert(name) {
                return Err("discovery selection contains duplicate routes");
            }
            let captured = self
                .entries
                .get(name)
                .ok_or("discovery selection is outside the captured catalog")?;
            let current = state
                .catalog
                .get(name)
                .filter(|entry| Arc::ptr_eq(entry, captured))
                .ok_or("discovery selection is no longer current")?;
            selected.push(Arc::clone(current));
        }
        for entry in &selected {
            state.pending_restored.remove(&entry.activation_identity);
            state.activated.insert(entry.activation_identity.clone());
        }
        Ok(selected
            .into_iter()
            .map(|entry| entry.definition.clone())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state() -> DeferredToolState {
        let state = DeferredToolState::default();
        for (name, aliases) in [("Alpha", vec!["original".into()]), ("Beta", vec![])] {
            state.register_definition(ToolDef {
                name: name.into(),
                description: "shared description".into(),
                input_schema: json!({"type":"object", "properties":{"value":{"type":"string"}}}),
                deferred: true,
            }, format!("stable:{name}"), aliases);
        }
        state
    }

    fn capture(state: &DeferredToolState) -> DeferredSearchSnapshot {
        DeferredSearchSnapshot::capture(&state.inner.read().unwrap())
    }

    fn activate(
        state: &DeferredToolState,
        snapshot: &DeferredSearchSnapshot,
        names: &[&str],
    ) -> Result<Vec<ToolDef>, &'static str> {
        snapshot.activate(
            &mut state.inner.write().unwrap(),
            &names.iter().map(|name| (*name).into()).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn ranking_is_read_only_and_commit_preserves_selected_order_and_host_schema() {
        let state = state();
        let snapshot = capture(&state);
        assert_eq!(snapshot.rank("shared"), vec!["Alpha", "Beta"]);
        assert!(state.activated_identities().is_empty());
        let selected = activate(&state, &snapshot, &["Beta", "Alpha"]).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|def| def.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Beta", "Alpha"]
        );
        assert_eq!(
            selected[0].input_schema["properties"]["value"]["type"],
            "string"
        );
        assert_eq!(
            state.activated_identities(),
            vec!["stable:Alpha", "stable:Beta"]
        );
    }

    #[test]
    fn invalid_selection_never_activates_a_valid_prefix() {
        for names in [
            vec!["Alpha", "unknown"],
            vec!["Alpha", "Alpha"],
            vec!["Alpha", "original"],
            vec!["Alpha", "beta"],
            vec!["Alpha", " Beta"],
            vec!["Alpha"; MAX_DEFERRED_SEARCH_MATCHES + 1],
        ] {
            let state = state();
            state.restore_activation("waiting_for_dynamic");
            let before = state.session_identities();
            assert!(activate(&state, &capture(&state), &names).is_err());
            assert_eq!(state.session_identities(), before);
        }
    }

    #[test]
    fn removal_and_same_name_replacement_reject_stale_selection_atomically() {
        for replacement in [false, true] {
            let state = state();
            let snapshot = capture(&state);
            state.retain_definitions(&BTreeSet::from(["Alpha".into()]));
            if replacement {
                let old = snapshot.entries["Beta"].as_ref();
                state.register_definition(
                    old.definition.clone(),
                    old.activation_identity.clone(),
                    old.search_aliases.clone(),
                );
            }
            assert!(activate(&state, &snapshot, &["Alpha", "Beta"]).is_err());
            assert!(state.activated_identities().is_empty());
        }
    }

    #[test]
    fn later_registration_requires_a_new_snapshot_but_does_not_invalidate_existing_route() {
        let state = state();
        let snapshot = capture(&state);
        state.register_definition(
            ToolDef {
                name: "Later".into(),
                description: "shared".into(),
                input_schema: json!({"type":"object"}),
                deferred: true,
            },
            "stable:Later".into(),
            vec![],
        );
        assert!(activate(&state, &snapshot, &["Alpha", "Later"]).is_err());
        assert!(state.activated_identities().is_empty());
        assert!(activate(&state, &snapshot, &["Alpha"]).is_ok());
        assert!(activate(&state, &capture(&state), &["Later"]).is_ok());
    }

    #[test]
    fn clear_or_different_session_cannot_consume_captured_routes() {
        let original = state();
        let snapshot = capture(&original);
        let other = state();
        assert!(activate(&other, &snapshot, &["Alpha"]).is_err());
        assert!(other.activated_identities().is_empty());
        original.clear();
        assert!(activate(&original, &snapshot, &["Alpha"]).is_err());
    }
}
