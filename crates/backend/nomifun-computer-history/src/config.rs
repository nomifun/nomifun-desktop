//! Feature configuration with defaults. The mutable part of the settings
//! (pause state, rules, behaviors) is persisted in the feature-local
//! `feature_config` KV table (see [`crate::store`]); this struct carries the
//! static knobs resolved at startup.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Wire values follow the Codex `SkysightPauseDuration` enum.
pub const SEGMENT_DURATION_SECONDS: u64 = 600; // inferred from Codex segment dir names
/// Retention prune cadence, mirroring nomifun-companion/src/collector.rs.
pub const EVENT_PRUNE_INTERVAL_SECONDS: u64 = 6 * 60 * 60;
/// Default retention window in days. Inferred from Codex (no TTL string is
/// recoverable from the binary); segments are effectively ephemeral there.
pub const DEFAULT_RETENTION_DAYS: u32 = 30;
/// Field-level truncation, same cap as the companion collector.
pub const MAX_FIELD_CHARS: usize = 2000;

/// Static configuration for the computer-history feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerHistoryConfig {
    /// Master switch. Codex gates this in the host app settings; the feature
    /// ships DISABLED so nothing is captured until the user opts in.
    pub enabled: bool,
    /// Directory holding `history.db` and the event-stream root
    /// (`{dir}/segments/`), i.e. `{NOMIFUN_DATA_DIR}/computer-history`.
    pub data_dir: PathBuf,
    /// How often the observer polls the frontmost app/window.
    pub poll_interval_ms: u64,
    /// Segment rollover duration for the raw event stream.
    pub segment_duration_seconds: u64,
    /// Rows older than this are pruned on the retention tick.
    pub retention_days: u32,
}

impl Default for ComputerHistoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            data_dir: PathBuf::from("computer-history"),
            poll_interval_ms: 2_000,
            segment_duration_seconds: SEGMENT_DURATION_SECONDS,
            retention_days: DEFAULT_RETENTION_DAYS,
        }
    }
}

impl ComputerHistoryConfig {
    /// Validate config invariants, in nomifun error style.
    pub fn validate(&self) -> Result<(), nomifun_common::AppError> {
        use nomifun_common::AppError;
        if self.poll_interval_ms == 0 {
            return Err(AppError::BadRequest(
                "Computer History poll interval must be greater than zero".into(),
            ));
        }
        if self.segment_duration_seconds == 0 {
            return Err(AppError::BadRequest(
                "Computer History segment duration must be greater than zero".into(),
            ));
        }
        if self.retention_days == 0 {
            return Err(AppError::BadRequest(
                "Computer History retention must be at least one day".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled_with_spec_defaults() {
        let config = ComputerHistoryConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.segment_duration_seconds, 600);
        assert_eq!(config.retention_days, 30);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn rejects_zero_interval() {
        let config = ComputerHistoryConfig {
            poll_interval_ms: 0,
            ..ComputerHistoryConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_unknown_fields() {
        let raw = r#"{"enabled":false,"data_dir":"x","poll_interval_ms":1,"segment_duration_seconds":600,"retention_days":30,"nope":1}"#;
        assert!(serde_json::from_str::<ComputerHistoryConfig>(raw).is_err());
    }
}
