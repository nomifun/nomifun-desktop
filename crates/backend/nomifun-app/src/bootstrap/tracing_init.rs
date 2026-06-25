//! Tracing subscriber + log file initialization for the binary.
//!
//! Lives in the binary tree (not lib) because it owns process-global
//! subscriber registration. Integration tests reach it transitively via
//! `DesktopServer::start` → `init_environment`, possibly several times in one
//! test binary, so the registration is idempotent (`try_init`): the first call
//! installs the global subscriber and later calls are no-ops instead of a
//! panic ("a global default trace dispatcher has already been set").

use std::path::Path;

use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

const NOISE_SUPPRESSIONS: &[&str] = &["sqlx::query=warn", "hyper_util=warn", "reqwest=warn"];

const NOMI_TARGETS: &[&str] = &[
    "nomi_agent",
    "nomi_config",
    "nomi_compact",
    "nomi_mcp",
    "nomi_providers",
    "nomi_protocol",
    "nomi_tools",
    "nomi_skills",
    "nomi_memory",
];

fn build_env_filter(log_level: Option<&str>) -> EnvFilter {
    let user_directives = log_level.unwrap_or("info");
    let suppressions = NOISE_SUPPRESSIONS.join(",");
    EnvFilter::new(format!("{suppressions},{user_directives}"))
}

fn build_backend_filter(log_level: Option<&str>) -> EnvFilter {
    let user_directives = log_level.unwrap_or("info");
    let suppressions = NOISE_SUPPRESSIONS.join(",");
    let nomi_off: String = NOMI_TARGETS
        .iter()
        .map(|t| format!("{t}=off"))
        .collect::<Vec<_>>()
        .join(",");
    EnvFilter::new(format!("{suppressions},{nomi_off},{user_directives}"))
}

/// RAII guards that flush log buffers on drop. Hold for the process lifetime.
pub struct LogGuards {
    _backend: tracing_appender::non_blocking::WorkerGuard,
    _nomi: tracing_appender::non_blocking::WorkerGuard,
}

pub fn init_tracing(log_dir: &Path, log_level: Option<&str>) -> LogGuards {
    std::fs::create_dir_all(log_dir).expect("failed to create log directory");

    let console_layer = fmt::layer().with_target(true).with_filter(build_env_filter(log_level));

    // Backend file layer — excludes nomi_* targets
    let file_appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_suffix("nomicore.log")
        .build(log_dir)
        .expect("failed to create backend log file appender");
    let (non_blocking, backend_guard) = tracing_appender::non_blocking(file_appender);

    let backend_file_layer = fmt::layer()
        .json()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_filter(build_backend_filter(log_level));

    // Nomi file layer — only nomi_* targets
    let nomi_level = {
        let level = log_level.unwrap_or("info");
        NOMI_TARGETS
            .iter()
            .map(|t| format!("{t}={level}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let nomi_resolved = nomifun_ai_agent::nomi_config::logging::ResolvedLogging {
        enabled: true,
        level: nomi_level,
        dir: log_dir.to_path_buf(),
    };
    let (nomi_layer, nomi_guard) =
        nomifun_ai_agent::nomi_config::logging::create_file_layer(&nomi_resolved).expect("failed to create nomi log layer");

    // `try_init` (not `init`) so a second bootstrap in the same process — e.g.
    // several integration tests each calling `DesktopServer::start` — reuses the
    // first subscriber instead of panicking. In production this runs exactly once.
    if let Err(e) = tracing_subscriber::registry()
        .with(console_layer)
        .with(backend_file_layer)
        .with(nomi_layer)
        .try_init()
    {
        eprintln!("[init_tracing] global subscriber already installed, reusing it: {e}");
    }

    LogGuards {
        _backend: backend_guard,
        _nomi: nomi_guard,
    }
}
