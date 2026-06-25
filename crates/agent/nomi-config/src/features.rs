//! Feature-flag registry for the nomi-agent overhaul (staged rollout).
//!
//! Every behavioural change in the overhaul ships dark behind a flag so it can
//! be enabled per-profile or at runtime and rolled back instantly. Modelled on
//! Codex's `features` crate, kept minimal. See
//! docs/superpowers/specs/2026-06-21-nomi-agent-overhaul-design.md §6.

use std::collections::BTreeSet;

/// Lifecycle stage of a feature — gates UI exposure and metrics emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Built but not ready for any user; off by default, hidden.
    UnderDevelopment,
    /// Opt-in, surfaced in advanced settings.
    Experimental,
    /// On by default, generally available.
    Stable,
    /// On its way out; warns on use.
    Deprecated,
}

/// Stable identifier for a feature flag. New flags are added here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Feature {
    /// Engine winds down cooperatively via a `CancellationToken` instead of the
    /// manager dropping the run future mid-flight (Phase 0 F0.4).
    CooperativeCancel,
    /// Manager-level terminal-event guarantee guard rollout (Phase 0 F0.2).
    TerminationGuard,
}

/// Static declaration of a feature: identity, config key, stage, and default.
#[derive(Debug, Clone, Copy)]
pub struct FeatureSpec {
    pub id: Feature,
    pub key: &'static str,
    pub stage: Stage,
    pub default_enabled: bool,
}

/// The registry of every known feature. Single source of truth.
pub const FEATURES: &[FeatureSpec] = &[
    FeatureSpec {
        id: Feature::CooperativeCancel,
        key: "nomi_cooperative_cancel",
        stage: Stage::UnderDevelopment,
        default_enabled: false,
    },
    FeatureSpec {
        id: Feature::TerminationGuard,
        key: "nomi_termination_guard",
        stage: Stage::UnderDevelopment,
        default_enabled: false,
    },
];

/// Resolve a config key string to its `Feature` id, if known.
pub fn feature_for_key(key: &str) -> Option<Feature> {
    FEATURES.iter().find(|f| f.key == key).map(|f| f.id)
}

/// A resolved set of enabled features for a session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Features {
    enabled: BTreeSet<Feature>,
}

impl Features {
    /// Build from the registry's declared defaults.
    pub fn from_defaults() -> Self {
        let enabled = FEATURES
            .iter()
            .filter(|f| f.default_enabled)
            .map(|f| f.id)
            .collect();
        Self { enabled }
    }

    pub fn is_enabled(&self, id: Feature) -> bool {
        self.enabled.contains(&id)
    }

    pub fn enable(&mut self, id: Feature) {
        self.enabled.insert(id);
    }

    pub fn disable(&mut self, id: Feature) {
        self.enabled.remove(&id);
    }

    pub fn set_enabled(&mut self, id: Feature, on: bool) {
        if on {
            self.enable(id);
        } else {
            self.disable(id);
        }
    }

    /// Layer `(key, on)` overrides on top of `base`. Unknown keys are ignored
    /// with a warning so config stays forward/backward compatible across builds.
    pub fn from_sources(base: Features, overrides: impl IntoIterator<Item = (String, bool)>) -> Self {
        let mut f = base;
        for (key, on) in overrides {
            match feature_for_key(&key) {
                Some(id) => f.set_enabled(id, on),
                None => tracing::warn!(
                    target: "nomi_config",
                    feature_key = %key,
                    "unknown feature flag key ignored"
                ),
            }
        }
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_declared_specs() {
        let f = Features::from_defaults();
        for spec in FEATURES {
            assert_eq!(
                f.is_enabled(spec.id),
                spec.default_enabled,
                "flag {} default mismatch",
                spec.key
            );
        }
    }

    #[test]
    fn enable_disable_round_trip() {
        let mut f = Features::from_defaults();
        f.enable(Feature::CooperativeCancel);
        assert!(f.is_enabled(Feature::CooperativeCancel));
        f.disable(Feature::CooperativeCancel);
        assert!(!f.is_enabled(Feature::CooperativeCancel));
    }

    #[test]
    fn from_sources_applies_known_overrides() {
        let f = Features::from_sources(
            Features::from_defaults(),
            [("nomi_cooperative_cancel".to_string(), true)],
        );
        assert!(f.is_enabled(Feature::CooperativeCancel));
    }

    #[test]
    fn from_sources_ignores_unknown_keys() {
        // Unknown keys (e.g. a flag from a newer/older build) must be ignored,
        // not panic, so config stays forward/backward compatible across versions.
        let f = Features::from_sources(
            Features::from_defaults(),
            [("totally_unknown_flag".to_string(), true)],
        );
        assert_eq!(f, Features::from_defaults());
    }

    #[test]
    fn feature_for_key_round_trips() {
        for spec in FEATURES {
            assert_eq!(feature_for_key(spec.key), Some(spec.id));
        }
        assert_eq!(feature_for_key("nope"), None);
    }
}
