use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, serde::Serialize, Default)]
pub struct LoggingConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedLogging {
    pub enabled: bool,
    pub level: String,
    pub dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("failed to create log directory '{path}': {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to build log file appender: {0}")]
    AppenderInit(String),
    #[error("invalid log level filter '{filter}': {reason}")]
    InvalidFilter { filter: String, reason: String },
}

pub fn default_log_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .map(|h| h.join("Library").join("Logs").join("nomi"))
            .unwrap_or_else(|| PathBuf::from("nomi/logs"))
    }
    #[cfg(target_os = "linux")]
    {
        dirs::state_dir()
            .map(|d| d.join("nomi").join("logs"))
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .map(|h| h.join(".local").join("state").join("nomi").join("logs"))
                    .unwrap_or_else(|| PathBuf::from("nomi/logs"))
            })
    }
    #[cfg(target_os = "windows")]
    {
        dirs::data_dir()
            .map(|d| d.join("nomi").join("logs"))
            .unwrap_or_else(|| PathBuf::from("nomi/logs"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        PathBuf::from("nomi/logs")
    }
}

pub use tracing_appender::non_blocking::WorkerGuard as LoggingGuard;

use tracing::Subscriber;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::fmt;
use tracing_subscriber::registry::LookupSpan;

pub fn create_file_layer<S>(
    config: &ResolvedLogging,
) -> Result<(Box<dyn Layer<S> + Send + Sync>, WorkerGuard), LoggingError>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    std::fs::create_dir_all(&config.dir).map_err(|source| LoggingError::CreateDir {
        path: config.dir.clone(),
        source,
    })?;

    let file_appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_suffix("nomi.log")
        .build(&config.dir)
        .map_err(|e| LoggingError::AppenderInit(e.to_string()))?;

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_new(&config.level).map_err(|e| LoggingError::InvalidFilter {
        filter: config.level.clone(),
        reason: e.to_string(),
    })?;

    let layer = fmt::layer()
        .json()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_filter(filter);

    Ok((Box::new(layer), guard))
}

impl LoggingConfig {
    pub fn merge(global: Self, project: Self) -> Self {
        Self {
            enabled: project.enabled.or(global.enabled),
            level: project.level.or(global.level),
            dir: project.dir.or(global.dir),
        }
    }

    pub fn resolve(
        &self,
        cli_log_dir: Option<&str>,
        cli_log_level: Option<&str>,
    ) -> ResolvedLogging {
        let dir = cli_log_dir
            .map(PathBuf::from)
            .or_else(|| self.dir.as_ref().map(PathBuf::from))
            .unwrap_or_else(default_log_dir);

        let has_explicit_dir = cli_log_dir.is_some() || self.dir.is_some();
        let enabled = self.enabled.unwrap_or(has_explicit_dir);

        let level = cli_log_level
            .map(String::from)
            .or_else(|| self.level.clone())
            .unwrap_or_else(|| "info".to_string());

        ResolvedLogging {
            enabled,
            level,
            dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_all_none() {
        let cfg = LoggingConfig::default();
        assert!(cfg.enabled.is_none());
        assert!(cfg.level.is_none());
        assert!(cfg.dir.is_none());
    }

    #[test]
    fn toml_with_all_fields() {
        let toml_str = r#"
enabled = true
level = "debug"
dir = "/tmp/nomi-logs"
"#;
        let cfg: LoggingConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.enabled, Some(true));
        assert_eq!(cfg.level.as_deref(), Some("debug"));
        assert_eq!(cfg.dir.as_deref(), Some("/tmp/nomi-logs"));
    }

    #[test]
    fn toml_empty_uses_defaults() {
        let cfg: LoggingConfig = toml::from_str("").unwrap();
        assert!(cfg.enabled.is_none());
        assert!(cfg.level.is_none());
        assert!(cfg.dir.is_none());
    }

    #[test]
    fn merge_project_overrides_global() {
        let global = LoggingConfig {
            enabled: Some(false),
            level: Some("warn".into()),
            dir: Some("/global/logs".into()),
        };
        let project = LoggingConfig {
            enabled: Some(true),
            level: Some("debug".into()),
            dir: None,
        };
        let merged = LoggingConfig::merge(global, project);
        assert_eq!(merged.enabled, Some(true));
        assert_eq!(merged.level.as_deref(), Some("debug"));
        assert_eq!(merged.dir.as_deref(), Some("/global/logs"));
    }

    #[test]
    fn merge_falls_back_to_global() {
        let global = LoggingConfig {
            level: Some("info".into()),
            ..Default::default()
        };
        let project = LoggingConfig::default();
        let merged = LoggingConfig::merge(global, project);
        assert_eq!(merged.level.as_deref(), Some("info"));
    }

    #[test]
    fn merge_two_empty_configs() {
        let merged = LoggingConfig::merge(LoggingConfig::default(), LoggingConfig::default());
        assert!(merged.enabled.is_none());
        assert!(merged.level.is_none());
        assert!(merged.dir.is_none());
    }

    #[test]
    fn resolve_dir_set_implies_enabled() {
        let cfg = LoggingConfig {
            dir: Some("/tmp/logs".into()),
            ..Default::default()
        };
        let resolved = cfg.resolve(None, None);
        assert!(resolved.enabled);
        assert_eq!(resolved.dir, PathBuf::from("/tmp/logs"));
        assert_eq!(resolved.level, "info");
    }

    #[test]
    fn resolve_nothing_set_means_disabled() {
        let cfg = LoggingConfig::default();
        let resolved = cfg.resolve(None, None);
        assert!(!resolved.enabled);
    }

    #[test]
    fn resolve_cli_overrides_config() {
        let cfg = LoggingConfig {
            level: Some("warn".into()),
            dir: Some("/config/logs".into()),
            ..Default::default()
        };
        let resolved = cfg.resolve(Some("/cli/logs"), Some("debug"));
        assert_eq!(resolved.dir, PathBuf::from("/cli/logs"));
        assert_eq!(resolved.level, "debug");
        assert!(resolved.enabled);
    }

    #[test]
    fn resolve_level_defaults_to_info() {
        let cfg = LoggingConfig {
            enabled: Some(true),
            ..Default::default()
        };
        let resolved = cfg.resolve(None, None);
        assert_eq!(resolved.level, "info");
    }

    #[test]
    fn default_log_dir_returns_nonempty_path() {
        let dir = default_log_dir();
        assert!(!dir.as_os_str().is_empty());
    }

    #[test]
    fn default_log_dir_contains_nomi() {
        let dir = default_log_dir();
        let s = dir.to_string_lossy();
        assert!(s.contains("nomi"), "expected 'nomi' in path: {s}");
    }
}
