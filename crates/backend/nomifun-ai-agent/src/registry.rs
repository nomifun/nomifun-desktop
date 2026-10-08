//! Process-wide snapshot of the `agent_metadata` catalog.
//!
//! The table is the single source of truth for every agent the user can
//! spawn — builtin vendor rows, extension-installed rows, and custom
//! rows all live there. The registry:
//!
//! - hydrates `select *` into memory at startup;
//! - probes each row's spawn command via `which()` so the `available`
//!   field reflects PATH state right now (not a persisted column);
//! - exposes lookups the factory and routes use (`get`,
//!   `find_builtin_by_backend`, `list_by_agent_type`, etc.).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use nomifun_api_types::{AgentEnvEntry, AgentMetadata, AgentSource, AgentSourceInfo, BehaviorPolicy};
use nomifun_common::{AgentType, AppError};
use nomifun_db::{AgentMetadataRow, IAgentMetadataRepository};
use nomifun_runtime::resolve_command_path;
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

pub struct AgentRegistry {
    repo: Arc<dyn IAgentMetadataRepository>,
    by_id: RwLock<HashMap<String, AgentMetadata>>,
}

impl AgentRegistry {
    pub fn new(repo: Arc<dyn IAgentMetadataRepository>) -> Arc<Self> {
        Arc::new(Self {
            repo,
            by_id: RwLock::new(HashMap::new()),
        })
    }

    /// Reload every enabled row from the database and re-probe their
    /// spawn commands on `$PATH`.
    pub async fn hydrate(&self) -> Result<(), AppError> {
        let rows = self
            .repo
            .list_all()
            .await
            .map_err(|e| AppError::Internal(format!("load agent_metadata: {e}")))?;

        let mut map = HashMap::with_capacity(rows.len());
        for row in rows {
            let Some((meta, reason)) = decode_row(row) else {
                continue;
            };
            log_probe_result(&meta, &reason);
            map.insert(meta.agent_id.clone(), meta);
        }
        // Snapshot the summary off the local map before transferring it
        // into the lock — `log_availability_summary` borrows the values
        // and we don't want that borrow to outlive the move.
        log_availability_summary(map.values(), "AgentRegistry hydrated");
        *self.by_id.write().await = map;
        Ok(())
    }

    /// Re-probe every row's command without refetching from the DB.
    /// Useful after PATH has changed (e.g. `launchctl setenv`).
    pub async fn refresh_availability(&self) {
        let mut guard = self.by_id.write().await;
        for meta in guard.values_mut() {
            let (path, reason) = probe_with_reason(meta);
            meta.resolved_command = path;
            meta.available = meta.resolved_command.is_some()
                || (meta.enabled && meta.command.is_none() && meta.agent_source == AgentSource::Internal);
            log_probe_result(meta, &reason);
        }
        log_availability_summary(guard.values(), "AgentRegistry refresh_availability complete");
    }

    pub async fn get(&self, id: &str) -> Option<AgentMetadata> {
        self.by_id.read().await.get(id).cloned()
    }

    /// First row whose vendor label matches, among `agent_source = 'builtin'`.
    pub async fn find_builtin_by_backend(&self, vendor: &str) -> Option<AgentMetadata> {
        self.by_id
            .read()
            .await
            .values()
            .find(|m| m.backend.as_deref() == Some(vendor) && m.agent_source == AgentSource::Builtin)
            .cloned()
    }

    /// Every enabled, installed row whose `agent_type` matches,
    /// sorted by `sort_order`. See [`Self::list_all`] for the filter
    /// semantics.
    pub async fn list_by_agent_type(&self, agent_type: AgentType) -> Vec<AgentMetadata> {
        let guard = self.by_id.read().await;
        let mut rows: Vec<AgentMetadata> = guard
            .values()
            .filter(|m| m.agent_type == agent_type && is_visible(m))
            .cloned()
            .collect();
        rows.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then_with(|| a.name.cmp(&b.name)));
        rows
    }

    /// Snapshot of every row the caller is expected to see — rows
    /// that are user-disabled (`enabled = 0`) or whose spawn command
    /// could not be located on `$PATH` (`available = false`) are
    /// filtered out. `/api/agents` feeds the frontend pill bar, which
    /// would otherwise render unusable vendor chips that fail the
    /// moment the user tries to spawn them.
    pub async fn list_all(&self) -> Vec<AgentMetadata> {
        let mut rows: Vec<AgentMetadata> = self
            .by_id
            .read()
            .await
            .values()
            .filter(|m| is_visible(m))
            .cloned()
            .collect();
        rows.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then_with(|| a.name.cmp(&b.name)));
        rows
    }

    /// Unfiltered snapshot — used by internal paths that legitimately
    /// need to see user-disabled or missing rows (e.g. the UI's
    /// "manage agents" surface). Keep external API handlers on
    /// [`Self::list_all`].
    pub async fn list_all_including_hidden(&self) -> Vec<AgentMetadata> {
        let mut rows: Vec<AgentMetadata> = self.by_id.read().await.values().cloned().collect();
        rows.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then_with(|| a.name.cmp(&b.name)));
        rows
    }

    /// Like [`Self::list_all_including_hidden`] but pairs every row
    /// with a freshly-computed availability reason so callers (the
    /// `doctor` command, diagnostic UIs) can explain *why* a row is
    /// unavailable without depending on logs or re-implementing the
    /// probe rules.
    ///
    /// Reasons are only attached to rows whose `available` flag is
    /// `false`. Internal rows (e.g. the nomi row) intentionally
    /// have an empty `command`, so the underlying probe always
    /// reports `NoCommand` for them — surfacing that as a "reason"
    /// when `available = true` would just confuse the caller, so we
    /// suppress it here.
    pub async fn diagnostic_snapshot(&self) -> Vec<(AgentMetadata, Option<UnavailableReason>)> {
        let mut rows: Vec<(AgentMetadata, Option<UnavailableReason>)> = self
            .by_id
            .read()
            .await
            .values()
            .map(|m| {
                let reason = if m.available {
                    None
                } else {
                    probe_resolved_command(m).err()
                };
                (m.clone(), reason)
            })
            .collect();
        rows.sort_by(|(a, _), (b, _)| a.sort_order.cmp(&b.sort_order).then_with(|| a.name.cmp(&b.name)));
        rows
    }

}

/// A catalog row is visible to callers when the user has it enabled
/// and the spawn command was resolved at hydrate/refresh time. The
/// second check is what keeps uninstalled CLIs (e.g. `cursor` when
/// only `claude` is on PATH) off the pill bar.
fn is_visible(meta: &AgentMetadata) -> bool {
    meta.enabled && meta.available
}

/// Turn a DB row into the public `AgentMetadata`, probing the command
/// on disk so `available` reflects the current PATH state. Returns
/// the probe reason alongside the row so the caller can log a single
/// uniform `(meta, reason)` line per agent without re-running the
/// probe.
fn decode_row(row: AgentMetadataRow) -> Option<(AgentMetadata, Option<UnavailableReason>)> {
    let agent_type = parse_agent_type(&row.agent_type)?;
    let agent_source = parse_agent_source(&row.agent_source)?;
    let agent_source_info = decode_json_field(row.agent_source_info.as_deref(), "agent_source_info")
        .unwrap_or_else(AgentSourceInfo::default);
    let args = decode_json_field::<Vec<String>>(row.args.as_deref(), "args").unwrap_or_default();
    let env = decode_json_field::<Vec<AgentEnvEntry>>(row.env.as_deref(), "env").unwrap_or_default();
    let native_skills_dirs = decode_json_field::<Vec<String>>(row.native_skills_dirs.as_deref(), "native_skills_dirs");
    let behavior_policy =
        decode_json_field(row.behavior_policy.as_deref(), "behavior_policy").unwrap_or_else(BehaviorPolicy::default);

    let mut meta = AgentMetadata {
        agent_id: row.agent_id,
        icon: row.icon,
        name: row.name,
        name_i18n: parse_json(row.name_i18n.as_deref(), "name_i18n"),
        description: row.description,
        description_i18n: parse_json(row.description_i18n.as_deref(), "description_i18n"),
        backend: row.backend,
        agent_type,
        agent_source,
        agent_source_info,
        enabled: row.enabled,
        available: false,
        command: row.command,
        resolved_command: None,
        args,
        env,
        native_skills_dirs,
        behavior_policy,
        sort_order: row.sort_order,
    };

    let (path, reason) = probe_with_reason(&meta);
    meta.resolved_command = path;
    meta.available = meta.resolved_command.is_some()
        || (meta.enabled && meta.command.is_none() && meta.agent_source == AgentSource::Internal);
    Some((meta, reason))
}

/// Wrapper around [`probe_resolved_command`] that returns both the
/// resolved path (if any) and the failure reason as a tuple, so the
/// hydrate / refresh loops can persist the path and emit a single
/// uniform log line per row.
fn probe_with_reason(meta: &AgentMetadata) -> (Option<PathBuf>, Option<UnavailableReason>) {
    match probe_resolved_command(meta) {
        Ok(path) => (Some(path), None),
        Err(reason) => (None, Some(reason)),
    }
}

/// Emit a single per-row line summarizing the probe outcome. Available
/// rows go to `debug!` (one per startup × N agents is noisy at info);
/// unavailable rows go to `info!` so the default nomicore.log surfaces
/// the reason without needing `--log-level debug` after a user
/// reports "no agent works".
fn log_probe_result(meta: &AgentMetadata, reason: &Option<UnavailableReason>) {
    let backend = meta.backend.as_deref().unwrap_or("-");
    let source = format!("{:?}", meta.agent_source);
    match (meta.available, reason) {
        (true, _) => {
            debug!(
                agent_id = %meta.agent_id,
                name = %meta.name,
                backend,
                source = %source,
                command = meta.command.as_deref().unwrap_or("-"),
                resolved = %meta
                    .resolved_command
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<internal>".to_owned()),
                "agent   available"
            );
        }
        (false, Some(reason)) => {
            info!(
                agent_id = %meta.agent_id,
                name = %meta.name,
                backend,
                source = %source,
                command = meta.command.as_deref().unwrap_or("-"),
                reason = %reason,
                "agent unavailable"
            );
        }
        (false, None) => {
            // Probe succeeded internally but `available` still false —
            // shouldn't happen given current rules, but we'd want to
            // know if it does.
            warn!(
                agent_id = %meta.agent_id,
                name = %meta.name,
                backend,
                source = %source,
                "agent marked unavailable without a probe reason — registry invariant violated"
            );
        }
    }
}

/// One-line summary at the end of hydrate / refresh: total / available
/// / unavailable counts plus a comma-joined list of unavailable
/// `id:reason` pairs (truncated to the first 12 to keep log lines
/// bounded). Goes to `info!` so it's visible at the default level.
fn log_availability_summary<'a, I>(rows: I, message: &'static str)
where
    I: IntoIterator<Item = &'a AgentMetadata>,
{
    let mut total = 0usize;
    let mut available = 0usize;
    let mut unavailable_ids: Vec<String> = Vec::new();
    for meta in rows {
        total += 1;
        if meta.available {
            available += 1;
        } else {
            unavailable_ids.push(meta.agent_id.clone());
        }
    }
    let unavailable = total - available;
    let preview: String = if unavailable_ids.is_empty() {
        String::new()
    } else {
        let cap = unavailable_ids.len().min(12);
        let mut joined = unavailable_ids[..cap].join(", ");
        if unavailable_ids.len() > cap {
            joined.push_str(&format!(", … (+{} more)", unavailable_ids.len() - cap));
        }
        joined
    };
    info!(total, available, unavailable, unavailable_ids = %preview, "{}", message);
}

fn parse_agent_type(raw: &str) -> Option<AgentType> {
    serde_json::from_value(Value::String(raw.to_owned())).ok()
}

fn parse_agent_source(raw: &str) -> Option<AgentSource> {
    serde_json::from_value(Value::String(raw.to_owned())).ok()
}

fn decode_json_field<T: serde::de::DeserializeOwned>(raw: Option<&str>, field: &str) -> Option<T> {
    raw.and_then(|s| match serde_json::from_str(s) {
        Ok(v) => Some(v),
        Err(err) => {
            warn!(field, error = %err, "agent_metadata: failed to decode JSON column");
            None
        }
    })
}

fn parse_json(raw: Option<&str>, field: &str) -> Option<Value> {
    raw.and_then(|s| match serde_json::from_str::<Value>(s) {
        Ok(v) => Some(v),
        Err(err) => {
            warn!(field, error = %err, "agent_metadata: failed to parse JSON");
            None
        }
    })
}

/// Why a row's spawn command failed to resolve at hydrate/refresh time.
/// Carried alongside the resolved path so callers (logging, the
/// `doctor` command) can explain availability without re-running the
/// probe themselves. The variants line up 1:1 with the early-return
/// branches in [`probe_resolved_command`].
#[derive(Debug, Clone)]
pub enum UnavailableReason {
    /// Row is user-disabled (`enabled = 0`). The probe short-circuits
    /// without touching `$PATH`.
    Disabled,
    /// Row has no `command` set. Internal rows legitimately fall in
    /// this bucket (handled in `decode_row`); for everyone else this
    /// is a seed-data bug.
    NoCommand,
    /// Bridge binary (`agent_source_info.bridge_binary`, e.g. `bun`
    /// for `bun x @pkg`) is not on `$PATH`.
    BridgeMissing { bridge: String },
    /// Primary CLI (`agent_source_info.binary_name`, e.g. `claude`
    /// for the bridged Claude row) is not on `$PATH`.
    PrimaryMissing { binary: String },
    /// Spawn command itself (`command` field) is not on `$PATH`. For
    /// direct-CLI rows this is the same binary as `binary_name`; for
    /// bridge rows it's the bridge.
    CommandMissing { command: String },
}

impl std::fmt::Display for UnavailableReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("row disabled by user"),
            Self::NoCommand => f.write_str("no spawn command configured"),
            Self::BridgeMissing { bridge } => write!(f, "bridge binary `{bridge}` not on $PATH"),
            Self::PrimaryMissing { binary } => write!(f, "primary binary `{binary}` not on $PATH"),
            Self::CommandMissing { command } => write!(f, "spawn command `{command}` not on $PATH"),
        }
    }
}

/// Resolve the spawn command to an absolute path via `$PATH`. Returns
/// `Ok(path)` when every required binary is present, or `Err(reason)`
/// pinpointing the first missing piece. The value is the single
/// source of truth for `available` — callers never re-run `which()`
/// themselves.
///
/// Bridge-based rows (e.g. `bun x @pkg`) require both `bun` (the spawn
/// command) and the wrapped CLI (`claude`, recorded in
/// `agent_source_info.binary_name`) to be present. Direct-CLI rows
/// have `spawn command == primary binary`, so the primary-binary check
/// is a no-op for them.
fn probe_resolved_command(meta: &AgentMetadata) -> Result<PathBuf, UnavailableReason> {
    if !meta.enabled {
        return Err(UnavailableReason::Disabled);
    }
    let Some(cmd) = meta.command.as_deref().filter(|s| !s.is_empty()) else {
        return Err(UnavailableReason::NoCommand);
    };

    if let Some(bridge) = meta.agent_source_info.bridge_binary.as_deref()
        && bridge != cmd
        && resolve_command_path(bridge).is_none()
    {
        return Err(UnavailableReason::BridgeMissing {
            bridge: bridge.to_owned(),
        });
    }
    if let Some(primary) = meta.agent_source_info.binary_name.as_deref()
        && primary != cmd
        && meta.agent_source_info.bridge_binary.as_deref() != Some(primary)
        && resolve_command_path(primary).is_none()
    {
        return Err(UnavailableReason::PrimaryMissing {
            binary: primary.to_owned(),
        });
    }

    resolve_command_path(cmd).ok_or_else(|| UnavailableReason::CommandMissing {
        command: cmd.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::{SqliteAgentMetadataRepository, init_database_memory};

    /// The internal `nomi` seed row — the only `agent_metadata` row that still
    /// decodes into an `AgentType`.

    async fn registry() -> Arc<AgentRegistry> {
        let db = init_database_memory().await.unwrap();
        let repo = Arc::new(SqliteAgentMetadataRepository::new(db.pool().clone()));
        let reg = AgentRegistry::new(repo);
        reg.hydrate().await.unwrap();
        reg
    }

    #[tokio::test]
    async fn hydrate_loads_seed_rows() {
        // `list_all_including_hidden` bypasses the available/enabled
        // filter so this assertion keeps counting the seed rows even
        // when none of the CLIs are installed on the test host.
        //
        // 1, not the baseline's 20: every seeded row naming a retired engine
        // fails to decode into an `AgentType`, so `decode_row` drops them until
        // the migration deletes those rows. Only the internal nomi row is left.
        let reg = registry().await;
        let all = reg.list_all_including_hidden().await;
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn list_all_filters_out_unavailable_rows() {
        let reg = registry().await;
        let visible = reg.list_all().await;
        assert!(
            visible.iter().all(|m| m.enabled && m.available),
            "list_all must only return enabled + available rows, got: {:?}",
            visible
                .iter()
                .map(|m| (&m.agent_id, m.enabled, m.available))
                .collect::<Vec<_>>()
        );
        // Nomi (internal, no spawn command) is always available.
        assert!(
            visible.iter().any(|m| m.agent_type == AgentType::Nomi),
            "internal nomi row should survive the filter"
        );
    }

    #[tokio::test]
    async fn list_by_agent_type_counts_seed_rows() {
        // Seed counts — exercised against the unfiltered view because
        // on CI hosts the CLIs aren't installed, so `list_by_agent_type`
        // (which applies the visibility filter) would report zero.
        //
        // Rows whose `agent_type` names a retired engine no longer decode
        // (`parse_agent_type` returns `None` and `decode_row` drops them), so
        // they never reach this view even while the migration has yet to
        // delete them — hence exactly one decoded seed row.
        let reg = registry().await;
        let all = reg.list_all_including_hidden().await;
        let count = |t: AgentType| all.iter().filter(|m| m.agent_type == t).count();
        assert_eq!(count(AgentType::Nomi), 1);
        assert_eq!(all.len(), 1, "no retired-engine row may decode into the cache");
    }

    #[tokio::test]
    async fn nomi_internal_row_is_available_without_command() {
        let reg = registry().await;
        let nomi = reg
            .list_by_agent_type(AgentType::Nomi)
            .await
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(nomi.agent_source, AgentSource::Internal);
        assert!(nomi.command.is_none());
        assert!(nomi.available);
    }

    /// `diagnostic_snapshot` returns one entry per row, populates a
    /// reason for every unavailable row, and leaves available rows
    /// without one.
    ///
    /// NOTE: the seeded catalog now decodes to the single internal nomi row,
    /// which is always available. The `(false, Some(_))` pairing below is
    /// therefore no longer exercised by this fixture — it previously relied on
    /// the seeded vendor CLIs being absent on the test host. What still holds
    /// is one-entry-per-row and "an available row carries no reason".
    #[tokio::test]
    async fn diagnostic_snapshot_pairs_rows_with_reasons() {
        let reg = registry().await;
        let snapshot = reg.diagnostic_snapshot().await;
        assert_eq!(snapshot.len(), 1, "every row appears once");

        for (meta, reason) in &snapshot {
            match (meta.available, reason) {
                (true, None) => {}
                (false, Some(_)) => {}
                (true, Some(r)) => panic!("available row {} has unexpected reason {:?}", meta.agent_id, r),
                (false, None) => panic!(
                    "unavailable row {} (source={:?}) is missing a reason",
                    meta.agent_id, meta.agent_source
                ),
            }
        }

        // The internal nomi row is always available — its reason
        // slot must be None (sanity check that "available" doesn't
        // accidentally co-occur with a reason).
        let nomi = snapshot
            .iter()
            .find(|(m, _)| m.agent_type == AgentType::Nomi)
            .expect("nomi seed row");
        assert!(nomi.0.available);
        assert!(nomi.1.is_none());
    }

}
