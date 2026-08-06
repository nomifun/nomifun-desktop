//! Dataset-generation lifecycle and factory reset coordination.
//!
//! The reset coordinator deliberately lives outside the database crate.  It
//! owns the filesystem boundary (database family, managed side stores and the
//! generation receipt), while the database worker remains responsible for
//! probing/proving the database contract.  This keeps the destructive
//! transition before a writable `SqlitePool` exists.
//!
//! A reset is a durable, resumable move operation:
//!
//! 1. write an immutable plan and an `armed` phase;
//! 2. move every known managed root into a fixed retired-dataset directory;
//! 3. install one new generation;
//! 4. let database bootstrap create/prove the new database;
//! 5. write the v3 receipt and remove the pending plan.
//!
//! A crash at any point leaves the plan and fixed destinations in place.
//! Retry accepts only the source-present/destination-absent or
//! source-absent/destination-present states. It never walks or deletes an
//! arbitrary external workspace; the one resolved product-managed
//! `<work_dir>/conversations` root is explicitly part of the dataset and is
//! moved as a single root when it lives outside `data_dir`.

use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::dataset_roots::{
    DatasetRootKind, WORK_ROOT_BINDING_FILE, WORK_ROOT_OWNER_FILE,
    managed_dataset_roots, reset_managed_dataset_roots,
};
use crate::error::AppError;
use crate::id::validate_uuidv7;
use crate::timestamp::now_ms;

/// Current v3 explicit-reset request. It is a control-plane request, not a
/// historical dataset format, and is parsed strictly.
pub const V3_DATASET_RESET_REQUEST_FILE: &str = ".dataset-v3-reset.request.json";

/// Durable v3 automatic-reset plan directory.
pub const V3_DATASET_RESET_DIR: &str = ".id-reference-v3-dataset-reset.pending";
pub const V3_DATASET_RESET_PLAN_FILE: &str = "plan.json";
pub const V3_DATASET_RECEIPT_FILE: &str = "dataset-v3.json";
pub const V3_DATASET_BOOTSTRAP_FILE: &str = ".dataset-v3.bootstrap.json";
pub const V3_DATASET_CONTRACT_VERSION: u32 = 3;
pub const RETIRED_DATASETS_DIR: &str = "retired-datasets";
const WORK_RETIRED_DATASETS_DIR: &str = ".nomifun-retired-datasets";
const MANAGED_WORKSPACES_DIR: &str = "conversations";

const LEGACY_PLAN_VERSION: u32 = 1;
const PLAN_VERSION: u32 = 2;
const LEGACY_RESET_REQUEST_VERSION: u32 = 1;
const RESET_REQUEST_VERSION: u32 = 2;
const DB_FILE: &str = "nomifun-backend.db";
const STORAGE_GENERATION_FILE: &str = "storage-generation";
const RETIRED_FACTORY_RESET_MARKER: &str = "factory-reset.pending";
const IGNORED_LEGACY_RESET_REQUESTS_DIR: &str =
    "ignored-legacy-reset-requests";
const IGNORED_LEGACY_RESET_PLANS_DIR: &str =
    "ignored-legacy-reset-plans";
const CANCELLED_WORK_DIR_CHANGE_REQUESTS_DIR: &str =
    "cancelled-work-dir-change-requests";
const COMPLETED_RESET_REQUESTS_DIR: &str =
    "completed-reset-requests";
const REPLAYED_COMPLETED_RESET_REQUESTS_DIR: &str =
    "replayed-completed-reset-requests";
const COMPLETED_RESET_CONTROL_DIR: &str = "completed-reset-control";
const REPLAYED_COMPLETED_RESET_CONTROL_PREFIX: &str =
    "replayed-completed-reset-control";
/// Quarantine for a pending reset control whose plan was written against a
/// managed-root registry this build does not recognize.
const UNUSABLE_RESET_PLANS_DIR: &str = "unrecognized-reset-plans";
const COMPLETED_RESET_REQUEST_VERSION: u32 = 1;
const AUTOMATIC_LEGACY_RETIREMENT_FILE: &str =
    "automatic-legacy-retirement.completed.json";
const AUTOMATIC_LEGACY_RETIREMENT_VERSION: u32 = 1;
const MAX_CONTROL_FILE_BYTES: u64 = 64 * 1024;
// Order is deliberate: retire every sidecar/lock before the main database.
// A crash before the final rename therefore cannot leave a retired main file
// next to an active stale WAL/SHM family.
const DB_FAMILY: &[&str] = &[
    "nomifun-backend.db-wal",
    "nomifun-backend.db-shm",
    "nomifun-backend.db-journal",
    "nomifun-backend.db.migrate.lock",
    "nomifun-backend.db",
];
// Exact managed-root registry emitted by the released v1 reset planner.
//
// Never derive this from the current registry: adding a root in a later
// release must not silently change which historical plan bytes are accepted.
//
// See [`released_plan_shape`] for how a plan version selects its frozen
// registry, and `RELEASED_V2_MANAGED_ROOTS` for the shape written today.
const RELEASED_V1_MANAGED_ROOTS: &[(&str, ManagedRootKind)] = &[
    ("nomifun-backend.db-wal", ManagedRootKind::File),
    ("nomifun-backend.db-shm", ManagedRootKind::File),
    ("nomifun-backend.db-journal", ManagedRootKind::File),
    ("nomifun-backend.db.migrate.lock", ManagedRootKind::File),
    ("nomifun-backend.db", ManagedRootKind::File),
    ("storage-generation", ManagedRootKind::File),
    ("dataset-v3.json", ManagedRootKind::File),
    (".dataset-v3.bootstrap.json", ManagedRootKind::File),
    ("factory-reset.pending", ManagedRootKind::File),
    ("encryption_key", ManagedRootKind::File),
    ("dir-config.json", ManagedRootKind::File),
    ("conversations", ManagedRootKind::Directory),
    ("attachments", ManagedRootKind::Directory),
    ("knowledge", ManagedRootKind::Directory),
    ("projects", ManagedRootKind::Directory),
    ("companion", ManagedRootKind::Directory),
    ("cron", ManagedRootKind::Directory),
    ("workshop", ManagedRootKind::Directory),
    // Legacy root of the retired public-agent domain (cleanup only).
    ("public-agents", ManagedRootKind::Directory),
    ("preview-history", ManagedRootKind::Directory),
    ("nomi-sessions", ManagedRootKind::Directory),
    ("nomi-health-check-sessions", ManagedRootKind::Directory),
    ("browser-profile", ManagedRootKind::Directory),
    ("browser-profiles", ManagedRootKind::Directory),
    ("browser-data", ManagedRootKind::Directory),
    ("browser-state", ManagedRootKind::Directory),
    ("login-profile", ManagedRootKind::Directory),
    ("knowledge-browser", ManagedRootKind::Directory),
    ("skills", ManagedRootKind::Directory),
    ("builtin-skills", ManagedRootKind::Directory),
    ("builtin-rules", ManagedRootKind::Directory),
    (".builtin-skills.tmp", ManagedRootKind::Directory),
    (".builtin-skills.old", ManagedRootKind::Directory),
    (".builtin-skills.lock", ManagedRootKind::File),
    ("preset-rules", ManagedRootKind::Directory),
    ("preset-skills", ManagedRootKind::Directory),
    ("preset-instructions", ManagedRootKind::Directory),
    ("preset-avatars", ManagedRootKind::Directory),
    ("extensions", ManagedRootKind::Directory),
    ("extension-states.json", ManagedRootKind::File),
    ("custom-skill-paths.json", ManagedRootKind::File),
    // Legacy cleanup only: the companion credential feature no longer owns this directory.
    ("browser-secrets", ManagedRootKind::Directory),
    ("codex-acp-home", ManagedRootKind::Directory),
    ("agent-executions", ManagedRootKind::Directory),
    ("terminal-mcp", ManagedRootKind::Directory),
    ("mcp-endpoints.json", ManagedRootKind::File),
    ("mcp-endpoints.json.tmp", ManagedRootKind::File),
    ("local-ai", ManagedRootKind::Directory),
    (".relocated-from", ManagedRootKind::File),
    (".relocated-done", ManagedRootKind::File),
];
// Exact managed-root registry emitted by the v2 reset planner, which is the
// planner every current release still uses.
//
// This is frozen for the same reason v1 is. The plan is persisted in the user's
// data directory and is compared element-by-element against this build's
// registry by `validate_plan` and the completed-plan replay check, so deriving
// the v2 shape from `MANAGED_DATASET_ROOTS` at runtime would silently
// invalidate every plan written by an older build the moment that registry
// changes — which is exactly how the `browser-secrets` regression bricked
// interrupted resets carried across an upgrade.
//
// `released_v2_managed_roots_match_the_current_writer` proves this list is
// still what the writer produces. If that test fails, the live registry moved;
// pick one:
//   * the move is not intended -> restore the registry entry (removals also
//     abandon data left by older installations, so a retired subsystem's root
//     stays here marked cleanup-only);
//   * the move is intended -> mint a new plan version: add
//     `RELEASED_V3_MANAGED_ROOTS`, add an arm to `released_plan_shape`, and
//     leave this list untouched so plans already on disk keep validating.
// Editing this list in place is never correct.
const RELEASED_V2_MANAGED_ROOTS: &[(&str, ManagedRootKind)] = &[
    ("nomifun-backend.db-wal", ManagedRootKind::File),
    ("nomifun-backend.db-shm", ManagedRootKind::File),
    ("nomifun-backend.db-journal", ManagedRootKind::File),
    ("nomifun-backend.db.migrate.lock", ManagedRootKind::File),
    ("nomifun-backend.db", ManagedRootKind::File),
    ("storage-generation", ManagedRootKind::File),
    ("dataset-v3.json", ManagedRootKind::File),
    (".dataset-v3.bootstrap.json", ManagedRootKind::File),
    ("factory-reset.pending", ManagedRootKind::File),
    ("encryption_key", ManagedRootKind::File),
    // `dir-config.json` and the work-root owner are `ResetPolicy::Preserve`
    // host control files, so unlike v1 the v2 shape does not quarantine them.
    (".nomifun-work-root-binding.json", ManagedRootKind::File),
    ("conversations", ManagedRootKind::Directory),
    ("attachments", ManagedRootKind::Directory),
    ("knowledge", ManagedRootKind::Directory),
    ("projects", ManagedRootKind::Directory),
    ("companion", ManagedRootKind::Directory),
    ("cron", ManagedRootKind::Directory),
    ("workshop", ManagedRootKind::Directory),
    // Legacy root of the retired public-agent domain (cleanup only).
    ("public-agents", ManagedRootKind::Directory),
    ("preview-history", ManagedRootKind::Directory),
    ("agent-process-registry.json", ManagedRootKind::File),
    ("nomi-sessions", ManagedRootKind::Directory),
    ("nomi-health-check-sessions", ManagedRootKind::Directory),
    ("browser-profile", ManagedRootKind::Directory),
    ("browser-profiles", ManagedRootKind::Directory),
    ("browser-data", ManagedRootKind::Directory),
    ("browser-state", ManagedRootKind::Directory),
    ("login-profile", ManagedRootKind::Directory),
    ("knowledge-browser", ManagedRootKind::Directory),
    ("skills", ManagedRootKind::Directory),
    ("builtin-skills", ManagedRootKind::Directory),
    ("builtin-rules", ManagedRootKind::Directory),
    (".builtin-skills.tmp", ManagedRootKind::Directory),
    (".builtin-skills.old", ManagedRootKind::Directory),
    (".builtin-skills.lock", ManagedRootKind::File),
    ("preset-rules", ManagedRootKind::Directory),
    ("preset-skills", ManagedRootKind::Directory),
    ("preset-instructions", ManagedRootKind::Directory),
    ("preset-avatars", ManagedRootKind::Directory),
    ("extensions", ManagedRootKind::Directory),
    ("extension-states.json", ManagedRootKind::File),
    ("custom-skill-paths.json", ManagedRootKind::File),
    // Legacy cleanup only: the companion credential feature no longer owns
    // this directory. Removing it changes this frozen shape; see the note on
    // the registry entry in `dataset_roots`.
    ("browser-secrets", ManagedRootKind::Directory),
    ("codex-acp-home", ManagedRootKind::Directory),
    ("agent-executions", ManagedRootKind::Directory),
    ("terminal-mcp", ManagedRootKind::Directory),
    ("mcp-endpoints.json", ManagedRootKind::File),
    ("mcp-endpoints.json.tmp", ManagedRootKind::File),
    ("local-ai", ManagedRootKind::Directory),
    (".relocated-from", ManagedRootKind::File),
    (".relocated-done", ManagedRootKind::File),
];
// These paths identify a directory as a NomiFun data root rather than a safe
// external work root. `server.lock` is durable by design, so it also covers a
// first boot that stopped before storage-generation was installed.
const DATA_ROOT_IDENTITY_ARTIFACTS: &[&str] = &[
    "server.lock",
    "server.lock.info",
    STORAGE_GENERATION_FILE,
    V3_DATASET_RECEIPT_FILE,
    V3_DATASET_BOOTSTRAP_FILE,
    V3_DATASET_RESET_REQUEST_FILE,
    V3_DATASET_RESET_DIR,
    RETIRED_DATASETS_DIR,
    "dir-config.json",
];

fn lifecycle_managed_roots(
) -> impl Iterator<Item = (&'static str, ManagedRootKind)> {
    // Unknown files/directories are not recursively swept: logs, runtime
    // locks, developer caches and arbitrary user files must survive a reset.
    DB_FAMILY
        .iter()
        .copied()
        .map(|path| (path, ManagedRootKind::File))
        .chain([
            (STORAGE_GENERATION_FILE, ManagedRootKind::File),
            (V3_DATASET_RECEIPT_FILE, ManagedRootKind::File),
            (V3_DATASET_BOOTSTRAP_FILE, ManagedRootKind::File),
            // Retire the pre-v3 control artifact with its dataset; it is never
            // parsed as a compatibility request.
            (RETIRED_FACTORY_RESET_MARKER, ManagedRootKind::File),
        ])
}

fn dataset_managed_roots(
    preserve_host_control: bool,
) -> Vec<(&'static str, ManagedRootKind)> {
    let roots: Box<
        dyn Iterator<Item = &'static crate::dataset_roots::ManagedDatasetRoot>,
    > = if preserve_host_control {
        Box::new(reset_managed_dataset_roots())
    } else {
        Box::new(managed_dataset_roots())
    };
    roots
        .map(|root| {
            (
                root.path,
                match root.kind {
                    DatasetRootKind::File => ManagedRootKind::File,
                    DatasetRootKind::Directory => ManagedRootKind::Directory,
                },
            )
        })
        .collect()
}

/// Everything about a persisted plan that is fixed by its version.
///
/// A plan is a durable contract with older builds, so each released version
/// pins its own frozen shape here instead of inheriting whatever the current
/// process happens to compute.
#[derive(Debug, Clone, Copy)]
struct ReleasedPlanShape {
    managed_roots: &'static [(&'static str, ManagedRootKind)],
    /// v1 predates `persist_work_dir`; every later writer sets it.
    persists_work_dir: bool,
}

/// Resolve the frozen shape of a persisted plan version.
///
/// Every arm is written out explicitly: an unknown version is refused rather
/// than treated as "current", so a build that reads a plan from a newer release
/// cannot execute it against the wrong registry. Adding a version means adding
/// an arm plus a new `RELEASED_V*_MANAGED_ROOTS` list — never editing one.
fn released_plan_shape(version: u32) -> Result<ReleasedPlanShape, AppError> {
    match version {
        LEGACY_PLAN_VERSION => Ok(ReleasedPlanShape {
            managed_roots: RELEASED_V1_MANAGED_ROOTS,
            persists_work_dir: false,
        }),
        PLAN_VERSION => Ok(ReleasedPlanShape {
            managed_roots: RELEASED_V2_MANAGED_ROOTS,
            persists_work_dir: true,
        }),
        _ => Err(AppError::Internal(format!(
            "unsupported v3 dataset reset plan version {version}"
        ))),
    }
}

/// The managed-root registry the *current writer* emits.
///
/// Live derivation is correct here and only here: arming a new reset, and the
/// layout/probe checks that describe what a new reset would move, must reflect
/// the registry this build actually owns. Anything that reads a *persisted*
/// plan must instead go through [`released_plan_shape`], because those bytes
/// may have been written by another build.
///
/// The two are held together by
/// `released_v2_managed_roots_match_the_current_writer`. Drift is also
/// self-announcing at runtime: `arm_v3_dataset_reset` validates the plan it
/// just built against the frozen shape, so a drifted registry fails before any
/// data is moved rather than persisting a plan no reader accepts.
fn current_writer_managed_roots() -> Vec<(&'static str, ManagedRootKind)> {
    let mut roots = lifecycle_managed_roots().collect::<Vec<_>>();
    roots.extend(dataset_managed_roots(true));
    roots
}

/// Compare a persisted plan's root list against a frozen registry.
///
/// The comparison is positional: the quarantine order is part of the crash
/// safety argument, not an incidental detail.
fn plan_roots_match_registry(
    plan: &DatasetResetPlan,
    managed_roots: &[(&'static str, ManagedRootKind)],
    expects_work_root: bool,
) -> bool {
    let mut expected = managed_roots
        .iter()
        .map(|(path, kind)| {
            (
                ManagedRootBase::DataDir,
                *path,
                *kind,
                format!("{}/{}", plan.retired_dir, path),
            )
        })
        .collect::<Vec<_>>();
    if expects_work_root {
        expected.push((
            ManagedRootBase::WorkDir,
            MANAGED_WORKSPACES_DIR,
            ManagedRootKind::Directory,
            format!("{}/{}", plan.work_retired_dir, MANAGED_WORKSPACES_DIR),
        ));
    }
    plan.roots.len() == expected.len()
        && plan
            .roots
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| {
                actual.base == expected.0
                    && actual.relative_path == expected.1
                    && actual.kind == expected.2
                    && actual.retired_relative_path == expected.3
            })
}

/// Does this plan's root list match the frozen registry of its own version?
///
/// A `false` answer means the bytes were written by a build whose managed-root
/// registry differed from the frozen shape — an upgrade artifact, not an I/O
/// failure. Callers must treat it as "this is not a plan I can execute", never
/// as a reason to abort startup while a user's data directory is half moved.
/// An *unsupported* version still errors: refusing to touch a plan from a newer
/// release is the correct answer to a downgrade.
fn plan_roots_match_released_registry(
    plan: &DatasetResetPlan,
    data_dir: &Path,
) -> Result<bool, AppError> {
    let shape = released_plan_shape(plan.version)?;
    let canonical_data = canonical_data_dir(data_dir)?;
    let expects_work_root =
        !crate::paths::stored_path_matches(&plan.work_dir, &canonical_data);
    Ok(plan_roots_match_registry(
        plan,
        shape.managed_roots,
        expects_work_root,
    ))
}

/// Fixed-shape v3 explicit-reset request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatasetResetRequest {
    pub version: u32,
    pub operation_id: String,
    pub requested_at: i64,
    #[serde(default)]
    pub origin: Option<DatasetResetRequestOrigin>,
    /// Canonical work root bound to this destructive intent. Both explicit
    /// resets and work-dir changes carry it so a later environment/config
    /// change cannot redirect the reset at another workspace.
    #[serde(default)]
    pub work_dir: Option<String>,
}

/// Stable, generation-bound proof that a destructive request was consumed.
///
/// The active request is a single atomic file. On filesystems where a
/// directory entry can reappear after power loss, deleting that file alone
/// cannot provide exactly-once semantics. This tombstone remains outside the
/// active dataset generations and lets startup consume a replayed request
/// without touching the replacement v3 data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletedResetRequest {
    version: u32,
    operation_id: String,
    origin: DatasetResetRequestOrigin,
    work_dir: String,
    generation: String,
    requested_at: i64,
    completed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletedAutomaticLegacyRetirement {
    version: u32,
    operation_id: String,
    generation: String,
    data_dir: String,
    work_dir: String,
    reason: DatasetResetReason,
    requested_at: i64,
    completed_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DatasetResetRequestOrigin {
    UserExplicitFactoryReset,
    WorkDirChange,
}

impl DatasetResetRequest {
    fn new(
        origin: DatasetResetRequestOrigin,
        work_dir: Option<String>,
    ) -> Self {
        Self {
            version: RESET_REQUEST_VERSION,
            operation_id: Uuid::now_v7().to_string(),
            requested_at: now_ms(),
            origin: Some(origin),
            work_dir,
        }
    }

    fn validate(&self) -> Result<(), AppError> {
        match self.version {
            LEGACY_RESET_REQUEST_VERSION
                if self.work_dir.is_none() && self.origin.is_none() => {}
            RESET_REQUEST_VERSION
                if matches!(
                    (self.origin, self.work_dir.as_ref()),
                    (
                        Some(
                            DatasetResetRequestOrigin::UserExplicitFactoryReset
                        ),
                        Some(_)
                    ) | (
                        Some(DatasetResetRequestOrigin::WorkDirChange),
                        Some(_)
                    )
                ) => {}
            LEGACY_RESET_REQUEST_VERSION => {
                return Err(AppError::Internal(
                    "legacy v3 dataset reset request must not contain origin or work_dir"
                        .into(),
                ));
            }
            RESET_REQUEST_VERSION => {
                return Err(AppError::Internal(
                    "v3 dataset reset request origin does not match its work_dir"
                        .into(),
                ));
            }
            _ => {
                return Err(AppError::Internal(format!(
                    "unsupported v3 dataset reset request version {}",
                    self.version
                )));
            }
        }
        validate_uuidv7(&self.operation_id).map_err(|error| {
            AppError::Internal(format!(
                "invalid v3 dataset reset request operation_id: {error}"
            ))
        })?;
        if self.requested_at <= 0 {
            return Err(AppError::Internal(
                "v3 dataset reset request requested_at must be positive".into(),
            ));
        }
        if let Some(work_dir) = &self.work_dir {
            let path = Path::new(work_dir);
            if work_dir.is_empty()
                || !path.is_absolute()
                || crate::workspace_path_has_edge_whitespace_segment(path)
            {
                return Err(AppError::Internal(
                    "v3 dataset reset request work_dir must be a safe non-empty absolute path"
                        .into(),
                ));
            }
        }
        Ok(())
    }
}

impl CompletedResetRequest {
    fn validate(&self) -> Result<(), AppError> {
        if self.version != COMPLETED_RESET_REQUEST_VERSION {
            return Err(AppError::Internal(format!(
                "unsupported completed reset request version {}",
                self.version
            )));
        }
        validate_uuidv7(&self.operation_id).map_err(|error| {
            AppError::Internal(format!(
                "invalid completed reset request operation_id: {error}"
            ))
        })?;
        validate_uuidv7(&self.generation).map_err(|error| {
            AppError::Internal(format!(
                "invalid completed reset request generation: {error}"
            ))
        })?;
        if self.requested_at <= 0
            || self.completed_at <= 0
            || self.completed_at < self.requested_at
        {
            return Err(AppError::Internal(
                "completed reset request timestamps are invalid".into(),
            ));
        }
        let work_dir = Path::new(&self.work_dir);
        if self.work_dir.is_empty()
            || !work_dir.is_absolute()
            || crate::workspace_path_has_edge_whitespace_segment(work_dir)
        {
            return Err(AppError::Internal(
                "completed reset request work_dir is unsafe".into(),
            ));
        }
        Ok(())
    }
}

impl CompletedAutomaticLegacyRetirement {
    fn validate(&self) -> Result<(), AppError> {
        if self.version != AUTOMATIC_LEGACY_RETIREMENT_VERSION {
            return Err(AppError::Internal(format!(
                "unsupported automatic legacy retirement marker version {}",
                self.version
            )));
        }
        validate_uuidv7(&self.operation_id).map_err(|error| {
            AppError::Internal(format!(
                "invalid automatic legacy retirement operation ID: {error}"
            ))
        })?;
        validate_uuidv7(&self.generation).map_err(|error| {
            AppError::Internal(format!(
                "invalid automatic legacy retirement generation: {error}"
            ))
        })?;
        if self.requested_at <= 0
            || self.completed_at < self.requested_at
        {
            return Err(AppError::Internal(
                "automatic legacy retirement timestamps are invalid".into(),
            ));
        }
        if matches!(
            self.reason,
            DatasetResetReason::ExplicitFactoryReset
        ) {
            return Err(AppError::Internal(
                "an explicit reset cannot consume automatic legacy retirement"
                    .into(),
            ));
        }
        for (value, description) in [
            (&self.data_dir, "data root"),
            (&self.work_dir, "work root"),
        ] {
            let path = Path::new(value);
            if value.is_empty()
                || !path.is_absolute()
                || crate::workspace_path_has_edge_whitespace_segment(path)
            {
                return Err(AppError::Internal(format!(
                    "automatic legacy retirement {description} is unsafe"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetResetReason {
    NonV3Dataset,
    ExplicitFactoryReset,
    WorkDirChange,
}

fn request_origin_for_reason(
    reason: DatasetResetReason,
) -> Option<DatasetResetRequestOrigin> {
    match reason {
        DatasetResetReason::NonV3Dataset => None,
        DatasetResetReason::ExplicitFactoryReset => {
            Some(DatasetResetRequestOrigin::UserExplicitFactoryReset)
        }
        DatasetResetReason::WorkDirChange => {
            Some(DatasetResetRequestOrigin::WorkDirChange)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedRootBase {
    DataDir,
    WorkDir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedRootKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedRootPlan {
    pub base: ManagedRootBase,
    pub relative_path: String,
    pub retired_relative_path: String,
    pub kind: ManagedRootKind,
    pub initially_present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetResetPlan {
    pub version: u32,
    pub operation_id: String,
    pub reason: DatasetResetReason,
    pub data_dir: String,
    pub work_dir: String,
    /// Persist the plan's canonical work root after quarantine so every retry
    /// resolves the same root. New plans always set this; `serde(default)`
    /// keeps released legacy plans readable.
    #[serde(default)]
    pub persist_work_dir: bool,
    /// True only when the read-only legacy classifier, rather than an
    /// explicit v2 request, authorized this one-time retirement.
    #[serde(default)]
    pub automatic_legacy_retirement: bool,
    pub generation: String,
    pub retired_dir: String,
    pub work_retired_dir: String,
    pub requested_at: i64,
    pub roots: Vec<ManagedRootPlan>,
}

fn plan_requires_work_dir_persistence(
    plan: &DatasetResetPlan,
) -> bool {
    (plan.version == PLAN_VERSION && plan.persist_work_dir)
        || plan.reason == DatasetResetReason::WorkDirChange
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetReceipt {
    pub contract_version: u32,
    pub generation: String,
    /// Canonical resolved work root that belongs to this dataset generation.
    ///
    /// This is deliberately part of the receipt rather than inferred from the
    /// current process configuration.  A database receipt must never make an
    /// unrelated `<work_dir>/conversations` tree look like part of the same
    /// dataset merely because the operator changed `--work-dir`.
    pub work_root: String,
    /// Old receipts deserialize this as `false`. Once the one-time owner
    /// compatibility backfill succeeds, the receipt is atomically upgraded to
    /// `true`; thereafter neither owner nor data-side binding may be recreated
    /// from absence.
    #[serde(default)]
    pub work_root_binding_required: bool,
    pub installed_at: i64,
}

const WORK_ROOT_OWNER_VERSION: u32 = 1;
const WORK_ROOT_BINDING_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkRootOwner {
    version: u32,
    data_root: String,
    generation: String,
    installed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkRootBinding {
    version: u32,
    data_root: String,
    work_root: String,
    generation: String,
    installed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatasetBootstrapBinding {
    contract_version: u32,
    generation: String,
    work_root: String,
    prepared_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetReceiptStatus {
    Missing,
    Current,
    WorkRootMismatch,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetPreparation {
    Unchanged,
    ResetApplied,
}

fn request_path(data_dir: &Path) -> PathBuf {
    data_dir.join(V3_DATASET_RESET_REQUEST_FILE)
}

fn completed_request_path(
    data_dir: &Path,
    operation_id: &str,
) -> PathBuf {
    completed_requests_dir(data_dir)
        .join(format!("{operation_id}.json"))
}

fn completed_requests_dir(data_dir: &Path) -> PathBuf {
    data_dir
        .join(RETIRED_DATASETS_DIR)
        .join(COMPLETED_RESET_REQUESTS_DIR)
}

fn automatic_legacy_retirement_path(data_dir: &Path) -> PathBuf {
    data_dir
        .join(RETIRED_DATASETS_DIR)
        .join(AUTOMATIC_LEGACY_RETIREMENT_FILE)
}

fn completed_requests_dir_exists_and_is_safe(
    data_dir: &Path,
) -> Result<bool, AppError> {
    let retired_root = data_dir.join(RETIRED_DATASETS_DIR);
    for (path, description) in [
        (&retired_root, "retired-datasets directory"),
        (
            &completed_requests_dir(data_dir),
            "completed reset requests directory",
        ),
    ] {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                validate_root_metadata(
                    path,
                    &metadata,
                    ManagedRootKind::Directory,
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "inspect {description} {}: {error}",
                    path.display()
                )));
            }
        }
    }
    Ok(true)
}

fn ensure_completed_requests_dir(data_dir: &Path) -> Result<(), AppError> {
    let retired_root = data_dir.join(RETIRED_DATASETS_DIR);
    ensure_real_directory(&retired_root, "retired-datasets directory")?;
    sync_parent(&retired_root).map_err(|error| {
        AppError::Internal(format!(
            "sync retired-datasets directory publication: {error}"
        ))
    })?;
    let completed = completed_requests_dir(data_dir);
    ensure_real_directory(
        &completed,
        "completed reset requests directory",
    )?;
    sync_parent(&completed).map_err(|error| {
        AppError::Internal(format!(
            "sync completed reset requests directory publication: {error}"
        ))
    })
}

fn reset_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(V3_DATASET_RESET_DIR)
}

fn plan_path(data_dir: &Path) -> PathBuf {
    reset_dir(data_dir).join(V3_DATASET_RESET_PLAN_FILE)
}

fn phase_path(data_dir: &Path, phase: &str) -> PathBuf {
    reset_dir(data_dir).join(phase_file_name(phase))
}

fn phase_file_name(phase: &str) -> String {
    format!("phase-{phase}")
}

fn receipt_path(data_dir: &Path) -> PathBuf {
    data_dir.join(V3_DATASET_RECEIPT_FILE)
}

fn bootstrap_binding_path(data_dir: &Path) -> PathBuf {
    data_dir.join(V3_DATASET_BOOTSTRAP_FILE)
}

fn retired_dir_config_path(data_dir: &Path, generation: &str) -> PathBuf {
    data_dir
        .join(RETIRED_DATASETS_DIR)
        .join(format!("id-reference-v3-{generation}"))
        .join(crate::dir_config::DIR_CONFIG_FILE)
}

fn canonical_data_dir(data_dir: &Path) -> Result<PathBuf, AppError> {
    let metadata = fs::symlink_metadata(data_dir).map_err(|error| {
        AppError::Internal(format!(
            "inspect dataset root {}: {error}",
            data_dir.display()
        ))
    })?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Err(AppError::Internal(format!(
            "dataset root must be a real directory: {}",
            data_dir.display()
        )));
    }
    fs::canonicalize(data_dir)
        .map(|canonical| crate::paths::simplified(&canonical))
        .map_err(|error| {
            AppError::Internal(format!(
                "canonicalize dataset root {}: {error}",
                data_dir.display()
            ))
        })
}

fn canonical_work_dir(work_dir: &Path) -> Result<PathBuf, AppError> {
    ensure_real_directory(work_dir, "managed work directory")?;
    canonical_existing_work_dir(work_dir)
}

fn canonical_existing_work_dir(work_dir: &Path) -> Result<PathBuf, AppError> {
    let metadata = fs::symlink_metadata(work_dir).map_err(|error| {
        AppError::Internal(format!(
            "inspect managed work directory {}: {error}",
            work_dir.display()
        ))
    })?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Err(AppError::Internal(format!(
            "managed work directory must be a real directory: {}",
            work_dir.display()
        )));
    }
    fs::canonicalize(work_dir)
        .map(|canonical| crate::paths::simplified(&canonical))
        .map_err(|error| {
            AppError::Internal(format!(
                "canonicalize managed work directory {}: {error}",
                work_dir.display()
            ))
        })
}

fn work_root_owner_path(canonical_work: &Path) -> PathBuf {
    canonical_work.join(WORK_ROOT_OWNER_FILE)
}

fn work_root_binding_path(canonical_data: &Path) -> PathBuf {
    canonical_data.join(WORK_ROOT_BINDING_FILE)
}

fn root_contains_any_artifact(
    root: &Path,
    relative_paths: impl IntoIterator<Item = &'static str>,
    label: &str,
) -> Result<bool, AppError> {
    for relative_path in relative_paths {
        let path = root.join(relative_path);
        match fs::symlink_metadata(&path) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "inspect {label} artifact {}: {error}",
                    path.display()
                )));
            }
        }
    }
    Ok(false)
}

fn work_root_has_data_root_identity(canonical_work: &Path) -> Result<bool, AppError> {
    if root_contains_any_artifact(
        canonical_work,
        DB_FAMILY.iter().copied(),
        "database-family",
    )? {
        return Ok(true);
    }
    root_contains_any_artifact(
        canonical_work,
        DATA_ROOT_IDENTITY_ARTIFACTS.iter().copied(),
        "data-root identity",
    )
}

fn validate_work_root_owner(owner: &WorkRootOwner) -> Result<(), AppError> {
    if owner.version != WORK_ROOT_OWNER_VERSION
        || owner.installed_at <= 0
        || validate_uuidv7(&owner.generation).is_err()
    {
        return Err(AppError::Internal(
            "work-root owner marker has an invalid version, generation, or timestamp"
                .into(),
        ));
    }
    let data_root = Path::new(&owner.data_root);
    if owner.data_root.is_empty()
        || !data_root.is_absolute()
        || crate::workspace_path_has_edge_whitespace_segment(data_root)
    {
        return Err(AppError::Internal(
            "work-root owner marker has an unsafe data root".into(),
        ));
    }
    Ok(())
}

fn read_work_root_owner(
    canonical_work: &Path,
) -> Result<Option<WorkRootOwner>, AppError> {
    let path = work_root_owner_path(canonical_work);
    let bytes =
        match read_bounded_regular_file(&path, MAX_CONTROL_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(None);
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "read work-root owner marker {}: {error}",
                    path.display()
                )));
            }
        };
    let owner: WorkRootOwner =
        serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Internal(format!(
                "invalid work-root owner marker {}: {error}",
                path.display()
            ))
        })?;
    validate_work_root_owner(&owner)?;
    Ok(Some(owner))
}

fn validate_work_root_binding(
    binding: &WorkRootBinding,
) -> Result<(), AppError> {
    if binding.version != WORK_ROOT_BINDING_VERSION
        || binding.installed_at <= 0
        || validate_uuidv7(&binding.generation).is_err()
    {
        return Err(AppError::Internal(
            "work-root binding has an invalid version, generation, or timestamp"
                .into(),
        ));
    }
    for (label, value) in [
        ("data", binding.data_root.as_str()),
        ("work", binding.work_root.as_str()),
    ] {
        let path = Path::new(value);
        if value.is_empty()
            || !path.is_absolute()
            || crate::workspace_path_has_edge_whitespace_segment(path)
        {
            return Err(AppError::Internal(format!(
                "work-root binding has an unsafe {label} root"
            )));
        }
    }
    Ok(())
}

fn read_work_root_binding(
    canonical_data: &Path,
) -> Result<Option<WorkRootBinding>, AppError> {
    let path = work_root_binding_path(canonical_data);
    let bytes =
        match read_bounded_regular_file(&path, MAX_CONTROL_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(None);
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "read work-root binding {}: {error}",
                    path.display()
                )));
            }
        };
    let binding: WorkRootBinding =
        serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Internal(format!(
                "invalid work-root binding {}: {error}",
                path.display()
            ))
        })?;
    validate_work_root_binding(&binding)?;
    Ok(Some(binding))
}

/// Compare two durable marker spellings of canonical paths, tolerating the
/// Windows verbatim (`\\?\`) prefix on either side (see
/// [`crate::paths::paths_equivalent`]). Empty values never match.
fn stored_paths_equivalent(a: &str, b: &str) -> bool {
    !a.is_empty()
        && !b.is_empty()
        && crate::paths::paths_equivalent(Path::new(a), Path::new(b))
}

fn owner_matches(
    owner: &WorkRootOwner,
    canonical_data: &Path,
    generation: &str,
) -> bool {
    // Spelling-tolerant: markers written by older releases stored the
    // Windows verbatim (`\\?\`) canonical form; current code stores the
    // simplified form. Both refer to the same directory.
    crate::paths::stored_path_matches(&owner.data_root, canonical_data)
        && owner.generation == generation
}

fn binding_matches(
    binding: &WorkRootBinding,
    canonical_data: &Path,
    canonical_work: &Path,
    generation: &str,
) -> bool {
    crate::paths::stored_path_matches(&binding.data_root, canonical_data)
        && crate::paths::stored_path_matches(&binding.work_root, canonical_work)
        && binding.generation == generation
}

/// Reject a data directory that is already reserved as another dataset's
/// external work root.
///
/// The application also holds the work-root lock on `data_dir` for its full
/// lifetime. This durable owner check covers the non-concurrent case after the
/// other process exits, so a later reset cannot treat that dataset's active
/// `conversations` as this data root's own managed data.
pub fn require_data_root_not_owned_as_external_work(
    data_dir: &Path,
) -> Result<(), AppError> {
    let canonical_data = canonical_data_dir(data_dir)?;
    if let Some(owner) = read_work_root_owner(&canonical_data)?
        && !crate::paths::stored_path_matches(&owner.data_root, &canonical_data)
    {
        return Err(AppError::Conflict(format!(
            "data directory {} is reserved as the external work root of another NomiFun dataset",
            canonical_data.display()
        )));
    }
    Ok(())
}

fn ensure_v3_work_root_owner_with_policy(
    data_dir: &Path,
    work_dir: &Path,
    generation: &str,
    allow_generation_rotation: bool,
) -> Result<(), AppError> {
    validate_uuidv7(generation).map_err(|error| {
        AppError::Internal(format!(
            "invalid work-root owner generation: {error}"
        ))
    })?;
    let canonical_data = canonical_data_dir(data_dir)?;
    let canonical_work = canonical_existing_work_dir(work_dir)?;
    require_data_root_not_owned_as_external_work(&canonical_data)?;
    if canonical_work != canonical_data
        && work_root_has_data_root_identity(&canonical_work)?
    {
        return Err(AppError::Conflict(format!(
            "work directory {} is already a NomiFun data root",
            canonical_work.display()
        )));
    }
    let desired = WorkRootOwner {
        version: WORK_ROOT_OWNER_VERSION,
        data_root: canonical_data.display().to_string(),
        generation: generation.to_owned(),
        installed_at: now_ms(),
    };
    let path = work_root_owner_path(&canonical_work);

    if let Some(existing) = read_work_root_owner(&canonical_work)? {
        if !crate::paths::stored_path_matches(&existing.data_root, &canonical_data) {
            return Err(AppError::Conflict(format!(
                "work directory {} is already owned by another NomiFun data root",
                canonical_work.display()
            )));
        }
        if existing.generation == desired.generation {
            return Ok(());
        }
        if !allow_generation_rotation {
            return Err(AppError::Conflict(format!(
                "work directory {} is owned by a different v3 dataset generation; \
                 only a validated pending reset plan may rotate it",
                canonical_work.display()
            )));
        }
        let bytes = serde_json::to_vec_pretty(&desired).map_err(
            |error| {
                AppError::Internal(format!(
                    "serialize rotated work-root owner marker: {error}"
                ))
            },
        )?;
        return write_atomic(&path, &bytes).map_err(|error| {
            AppError::Internal(format!(
                "rotate work-root owner marker {}: {error}",
                path.display()
            ))
        });
    }

    let bytes = serde_json::to_vec_pretty(&desired).map_err(|error| {
        AppError::Internal(format!(
            "serialize work-root owner marker: {error}"
        ))
    })?;
    match write_atomic_new(&path, &bytes) {
        Ok(()) => Ok(()),
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            let existing = read_work_root_owner(&canonical_work)?
                .ok_or_else(|| {
                    AppError::Conflict(
                        "work-root owner changed concurrently".into(),
                    )
                })?;
            if owner_matches(&existing, &canonical_data, generation) {
                Ok(())
            } else {
                Err(AppError::Conflict(format!(
                    "work directory {} was claimed concurrently",
                    canonical_work.display()
                )))
            }
        }
        Err(error) => Err(AppError::Internal(format!(
            "install work-root owner marker {}: {error}",
            path.display()
        ))),
    }
}

/// Install or validate a persistent work-root owner without changing an
/// existing generation.
///
/// This is the safe default for bootstrap, finalized compatibility backfill,
/// backup, and receipt publication. A receipt/storage rollback must never be
/// able to rewrite an existing owner and make stale state look current.
fn ensure_v3_work_root_owner(
    data_dir: &Path,
    work_dir: &Path,
    generation: &str,
) -> Result<(), AppError> {
    ensure_v3_work_root_owner_with_policy(
        data_dir,
        work_dir,
        generation,
        false,
    )
}

/// Rotate an owner only while executing a validated reset transition.
fn ensure_v3_work_root_owner_for_reset(
    data_dir: &Path,
    work_dir: &Path,
    generation: &str,
) -> Result<(), AppError> {
    ensure_v3_work_root_owner_with_policy(
        data_dir,
        work_dir,
        generation,
        true,
    )
}

/// Install the data-side proof that the owner marker has been backfilled.
///
/// Once this proof exists, a missing/recreated work-root owner is never
/// silently reinstalled. Absence is accepted exactly once for pre-binding v3
/// datasets and fresh
/// bootstrap; owner publication happens first so every crash point is
/// retryable without weakening the fail-closed rule.
fn ensure_v3_work_root_binding_with_requirement(
    data_dir: &Path,
    work_dir: &Path,
    generation: &str,
    binding_required: bool,
) -> Result<(), AppError> {
    validate_uuidv7(generation).map_err(|error| {
        AppError::Internal(format!(
            "invalid work-root binding generation: {error}"
        ))
    })?;
    let canonical_data = canonical_data_dir(data_dir)?;
    let canonical_work = canonical_existing_work_dir(work_dir)?;
    if let Some(existing) = read_work_root_binding(&canonical_data)? {
        if !binding_matches(
            &existing,
            &canonical_data,
            &canonical_work,
            generation,
        ) {
            return Err(AppError::Conflict(format!(
                "work-root binding for {} does not match the current dataset generation",
                canonical_work.display()
            )));
        }
        return require_v3_work_root_owner(
            &canonical_data,
            &canonical_work,
            generation,
        );
    }
    if binding_required {
        return Err(AppError::Conflict(format!(
            "work-root binding required by the finalized v3 receipt is missing for {}",
            canonical_work.display()
        )));
    }

    ensure_v3_work_root_owner(
        &canonical_data,
        &canonical_work,
        generation,
    )?;
    let desired = WorkRootBinding {
        version: WORK_ROOT_BINDING_VERSION,
        data_root: canonical_data.display().to_string(),
        work_root: canonical_work.display().to_string(),
        generation: generation.to_owned(),
        installed_at: now_ms(),
    };
    let bytes = serde_json::to_vec_pretty(&desired).map_err(|error| {
        AppError::Internal(format!(
            "serialize work-root binding: {error}"
        ))
    })?;
    let path = work_root_binding_path(&canonical_data);
    match write_atomic_new(&path, &bytes) {
        Ok(()) => Ok(()),
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            let existing = read_work_root_binding(&canonical_data)?
                .ok_or_else(|| {
                    AppError::Conflict(
                        "work-root binding changed concurrently".into(),
                    )
                })?;
            if binding_matches(
                &existing,
                &canonical_data,
                &canonical_work,
                generation,
            ) {
                require_v3_work_root_owner(
                    &canonical_data,
                    &canonical_work,
                    generation,
                )
            } else {
                Err(AppError::Conflict(
                    "work-root binding was claimed concurrently".into(),
                ))
            }
        }
        Err(error) => Err(AppError::Internal(format!(
            "install work-root binding {}: {error}",
            path.display()
        ))),
    }
}

fn require_v3_work_root_binding(
    data_dir: &Path,
    work_dir: &Path,
    generation: &str,
) -> Result<(), AppError> {
    let canonical_data = canonical_data_dir(data_dir)?;
    let canonical_work = canonical_existing_work_dir(work_dir)?;
    let binding = read_work_root_binding(&canonical_data)?.ok_or_else(|| {
        AppError::Conflict(format!(
            "finalized v3 receipt requires a work-root binding for {}, but it is missing",
            canonical_work.display()
        ))
    })?;
    if !binding_matches(
        &binding,
        &canonical_data,
        &canonical_work,
        generation,
    ) {
        return Err(AppError::Conflict(format!(
            "work-root binding for {} does not match the finalized v3 dataset generation",
            canonical_work.display()
        )));
    }
    require_v3_work_root_owner(
        &canonical_data,
        &canonical_work,
        generation,
    )
}

pub fn ensure_v3_work_root_binding(
    data_dir: &Path,
    work_dir: &Path,
    generation: &str,
) -> Result<(), AppError> {
    let canonical_work = canonical_existing_work_dir(work_dir)?;
    let binding_required = match read_bounded_regular_file(
        &receipt_path(data_dir),
        MAX_CONTROL_FILE_BYTES,
    ) {
        Ok(bytes) => {
            let receipt: DatasetReceipt =
                serde_json::from_slice(&bytes).map_err(|error| {
                    AppError::Internal(format!(
                        "parse receipt while enforcing work-root binding: {error}"
                    ))
                })?;
            if receipt.contract_version != V3_DATASET_CONTRACT_VERSION
                || receipt.generation != generation
                || !crate::paths::stored_path_matches(
                    &receipt.work_root,
                    &canonical_work,
                )
            {
                return Err(AppError::Conflict(
                    "finalized receipt does not match the requested work-root binding"
                        .into(),
                ));
            }
            receipt.work_root_binding_required
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            false
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read receipt while enforcing work-root binding: {error}"
            )));
        }
    };
    ensure_v3_work_root_binding_with_requirement(
        data_dir,
        &canonical_work,
        generation,
        binding_required,
    )
}

pub fn require_v3_work_root_owner(
    data_dir: &Path,
    work_dir: &Path,
    generation: &str,
) -> Result<(), AppError> {
    let canonical_data = canonical_data_dir(data_dir)?;
    let canonical_work = canonical_existing_work_dir(work_dir)?;
    let owner = read_work_root_owner(&canonical_work)?.ok_or_else(|| {
        AppError::Internal(format!(
            "work directory {} has no v3 owner marker",
            canonical_work.display()
        ))
    })?;
    if !owner_matches(&owner, &canonical_data, generation) {
        return Err(AppError::Conflict(format!(
            "work directory {} is not owned by this v3 dataset generation",
            canonical_work.display()
        )));
    }
    Ok(())
}

fn data_and_work_roots_overlap(
    canonical_data: &Path,
    canonical_work: &Path,
) -> bool {
    canonical_data != canonical_work
        && (canonical_data.starts_with(canonical_work)
            || canonical_work.starts_with(canonical_data))
}

#[cfg(windows)]
fn path_component_matches_product_name(
    component: &std::ffi::OsStr,
    product_name: &str,
) -> bool {
    component
        .to_str()
        .is_some_and(|value| value.eq_ignore_ascii_case(product_name))
}

#[cfg(not(windows))]
fn path_component_matches_product_name(
    component: &std::ffi::OsStr,
    product_name: &str,
) -> bool {
    component == std::ffi::OsStr::new(product_name)
}

fn external_work_target_has_conversations(
    canonical_data: &Path,
    canonical_work: &Path,
) -> Result<bool, AppError> {
    if canonical_data == canonical_work {
        return Ok(false);
    }
    let conversations = canonical_work.join(MANAGED_WORKSPACES_DIR);
    match fs::symlink_metadata(&conversations) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(AppError::Internal(format!(
            "inspect work-dir change target {}: {error}",
            conversations.display()
        ))),
    }
}

fn validate_disjoint_data_and_work_roots(
    canonical_data: &Path,
    canonical_work: &Path,
) -> Result<(), AppError> {
    if data_and_work_roots_overlap(canonical_data, canonical_work) {
        return Err(AppError::Internal(format!(
            "data directory {} and external work directory {} must not contain one another",
            canonical_data.display(),
            canonical_work.display()
        )));
    }
    Ok(())
}

fn validate_safe_reset_data_and_work_roots(
    canonical_data: &Path,
    canonical_work: &Path,
    data_roots: &[(&str, ManagedRootKind)],
) -> Result<(), AppError> {
    if canonical_data == canonical_work
        || !data_and_work_roots_overlap(canonical_data, canonical_work)
    {
        return Ok(());
    }

    if let Ok(relative_work) = canonical_work.strip_prefix(canonical_data) {
        let first = relative_work.components().next().ok_or_else(|| {
            AppError::Internal("nested work directory has no relative component".into())
        })?;
        let Component::Normal(first) = first else {
            return Err(AppError::Internal(
                "nested work directory has an unsafe relative path".into(),
            ));
        };
        let collides_with_data_root = data_roots.iter().any(|(root, _)| {
            Path::new(root)
                .components()
                .next()
                .and_then(|component| component.as_os_str().to_str())
                .is_some_and(|name| {
                    path_component_matches_product_name(first, name)
                })
        });
        let collides_with_control = [
            RETIRED_DATASETS_DIR,
            V3_DATASET_RESET_DIR,
            V3_DATASET_RESET_REQUEST_FILE,
            crate::dir_config::DIR_CONFIG_FILE,
            WORK_ROOT_OWNER_FILE,
            WORK_ROOT_BINDING_FILE,
        ]
        .iter()
        .any(|reserved| {
            path_component_matches_product_name(first, reserved)
        });
        if collides_with_data_root || collides_with_control {
            return Err(AppError::Internal(format!(
                "nested work directory {} overlaps a product-managed data root",
                canonical_work.display()
            )));
        }
        return Ok(());
    }

    if let Ok(relative_data) = canonical_data.strip_prefix(canonical_work) {
        let first = relative_data.components().next().ok_or_else(|| {
            AppError::Internal("nested data directory has no relative component".into())
        })?;
        let Component::Normal(first) = first else {
            return Err(AppError::Internal(
                "nested data directory has an unsafe relative path".into(),
            ));
        };
        if path_component_matches_product_name(
            first,
            MANAGED_WORKSPACES_DIR,
        ) || path_component_matches_product_name(
            first,
            WORK_RETIRED_DATASETS_DIR,
        )
        {
            return Err(AppError::Internal(format!(
                "nested data directory {} overlaps a product-managed work root",
                canonical_data.display()
            )));
        }
        return Ok(());
    }

    Err(AppError::Internal(
        "unable to prove a safe data/work directory layout".into(),
    ))
}

/// Validate that the current data/work layout can be reset without any planned
/// source containing another source or reset-control destination.
///
/// Historical releases allowed a work root such as
/// `<data_dir>/chosen-workspace`. That layout remains safe when its first
/// relative component is not a product-managed data root, so existing users
/// must not be stranded merely because newer work-dir changes use the stricter
/// disjoint layout.
pub fn require_safe_data_work_root_layout(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<(), AppError> {
    let canonical_data = canonical_data_dir(data_dir)?;
    let canonical_work = canonical_existing_work_dir(work_dir)?;
    let managed_roots = current_writer_managed_roots();
    validate_safe_reset_data_and_work_roots(
        &canonical_data,
        &canonical_work,
        &managed_roots,
    )
}

/// Require a new work-root target to be disjoint and free of the reserved
/// `conversations` directory. Without an existing receipt binding, that
/// directory is not proven to belong to this dataset and must never be moved
/// or attached automatically.
pub fn require_safe_work_dir_change_target(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<(), AppError> {
    let canonical_data = canonical_data_dir(data_dir)?;
    let canonical_work = canonical_existing_work_dir(work_dir)?;
    validate_disjoint_data_and_work_roots(
        &canonical_data,
        &canonical_work,
    )
    .map_err(|_| {
        AppError::BadRequest(format!(
            "work_dir {} must be disjoint from data_dir {}",
            canonical_work.display(),
            canonical_data.display()
        ))
    })?;
    require_data_root_not_owned_as_external_work(&canonical_data)?;
    if work_root_has_data_root_identity(&canonical_work)? {
        return Err(AppError::Conflict(format!(
            "work-dir change target {} is already a NomiFun data root",
            canonical_work.display()
        )));
    }
    if external_work_target_has_conversations(
        &canonical_data,
        &canonical_work,
    )? {
        return Err(AppError::BadRequest(format!(
            "work-dir change target {} already contains the reserved conversations directory",
            canonical_work.display()
        )));
    }
    if let Some(owner) = read_work_root_owner(&canonical_work)?
        && !crate::paths::stored_path_matches(&owner.data_root, &canonical_data)
    {
        return Err(AppError::Conflict(format!(
            "work-dir change target {} is owned by another NomiFun data root",
            canonical_work.display()
        )));
    }
    Ok(())
}

fn read_bounded_regular_file(
    path: &Path,
    max_bytes: u64,
) -> std::io::Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("control path is not a regular file: {}", path.display()),
        ));
    }
    if metadata.len() > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("control file exceeds {max_bytes} bytes: {}", path.display()),
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    use std::io::Read;
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("control file grew beyond {max_bytes} bytes: {}", path.display()),
        ));
    }
    Ok(bytes)
}

fn bounded_regular_file_matches(
    path: &Path,
    expected: &[u8],
    max_bytes: u64,
) -> bool {
    matches!(
        read_bounded_regular_file(path, max_bytes),
        Ok(bytes) if bytes == expected
    )
}

/// Decide whether these plan bytes can be one of *this* build's own reset
/// plans, replayed after its control directory was already consumed.
///
/// Returns the reason it cannot be, and `None` when it can. Nothing here is an
/// error: every check is a statement about persisted bytes that another build
/// may have written, and the callers are predicates that must be able to answer
/// "no". Genuine I/O failures (an unreadable data root) still propagate — those
/// are facts about *this* machine, not about the bytes.
fn completed_plan_replay_mismatch(
    plan: &DatasetResetPlan,
    data_dir: &Path,
) -> Result<Option<String>, AppError> {
    let Ok(shape) = released_plan_shape(plan.version) else {
        return Ok(Some(format!(
            "plan version {} is not a released plan shape",
            plan.version
        )));
    };
    if shape.persists_work_dir != plan.persist_work_dir {
        return Ok(Some(
            "completed reset plan replay has an invalid persistence/version pair"
                .into(),
        ));
    }
    if plan.version == LEGACY_PLAN_VERSION
        && plan.automatic_legacy_retirement
        || plan.version != LEGACY_PLAN_VERSION
            && matches!(
                plan.reason,
                DatasetResetReason::NonV3Dataset
            )
            && !plan.automatic_legacy_retirement
        || matches!(
            plan.reason,
            DatasetResetReason::ExplicitFactoryReset
        ) && plan.automatic_legacy_retirement
    {
        return Ok(Some(
            "completed reset plan replay has an invalid automatic-retirement flag"
                .into(),
        ));
    }
    if let Err(error) = validate_uuidv7(&plan.operation_id) {
        return Ok(Some(format!(
            "completed reset plan replay operation ID is invalid: {error}"
        )));
    }
    if let Err(error) = validate_uuidv7(&plan.generation) {
        return Ok(Some(format!(
            "completed reset plan replay generation is invalid: {error}"
        )));
    }
    if plan.requested_at <= 0 {
        return Ok(Some(
            "completed reset plan replay timestamp is invalid".into(),
        ));
    }
    let canonical_data = canonical_data_dir(data_dir)?;
    if !crate::paths::stored_path_matches(&plan.data_dir, &canonical_data) {
        return Ok(Some(
            "completed reset plan replay belongs to a different data directory"
                .into(),
        ));
    }
    let work_path = Path::new(&plan.work_dir);
    if plan.work_dir.is_empty()
        || !work_path.is_absolute()
        || crate::workspace_path_has_edge_whitespace_segment(work_path)
    {
        return Ok(Some(
            "completed reset plan replay contains an unsafe work root".into(),
        ));
    }
    let targets_data_root =
        crate::paths::stored_path_matches(&plan.work_dir, &canonical_data);
    if plan.reason == DatasetResetReason::WorkDirChange && targets_data_root {
        return Ok(Some(
            "completed work-dir change replay cannot target its data root"
                .into(),
        ));
    }
    let expected_retired_dir = format!(
        "{RETIRED_DATASETS_DIR}/id-reference-v3-{}",
        plan.generation
    );
    if plan.retired_dir != expected_retired_dir {
        return Ok(Some(
            "completed reset plan replay has an invalid retired directory"
                .into(),
        ));
    }
    let expected_work_retired_dir = format!(
        "{WORK_RETIRED_DATASETS_DIR}/id-reference-v3-{}",
        plan.generation
    );
    if plan.work_retired_dir != expected_work_retired_dir {
        return Ok(Some(
            "completed reset plan replay has an invalid work-retired directory"
                .into(),
        ));
    }
    if !plan_roots_match_registry(
        plan,
        shape.managed_roots,
        !targets_data_root,
    ) {
        return Ok(Some(format!(
            "completed reset plan replay was written against a different \
             managed-root registry than this build's frozen v{} shape",
            plan.version
        )));
    }
    Ok(None)
}

/// Do these active plan bytes belong to a reset that already completed?
///
/// This is a predicate, and it answers `false` — never an error — when the bytes
/// simply are not one of this build's plans. Erroring instead used to abort
/// `apply_pending_v3_dataset_reset` with `AppError::Internal` at startup, on a
/// user's data directory, for nothing worse than a plan written before the
/// managed-root registry changed.
fn completed_reset_control_matches_plan_bytes(
    data_dir: &Path,
    plan: &DatasetResetPlan,
    active_plan_bytes: &[u8],
) -> Result<bool, AppError> {
    if let Some(reason) = completed_plan_replay_mismatch(plan, data_dir)? {
        tracing::warn!(
            target: "factory_reset",
            operation_id = %plan.operation_id,
            reason = %reason,
            "active reset plan is not a completed-control replay of this build"
        );
        return Ok(false);
    }
    let retired_root = data_dir.join(RETIRED_DATASETS_DIR);
    let generation_root = data_dir.join(&plan.retired_dir);
    let completed_control =
        generation_root.join(COMPLETED_RESET_CONTROL_DIR);
    for (path, description) in [
        (&retired_root, "retired-datasets directory"),
        (&generation_root, "completed reset generation directory"),
        (&completed_control, "completed reset control directory"),
    ] {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                validate_root_metadata(
                    path,
                    &metadata,
                    ManagedRootKind::Directory,
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "inspect {description} {}: {error}",
                    path.display()
                )));
            }
        }
    }
    let completed_plan_path =
        completed_control.join(V3_DATASET_RESET_PLAN_FILE);
    let completed_plan_bytes = read_bounded_regular_file(
        &completed_plan_path,
        MAX_CONTROL_FILE_BYTES,
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "read completed reset control plan {}: {error}",
            completed_plan_path.display()
        ))
    })?;
    if completed_plan_bytes != active_plan_bytes {
        return Err(AppError::Conflict(
            "active reset plan collides with a different completed reset control"
                .into(),
        ));
    }
    let installed_phase =
        completed_control.join(phase_file_name("generation-installed"));
    let phase_bytes =
        read_bounded_regular_file(&installed_phase, 16).map_err(
            |error| {
                AppError::Internal(format!(
                    "read completed reset generation phase {}: {error}",
                    installed_phase.display()
                ))
            },
        )?;
    if phase_bytes != b"v1\n" {
        return Err(AppError::Internal(
            "completed reset generation phase has invalid contents".into(),
        ));
    }
    Ok(true)
}

/// Match a released v1 reset control against the immutable copy that was
/// already rolled back and rejected.
///
/// The archived copy is a permanent negative-authority proof. In particular,
/// an old `generation-installed` phase must never make the same v1 plan
/// destructive again after a newer v3 generation has been installed.
fn ignored_legacy_reset_control_matches_plan_bytes(
    data_dir: &Path,
    plan: &DatasetResetPlan,
    active_plan_bytes: &[u8],
) -> Result<bool, AppError> {
    if plan.version != LEGACY_PLAN_VERSION {
        return Ok(false);
    }
    // Same contract as above: an unrecognizable plan is a `false` answer, not a
    // failure. `plan.operation_id` is used as a path component below, so the
    // UUID check inside the mismatch classifier gates that too.
    if let Some(reason) = completed_plan_replay_mismatch(plan, data_dir)? {
        tracing::warn!(
            target: "factory_reset",
            operation_id = %plan.operation_id,
            reason = %reason,
            "active legacy reset plan is not an archived-control replay of this build"
        );
        return Ok(false);
    }

    let retired_root = data_dir.join(RETIRED_DATASETS_DIR);
    let archive_root = retired_root.join(IGNORED_LEGACY_RESET_PLANS_DIR);
    let archived_control = archive_root.join(&plan.operation_id);
    for (path, description) in [
        (&retired_root, "retired-datasets directory"),
        (&archive_root, "ignored legacy reset plan directory"),
        (&archived_control, "ignored legacy reset control directory"),
    ] {
        match fs::symlink_metadata(path) {
            Ok(metadata) => validate_root_metadata(
                path,
                &metadata,
                ManagedRootKind::Directory,
            )?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "inspect {description} {}: {error}",
                    path.display()
                )));
            }
        }
    }

    let archived_plan_path =
        archived_control.join(V3_DATASET_RESET_PLAN_FILE);
    let archived_plan_bytes = read_bounded_regular_file(
        &archived_plan_path,
        MAX_CONTROL_FILE_BYTES,
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "read ignored legacy reset plan {}: {error}",
            archived_plan_path.display()
        ))
    })?;
    if archived_plan_bytes != active_plan_bytes {
        return Err(AppError::Conflict(
            "active legacy reset plan collides with a different permanently ignored plan"
                .into(),
        ));
    }
    Ok(true)
}

fn active_completed_reset_control_replay(
    data_dir: &Path,
) -> Result<Option<DatasetResetPlan>, AppError> {
    let control_dir = reset_dir(data_dir);
    match fs::symlink_metadata(&control_dir) {
        Ok(metadata) => validate_root_metadata(
            &control_dir,
            &metadata,
            ManagedRootKind::Directory,
        )?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect active reset control replay {}: {error}",
                control_dir.display()
            )));
        }
    }
    let active_plan_path = plan_path(data_dir);
    let active_plan_bytes = match read_bounded_regular_file(
        &active_plan_path,
        MAX_CONTROL_FILE_BYTES,
    ) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read active reset control replay plan {}: {error}",
                active_plan_path.display()
            )));
        }
    };
    let plan: DatasetResetPlan =
        serde_json::from_slice(&active_plan_bytes).map_err(|error| {
            AppError::Internal(format!(
                "malformed active reset control replay plan: {error}"
            ))
        })?;
    if completed_reset_control_matches_plan_bytes(
        data_dir,
        &plan,
        &active_plan_bytes,
    )? {
        Ok(Some(plan))
    } else {
        Ok(None)
    }
}

fn active_ignored_legacy_reset_control_replay(
    data_dir: &Path,
) -> Result<Option<DatasetResetPlan>, AppError> {
    let control_dir = reset_dir(data_dir);
    match fs::symlink_metadata(&control_dir) {
        Ok(metadata) => validate_root_metadata(
            &control_dir,
            &metadata,
            ManagedRootKind::Directory,
        )?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect active ignored legacy reset replay {}: {error}",
                control_dir.display()
            )));
        }
    }
    let active_plan_path = plan_path(data_dir);
    let active_plan_bytes = match read_bounded_regular_file(
        &active_plan_path,
        MAX_CONTROL_FILE_BYTES,
    ) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read active ignored legacy reset replay plan {}: {error}",
                active_plan_path.display()
            )));
        }
    };
    let plan: DatasetResetPlan =
        serde_json::from_slice(&active_plan_bytes).map_err(|error| {
            AppError::Internal(format!(
                "malformed active ignored legacy reset replay plan: {error}"
            ))
        })?;
    if ignored_legacy_reset_control_matches_plan_bytes(
        data_dir,
        &plan,
        &active_plan_bytes,
    )? {
        Ok(Some(plan))
    } else {
        Ok(None)
    }
}

fn archive_active_ignored_legacy_reset_control_replay(
    data_dir: &Path,
) -> Result<bool, AppError> {
    let Some(plan) =
        active_ignored_legacy_reset_control_replay(data_dir)?
    else {
        return Ok(false);
    };
    let archive_root = data_dir
        .join(RETIRED_DATASETS_DIR)
        .join(IGNORED_LEGACY_RESET_PLANS_DIR);
    let destination = archive_root.join(format!(
        "{}-replay-{}",
        plan.operation_id,
        Uuid::now_v7()
    ));
    let source = reset_dir(data_dir);
    rename_with_retry(&source, &destination).map_err(|error| {
        AppError::Internal(format!(
            "archive replayed ignored legacy reset control {} -> {}: {error}",
            source.display(),
            destination.display()
        ))
    })?;
    sync_parent(&source).map_err(|error| {
        AppError::Internal(format!(
            "sync replayed ignored legacy reset control removal: {error}"
        ))
    })?;
    sync_parent(&destination).map_err(|error| {
        AppError::Internal(format!(
            "sync replayed ignored legacy reset control archive: {error}"
        ))
    })?;
    tracing::warn!(
        target: "factory_reset",
        operation_id = %plan.operation_id,
        generation = %plan.generation,
        "archived a replayed permanently ignored legacy reset control without touching the active dataset"
    );
    Ok(true)
}

fn archive_active_completed_reset_control_replay(
    data_dir: &Path,
) -> Result<bool, AppError> {
    let Some(plan) = active_completed_reset_control_replay(data_dir)? else {
        return Ok(false);
    };
    let generation_root = data_dir.join(&plan.retired_dir);
    let destination = generation_root.join(format!(
        "{REPLAYED_COMPLETED_RESET_CONTROL_PREFIX}-{}",
        Uuid::now_v7()
    ));
    let source = reset_dir(data_dir);
    rename_with_retry(&source, &destination).map_err(|error| {
        AppError::Internal(format!(
            "archive replayed completed reset control {} -> {}: {error}",
            source.display(),
            destination.display()
        ))
    })?;
    sync_parent(&source).map_err(|error| {
        AppError::Internal(format!(
            "sync replayed reset control removal: {error}"
        ))
    })?;
    sync_parent(&destination).map_err(|error| {
        AppError::Internal(format!(
            "sync replayed reset control archive: {error}"
        ))
    })?;
    tracing::warn!(
        target: "factory_reset",
        operation_id = %plan.operation_id,
        generation = %plan.generation,
        "archived a replayed completed reset control without touching the active dataset"
    );
    Ok(true)
}

fn warn_unusable_reset_control_shape(plan: &DatasetResetPlan) {
    tracing::warn!(
        target: "factory_reset",
        operation_id = %plan.operation_id,
        generation = %plan.generation,
        version = plan.version,
        "pending reset plan was written against a different managed-root \
         registry than this build's frozen shape; treating it as no pending \
         reset and leaving every managed root untouched"
    );
}

/// Move a pending reset control directory aside when its plan describes a
/// managed-root registry this build does not recognize.
///
/// Such a plan is unexecutable — `read_pending_v3_reset` reports "no pending
/// reset" for it — but leaving it in place is not neutral: the control directory
/// also holds the phase markers, and a stale `generation-installed` marker would
/// be inherited by the next plan armed in the same directory. Renaming the whole
/// directory into `retired-datasets` is atomic, keeps the bytes for support, and
/// destroys nothing: any root the interrupted reset already moved is still in
/// its retired generation directory.
///
/// An *unsupported version* is deliberately not archived. That is the signature
/// of a downgrade — a plan from a newer release — and the reader's hard refusal
/// is the right answer there.
fn archive_unusable_reset_control(
    data_dir: &Path,
) -> Result<bool, AppError> {
    let control_dir = reset_dir(data_dir);
    match fs::symlink_metadata(&control_dir) {
        Ok(metadata) => validate_root_metadata(
            &control_dir,
            &metadata,
            ManagedRootKind::Directory,
        )?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect reset control directory {}: {error}",
                control_dir.display()
            )));
        }
    }
    let plan_bytes = match read_bounded_regular_file(
        &plan_path(data_dir),
        MAX_CONTROL_FILE_BYTES,
    ) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read reset plan while classifying its shape {}: {error}",
                plan_path(data_dir).display()
            )));
        }
    };
    // A plan that does not even parse, and one whose version this build does
    // not know, both stay put: the reader reports those precisely and this
    // helper must not swallow them.
    let Ok(plan) = serde_json::from_slice::<DatasetResetPlan>(&plan_bytes)
    else {
        return Ok(false);
    };
    if released_plan_shape(plan.version).is_err()
        || plan_roots_match_released_registry(&plan, data_dir)?
    {
        return Ok(false);
    }
    let retired_root = data_dir.join(RETIRED_DATASETS_DIR);
    ensure_real_directory(&retired_root, "retired-datasets directory")?;
    let archive_root = retired_root.join(UNUSABLE_RESET_PLANS_DIR);
    ensure_real_directory(
        &archive_root,
        "unrecognized reset plan directory",
    )?;
    // The operation ID comes from an untrusted file, so it is only used as a
    // path component after proving it is a UUID; the appended fresh UUID keeps
    // the destination unique across repeated replays either way.
    let label = if validate_uuidv7(&plan.operation_id).is_ok() {
        plan.operation_id.as_str()
    } else {
        "unidentified"
    };
    let destination =
        archive_root.join(format!("{label}-{}", Uuid::now_v7()));
    rename_with_retry(&control_dir, &destination).map_err(|error| {
        AppError::Internal(format!(
            "archive unrecognized reset control {} -> {}: {error}",
            control_dir.display(),
            destination.display()
        ))
    })?;
    sync_parent(&control_dir).map_err(|error| {
        AppError::Internal(format!(
            "sync unrecognized reset control removal: {error}"
        ))
    })?;
    sync_parent(&destination).map_err(|error| {
        AppError::Internal(format!(
            "sync unrecognized reset control archive: {error}"
        ))
    })?;
    tracing::warn!(
        target: "factory_reset",
        operation_id = %plan.operation_id,
        generation = %plan.generation,
        version = plan.version,
        destination = %destination.display(),
        "quarantined a reset control directory whose plan belongs to a \
         different managed-root registry; no dataset root was moved"
    );
    Ok(true)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictWorkDirConfig {
    work_dir: PathBuf,
}

fn read_strict_work_dir_config(path: &Path) -> Result<PathBuf, AppError> {
    let bytes =
        read_bounded_regular_file(path, MAX_CONTROL_FILE_BYTES).map_err(
            |error| {
                AppError::Internal(format!(
                    "read strict work-dir config {}: {error}",
                    path.display()
                ))
            },
        )?;
    let config: StrictWorkDirConfig =
        serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Internal(format!(
                "invalid strict work-dir config {}: {error}",
                path.display()
            ))
        })?;
    let work_dir = config.work_dir;
    if work_dir.as_os_str().is_empty()
        || !work_dir.is_absolute()
        || crate::workspace_path_has_edge_whitespace_segment(&work_dir)
    {
        return Err(AppError::Internal(format!(
            "strict work-dir config contains an unsafe path: {}",
            path.display()
        )));
    }
    Ok(work_dir)
}

/// Return the immutable work root recorded by a pending v1/v2 reset plan.
///
/// A v1 plan may already have moved `dir-config.json`, so this plan must be
/// authoritative during crash recovery. This function never writes the
/// control file back while the plan is pending: doing so would create an
/// ambiguous source-present/destination-present state for the v1 move.
pub fn pending_v3_reset_work_dir(
    data_dir: &Path,
) -> Result<Option<PathBuf>, AppError> {
    let control_dir = reset_dir(data_dir);
    match fs::symlink_metadata(&control_dir) {
        Ok(metadata) => validate_root_metadata(
            &control_dir,
            &metadata,
            ManagedRootKind::Directory,
        )?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect v3 dataset reset control directory during work-dir recovery: {error}"
            )));
        }
    }
    let pending_path = plan_path(data_dir);
    let pending_plan = match fs::symlink_metadata(&pending_path) {
        Ok(metadata) => {
            validate_root_metadata(
                &pending_path,
                &metadata,
                ManagedRootKind::File,
            )?;
            if metadata.len() > MAX_CONTROL_FILE_BYTES {
                return Err(AppError::Internal(
                    "v3 dataset reset plan is too large during work-dir recovery"
                        .into(),
                ));
            }
            let bytes =
                read_bounded_regular_file(&pending_path, MAX_CONTROL_FILE_BYTES)
                    .map_err(|error| {
                AppError::Internal(format!(
                    "read v3 dataset reset plan during work-dir recovery: {error}"
                ))
            })?;
            let plan =
                serde_json::from_slice::<DatasetResetPlan>(&bytes).map_err(
                    |error| {
                        AppError::Internal(format!(
                            "malformed v3 dataset reset plan during work-dir recovery: {error}"
                        ))
                    },
                )?;
            Some((plan, bytes))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect v3 dataset reset plan during work-dir recovery: {error}"
            )));
        }
    };

    if let Some((plan, bytes)) = pending_plan {
        if ignored_legacy_reset_control_matches_plan_bytes(
            data_dir, &plan, &bytes,
        )? {
            // A released v1 plan that was already rolled back and archived is
            // permanently non-authoritative, even if stale phase files replay.
            return Ok(None);
        }
        if completed_reset_control_matches_plan_bytes(
            data_dir, &plan, &bytes,
        )? {
            // A completed plan whose old directory entry reappeared must not
            // redirect resolution to a retired or now-missing work root.
            return Ok(None);
        }
        if !plan_roots_match_released_registry(&plan, data_dir)? {
            // Not authoritative and not executable: fall back to the ordinary
            // work-root resolution instead of failing the boot. The control
            // directory is quarantined by the next mutating entry point.
            warn_unusable_reset_control_shape(&plan);
            return Ok(None);
        }
        let work_dir = PathBuf::from(&plan.work_dir);
        let canonical_work = canonical_existing_work_dir(&work_dir)?;
        validate_plan(&plan, data_dir, &canonical_work)?;
        return Ok(Some(canonical_work));
    }
    Ok(None)
}

/// Repair the historical v1-reset bug that retired `dir-config.json` while
/// leaving a finalized v3 receipt bound to that external work root.
///
/// This runs only when the active config is absent, and only accepts a config
/// from the receipt's exact generation after validating the receipt, storage
/// generation, database, directory types and canonical work-root binding.
/// It publishes a minimal new control file; no retired business data is read,
/// copied or reactivated.
pub fn repair_finalized_legacy_v1_work_dir(
    data_dir: &Path,
) -> Result<Option<PathBuf>, AppError> {
    match fs::symlink_metadata(
        data_dir.join(crate::dir_config::DIR_CONFIG_FILE),
    ) {
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect active work-dir config during v1 recovery: {error}"
            )));
        }
    }

    let receipt_path = receipt_path(data_dir);
    let receipt_metadata = match fs::symlink_metadata(&receipt_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect v3 receipt during work-dir recovery: {error}"
            )));
        }
    };
    validate_root_metadata(
        &receipt_path,
        &receipt_metadata,
        ManagedRootKind::File,
    )?;
    if receipt_metadata.len() > MAX_CONTROL_FILE_BYTES {
        return Err(AppError::Internal(
            "v3 receipt is too large during work-dir recovery".into(),
        ));
    }
    let receipt_bytes =
        read_bounded_regular_file(&receipt_path, MAX_CONTROL_FILE_BYTES)
            .map_err(|error| {
                AppError::Internal(format!(
                    "read v3 receipt during work-dir recovery: {error}"
                ))
            })?;
    let receipt: DatasetReceipt =
        serde_json::from_slice(&receipt_bytes).map_err(|error| {
            AppError::Internal(format!(
                "invalid v3 receipt during work-dir recovery: {error}"
            ))
        })?;
    if receipt.contract_version != V3_DATASET_CONTRACT_VERSION
        || receipt.installed_at <= 0
    {
        return Ok(None);
    }
    validate_uuidv7(&receipt.generation).map_err(|error| {
        AppError::Internal(format!(
            "invalid receipt generation during work-dir recovery: {error}"
        ))
    })?;
    if !bounded_regular_file_matches(
        &data_dir.join(STORAGE_GENERATION_FILE),
        receipt.generation.as_bytes(),
        128,
    ) {
        return Ok(None);
    }

    let retired_root = data_dir.join(RETIRED_DATASETS_DIR);
    let generation_root = retired_root.join(format!(
        "id-reference-v3-{}",
        receipt.generation
    ));
    for (path, description) in [
        (&retired_root, "retired-datasets root"),
        (&generation_root, "retired generation root"),
    ] {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "inspect {description} {}: {error}",
                    path.display()
                )));
            }
        };
        validate_root_metadata(path, &metadata, ManagedRootKind::Directory)?;
    }

    let retired_path = retired_dir_config_path(data_dir, &receipt.generation);
    let metadata = match fs::symlink_metadata(&retired_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect retired work-dir config {}: {error}",
                retired_path.display()
            )));
        }
    };
    validate_root_metadata(&retired_path, &metadata, ManagedRootKind::File)?;
    if metadata.len() > MAX_CONTROL_FILE_BYTES {
        return Err(AppError::Internal(
            "retired work-dir config is too large".into(),
        ));
    }
    let work_dir = read_strict_work_dir_config(&retired_path)?;
    let canonical_work = canonical_existing_work_dir(&work_dir)?;
    if !crate::paths::stored_path_matches(&receipt.work_root, &canonical_work) {
        return Err(AppError::Internal(
            "retired work-dir config does not match its v3 generation binding".into(),
        ));
    }
    if inspect_v3_dataset_receipt(data_dir, &canonical_work)?
        != DatasetReceiptStatus::Current
    {
        return Err(AppError::Internal(
            "retired work-dir config does not resolve the finalized v3 receipt".into(),
        ));
    }
    crate::dir_config::install_work_dir_if_absent(data_dir, &canonical_work)?;
    tracing::info!(
        target: "factory_reset",
        generation = %receipt.generation,
        work_dir = %canonical_work.display(),
        "repaired work-dir control config retired by a legacy v1 reset"
    );
    Ok(Some(canonical_work))
}

fn validate_relative_path(value: &str) -> Result<(), AppError> {
    let path = Path::new(value);
    if value.is_empty() || path.is_absolute() {
        return Err(AppError::Internal(format!(
            "dataset reset path is not relative: {value:?}"
        )));
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)) {
            return Err(AppError::Internal(format!(
                "dataset reset path escapes its root: {value:?}"
            )));
        }
    }
    Ok(())
}

fn relative_path_is_under(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).is_ok()
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn ensure_real_directory(path: &Path, description: &str) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() => {
            Err(AppError::Internal(format!(
                "{description} must be a real directory: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|create_error| {
                AppError::Internal(format!(
                    "create {description} {}: {create_error}",
                    path.display()
                ))
            })?;
            let metadata = fs::symlink_metadata(path).map_err(|inspect_error| {
                AppError::Internal(format!(
                    "inspect created {description} {}: {inspect_error}",
                    path.display()
                ))
            })?;
            if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
                return Err(AppError::Internal(format!(
                    "created {description} is not a real directory: {}",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(error) => Err(AppError::Internal(format!(
            "inspect {description} {}: {error}",
            path.display()
        ))),
    }
}

fn ensure_safe_destination_parent(
    destination: &Path,
    retired_root: &Path,
) -> Result<(), AppError> {
    let parent = destination.parent().ok_or_else(|| {
        AppError::Internal(format!(
            "reset destination has no parent: {}",
            destination.display()
        ))
    })?;
    if !relative_path_is_under(parent, retired_root) {
        return Err(AppError::Internal(format!(
            "reset destination parent escapes retired root: {}",
            parent.display()
        )));
    }

    // Check every existing component.  `create_dir_all` follows a symlink, so
    // checking only the leaf would permit a malicious/stale junction in the
    // quarantine tree to redirect a move outside the data directory.
    let mut current = retired_root.to_path_buf();
    let relative = parent.strip_prefix(retired_root).map_err(|_| {
        AppError::Internal("reset destination is outside retired root".into())
    })?;
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(AppError::Internal(format!(
                "reset destination parent contains unsafe component: {}",
                parent.display()
            )));
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() => {
                return Err(AppError::Internal(format!(
                    "reset destination parent is not a real directory: {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|create_error| {
                    AppError::Internal(format!(
                        "create reset destination parent {}: {create_error}",
                        current.display()
                    ))
                })?;
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "inspect reset destination parent {}: {error}",
                    current.display()
                )));
            }
        }
    }
    Ok(())
}

fn validate_root_metadata(
    path: &Path,
    metadata: &fs::Metadata,
    kind: ManagedRootKind,
) -> Result<(), AppError> {
    if metadata_is_link_or_reparse(metadata)
        || match kind {
            ManagedRootKind::File => !metadata.is_file(),
            ManagedRootKind::Directory => !metadata.is_dir(),
        }
    {
        return Err(AppError::Internal(format!(
            "managed reset root has the wrong type or symlink/reparse indirection: {}",
            path.display()
        )));
    }
    Ok(())
}

fn inspect_planned_root(path: &Path, kind: ManagedRootKind) -> Result<bool, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_root_metadata(path, &metadata, kind)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(AppError::Internal(format!(
            "inspect managed reset root {}: {error}",
            path.display()
        ))),
    }
}

fn validate_plan(
    plan: &DatasetResetPlan,
    data_dir: &Path,
    work_dir: &Path,
) -> Result<(), AppError> {
    let shape = released_plan_shape(plan.version)?;
    let managed_roots = shape.managed_roots;
    if shape.persists_work_dir != plan.persist_work_dir {
        return Err(AppError::Internal(
            "v3 dataset reset plan work-root persistence flag does not match its version"
                .into(),
        ));
    }
    if plan.version == LEGACY_PLAN_VERSION
        && plan.automatic_legacy_retirement
        || plan.version != LEGACY_PLAN_VERSION
            && matches!(
                plan.reason,
                DatasetResetReason::NonV3Dataset
            )
            && !plan.automatic_legacy_retirement
        || matches!(
            plan.reason,
            DatasetResetReason::ExplicitFactoryReset
        ) && plan.automatic_legacy_retirement
    {
        return Err(AppError::Internal(
            "v3 dataset reset plan automatic-retirement flag does not match its authority"
                .into(),
        ));
    }
    validate_uuidv7(&plan.operation_id).map_err(|error| {
        AppError::Internal(format!(
            "v3 dataset reset operation ID is invalid: {} ({error})",
            plan.operation_id
        ))
    })?;
    validate_uuidv7(&plan.generation).map_err(|error| {
        AppError::Internal(format!(
            "v3 dataset reset generation is invalid: {} ({error})",
            plan.generation
        ))
    })?;
    if !crate::paths::stored_path_matches(
        &plan.data_dir,
        &canonical_data_dir(data_dir)?,
    ) {
        return Err(AppError::Internal(
            "v3 dataset reset plan belongs to a different data directory".into(),
        ));
    }
    if !crate::paths::stored_path_matches(
        &plan.work_dir,
        &canonical_existing_work_dir(work_dir)?,
    ) {
        return Err(AppError::Internal(
            "v3 dataset reset plan belongs to a different managed work directory".into(),
        ));
    }
    if plan.requested_at <= 0 {
        return Err(AppError::Internal(
            "v3 dataset reset plan requested_at must be positive".into(),
        ));
    }
    validate_relative_path(&plan.retired_dir)?;
    let expected_retired_dir =
        format!("{RETIRED_DATASETS_DIR}/id-reference-v3-{}", plan.generation);
    if plan.retired_dir != expected_retired_dir {
        return Err(AppError::Internal(
            "v3 dataset reset retired directory does not match its generation".into(),
        ));
    }
    validate_relative_path(&plan.work_retired_dir)?;
    let expected_work_retired_dir = format!(
        "{WORK_RETIRED_DATASETS_DIR}/id-reference-v3-{}",
        plan.generation
    );
    if plan.work_retired_dir != expected_work_retired_dir {
        return Err(AppError::Internal(
            "v3 dataset reset work-retired directory does not match its generation".into(),
        ));
    }
    if plan.roots.is_empty() {
        return Err(AppError::Internal(
            "v3 dataset reset plan contains no managed roots".into(),
        ));
    }
    for root in &plan.roots {
        validate_relative_path(&root.relative_path)?;
        validate_relative_path(&root.retired_relative_path)?;
        if root.base == ManagedRootBase::DataDir
            && (root.relative_path == V3_DATASET_RESET_DIR
                || root.relative_path == RETIRED_DATASETS_DIR)
        {
            return Err(AppError::Internal(
                "v3 dataset reset plan attempts to move its own control directory".into(),
            ));
        }
        let expected_retired_root = match root.base {
            ManagedRootBase::DataDir => &plan.retired_dir,
            ManagedRootBase::WorkDir => &plan.work_retired_dir,
        };
        if !root
            .retired_relative_path
            .starts_with(&format!("{expected_retired_root}/"))
        {
            return Err(AppError::Internal(
                "v3 dataset reset root destination is outside its retired directory".into(),
            ));
        }
    }

    let canonical = canonical_data_dir(data_dir)?;
    let canonical_work = canonical_existing_work_dir(work_dir)?;
    if plan.reason == DatasetResetReason::WorkDirChange {
        validate_disjoint_data_and_work_roots(&canonical, &canonical_work)?;
    } else {
        validate_safe_reset_data_and_work_roots(
            &canonical,
            &canonical_work,
            managed_roots,
        )?;
    }
    if !plan_roots_match_registry(
        plan,
        managed_roots,
        canonical_work != canonical,
    ) {
        // Reaching here means the caller decided to act on a plan whose shape
        // it had not classified. `read_pending_v3_reset` and
        // `pending_v3_reset_work_dir` filter this case out first precisely so
        // that a plan from another registry shape cannot abort startup.
        return Err(AppError::Internal(
            "v3 dataset reset plan managed-root registry does not match this build".into(),
        ));
    }
    if plan.reason == DatasetResetReason::WorkDirChange
        && plan.roots.iter().any(|root| {
            root.base == ManagedRootBase::WorkDir
                && root.relative_path == MANAGED_WORKSPACES_DIR
                && root.initially_present
        })
    {
        return Err(AppError::Internal(
            "work-dir change plan must never claim a pre-existing target conversations directory"
                .into(),
        ));
    }
    let retired_root = canonical.join(&plan.retired_dir);
    if let Ok(metadata) = fs::symlink_metadata(&canonical.join(RETIRED_DATASETS_DIR)) {
        if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
            return Err(AppError::Internal(
                "retired-datasets must be a real directory".into(),
            ));
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(&retired_root)
        && (metadata_is_link_or_reparse(&metadata) || !metadata.is_dir())
    {
        return Err(AppError::Internal(
            "retired dataset generation directory must be a real directory".into(),
        ));
    }
    let work_retired_root = canonical_work.join(&plan.work_retired_dir);
    if let Ok(metadata) = fs::symlink_metadata(&work_retired_root)
        && (metadata_is_link_or_reparse(&metadata) || !metadata.is_dir())
    {
        return Err(AppError::Internal(
            "managed-workspace retired dataset directory must be a real directory".into(),
        ));
    }
    Ok(())
}

fn sync_parent(path: &Path) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    OpenOptions::new().read(true).open(path)?.sync_all()
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    // Windows does not support opening a directory with ordinary
    // `CreateFile` flags through `std::fs::OpenOptions`.  Directory metadata
    // is nevertheless protected by the atomic rename itself; use a no-op for
    // the directory fsync step while still syncing every written file.
    let _ = path;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    let _ = path;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("state"),
        Uuid::now_v7()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(&tmp, path)?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let target: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn write_atomic_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state"),
        Uuid::now_v7()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        publish_new_file(&tmp, path)?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(target_os = "macos")]
fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "source path contains a NUL byte",
            )
        },
    )?;
    let target = CString::new(target.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "target path contains a NUL byte",
            )
        },
    )?;
    if unsafe {
        libc::renamex_np(
            source.as_ptr(),
            target.as_ptr(),
            libc::RENAME_EXCL,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "source path contains a NUL byte",
            )
        },
    )?;
    let target = CString::new(target.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "target path contains a NUL byte",
            )
        },
    )?;
    if unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos"))
))]
fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::hard_link(source, target)?;
    fs::remove_file(source)
}

#[cfg(not(any(unix, windows)))]
fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::hard_link(source, target)?;
    fs::remove_file(source)
}

#[cfg(windows)]
fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let target: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } != 0
    {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    // MoveFileExW may report either ERROR_FILE_EXISTS (80) or
    // ERROR_ALREADY_EXISTS (183), depending on the filesystem. Normalize both
    // so callers retain the create_new-style conflict contract.
    if matches!(error.raw_os_error(), Some(80 | 183)) {
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            error,
        ))
    } else {
        Err(error)
    }
}

fn write_phase(data_dir: &Path, phase: &str) -> Result<(), AppError> {
    let path = phase_path(data_dir, phase);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            validate_root_metadata(
                &path,
                &metadata,
                ManagedRootKind::File,
            )?;
            let bytes = read_bounded_regular_file(&path, 16).map_err(
                |error| {
                    AppError::Internal(format!(
                        "read reset phase {phase}: {error}"
                    ))
                },
            )?;
            if bytes != b"v1\n" {
                return Err(AppError::Internal(format!(
                    "reset phase {phase} has invalid contents"
                )));
            }
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect reset phase {phase}: {error}"
            )));
        }
    }
    write_atomic(&path, b"v1\n")
        .map_err(|error| AppError::Internal(format!("write reset phase {phase}: {error}")))
}

fn has_phase(data_dir: &Path, phase: &str) -> bool {
    matches!(
        read_bounded_regular_file(&phase_path(data_dir, phase), 16),
        Ok(bytes) if bytes == b"v1\n"
    )
}

fn commit_reset_finalization(
    data_dir: &Path,
    plan: &DatasetResetPlan,
) -> Result<(), AppError> {
    let source = reset_dir(data_dir);
    let metadata = fs::symlink_metadata(&source).map_err(|error| {
        AppError::Internal(format!(
            "inspect completed v3 dataset reset plan {}: {error}",
            source.display()
        ))
    })?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Err(AppError::Internal(format!(
            "refusing to finalize non-directory v3 dataset reset control path {}",
            source.display()
        )));
    }
    let retired_root = data_dir.join(&plan.retired_dir);
    ensure_real_directory(
        &retired_root,
        "retired dataset generation directory",
    )?;
    let destination = retired_root.join(COMPLETED_RESET_CONTROL_DIR);
    match fs::symlink_metadata(&destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(AppError::Internal(format!(
                "completed reset control destination already exists: {}",
                destination.display()
            )));
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect completed reset control destination {}: {error}",
                destination.display()
            )));
        }
    }
    rename_with_retry(&source, &destination).map_err(|error| {
        AppError::Internal(format!(
            "atomically finalize reset control {} -> {}: {error}",
            source.display(),
            destination.display()
        ))
    })?;
    sync_parent(&source).map_err(|error| {
        AppError::Internal(format!(
            "sync active reset control removal: {error}"
        ))
    })?;
    sync_parent(&destination).map_err(|error| {
        AppError::Internal(format!(
            "sync completed reset control publication: {error}"
        ))
    })
}

/// Arm an immutable v3 reset plan.  If a plan already exists, it is validated
/// and returned; no new generation or destination is minted on retry.
pub fn arm_v3_dataset_reset(
    data_dir: &Path,
    work_dir: &Path,
    reason: DatasetResetReason,
) -> Result<DatasetResetPlan, AppError> {
    require_data_root_not_owned_as_external_work(data_dir)?;
    archive_active_ignored_legacy_reset_control_replay(data_dir)?;
    archive_active_completed_reset_control_replay(data_dir)?;
    // A plan this build cannot execute must not leave its phase markers behind
    // for the plan armed below to inherit.
    archive_unusable_reset_control(data_dir)?;
    if let Some(existing) = read_pending_v3_reset(data_dir, work_dir)? {
        validate_active_reset_request_against_plan(data_dir, &existing)?;
        if existing.reason == DatasetResetReason::WorkDirChange
            && !has_phase(data_dir, "generation-installed")
            && inspect_planned_root(
                &Path::new(&existing.work_dir)
                    .join(MANAGED_WORKSPACES_DIR),
                ManagedRootKind::Directory,
            )?
        {
            return Err(AppError::Internal(
                "work-dir change target gained a conversations directory before generation installation; preserving both datasets"
                    .into(),
            ));
        }
        ensure_v3_work_root_owner_for_reset(
            data_dir,
            Path::new(&existing.work_dir),
            &existing.generation,
        )?;
        // The atomic, validated plan is the destructive authority. If the
        // process crashed between publishing it and publishing the redundant
        // phase marker, safely complete that commit on retry.
        write_phase(data_dir, "armed")?;
        clear_reset_request_for_plan(data_dir, &existing)?;
        return Ok(existing);
    }

    let canonical = canonical_data_dir(data_dir)?;
    let canonical_work = canonical_work_dir(work_dir)?;
    let canonical_work_string = canonical_work.display().to_string();
    let active_request = read_v3_dataset_reset_request(data_dir)?;
    let automatic_legacy_retirement = active_request.is_none()
        && !matches!(
            reason,
            DatasetResetReason::ExplicitFactoryReset
        );
    let managed_roots = current_writer_managed_roots();
    if reason == DatasetResetReason::WorkDirChange {
        validate_disjoint_data_and_work_roots(&canonical, &canonical_work)?;
    } else {
        validate_safe_reset_data_and_work_roots(
            &canonical,
            &canonical_work,
            &managed_roots,
        )?;
    }
    let generation = Uuid::now_v7().to_string();
    let (operation_id, requested_at) =
        if let Some(request) = active_request.as_ref() {
            if request.version != RESET_REQUEST_VERSION
                || request.origin != request_origin_for_reason(reason)
                || !request.work_dir.as_deref().is_some_and(|requested| {
                    crate::paths::stored_path_matches(requested, &canonical_work)
                })
            {
                return Err(AppError::Conflict(
                    "active reset request does not authorize the requested reset plan"
                        .into(),
                ));
            }
            (request.operation_id.clone(), request.requested_at)
        } else {
            (Uuid::now_v7().to_string(), now_ms())
        };
    let retired_dir = format!("{RETIRED_DATASETS_DIR}/id-reference-v3-{generation}");
    let work_retired_dir =
        format!("{WORK_RETIRED_DATASETS_DIR}/id-reference-v3-{generation}");
    let mut roots = managed_roots
        .into_iter()
        .map(|(relative_path, kind)| {
            let source = canonical.join(relative_path);
            let initially_present = inspect_planned_root(&source, kind)?;
            Ok(ManagedRootPlan {
                base: ManagedRootBase::DataDir,
                relative_path: relative_path.to_owned(),
                retired_relative_path: format!("{retired_dir}/{relative_path}"),
                kind,
                initially_present,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    if canonical_work != canonical {
        let source = canonical_work.join(MANAGED_WORKSPACES_DIR);
        let initially_present =
            inspect_planned_root(&source, ManagedRootKind::Directory)?;
        if reason == DatasetResetReason::WorkDirChange
            && initially_present
        {
            return Err(AppError::Internal(format!(
                "work-dir change target already contains {}; refusing to move unowned files",
                source.display()
            )));
        }
        roots.push(ManagedRootPlan {
            base: ManagedRootBase::WorkDir,
            relative_path: MANAGED_WORKSPACES_DIR.to_owned(),
            retired_relative_path: format!("{work_retired_dir}/{MANAGED_WORKSPACES_DIR}"),
            kind: ManagedRootKind::Directory,
            initially_present,
        });
    }
    let plan = DatasetResetPlan {
        version: PLAN_VERSION,
        operation_id,
        reason,
        data_dir: canonical.display().to_string(),
        work_dir: canonical_work_string,
        persist_work_dir: true,
        automatic_legacy_retirement,
        generation,
        retired_dir,
        work_retired_dir,
        requested_at,
        roots,
    };
    validate_plan(&plan, data_dir, work_dir)?;
    // Reserve the work root before publishing destructive authority. If this
    // process crashes here, the marker contains no business data and a retry
    // by the same data root may safely rotate it to the retried generation.
    // Another data root can no longer claim this target after the lock drops.
    ensure_v3_work_root_owner_for_reset(
        data_dir,
        Path::new(&plan.work_dir),
        &plan.generation,
    )?;
    let bytes = serde_json::to_vec_pretty(&plan)
        .map_err(|error| AppError::Internal(format!("serialize v3 reset plan: {error}")))?;
    ensure_real_directory(&reset_dir(data_dir), "v3 reset plan directory")?;
    write_atomic(&plan_path(data_dir), &bytes)
        .map_err(|error| AppError::Internal(format!("write v3 reset plan: {error}")))?;
    write_phase(data_dir, "armed")?;

    // The request has now been durably superseded by the immutable plan. A
    // crash after this point is recovered from the plan.
    clear_reset_request_for_plan(data_dir, &plan)?;
    Ok(plan)
}

pub fn read_pending_v3_reset(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<Option<DatasetResetPlan>, AppError> {
    let directory = reset_dir(data_dir);
    match fs::symlink_metadata(&directory) {
        Ok(metadata)
            if metadata_is_link_or_reparse(&metadata)
                || !metadata.is_dir() =>
        {
            return Err(AppError::Internal(format!(
                "v3 dataset reset control path is not a real directory: {}",
                directory.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect v3 dataset reset control directory {}: {error}",
                directory.display()
            )));
        }
    }
    let path = plan_path(data_dir);
    match read_bounded_regular_file(&path, MAX_CONTROL_FILE_BYTES) {
        Ok(bytes) => {
            let plan: DatasetResetPlan = serde_json::from_slice(&bytes).map_err(|error| {
                AppError::Internal(format!(
                    "malformed v3 dataset reset plan {}: {error}",
                    path.display()
                ))
            })?;
            if ignored_legacy_reset_control_matches_plan_bytes(
                data_dir, &plan, &bytes,
            )? {
                return Ok(None);
            }
            if completed_reset_control_matches_plan_bytes(
                data_dir, &plan, &bytes,
            )? {
                return Ok(None);
            }
            if !plan_roots_match_released_registry(&plan, data_dir)? {
                // These bytes describe a quarantine this build cannot prove it
                // authored, so there is nothing here to execute. Reporting "no
                // pending reset" leaves every managed root exactly where it is;
                // `archive_unusable_reset_control` then moves the control
                // directory aside so its stale phase markers cannot be
                // inherited by the next plan.
                warn_unusable_reset_control_shape(&plan);
                return Ok(None);
            }
            validate_plan(&plan, data_dir, work_dir)?;
            Ok(Some(plan))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::Internal(format!(
            "read v3 dataset reset plan {}: {error}",
            path.display()
        ))),
    }
}

fn install_generation(data_dir: &Path, generation: &str) -> Result<(), AppError> {
    let path = data_dir.join(STORAGE_GENERATION_FILE);
    match read_bounded_regular_file(&path, 128) {
        Ok(existing) if existing == generation.as_bytes() => return Ok(()),
        Ok(_) => {
            return Err(AppError::Internal(format!(
                "storage-generation has an unexpected value at {}",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read storage-generation {}: {error}",
                path.display()
            )));
        }
    }
    write_atomic(path.as_path(), generation.as_bytes()).map_err(|error| {
        AppError::Internal(format!("install new storage generation: {error}"))
    })
}

fn rollback_uncommitted_legacy_reset_plan(
    data_dir: &Path,
    work_dir: &Path,
    plan: &DatasetResetPlan,
) -> Result<(), AppError> {
    write_phase(data_dir, "rollback-started")?;
    let canonical_work = canonical_existing_work_dir(work_dir)?;
    for root in plan.roots.iter().rev() {
        let base = match root.base {
            ManagedRootBase::DataDir => data_dir,
            ManagedRootBase::WorkDir => canonical_work.as_path(),
        };
        let source = base.join(&root.relative_path);
        let destination = base.join(&root.retired_relative_path);
        let source_metadata = match fs::symlink_metadata(&source) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "inspect legacy reset source {}: {error}",
                    source.display()
                )));
            }
        };
        let destination_metadata =
            match fs::symlink_metadata(&destination) {
                Ok(metadata) => Some(metadata),
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    None
                }
                Err(error) => {
                    return Err(AppError::Internal(format!(
                        "inspect legacy reset destination {}: {error}",
                        destination.display()
                    )));
                }
            };
        if let Some(metadata) = &source_metadata {
            validate_root_metadata(&source, metadata, root.kind)?;
        }
        if let Some(metadata) = &destination_metadata {
            validate_root_metadata(&destination, metadata, root.kind)?;
        }
        let storage_generation_root = root.base == ManagedRootBase::DataDir
            && root.relative_path == STORAGE_GENERATION_FILE;

        if storage_generation_root {
            if let Some(_) = &source_metadata
                && bounded_regular_file_matches(
                    &source,
                    plan.generation.as_bytes(),
                    128,
                )
            {
                fs::remove_file(&source).map_err(|error| {
                    AppError::Internal(format!(
                        "remove uncommitted v1 storage generation {}: {error}",
                        source.display()
                    ))
                })?;
                sync_parent(&source).map_err(|error| {
                    AppError::Internal(format!(
                        "sync uncommitted v1 storage generation removal: {error}"
                    ))
                })?;
            }
        }

        let source_exists = fs::symlink_metadata(&source);
        let destination_exists = fs::symlink_metadata(&destination);
        match (source_exists, destination_exists) {
            (Ok(source_metadata), Err(error))
                if root.initially_present
                    && error.kind() == std::io::ErrorKind::NotFound =>
            {
                validate_root_metadata(&source, &source_metadata, root.kind)?;
            }
            (Err(error), Ok(destination_metadata))
                if root.initially_present
                    && error.kind() == std::io::ErrorKind::NotFound =>
            {
                validate_root_metadata(
                    &destination,
                    &destination_metadata,
                    root.kind,
                )?;
                rename_with_retry(&destination, &source).map_err(
                    |rename_error| {
                        AppError::Internal(format!(
                            "roll back legacy reset root {} -> {}: {rename_error}",
                            destination.display(),
                            source.display()
                        ))
                    },
                )?;
                sync_parent(&destination).map_err(|error| {
                    AppError::Internal(format!(
                        "sync retired parent after legacy rollback: {error}"
                    ))
                })?;
                sync_parent(&source).map_err(|error| {
                    AppError::Internal(format!(
                        "sync active parent after legacy rollback: {error}"
                    ))
                })?;
            }
            (Err(source_error), Err(destination_error))
                if !root.initially_present
                    && source_error.kind() == std::io::ErrorKind::NotFound
                    && destination_error.kind()
                        == std::io::ErrorKind::NotFound => {}
            (Ok(_), Ok(_)) => {
                return Err(AppError::Internal(format!(
                    "cannot safely roll back legacy reset root because both locations exist: {}",
                    root.relative_path
                )));
            }
            (Err(source_error), Err(destination_error))
                if root.initially_present
                    && source_error.kind() == std::io::ErrorKind::NotFound
                    && destination_error.kind()
                        == std::io::ErrorKind::NotFound =>
            {
                return Err(AppError::Internal(format!(
                    "cannot safely roll back legacy reset root missing from both locations: {}",
                    root.relative_path
                )));
            }
            (Ok(_), Err(destination_error))
                if !root.initially_present
                    && destination_error.kind()
                        == std::io::ErrorKind::NotFound =>
            {
                return Err(AppError::Internal(format!(
                    "cannot safely roll back unexpected active legacy reset root: {}",
                    root.relative_path
                )));
            }
            (Err(source_error), Ok(_))
                if !root.initially_present
                    && source_error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Err(AppError::Internal(format!(
                    "cannot safely roll back unexpected retired legacy reset root: {}",
                    root.relative_path
                )));
            }
            (Err(error), _) => {
                return Err(AppError::Internal(format!(
                    "inspect legacy rollback source {}: {error}",
                    source.display()
                )));
            }
            (_, Err(error)) => {
                return Err(AppError::Internal(format!(
                    "inspect legacy rollback destination {}: {error}",
                    destination.display()
                )));
            }
        }
    }

    // Re-prove the fully restored state before archiving the control plan.
    for root in &plan.roots {
        let base = match root.base {
            ManagedRootBase::DataDir => data_dir,
            ManagedRootBase::WorkDir => canonical_work.as_path(),
        };
        let source = base.join(&root.relative_path);
        let destination = base.join(&root.retired_relative_path);
        match (
            fs::symlink_metadata(&source),
            fs::symlink_metadata(&destination),
        ) {
            (Ok(source_metadata), Err(error))
                if root.initially_present
                    && error.kind() == std::io::ErrorKind::NotFound =>
            {
                validate_root_metadata(&source, &source_metadata, root.kind)?;
            }
            (Err(source_error), Err(destination_error))
                if !root.initially_present
                    && source_error.kind() == std::io::ErrorKind::NotFound
                    && destination_error.kind()
                        == std::io::ErrorKind::NotFound => {}
            _ => {
                return Err(AppError::Internal(format!(
                    "legacy reset rollback did not restore root {}",
                    root.relative_path
                )));
            }
        }
    }
    Ok(())
}

fn archive_unstarted_legacy_reset_plan(
    data_dir: &Path,
    plan: &DatasetResetPlan,
) -> Result<(), AppError> {
    if let Some(request) = read_v3_dataset_reset_request(data_dir)? {
        if request.version != LEGACY_RESET_REQUEST_VERSION {
            return Err(AppError::Internal(
                "unstarted legacy plan conflicts with a current reset request"
                    .into(),
            ));
        }
        archive_ignored_legacy_reset_request(data_dir, &request)?;
    }
    let retired_root = data_dir.join(RETIRED_DATASETS_DIR);
    ensure_real_directory(&retired_root, "retired-datasets directory")?;
    sync_parent(&retired_root).map_err(|error| {
        AppError::Internal(format!(
            "sync ignored legacy plan parent publication: {error}"
        ))
    })?;
    let archive_root = retired_root.join(IGNORED_LEGACY_RESET_PLANS_DIR);
    ensure_real_directory(
        &archive_root,
        "ignored legacy reset plan directory",
    )?;
    sync_parent(&archive_root).map_err(|error| {
        AppError::Internal(format!(
            "sync ignored legacy plan archive publication: {error}"
        ))
    })?;
    let source = reset_dir(data_dir);
    let mut destination = archive_root.join(&plan.operation_id);
    match fs::symlink_metadata(&destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(metadata) => {
            validate_root_metadata(
                &destination,
                &metadata,
                ManagedRootKind::Directory,
            )?;
            let active_plan = read_bounded_regular_file(
                &source.join(V3_DATASET_RESET_PLAN_FILE),
                MAX_CONTROL_FILE_BYTES,
            )
            .map_err(|error| {
                AppError::Internal(format!(
                    "read replayed active legacy reset plan: {error}"
                ))
            })?;
            let archived_plan = read_bounded_regular_file(
                &destination.join(V3_DATASET_RESET_PLAN_FILE),
                MAX_CONTROL_FILE_BYTES,
            )
            .map_err(|error| {
                AppError::Internal(format!(
                    "read previously archived legacy reset plan: {error}"
                ))
            })?;
            if active_plan != archived_plan {
                return Err(AppError::Conflict(
                    "replayed legacy reset control collides with a different archived plan"
                        .into(),
                ));
            }
            destination = archive_root.join(format!(
                "{}-replay-{}",
                plan.operation_id,
                Uuid::now_v7()
            ));
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect ignored legacy reset plan destination: {error}"
            )));
        }
    }
    rename_with_retry(&source, &destination).map_err(|error| {
        AppError::Internal(format!(
            "archive unstarted legacy reset plan {} -> {}: {error}",
            source.display(),
            destination.display()
        ))
    })?;
    sync_parent(&source).map_err(|error| {
        AppError::Internal(format!(
            "sync ignored legacy reset plan removal: {error}"
        ))
    })?;
    sync_parent(&destination).map_err(|error| {
        AppError::Internal(format!(
            "sync ignored legacy reset plan archive: {error}"
        ))
    })
}

/// Apply the pending filesystem transition.  The plan remains until database
/// bootstrap has completed and [`finalize_v3_dataset_reset`] is called.
pub fn apply_pending_v3_dataset_reset(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<bool, AppError> {
    require_data_root_not_owned_as_external_work(data_dir)?;
    let Some(plan) = read_pending_v3_reset(data_dir, work_dir)? else {
        return Ok(false);
    };
    if plan.version == LEGACY_PLAN_VERSION
        && !has_phase(data_dir, "generation-installed")
    {
        rollback_uncommitted_legacy_reset_plan(
            data_dir, work_dir, &plan,
        )?;
        archive_unstarted_legacy_reset_plan(data_dir, &plan)?;
        tracing::warn!(
            target: "factory_reset",
            operation_id = %plan.operation_id,
            "rolled back and archived an unproven legacy reset plan without losing dataset data"
        );
        return Ok(false);
    }
    let canonical_work = canonical_work_dir(work_dir)?;
    if plan.reason == DatasetResetReason::WorkDirChange
        && !has_phase(data_dir, "generation-installed")
        && inspect_planned_root(
            &canonical_work.join(MANAGED_WORKSPACES_DIR),
            ManagedRootKind::Directory,
        )?
    {
        return Err(AppError::Internal(
            "work-dir change target gained a conversations directory before generation installation; preserving both datasets"
                .into(),
        ));
    }
    ensure_v3_work_root_owner_for_reset(
        data_dir,
        &canonical_work,
        &plan.generation,
    )?;
    write_phase(data_dir, "armed")?;

    // `storage-generation` is atomically installed immediately before its
    // phase marker. A crash in that tiny gap leaves a fully committed marker
    // but no phase, which must be recognized before validating root
    // source/destination pairs on retry.
    if !has_phase(data_dir, "generation-installed")
        && has_phase(data_dir, "quarantined")
        && bounded_regular_file_matches(
            &data_dir.join(STORAGE_GENERATION_FILE),
            plan.generation.as_bytes(),
            128,
        )
    {
        write_phase(data_dir, "quarantined")?;
        write_phase(data_dir, "generation-installed")?;
    }

    let retired_root = data_dir.join(&plan.retired_dir);
    let work_retired_root = canonical_work.join(&plan.work_retired_dir);
    ensure_real_directory(
        &data_dir.join(RETIRED_DATASETS_DIR),
        "retired-datasets directory",
    )?;
    ensure_real_directory(&retired_root, "retired dataset generation directory")?;
    if plan
        .roots
        .iter()
        .any(|root| root.base == ManagedRootBase::WorkDir)
    {
        ensure_real_directory(
            &canonical_work.join(WORK_RETIRED_DATASETS_DIR),
            "managed-workspace retired-datasets directory",
        )?;
        ensure_real_directory(
            &work_retired_root,
            "managed-workspace retired dataset generation directory",
        )?;
    }
    write_phase(data_dir, "quarantine-started")?;

    for root in &plan.roots {
        let (base, retired_base) = match root.base {
            ManagedRootBase::DataDir => (data_dir, retired_root.as_path()),
            ManagedRootBase::WorkDir => (canonical_work.as_path(), work_retired_root.as_path()),
        };
        let source = base.join(&root.relative_path);
        let destination = base.join(&root.retired_relative_path);
        ensure_safe_destination_parent(&destination, retired_base)?;
        let source_state = fs::symlink_metadata(&source);
        let destination_state = fs::symlink_metadata(&destination);
        let generation_installed = has_phase(data_dir, "generation-installed");
        match (source_state, destination_state) {
            (Ok(source_metadata), Err(error))
                if root.initially_present
                    && !generation_installed
                    && error.kind() == std::io::ErrorKind::NotFound =>
            {
                validate_root_metadata(&source, &source_metadata, root.kind)?;
                rename_with_retry(&source, &destination).map_err(|error| {
                    AppError::Internal(format!(
                        "quarantine managed root {} -> {}: {error}",
                        source.display(),
                        destination.display()
                    ))
                })?;
                sync_parent(&source).map_err(|error| {
                    AppError::Internal(format!(
                        "sync source parent after quarantining {}: {error}",
                        source.display()
                    ))
                })?;
                sync_parent(&destination).map_err(|error| {
                    AppError::Internal(format!(
                        "sync retired parent after quarantining {}: {error}",
                        destination.display()
                    ))
                })?;
            }
            (Err(error), Ok(destination_metadata))
                if root.initially_present
                    && error.kind() == std::io::ErrorKind::NotFound =>
            {
                validate_root_metadata(&destination, &destination_metadata, root.kind)?;
                // Crash after the rename: the fixed destination proves this
                // root's transition is complete.
            }
            (Err(error), Err(dest_error))
                if !root.initially_present
                    && error.kind() == std::io::ErrorKind::NotFound
                    && dest_error.kind() == std::io::ErrorKind::NotFound =>
            {}
            (Ok(source_metadata), Ok(destination_metadata))
                if root.initially_present && generation_installed =>
            {
                // The generation-installed phase means the destination is the
                // retired source and the active source is a newly created v3
                // root. This is the normal crash-recovery state after DB or
                // side-store bootstrap started but before receipt/finalize.
                validate_root_metadata(&source, &source_metadata, root.kind)?;
                validate_root_metadata(&destination, &destination_metadata, root.kind)?;
                if plan.version == LEGACY_PLAN_VERSION
                    && root.base == ManagedRootBase::DataDir
                    && root.relative_path
                        == crate::dir_config::DIR_CONFIG_FILE
                {
                    // dir-config is not generated dataset content. A v1 plan
                    // may see both copies only if the control pointer was
                    // deliberately recreated after quarantine; accept it
                    // solely when both still bind to this immutable plan.
                    let active_work =
                        canonical_existing_work_dir(
                            &read_strict_work_dir_config(&source)?,
                        )?;
                    let retired_work = canonical_existing_work_dir(
                        &read_strict_work_dir_config(&destination)?,
                    )?;
                    if !crate::paths::stored_path_matches(&plan.work_dir, &active_work)
                        || !crate::paths::stored_path_matches(
                            &plan.work_dir,
                            &retired_work,
                        )
                    {
                        return Err(AppError::Internal(
                            "active and retired v1 work-dir configs do not both match the reset plan"
                                .into(),
                        ));
                    }
                }
            }
            (Ok(source_metadata), Err(dest_error))
                if !root.initially_present
                    && generation_installed
                    && dest_error.kind() == std::io::ErrorKind::NotFound =>
            {
                // This root did not exist in the retired dataset and was
                // created only by the fresh v3 bootstrap.
                validate_root_metadata(&source, &source_metadata, root.kind)?;
            }
            (Ok(_), Ok(_)) => {
                return Err(AppError::Internal(format!(
                    "ambiguous v3 reset root state: both {} and {} exist",
                    source.display(),
                    destination.display()
                )));
            }
            (Err(error), Err(dest_error))
                if root.initially_present
                    && error.kind() == std::io::ErrorKind::NotFound
                    && dest_error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Err(AppError::Internal(format!(
                    "managed reset root disappeared from both active and retired locations: {}",
                    root.relative_path
                )));
            }
            (Err(error), Ok(_))
                if !root.initially_present
                    && error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Err(AppError::Internal(format!(
                    "unexpected retired copy exists for initially absent root {}",
                    root.relative_path
                )));
            }
            (Ok(_), Err(error))
                if !generation_installed
                    && !root.initially_present
                    && error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Err(AppError::Internal(format!(
                    "initially absent managed root appeared before generation installation: {}",
                    source.display()
                )));
            }
            (Err(error), _) => {
                return Err(AppError::Internal(format!(
                    "inspect reset source {}: {error}",
                    source.display()
                )));
            }
            (Ok(_), Err(error)) => {
                return Err(AppError::Internal(format!(
                    "inspect reset destination {}: {error}",
                    destination.display()
                )));
            }
        }
    }

    if !has_phase(data_dir, "generation-installed") {
        // A pre-upgrade backend that died without process supervision may
        // still have a descendant holding an absolute path to a managed root.
        // Re-scan after every quarantine rename and fail closed if such a
        // writer recreated anything before the replacement generation is
        // installed. The process-level supervisor closes this window for all
        // new versions; this check protects the upgrade boundary.
        for root in &plan.roots {
            let base = match root.base {
                ManagedRootBase::DataDir => data_dir,
                ManagedRootBase::WorkDir => canonical_work.as_path(),
            };
            let source = base.join(&root.relative_path);
            match fs::symlink_metadata(&source) {
                Err(error)
                    if error.kind()
                        == std::io::ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(AppError::Conflict(format!(
                        "managed root {} was recreated by a residual writer during quarantine; preserving both generations",
                        source.display()
                    )));
                }
                Err(error) => {
                    return Err(AppError::Internal(format!(
                        "recheck managed root {} before generation installation: {error}",
                        source.display()
                    )));
                }
            }
        }
    }

    write_phase(data_dir, "quarantined")?;
    install_generation(data_dir, &plan.generation)?;
    write_phase(data_dir, "generation-installed")?;
    tracing::warn!(
        target: "factory_reset",
        reason = ?plan.reason,
        generation = %plan.generation,
        retired_dir = %plan.retired_dir,
        "managed dataset quarantined; awaiting fresh database bootstrap"
    );
    Ok(true)
}

/// Record the v3 receipt after the fresh database has been opened and passed
/// the database worker's contract checks.
pub fn write_v3_dataset_receipt(
    data_dir: &Path,
    generation: &str,
) -> Result<(), AppError> {
    // Keep the old API usable by restore/maintenance code that only owns a
    // data directory.  During a pending reset the immutable plan contains
    // the authoritative resolved work root, so use it when available; a
    // standalone data-only dataset naturally binds to data_dir itself.
    let work_dir = pending_plan_work_dir(data_dir)?.unwrap_or_else(|| data_dir.to_path_buf());
    write_v3_dataset_receipt_for_work_dir(data_dir, &work_dir, generation)
}

pub fn write_v3_dataset_receipt_for_work_dir(
    data_dir: &Path,
    work_dir: &Path,
    generation: &str,
) -> Result<(), AppError> {
    validate_uuidv7(generation)
        .map_err(|error| AppError::Internal(format!("invalid dataset generation: {error}")))?;
    let canonical_work = canonical_work_dir(work_dir)?;
    if let Some(plan) = read_pending_v3_reset(data_dir, &canonical_work)? {
        if plan.generation != generation {
            return Err(AppError::Conflict(
                "pending reset generation does not match the receipt being published"
                    .into(),
            ));
        }
        // A validated pending plan installed the owner before destructive
        // authority was published. Do not silently reclaim a replaced work
        // volume here merely because a stale/forged receipt is being
        // superseded by the plan.
        require_v3_work_root_owner(
            data_dir,
            &canonical_work,
            generation,
        )?;
        ensure_v3_work_root_binding_with_requirement(
            data_dir,
            &canonical_work,
            generation,
            false,
        )?;
    } else {
        ensure_v3_work_root_binding(
            data_dir,
            &canonical_work,
            generation,
        )?;
    }
    let receipt = DatasetReceipt {
        contract_version: V3_DATASET_CONTRACT_VERSION,
        generation: generation.to_owned(),
        work_root: canonical_work.display().to_string(),
        work_root_binding_required: true,
        installed_at: now_ms(),
    };
    let bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|error| AppError::Internal(format!("serialize v3 dataset receipt: {error}")))?;
    write_atomic(&receipt_path(data_dir), &bytes)
        .map_err(|error| AppError::Internal(format!("write v3 dataset receipt: {error}")))?;
    clear_v3_dataset_bootstrap_binding(data_dir)?;
    Ok(())
}

/// Write a complete single-root v3 lifecycle into a sibling staging directory
/// before that directory is atomically renamed to its final restore path.
///
/// The destination must not exist yet. Its canonical identity is derived from
/// its real parent plus one final path component, so neither the receipt nor
/// owner marker is accidentally bound to the temporary staging name.
pub fn write_v3_single_root_lifecycle_for_atomic_install(
    staging_data_dir: &Path,
    destination_data_dir: &Path,
    generation: &str,
) -> Result<(), AppError> {
    validate_uuidv7(generation).map_err(|error| {
        AppError::Internal(format!(
            "invalid atomic-install dataset generation: {error}"
        ))
    })?;
    match fs::symlink_metadata(destination_data_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(AppError::Conflict(format!(
                "atomic-install destination already exists: {}",
                destination_data_dir.display()
            )));
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect atomic-install destination {}: {error}",
                destination_data_dir.display()
            )));
        }
    }
    let destination_parent =
        destination_data_dir.parent().ok_or_else(|| {
            AppError::Internal(
                "atomic-install destination has no parent".into(),
            )
        })?;
    let destination_name =
        destination_data_dir.file_name().ok_or_else(|| {
            AppError::Internal(
                "atomic-install destination has no final component".into(),
            )
        })?;
    let canonical_parent = fs::canonicalize(destination_parent)
        .map(|canonical| crate::paths::simplified(&canonical))
        .map_err(|error| {
            AppError::Internal(format!(
                "canonicalize atomic-install destination parent {}: {error}",
                destination_parent.display()
            ))
        })?;
    let canonical_staging = canonical_data_dir(staging_data_dir)?;
    if canonical_staging.parent() != Some(canonical_parent.as_path()) {
        return Err(AppError::Internal(
            "atomic-install staging directory is not a sibling of its destination"
                .into(),
        ));
    }
    let canonical_destination = canonical_parent.join(destination_name);
    if !bounded_regular_file_matches(
        &canonical_staging.join(STORAGE_GENERATION_FILE),
        generation.as_bytes(),
        128,
    ) {
        return Err(AppError::Internal(
            "atomic-install staging storage-generation does not match".into(),
        ));
    }
    let database = canonical_staging.join(DB_FILE);
    let database_metadata = fs::symlink_metadata(&database).map_err(
        |error| {
            AppError::Internal(format!(
                "inspect atomic-install staging database {}: {error}",
                database.display()
            ))
        },
    )?;
    validate_root_metadata(
        &database,
        &database_metadata,
        ManagedRootKind::File,
    )?;

    let receipt = DatasetReceipt {
        contract_version: V3_DATASET_CONTRACT_VERSION,
        generation: generation.to_owned(),
        work_root: canonical_destination.display().to_string(),
        work_root_binding_required: true,
        installed_at: now_ms(),
    };
    let owner = WorkRootOwner {
        version: WORK_ROOT_OWNER_VERSION,
        data_root: canonical_destination.display().to_string(),
        generation: generation.to_owned(),
        installed_at: now_ms(),
    };
    let binding = WorkRootBinding {
        version: WORK_ROOT_BINDING_VERSION,
        data_root: canonical_destination.display().to_string(),
        work_root: canonical_destination.display().to_string(),
        generation: generation.to_owned(),
        installed_at: now_ms(),
    };
    let receipt_bytes = serde_json::to_vec_pretty(&receipt).map_err(
        |error| {
            AppError::Internal(format!(
                "serialize atomic-install v3 receipt: {error}"
            ))
        },
    )?;
    let owner_bytes = serde_json::to_vec_pretty(&owner).map_err(|error| {
        AppError::Internal(format!(
            "serialize atomic-install work-root owner: {error}"
        ))
    })?;
    let binding_bytes =
        serde_json::to_vec_pretty(&binding).map_err(|error| {
            AppError::Internal(format!(
                "serialize atomic-install work-root binding: {error}"
            ))
        })?;
    write_atomic(
        &canonical_staging.join(V3_DATASET_RECEIPT_FILE),
        &receipt_bytes,
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "write atomic-install v3 receipt: {error}"
        ))
    })?;
    write_atomic(
        &canonical_staging.join(WORK_ROOT_OWNER_FILE),
        &owner_bytes,
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "write atomic-install work-root owner: {error}"
        ))
    })?;
    write_atomic(
        &canonical_staging.join(WORK_ROOT_BINDING_FILE),
        &binding_bytes,
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "write atomic-install work-root binding: {error}"
        ))
    })?;
    Ok(())
}

/// Finish a reset only after the new database has been initialized.
pub fn finalize_v3_dataset_reset(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<bool, AppError> {
    let Some(plan) = read_pending_v3_reset(data_dir, work_dir)? else {
        return Ok(false);
    };
    if !has_phase(data_dir, "generation-installed") {
        return Err(AppError::Internal(
            "cannot finalize v3 dataset reset before generation installation".into(),
        ));
    }
    let receipt_bytes =
        read_bounded_regular_file(&receipt_path(data_dir), MAX_CONTROL_FILE_BYTES)
            .map_err(|error| {
                AppError::Internal(format!(
                    "read v3 dataset receipt during finalize: {error}"
                ))
            })?;
    let receipt: DatasetReceipt = serde_json::from_slice(&receipt_bytes)
        .map_err(|error| AppError::Internal(format!("invalid v3 dataset receipt: {error}")))?;
    if receipt.contract_version != V3_DATASET_CONTRACT_VERSION
        || receipt.generation != plan.generation
        || !stored_paths_equivalent(&receipt.work_root, &plan.work_dir)
        || !receipt.work_root_binding_required
    {
        return Err(AppError::Internal(
            "v3 dataset receipt does not match the reset plan".into(),
        ));
    }
    let database = data_dir.join(DB_FILE);
    let metadata = fs::symlink_metadata(&database).map_err(|error| {
        AppError::Internal(format!(
            "fresh database missing while finalizing reset {}: {error}",
            database.display()
        ))
    })?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(AppError::Internal(
            "fresh database is not a regular file while finalizing reset".into(),
        ));
    }
    if plan_requires_work_dir_persistence(&plan) {
        let configured_work = canonical_existing_work_dir(
            &read_strict_work_dir_config(
                &data_dir.join(crate::dir_config::DIR_CONFIG_FILE),
            )?,
        )?;
        if !crate::paths::stored_path_matches(&plan.work_dir, &configured_work) {
            return Err(AppError::Internal(
                "cannot finalize work-dir change before dir-config is durably bound to the reset plan"
                    .into(),
            ));
        }
    }
    require_v3_work_root_binding(
        data_dir,
        Path::new(&plan.work_dir),
        &plan.generation,
    )?;
    validate_active_reset_request_against_plan(data_dir, &plan)?;
    write_completed_automatic_legacy_retirement(data_dir, &plan)?;
    write_completed_reset_request(data_dir, &plan)?;
    commit_reset_finalization(data_dir, &plan)?;
    clear_reset_request_for_plan(data_dir, &plan)?;
    tracing::info!(
        target: "factory_reset",
        generation = %plan.generation,
        "v3 managed dataset reset finalized"
    );
    Ok(true)
}

fn pending_plan_work_dir(data_dir: &Path) -> Result<Option<PathBuf>, AppError> {
    pending_v3_reset_work_dir(data_dir)
}

pub fn write_v3_dataset_bootstrap_binding(
    data_dir: &Path,
    work_dir: &Path,
    generation: &str,
) -> Result<(), AppError> {
    validate_uuidv7(generation)
        .map_err(|error| AppError::Internal(format!("invalid dataset generation: {error}")))?;
    let canonical_work = canonical_work_dir(work_dir)?;
    ensure_v3_work_root_binding(
        data_dir,
        &canonical_work,
        generation,
    )?;
    let binding = DatasetBootstrapBinding {
        contract_version: V3_DATASET_CONTRACT_VERSION,
        generation: generation.to_owned(),
        work_root: canonical_work.display().to_string(),
        prepared_at: now_ms(),
    };

    match inspect_v3_dataset_bootstrap_binding(data_dir, work_dir)? {
        DatasetReceiptStatus::Missing => {}
        DatasetReceiptStatus::Current => {
            let existing: DatasetBootstrapBinding = serde_json::from_slice(
                &read_bounded_regular_file(
                    &bootstrap_binding_path(data_dir),
                    MAX_CONTROL_FILE_BYTES,
                )
                .map_err(|error| {
                    AppError::Internal(format!(
                        "read current v3 bootstrap binding {}: {error}",
                        bootstrap_binding_path(data_dir).display()
                    ))
                })?,
            )
            .map_err(|error| {
                AppError::Internal(format!("invalid current v3 bootstrap binding: {error}"))
            })?;
            if existing.generation == generation {
                return Ok(());
            }
            return Err(AppError::Internal(
                "v3 bootstrap binding generation does not match storage-generation".into(),
            ));
        }
        DatasetReceiptStatus::WorkRootMismatch => {
            return Err(AppError::Internal(
                "v3 bootstrap binding belongs to a different resolved work root".into(),
            ));
        }
        DatasetReceiptStatus::Invalid => {
            return Err(AppError::Internal(
                "v3 bootstrap binding is malformed or inconsistent".into(),
            ));
        }
    }

    let bytes = serde_json::to_vec_pretty(&binding).map_err(|error| {
        AppError::Internal(format!("serialize v3 dataset bootstrap binding: {error}"))
    })?;
    write_atomic(&bootstrap_binding_path(data_dir), &bytes).map_err(|error| {
        AppError::Internal(format!("write v3 dataset bootstrap binding: {error}"))
    })
}

pub fn inspect_v3_dataset_bootstrap_binding(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<DatasetReceiptStatus, AppError> {
    let bytes = match read_bounded_regular_file(
        &bootstrap_binding_path(data_dir),
        MAX_CONTROL_FILE_BYTES,
    ) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DatasetReceiptStatus::Missing);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read v3 dataset bootstrap binding {}: {error}",
                bootstrap_binding_path(data_dir).display()
            )));
        }
    };
    let Ok(binding) = serde_json::from_slice::<DatasetBootstrapBinding>(&bytes) else {
        return Ok(DatasetReceiptStatus::Invalid);
    };
    if binding.contract_version != V3_DATASET_CONTRACT_VERSION
        || validate_uuidv7(&binding.generation).is_err()
        || !bounded_regular_file_matches(
            &data_dir.join(STORAGE_GENERATION_FILE),
            binding.generation.as_bytes(),
            128,
        )
    {
        return Ok(DatasetReceiptStatus::Invalid);
    }
    let canonical_work = canonical_existing_work_dir(work_dir)?;
    if !crate::paths::stored_path_matches(&binding.work_root, &canonical_work) {
        return Ok(DatasetReceiptStatus::WorkRootMismatch);
    }
    Ok(DatasetReceiptStatus::Current)
}

fn clear_v3_dataset_bootstrap_binding(data_dir: &Path) -> Result<(), AppError> {
    let path = bootstrap_binding_path(data_dir);
    match fs::remove_file(&path) {
        Ok(()) => sync_parent(&path).map_err(|error| {
            AppError::Internal(format!(
                "sync v3 dataset bootstrap binding removal {}: {error}",
                path.display()
            ))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Internal(format!(
            "remove v3 dataset bootstrap binding {}: {error}",
            path.display()
        ))),
    }
}

pub fn inspect_v3_dataset_receipt(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<DatasetReceiptStatus, AppError> {
    let bytes = match read_bounded_regular_file(
        &receipt_path(data_dir),
        MAX_CONTROL_FILE_BYTES,
    ) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DatasetReceiptStatus::Missing);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read v3 dataset receipt {}: {error}",
                receipt_path(data_dir).display()
            )));
        }
    };
    let Ok(receipt) = serde_json::from_slice::<DatasetReceipt>(&bytes) else {
        return Ok(DatasetReceiptStatus::Invalid);
    };
    if receipt.contract_version != V3_DATASET_CONTRACT_VERSION
        || validate_uuidv7(&receipt.generation).is_err()
    {
        return Ok(DatasetReceiptStatus::Invalid);
    }
    if !bounded_regular_file_matches(
        &data_dir.join(STORAGE_GENERATION_FILE),
        receipt.generation.as_bytes(),
        128,
    ) || !matches!(
        fs::symlink_metadata(data_dir.join(DB_FILE)),
        Ok(metadata) if metadata.is_file() && !metadata_is_link_or_reparse(&metadata)
    ) {
        return Ok(DatasetReceiptStatus::Invalid);
    }
    let canonical_work = canonical_existing_work_dir(work_dir)?;
    if !crate::paths::stored_path_matches(&receipt.work_root, &canonical_work) {
        return Ok(DatasetReceiptStatus::WorkRootMismatch);
    }
    Ok(DatasetReceiptStatus::Current)
}

/// Return the canonical work root of a structurally current finalized v3
/// receipt. Missing receipts return `None`; malformed, stale or unsafe
/// bindings fail closed.
pub fn finalized_v3_work_dir(
    data_dir: &Path,
) -> Result<Option<PathBuf>, AppError> {
    require_data_root_not_owned_as_external_work(data_dir)?;
    let path = receipt_path(data_dir);
    let bytes = match read_bounded_regular_file(
        &path,
        MAX_CONTROL_FILE_BYTES,
    ) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read finalized v3 receipt {}: {error}",
                path.display()
            )));
        }
    };
    let receipt: DatasetReceipt = serde_json::from_slice(&bytes).map_err(
        |error| {
            AppError::Internal(format!(
                "invalid finalized v3 receipt {}: {error}",
                path.display()
            ))
        },
    )?;
    if receipt.contract_version != V3_DATASET_CONTRACT_VERSION
        || validate_uuidv7(&receipt.generation).is_err()
    {
        return Err(AppError::Internal(
            "finalized v3 receipt has an invalid contract or generation".into(),
        ));
    }
    let work_dir = PathBuf::from(&receipt.work_root);
    if work_dir.as_os_str().is_empty()
        || !work_dir.is_absolute()
        || crate::workspace_path_has_edge_whitespace_segment(&work_dir)
    {
        return Err(AppError::Internal(
            "finalized v3 receipt has an unsafe work root".into(),
        ));
    }
    let canonical_work = canonical_existing_work_dir(&work_dir)?;
    if !crate::paths::stored_path_matches(&receipt.work_root, &canonical_work)
        || inspect_v3_dataset_receipt(data_dir, &canonical_work)?
            != DatasetReceiptStatus::Current
    {
        return Err(AppError::Internal(
            "finalized v3 receipt is stale or inconsistent".into(),
        ));
    }
    Ok(Some(canonical_work))
}

fn receipt_is_current(data_dir: &Path, work_dir: &Path) -> Result<bool, AppError> {
    Ok(matches!(
        inspect_v3_dataset_receipt(data_dir, work_dir)?,
        DatasetReceiptStatus::Current
    ))
}

/// Require a fully finalized current-generation v3 dataset without mutating it.
///
/// Offline preservation commands use this gate instead of
/// [`prepare_v3_dataset`]: backup must never create lifecycle markers, reset
/// old data, or capture a half-initialized post-reset dataset.
pub fn require_current_v3_dataset(data_dir: &Path) -> Result<(), AppError> {
    let receipt_bytes =
        read_bounded_regular_file(&receipt_path(data_dir), MAX_CONTROL_FILE_BYTES)
            .map_err(|error| {
                AppError::Internal(format!(
                    "read v3 dataset receipt {}: {error}",
                    receipt_path(data_dir).display()
                ))
            })?;
    let receipt: DatasetReceipt = serde_json::from_slice(&receipt_bytes)
        .map_err(|error| AppError::Internal(format!("invalid v3 dataset receipt: {error}")))?;
    require_current_v3_dataset_for_work_dir(data_dir, Path::new(&receipt.work_root))
}

pub fn require_current_v3_dataset_for_work_dir(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<(), AppError> {
    require_data_root_not_owned_as_external_work(data_dir)?;
    match fs::symlink_metadata(request_path(data_dir)) {
        Ok(_) => {
            return Err(AppError::Internal(
                "an explicit v3 dataset reset has been requested".into(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect explicit v3 dataset reset request: {error}"
            )));
        }
    }
    match fs::symlink_metadata(reset_dir(data_dir)) {
        Ok(_) => {
            return Err(AppError::Internal(
                "a v3 dataset reset is still pending".into(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect v3 dataset reset state: {error}"
            )));
        }
    }
    if !receipt_is_current(data_dir, work_dir)? {
        return Err(AppError::Internal(
            "dataset does not have a matching finalized v3 receipt, generation, and work root"
                .into(),
        ));
    }
    let receipt_bytes =
        read_bounded_regular_file(&receipt_path(data_dir), MAX_CONTROL_FILE_BYTES)
            .map_err(|error| {
                AppError::Internal(format!(
                    "read current v3 receipt while enforcing its work-root binding: {error}"
                ))
            })?;
    let receipt: DatasetReceipt =
        serde_json::from_slice(&receipt_bytes).map_err(|error| {
            AppError::Internal(format!(
                "parse current v3 receipt while enforcing its work-root binding: {error}"
            ))
        })?;
    if receipt.work_root_binding_required {
        require_v3_work_root_binding(
            data_dir,
            work_dir,
            &receipt.generation,
        )?;
    }
    Ok(())
}

/// Install or validate the persistent work-root owner for an already finalized
/// current dataset. This is the one-time compatibility bridge for v3 receipts
/// written before the owner marker existed.
pub fn ensure_current_v3_work_root_owner(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<(), AppError> {
    require_current_v3_dataset_for_work_dir(data_dir, work_dir)?;
    let bytes =
        read_bounded_regular_file(&receipt_path(data_dir), MAX_CONTROL_FILE_BYTES)
            .map_err(|error| {
                AppError::Internal(format!(
                    "read finalized receipt while installing work-root owner: {error}"
                ))
            })?;
    let mut receipt: DatasetReceipt =
        serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Internal(format!(
                "parse finalized receipt while installing work-root owner: {error}"
            ))
        })?;
    ensure_v3_work_root_binding_with_requirement(
        data_dir,
        work_dir,
        &receipt.generation,
        receipt.work_root_binding_required,
    )?;
    if !receipt.work_root_binding_required {
        receipt.work_root_binding_required = true;
        let bytes =
            serde_json::to_vec_pretty(&receipt).map_err(|error| {
                AppError::Internal(format!(
                    "serialize upgraded finalized v3 receipt: {error}"
                ))
            })?;
        write_atomic(&receipt_path(data_dir), &bytes).map_err(
            |error| {
                AppError::Internal(format!(
                    "persist one-time work-root binding requirement: {error}"
                ))
            },
        )?;
    }
    Ok(())
}

/// Prove that a root is either completely fresh or stopped during the
/// first-bootstrap control-file sequence.
///
/// `storage-generation` is installed before the owner and bootstrap binding.
/// A crash between those durable writes must be retryable, but only when no
/// database or business-data root exists and any already-installed owner
/// matches the exact data root and generation.
fn fresh_or_interrupted_bootstrap_root(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<bool, AppError> {
    let mut storage_generation = None;
    let mut binding_present = false;
    for (root, _) in current_writer_managed_roots() {
        let path = data_dir.join(root);
        match fs::symlink_metadata(&path) {
            Ok(_) if root == STORAGE_GENERATION_FILE => {
                let bytes =
                    read_bounded_regular_file(&path, 128).map_err(|error| {
                        AppError::Internal(format!(
                            "read storage generation while proving an interrupted bootstrap {}: {error}",
                            path.display()
                        ))
                    })?;
                let generation = std::str::from_utf8(&bytes).map_err(|error| {
                    AppError::Internal(format!(
                        "storage generation is not UTF-8 while proving an interrupted bootstrap: {error}"
                    ))
                })?;
                validate_uuidv7(generation).map_err(|error| {
                    AppError::Internal(format!(
                        "storage generation is invalid while proving an interrupted bootstrap: {error}"
                    ))
                })?;
                storage_generation = Some(generation.to_owned());
            }
            Ok(_) if root == WORK_ROOT_BINDING_FILE => {
                binding_present = true;
            }
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "inspect managed root {} while proving a fresh or interrupted bootstrap: {error}",
                    path.display()
                )));
            }
        }
    }

    // A relocated conversation workspace remains part of the managed dataset
    // even when data_dir itself is otherwise empty. Treating this as a fresh
    // bootstrap would leave pre-v3 workspaces active beside a new v3 database.
    let canonical_data = canonical_data_dir(data_dir)?;
    let canonical_work = canonical_work_dir(work_dir)?;
    if canonical_work != canonical_data
        && inspect_planned_root(
            &canonical_work.join(MANAGED_WORKSPACES_DIR),
            ManagedRootKind::Directory,
        )?
    {
        return Ok(false);
    }

    let owner = read_work_root_owner(&canonical_work)?;
    let binding = if binding_present {
        read_work_root_binding(&canonical_data)?
    } else {
        None
    };
    match (storage_generation, binding, owner) {
        (None, None, None) => Ok(true),
        (Some(_), None, None) => Ok(true),
        (Some(generation), None, Some(owner))
            if owner_matches(&owner, &canonical_data, &generation) =>
        {
            Ok(true)
        }
        (Some(generation), Some(binding), Some(owner)) => {
            Ok(owner_matches(
                &owner,
                &canonical_data,
                &generation,
            ) && binding_matches(
                &binding,
                &canonical_data,
                &canonical_work,
                &generation,
            ))
        }
        // An owner cannot precede storage-generation in the supported write
        // order, and a mismatched owner may belong to another installation.
        // Preserve both cases without claiming or rewriting the work root.
        _ => Ok(false),
    }
}

/// Detect the filesystem-level dataset state before the database is opened.
///
/// The database worker should provide the authoritative schema/value probe.
/// This receipt is only the lifecycle hand-off: it prevents a clean fresh boot
/// from being reset again after the database worker has accepted the new
/// dataset.
pub fn prepare_v3_dataset(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<DatasetPreparation, AppError> {
    require_data_root_not_owned_as_external_work(data_dir)?;
    archive_active_ignored_legacy_reset_control_replay(data_dir)?;
    archive_active_completed_reset_control_replay(data_dir)?;
    archive_unusable_reset_control(data_dir)?;
    if let Some(plan) = read_pending_v3_reset(data_dir, work_dir)? {
        // The immutable plan is authoritative. A crash may have happened after
        // the plan commit but before the transient request was removed.
        validate_active_reset_request_against_plan(data_dir, &plan)?;
        let applied =
            apply_pending_v3_dataset_reset(data_dir, work_dir)?;
        if applied {
            if plan_requires_work_dir_persistence(&plan) {
                crate::dir_config::set_work_dir(
                    data_dir,
                    Path::new(&plan.work_dir),
                )?;
            }
            clear_reset_request_for_plan(data_dir, &plan)?;
            return Ok(DatasetPreparation::ResetApplied);
        }
        return Ok(DatasetPreparation::Unchanged);
    }
    if let Some(request) = read_v3_dataset_reset_request(data_dir)? {
        if completed_reset_matches_request(data_dir, &request)? {
            archive_reset_request(
                data_dir,
                &request,
                REPLAYED_COMPLETED_RESET_REQUESTS_DIR,
            )?;
            tracing::warn!(
                target: "factory_reset",
                operation_id = %request.operation_id,
                "consumed a replayed completed reset request without touching the current v3 dataset"
            );
            return Ok(DatasetPreparation::Unchanged);
        } else if cancelled_reset_matches_request(data_dir, &request)? {
            archive_cancelled_work_dir_change_request(
                data_dir, &request,
            )?;
            tracing::warn!(
                target: "factory_reset",
                operation_id = %request.operation_id,
                "consumed a replayed cancelled work-dir request without touching the current v3 dataset"
            );
            return Ok(DatasetPreparation::Unchanged);
        } else if request.version == LEGACY_RESET_REQUEST_VERSION {
            archive_ignored_legacy_reset_request(data_dir, &request)?;
            tracing::warn!(
                target: "factory_reset",
                operation_id = %request.operation_id,
                "ignored an unproven legacy reset request without mutating dataset data; \
                 the user can explicitly request a new reset"
            );
        } else {
            if let Some(requested_work_dir) = &request.work_dir {
                let canonical_work = canonical_work_dir(work_dir)?;
                if !crate::paths::stored_path_matches(
                    requested_work_dir,
                    &canonical_work,
                ) {
                    return Err(AppError::Internal(
                        "resolved work root does not match the pending v3 reset request"
                            .into(),
                    ));
                }
            }
            let reason = match request.origin {
                Some(DatasetResetRequestOrigin::WorkDirChange) => {
                    DatasetResetReason::WorkDirChange
                }
                Some(DatasetResetRequestOrigin::UserExplicitFactoryReset) => {
                    DatasetResetReason::ExplicitFactoryReset
                }
                None => {
                    return Err(AppError::Internal(
                        "current reset request is missing its origin".into(),
                    ));
                }
            };
            if reason == DatasetResetReason::WorkDirChange
                && let Err(error) =
                    require_safe_work_dir_change_target(data_dir, work_dir)
            {
                archive_cancelled_work_dir_change_request(
                    data_dir, &request,
                )?;
                return Err(AppError::Internal(format!(
                    "work-dir change request was cancelled without changing the active dataset: {error}"
                )));
            }
            let plan =
                arm_v3_dataset_reset(data_dir, work_dir, reason)?;
            apply_pending_v3_dataset_reset(data_dir, work_dir)?;
            if plan_requires_work_dir_persistence(&plan) {
                crate::dir_config::set_work_dir(
                    data_dir,
                    Path::new(&plan.work_dir),
                )?;
            }
            return Ok(DatasetPreparation::ResetApplied);
        }
    }
    // A database file is the point at which the application probe becomes
    // authoritative.  Do not retire it merely because a receipt is missing,
    // stale, or was written by an older process: a perfectly valid v3
    // database can exist after a crash before receipt finalization.  The app
    // probes this file read-only and only then calls
    // `retire_non_v3_dataset_after_probe` for a rejected lineage.
    if matches!(
        fs::symlink_metadata(data_dir.join(DB_FILE)),
        Ok(_)
    ) {
        return Ok(DatasetPreparation::Unchanged);
    }
    let bootstrap_status = inspect_v3_dataset_bootstrap_binding(data_dir, work_dir)?;
    if bootstrap_status == DatasetReceiptStatus::WorkRootMismatch {
        return Err(AppError::Internal(
            "v3 bootstrap binding belongs to a different resolved work root".into(),
        ));
    }
    if receipt_is_current(data_dir, work_dir)?
        || bootstrap_status == DatasetReceiptStatus::Current
        || fresh_or_interrupted_bootstrap_root(data_dir, work_dir)?
    {
        return Ok(DatasetPreparation::Unchanged);
    }

    let plan =
        arm_v3_dataset_reset(data_dir, work_dir, DatasetResetReason::NonV3Dataset)?;
    apply_pending_v3_dataset_reset(data_dir, work_dir)?;
    if plan_requires_work_dir_persistence(&plan) {
        crate::dir_config::set_work_dir(
            data_dir,
            Path::new(&plan.work_dir),
        )?;
    }
    Ok(DatasetPreparation::ResetApplied)
}

/// Retire an active dataset after a read-only database probe proved that its
/// claimed v3 receipt does not match the database identity/schema.
///
/// This is deliberately separate from [`prepare_v3_dataset`]. The filesystem
/// coordinator cannot prove SQLite lineage itself, while the application must
/// not open the rejected database through writable initialization merely to
/// discover that the receipt was forged or stale.
pub fn retire_non_v3_dataset_after_probe(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<DatasetPreparation, AppError> {
    if read_pending_v3_reset(data_dir, work_dir)?.is_some() {
        return Err(AppError::Internal(
            "the active database failed its v3 probe while a dataset reset is already pending"
                .into(),
        ));
    }

    let request = read_v3_dataset_reset_request(data_dir)?;
    let request_is_fresh_explicit_authority =
        if let Some(request) = request.as_ref() {
            request.version == RESET_REQUEST_VERSION
                && !completed_reset_matches_request(
                    data_dir, request,
                )?
                && !cancelled_reset_matches_request(
                    data_dir, request,
                )?
        } else {
            false
        };
    if read_completed_automatic_legacy_retirement(data_dir)?.is_some()
        && !request_is_fresh_explicit_authority
    {
        return Err(AppError::Conflict(
            "this installation has already consumed its one automatic legacy-data retirement; \
             preserving the detected legacy data without mutation; use an explicit factory reset \
             if the replacement dataset should be cleared"
                .into(),
        ));
    }
    let reason = if request.is_some() {
        DatasetResetReason::ExplicitFactoryReset
    } else {
        DatasetResetReason::NonV3Dataset
    };
    let plan = arm_v3_dataset_reset(data_dir, work_dir, reason)?;
    apply_pending_v3_dataset_reset(data_dir, work_dir)?;
    if plan_requires_work_dir_persistence(&plan) {
        crate::dir_config::set_work_dir(
            data_dir,
            Path::new(&plan.work_dir),
        )?;
    }
    Ok(DatasetPreparation::ResetApplied)
}

/// Arm an explicit v3 reset request. The destructive transition occurs during
/// the next pre-database boot so it cannot race live pools or background jobs.
pub fn request_v3_dataset_reset(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<(), AppError> {
    require_data_root_not_owned_as_external_work(data_dir)?;
    let canonical_data = canonical_data_dir(data_dir)?;
    let work_dir = canonical_existing_work_dir(work_dir)?;
    let managed_roots = current_writer_managed_roots();
    validate_safe_reset_data_and_work_roots(
        &canonical_data,
        &work_dir,
        &managed_roots,
    )?;
    write_v3_dataset_reset_request(
        data_dir,
        DatasetResetRequest::new(
            DatasetResetRequestOrigin::UserExplicitFactoryReset,
            Some(work_dir.display().to_string()),
        ),
    )
}

/// Atomically request a reset that rebinds the fresh dataset to `work_dir`.
///
/// The target lives in the same durable request as the destructive intent so a
/// crash can never persist only half of a work-root change. The boot resolver
/// reads this target before the request is consumed.
pub fn request_v3_dataset_reset_for_work_dir(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<(), AppError> {
    require_data_root_not_owned_as_external_work(data_dir)?;
    let canonical_work = canonical_work_dir(work_dir)?;
    require_safe_work_dir_change_target(data_dir, &canonical_work)?;
    write_v3_dataset_reset_request(
        data_dir,
        DatasetResetRequest::new(
            DatasetResetRequestOrigin::WorkDirChange,
            Some(canonical_work.display().to_string()),
        ),
    )
}

fn write_v3_dataset_reset_request(
    data_dir: &Path,
    request: DatasetResetRequest,
) -> Result<(), AppError> {
    if let Some(existing) = read_v3_dataset_reset_request(data_dir)? {
        if existing.version == request.version
            && existing.origin == request.origin
            && existing.work_dir == request.work_dir
        {
            return Ok(());
        }
        return Err(AppError::Conflict(
            "a different v3 dataset reset request is already pending".into(),
        ));
    }
    let json = serde_json::to_vec_pretty(&request).map_err(|error| {
        AppError::Internal(format!("serialize v3 dataset reset request: {error}"))
    })?;
    match write_atomic_new(&request_path(data_dir), &json) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = read_v3_dataset_reset_request(data_dir)?
                .ok_or_else(|| {
                    AppError::Conflict(
                        "a concurrent v3 dataset reset request changed before it could be read"
                            .into(),
                    )
                })?;
            if existing.version == request.version
                && existing.origin == request.origin
                && existing.work_dir == request.work_dir
            {
                Ok(())
            } else {
                Err(AppError::Conflict(
                    "a different v3 dataset reset request is already pending"
                        .into(),
                ))
            }
        }
        Err(error) => Err(AppError::Internal(format!(
            "write v3 dataset reset request: {error}"
        ))),
    }
}

/// Return the canonical work root carried by a pending reset request.
///
/// Both explicit factory-reset and work-directory-change requests are bound
/// to the work root that the next boot must use.
pub fn requested_v3_reset_work_dir(
    data_dir: &Path,
) -> Result<Option<PathBuf>, AppError> {
    let Some(request) = read_v3_dataset_reset_request(data_dir)? else {
        return Ok(None);
    };
    if completed_reset_matches_request(data_dir, &request)? {
        // A power-loss replay must not redirect even this boot's resolver to
        // the work root of an already-consumed generation. The locked
        // preparation step will archive the request; resolution continues
        // through the pending plan/current persisted binding meanwhile.
        return Ok(None);
    }
    if cancelled_reset_matches_request(data_dir, &request)? {
        // Cancellation permanently consumes this exact operation ID. A
        // replayed entry cannot redirect startup merely because its old
        // target has since become empty or available again.
        return Ok(None);
    }
    let Some(work_dir) = request.work_dir else {
        return Ok(None);
    };
    let canonical = canonical_existing_work_dir(Path::new(&work_dir))?;
    if !crate::paths::stored_path_matches(&work_dir, &canonical) {
        return Err(AppError::Internal(
            "v3 dataset reset request work_dir is not canonical".into(),
        ));
    }
    Ok(Some(canonical))
}

fn read_v3_dataset_reset_request(
    data_dir: &Path,
) -> Result<Option<DatasetResetRequest>, AppError> {
    let path = request_path(data_dir);
    let bytes =
        match read_bounded_regular_file(&path, MAX_CONTROL_FILE_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read v3 dataset reset request {}: {error}",
                path.display()
            )));
        }
    };
    let request: DatasetResetRequest = serde_json::from_slice(&bytes).map_err(|error| {
        AppError::Internal(format!(
            "malformed v3 dataset reset request {}: {error}",
            path.display()
        ))
    })?;
    request.validate()?;
    Ok(Some(request))
}

fn reset_request_matches_plan(
    request: &DatasetResetRequest,
    plan: &DatasetResetPlan,
) -> bool {
    request.version == RESET_REQUEST_VERSION
        && request.operation_id == plan.operation_id
        && request.requested_at == plan.requested_at
        && request.origin == request_origin_for_reason(plan.reason)
        && request
            .work_dir
            .as_deref()
            .is_some_and(|work_dir| {
                stored_paths_equivalent(work_dir, &plan.work_dir)
            })
}

fn validate_active_reset_request_against_plan(
    data_dir: &Path,
    plan: &DatasetResetPlan,
) -> Result<(), AppError> {
    let Some(request) = read_v3_dataset_reset_request(data_dir)? else {
        return Ok(());
    };
    if reset_request_matches_plan(&request, plan) {
        Ok(())
    } else if request.version == LEGACY_RESET_REQUEST_VERSION {
        // A v1 request carried no root or reason and is never destructive
        // authority in this build. A validated immutable plan is already the
        // sole authority, so an old directory-entry replay may be archived
        // without blocking recovery of either a v1 or v2 plan.
        archive_ignored_legacy_reset_request(data_dir, &request)?;
        tracing::warn!(
            target: "factory_reset",
            operation_id = %request.operation_id,
            pending_operation_id = %plan.operation_id,
            "archived an unproven legacy reset request while resuming the immutable pending plan"
        );
        Ok(())
    } else if completed_reset_matches_request(data_dir, &request)? {
        // A completed older request can be resurrected by a filesystem or
        // power-loss rollback after a newer plan has become authoritative.
        // Permanently consumed authority must not block that newer plan.
        archive_reset_request(
            data_dir,
            &request,
            REPLAYED_COMPLETED_RESET_REQUESTS_DIR,
        )?;
        tracing::warn!(
            target: "factory_reset",
            operation_id = %request.operation_id,
            pending_operation_id = %plan.operation_id,
            "archived a replayed completed reset request while resuming the immutable pending plan"
        );
        Ok(())
    } else if cancelled_reset_matches_request(data_dir, &request)? {
        archive_cancelled_work_dir_change_request(data_dir, &request)?;
        tracing::warn!(
            target: "factory_reset",
            operation_id = %request.operation_id,
            pending_operation_id = %plan.operation_id,
            "consumed a replayed cancelled work-dir request while resuming the immutable pending plan"
        );
        Ok(())
    } else {
        Err(AppError::Conflict(
            "active reset request does not match the immutable pending reset plan"
                .into(),
        ))
    }
}

fn clear_reset_request_for_plan(
    data_dir: &Path,
    plan: &DatasetResetPlan,
) -> Result<(), AppError> {
    let Some(request) = read_v3_dataset_reset_request(data_dir)? else {
        return Ok(());
    };
    if !reset_request_matches_plan(&request, plan) {
        return Err(AppError::Conflict(
            "refusing to clear a reset request that does not match the immutable pending plan"
                .into(),
        ));
    }
    clear_v3_dataset_reset_request(data_dir)
}

fn read_completed_automatic_legacy_retirement(
    data_dir: &Path,
) -> Result<Option<CompletedAutomaticLegacyRetirement>, AppError> {
    let retired_root = data_dir.join(RETIRED_DATASETS_DIR);
    match fs::symlink_metadata(&retired_root) {
        Ok(metadata) => validate_root_metadata(
            &retired_root,
            &metadata,
            ManagedRootKind::Directory,
        )?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect automatic legacy retirement parent {}: {error}",
                retired_root.display()
            )));
        }
    }
    let path = automatic_legacy_retirement_path(data_dir);
    let bytes =
        match read_bounded_regular_file(&path, MAX_CONTROL_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "read automatic legacy retirement marker {}: {error}",
                    path.display()
                )));
            }
        };
    let completed: CompletedAutomaticLegacyRetirement =
        serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Internal(format!(
                "invalid automatic legacy retirement marker {}: {error}",
                path.display()
            ))
        })?;
    completed.validate()?;
    if !crate::paths::stored_path_matches(
        &completed.data_dir,
        &canonical_data_dir(data_dir)?,
    ) {
        return Err(AppError::Conflict(
            "automatic legacy retirement marker belongs to a different data root"
                .into(),
        ));
    }
    Ok(Some(completed))
}

fn plan_consumes_automatic_legacy_retirement(
    plan: &DatasetResetPlan,
) -> bool {
    plan.automatic_legacy_retirement
        || plan.version == LEGACY_PLAN_VERSION
            && !matches!(
                plan.reason,
                DatasetResetReason::ExplicitFactoryReset
            )
}

fn write_completed_automatic_legacy_retirement(
    data_dir: &Path,
    plan: &DatasetResetPlan,
) -> Result<(), AppError> {
    if !plan_consumes_automatic_legacy_retirement(plan) {
        return Ok(());
    }
    let completed = CompletedAutomaticLegacyRetirement {
        version: AUTOMATIC_LEGACY_RETIREMENT_VERSION,
        operation_id: plan.operation_id.clone(),
        generation: plan.generation.clone(),
        data_dir: plan.data_dir.clone(),
        work_dir: plan.work_dir.clone(),
        reason: plan.reason,
        requested_at: plan.requested_at,
        completed_at: now_ms().max(plan.requested_at),
    };
    completed.validate()?;
    let retired_root = data_dir.join(RETIRED_DATASETS_DIR);
    ensure_real_directory(&retired_root, "retired-datasets directory")?;
    sync_parent(&retired_root).map_err(|error| {
        AppError::Internal(format!(
            "sync automatic legacy retirement parent publication: {error}"
        ))
    })?;
    let path = automatic_legacy_retirement_path(data_dir);
    let bytes = serde_json::to_vec_pretty(&completed).map_err(|error| {
        AppError::Internal(format!(
            "serialize automatic legacy retirement marker: {error}"
        ))
    })?;
    match write_atomic_new(&path, &bytes) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing =
                read_completed_automatic_legacy_retirement(data_dir)?
                    .ok_or_else(|| {
                        AppError::Conflict(
                            "automatic legacy retirement marker changed concurrently"
                                .into(),
                        )
                    })?;
            if existing.operation_id == completed.operation_id
                && existing.generation == completed.generation
                && stored_paths_equivalent(&existing.data_dir, &completed.data_dir)
                && stored_paths_equivalent(&existing.work_dir, &completed.work_dir)
                && existing.reason == completed.reason
                && existing.requested_at == completed.requested_at
            {
                Ok(())
            } else {
                Err(AppError::Conflict(
                    "automatic legacy retirement has already been consumed by another generation"
                        .into(),
                ))
            }
        }
        Err(error) => Err(AppError::Internal(format!(
            "publish automatic legacy retirement marker {}: {error}",
            path.display()
        ))),
    }
}

fn write_completed_reset_request(
    data_dir: &Path,
    plan: &DatasetResetPlan,
) -> Result<(), AppError> {
    let Some(origin) = request_origin_for_reason(plan.reason) else {
        return Ok(());
    };
    let completed = CompletedResetRequest {
        version: COMPLETED_RESET_REQUEST_VERSION,
        operation_id: plan.operation_id.clone(),
        origin,
        work_dir: plan.work_dir.clone(),
        generation: plan.generation.clone(),
        requested_at: plan.requested_at,
        completed_at: now_ms().max(plan.requested_at),
    };
    completed.validate()?;
    ensure_completed_requests_dir(data_dir)?;
    let path = completed_request_path(data_dir, &plan.operation_id);
    let bytes = serde_json::to_vec_pretty(&completed).map_err(|error| {
        AppError::Internal(format!(
            "serialize completed reset request: {error}"
        ))
    })?;
    match write_atomic_new(&path, &bytes) {
        Ok(()) => Ok(()),
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            let existing_bytes =
                read_bounded_regular_file(&path, MAX_CONTROL_FILE_BYTES)
                    .map_err(|read_error| {
                        AppError::Internal(format!(
                            "read completed reset request during retry: {read_error}"
                        ))
                    })?;
            let existing: CompletedResetRequest =
                serde_json::from_slice(&existing_bytes).map_err(
                    |parse_error| {
                        AppError::Internal(format!(
                            "invalid completed reset request during retry: {parse_error}"
                        ))
                    },
                )?;
            existing.validate()?;
            if existing.operation_id == completed.operation_id
                && existing.origin == completed.origin
                && stored_paths_equivalent(&existing.work_dir, &completed.work_dir)
                && existing.generation == completed.generation
                && existing.requested_at == completed.requested_at
            {
                Ok(())
            } else {
                Err(AppError::Conflict(
                    "completed reset request tombstone collision".into(),
                ))
            }
        }
        Err(error) => Err(AppError::Internal(format!(
            "publish completed reset request {}: {error}",
            path.display()
        ))),
    }
}

fn completed_reset_matches_request(
    data_dir: &Path,
    request: &DatasetResetRequest,
) -> Result<bool, AppError> {
    if request.version != RESET_REQUEST_VERSION {
        return Ok(false);
    }
    if !completed_requests_dir_exists_and_is_safe(data_dir)? {
        return Ok(false);
    }
    let path = completed_request_path(data_dir, &request.operation_id);
    let bytes =
        match read_bounded_regular_file(&path, MAX_CONTROL_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(false);
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "read completed reset request {}: {error}",
                    path.display()
                )));
            }
        };
    let completed: CompletedResetRequest =
        serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Internal(format!(
                "invalid completed reset request {}: {error}",
                path.display()
            ))
        })?;
    completed.validate()?;
    if completed.operation_id != request.operation_id
        || Some(completed.origin) != request.origin
        || !request.work_dir.as_deref().is_some_and(|work_dir| {
            stored_paths_equivalent(work_dir, &completed.work_dir)
        })
        || completed.requested_at != request.requested_at
    {
        return Err(AppError::Conflict(
            "replayed reset request collides with a different completed operation"
                .into(),
        ));
    }
    // Consumption is permanent. A later legitimate reset/restore may have
    // advanced the active generation, but it must never restore destructive
    // authority to an older operation whose directory entry reappears.
    Ok(true)
}

fn cancelled_reset_matches_request(
    data_dir: &Path,
    request: &DatasetResetRequest,
) -> Result<bool, AppError> {
    if request.version != RESET_REQUEST_VERSION
        || request.origin
            != Some(DatasetResetRequestOrigin::WorkDirChange)
    {
        return Ok(false);
    }
    let retired_root = data_dir.join(RETIRED_DATASETS_DIR);
    let archive_dir =
        retired_root.join(CANCELLED_WORK_DIR_CHANGE_REQUESTS_DIR);
    for (path, description) in [
        (&retired_root, "retired-datasets directory"),
        (&archive_dir, "cancelled work-dir request archive"),
    ] {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                validate_root_metadata(
                    path,
                    &metadata,
                    ManagedRootKind::Directory,
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "inspect {description} {}: {error}",
                    path.display()
                )));
            }
        }
    }
    let archived_path =
        archive_dir.join(format!("{}.json", request.operation_id));
    let archived_bytes =
        match read_bounded_regular_file(
            &archived_path,
            MAX_CONTROL_FILE_BYTES,
        ) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "read cancelled work-dir request proof {}: {error}",
                    archived_path.display()
                )));
            }
        };
    let archived: DatasetResetRequest =
        serde_json::from_slice(&archived_bytes).map_err(|error| {
            AppError::Internal(format!(
                "invalid cancelled work-dir request proof {}: {error}",
                archived_path.display()
            ))
        })?;
    archived.validate()?;
    if archived.version != request.version
        || archived.operation_id != request.operation_id
        || archived.requested_at != request.requested_at
        || archived.origin != request.origin
        || archived.work_dir != request.work_dir
    {
        return Err(AppError::Conflict(
            "replayed work-dir request collides with a different cancelled operation"
                .into(),
        ));
    }
    let active_bytes = read_bounded_regular_file(
        &request_path(data_dir),
        MAX_CONTROL_FILE_BYTES,
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "re-read active work-dir request while checking cancellation proof: {error}"
        ))
    })?;
    if active_bytes != archived_bytes {
        return Err(AppError::Conflict(
            "replayed work-dir request bytes differ from its cancellation proof"
                .into(),
        ));
    }
    Ok(true)
}

fn clear_v3_dataset_reset_request(data_dir: &Path) -> Result<(), AppError> {
    let path = request_path(data_dir);
    match fs::remove_file(&path) {
        Ok(()) => sync_parent(&path).map_err(|error| {
            AppError::Internal(format!(
                "sync v3 dataset reset request removal {}: {error}",
                path.display()
            ))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Internal(format!(
            "remove v3 dataset reset request {}: {error}",
            path.display()
        ))),
    }
}

fn archive_ignored_legacy_reset_request(
    data_dir: &Path,
    request: &DatasetResetRequest,
) -> Result<(), AppError> {
    if request.version != LEGACY_RESET_REQUEST_VERSION {
        return Err(AppError::Internal(
            "only a legacy reset request may be archived as unproven".into(),
        ));
    }
    archive_reset_request(
        data_dir,
        request,
        IGNORED_LEGACY_RESET_REQUESTS_DIR,
    )
}

fn archive_cancelled_work_dir_change_request(
    data_dir: &Path,
    request: &DatasetResetRequest,
) -> Result<(), AppError> {
    if request.origin != Some(DatasetResetRequestOrigin::WorkDirChange) {
        return Err(AppError::Internal(
            "only a work-dir change request may be archived as cancelled"
                .into(),
        ));
    }
    archive_reset_request(
        data_dir,
        request,
        CANCELLED_WORK_DIR_CHANGE_REQUESTS_DIR,
    )
}

fn archive_reset_request(
    data_dir: &Path,
    request: &DatasetResetRequest,
    archive_directory_name: &str,
) -> Result<(), AppError> {
    let retired_root = data_dir.join(RETIRED_DATASETS_DIR);
    ensure_real_directory(&retired_root, "retired-datasets directory")?;
    sync_parent(&retired_root).map_err(|error| {
        AppError::Internal(format!(
            "sync retired-datasets directory publication: {error}"
        ))
    })?;
    let archive_dir = retired_root.join(archive_directory_name);
    ensure_real_directory(
        &archive_dir,
        "ignored legacy reset request directory",
    )?;
    sync_parent(&archive_dir).map_err(|error| {
        AppError::Internal(format!(
            "sync reset request archive directory publication: {error}"
        ))
    })?;
    let source = request_path(data_dir);
    let destination =
        archive_dir.join(format!("{}.json", request.operation_id));
    let source_bytes =
        read_bounded_regular_file(&source, MAX_CONTROL_FILE_BYTES)
            .map_err(|read_error| {
                AppError::Internal(format!(
                    "read legacy reset request before archival: {read_error}"
                ))
            })?;
    let current: DatasetResetRequest =
        serde_json::from_slice(&source_bytes).map_err(|error| {
            AppError::Internal(format!(
                "invalid active reset request before archival: {error}"
            ))
        })?;
    current.validate()?;
    if current.version != request.version
        || current.operation_id != request.operation_id
        || current.requested_at != request.requested_at
        || current.origin != request.origin
        || current.work_dir != request.work_dir
    {
        return Err(AppError::Conflict(
            "active reset request changed before it could be archived"
                .into(),
        ));
    }
    match write_atomic_new(&destination, &source_bytes) {
        Ok(()) => {}
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            let destination_bytes = read_bounded_regular_file(
                &destination,
                MAX_CONTROL_FILE_BYTES,
            )
            .map_err(|read_error| {
                AppError::Internal(format!(
                    "read archived legacy reset request during retry: {read_error}"
                ))
            })?;
            if source_bytes != destination_bytes {
                return Err(AppError::Internal(
                    "archived legacy reset request collision".into(),
                ));
            }
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "atomically archive unproven legacy reset request {} -> {}: {error}",
                source.display(),
                destination.display()
            )));
        }
    }
    fs::remove_file(&source).map_err(|error| {
        AppError::Internal(format!(
            "remove active legacy reset request after archival: {error}"
        ))
    })?;
    sync_parent(&source).map_err(|error| {
        AppError::Internal(format!(
            "sync legacy reset request removal: {error}"
        ))
    })?;
    sync_parent(&destination).map_err(|error| {
        AppError::Internal(format!(
            "sync archived legacy reset request: {error}"
        ))
    })
}

#[cfg(target_os = "macos")]
fn rename_noreplace(
    source: &Path,
    destination: &Path,
) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "source path contains a NUL byte",
            )
        },
    )?;
    let destination =
        CString::new(destination.as_os_str().as_bytes()).map_err(
            |_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "destination path contains a NUL byte",
                )
            },
        )?;
    if unsafe {
        libc::renamex_np(
            source.as_ptr(),
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn rename_noreplace(
    source: &Path,
    destination: &Path,
) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "source path contains a NUL byte",
            )
        },
    )?;
    let destination =
        CString::new(destination.as_os_str().as_bytes()).map_err(
            |_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "destination path contains a NUL byte",
                )
            },
        )?;
    if unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn rename_noreplace(
    source: &Path,
    destination: &Path,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } != 0
    {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(80 | 183)) {
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            error,
        ))
    } else {
        Err(error)
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    windows
)))]
fn rename_noreplace(
    _source: &Path,
    _destination: &Path,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "this platform has no proven atomic no-replace directory rename",
    ))
}

fn rename_with_retry(source: &Path, destination: &Path) -> std::io::Result<()> {
    const MAX_ATTEMPTS: u32 = 5;
    for attempt in 1..=MAX_ATTEMPTS {
        match rename_noreplace(source, destination) {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt < MAX_ATTEMPTS
                    && matches!(error.raw_os_error(), Some(5) | Some(32) | Some(33)) =>
            {
                std::thread::sleep(Duration::from_millis(80 * u64::from(attempt)));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("rename retry loop returns on every iteration")
}

// ── Data-root relocation rebinding ──────────────────────────────────
//
// The default on-disk layout moved from `NomiFun/Nomi<suffix>` to the sibling
// `NomiFun<suffix>` root. After `bootstrap::data_root` physically moves the
// dataset, the durable lifecycle markers still carry absolute path STRINGS
// naming the old root: the receipt/bootstrap `work_root`, the data-side
// binding `data_root`/`work_root`, the work-root owner `data_root` (in the
// data root itself for the default internal work root, or in an external
// work root), and the dir-config `work_dir`. Left stale, every one of them
// fails the fail-closed identity checks above and bricks the boot.

/// Rewrite every durable lifecycle marker that still names `old_data_root`
/// after the dataset physically moved into `new_data_dir`.
///
/// Values naming other locations (an external work root, for example) are
/// left untouched — except that an external work root's *owner marker* has
/// its `data_root` re-pointed at the new root, because that marker is what
/// lets the moved dataset keep its claim on the external workspace.
///
/// Idempotent: values already naming the new root are skipped, so a crashed
/// migration may safely re-run this. Returns human-readable warnings for
/// conditions that were tolerated (for boot notes); hard I/O or parse
/// failures on an existing marker are errors, because booting with a
/// half-rebound marker set would fail closed anyway.
pub fn rebind_data_root_after_relocation(
    new_data_dir: &Path,
    old_data_root: &Path,
) -> Result<Vec<String>, AppError> {
    let canonical_new = canonical_data_dir(new_data_dir)?;
    let new_string = crate::paths::marker_string(&canonical_new);
    let old_matches = |stored: &str| {
        !stored.is_empty()
            && crate::paths::paths_equivalent(Path::new(stored), old_data_root)
    };
    let mut warnings = Vec::new();
    let mut external_work_roots: Vec<PathBuf> = Vec::new();
    let note_external = |stored: &str, external: &mut Vec<PathBuf>| {
        let path = PathBuf::from(stored);
        if !stored.is_empty()
            && !crate::paths::paths_equivalent(&path, old_data_root)
            && !crate::paths::paths_equivalent(&path, &canonical_new)
            && !external
                .iter()
                .any(|known| crate::paths::paths_equivalent(known, &path))
        {
            external.push(path);
        }
    };

    // dataset-v3.json (finalized receipt).
    let receipt_path = receipt_path(&canonical_new);
    match read_bounded_regular_file(&receipt_path, MAX_CONTROL_FILE_BYTES) {
        Ok(bytes) => {
            let mut receipt: DatasetReceipt = serde_json::from_slice(&bytes)
                .map_err(|error| {
                    AppError::Internal(format!(
                        "invalid v3 receipt during relocation rebind: {error}"
                    ))
                })?;
            note_external(&receipt.work_root, &mut external_work_roots);
            if old_matches(&receipt.work_root) {
                receipt.work_root = new_string.clone();
                let bytes = serde_json::to_vec_pretty(&receipt).map_err(|error| {
                    AppError::Internal(format!(
                        "serialize rebound v3 receipt: {error}"
                    ))
                })?;
                write_atomic(&receipt_path, &bytes).map_err(|error| {
                    AppError::Internal(format!(
                        "rebind v3 receipt {}: {error}",
                        receipt_path.display()
                    ))
                })?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read v3 receipt during relocation rebind {}: {error}",
                receipt_path.display()
            )));
        }
    }

    // .dataset-v3.bootstrap.json (unfinished bootstrap binding).
    let bootstrap_path = bootstrap_binding_path(&canonical_new);
    match read_bounded_regular_file(&bootstrap_path, MAX_CONTROL_FILE_BYTES) {
        Ok(bytes) => {
            let mut binding: DatasetBootstrapBinding =
                serde_json::from_slice(&bytes).map_err(|error| {
                    AppError::Internal(format!(
                        "invalid v3 bootstrap binding during relocation rebind: {error}"
                    ))
                })?;
            note_external(&binding.work_root, &mut external_work_roots);
            if old_matches(&binding.work_root) {
                binding.work_root = new_string.clone();
                let bytes =
                    serde_json::to_vec_pretty(&binding).map_err(|error| {
                        AppError::Internal(format!(
                            "serialize rebound v3 bootstrap binding: {error}"
                        ))
                    })?;
                write_atomic(&bootstrap_path, &bytes).map_err(|error| {
                    AppError::Internal(format!(
                        "rebind v3 bootstrap binding {}: {error}",
                        bootstrap_path.display()
                    ))
                })?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read v3 bootstrap binding during relocation rebind {}: {error}",
                bootstrap_path.display()
            )));
        }
    }

    // .nomifun-work-root-binding.json (data-side binding).
    if let Some(mut binding) = read_work_root_binding(&canonical_new)? {
        note_external(&binding.work_root, &mut external_work_roots);
        let mut changed = false;
        if old_matches(&binding.data_root) {
            binding.data_root = new_string.clone();
            changed = true;
        }
        if old_matches(&binding.work_root) {
            binding.work_root = new_string.clone();
            changed = true;
        }
        if changed {
            let path = work_root_binding_path(&canonical_new);
            let bytes = serde_json::to_vec_pretty(&binding).map_err(|error| {
                AppError::Internal(format!(
                    "serialize rebound work-root binding: {error}"
                ))
            })?;
            write_atomic(&path, &bytes).map_err(|error| {
                AppError::Internal(format!(
                    "rebind work-root binding {}: {error}",
                    path.display()
                ))
            })?;
        }
    }

    // dir-config.json (persisted UI work-dir choice).
    match crate::dir_config::checked_persisted_work_dir(&canonical_new) {
        Ok(Some(work_dir)) => {
            note_external(
                &work_dir.display().to_string(),
                &mut external_work_roots,
            );
            if crate::paths::paths_equivalent(&work_dir, old_data_root) {
                crate::dir_config::set_work_dir(&canonical_new, &canonical_new)?;
            }
        }
        Ok(None) => {}
        Err(error) => {
            // A malformed dir-config keeps its own dedicated repair path
            // (lifecycle-proof repair); relocation must not overwrite it.
            warnings.push(format!(
                "relocation left an unreadable dir-config for the normal repair path: {error}"
            ));
        }
    }

    // Work-root owner markers. The internal owner moved together with the
    // dataset; an external work root keeps its own marker in place.
    let mut owner_roots = vec![canonical_new.clone()];
    owner_roots.extend(external_work_roots);
    for work_root in owner_roots {
        let canonical_work = if crate::paths::paths_equivalent(
            &work_root,
            &canonical_new,
        ) {
            canonical_new.clone()
        } else {
            match canonical_existing_work_dir(&work_root) {
                Ok(canonical) => canonical,
                Err(error) => {
                    warnings.push(format!(
                        "external work root {} was not reachable during relocation rebind: {error}",
                        work_root.display()
                    ));
                    continue;
                }
            }
        };
        let Some(mut owner) = read_work_root_owner(&canonical_work)? else {
            continue;
        };
        if !old_matches(&owner.data_root) {
            continue;
        }
        owner.data_root = new_string.clone();
        let path = work_root_owner_path(&canonical_work);
        let bytes = serde_json::to_vec_pretty(&owner).map_err(|error| {
            AppError::Internal(format!(
                "serialize rebound work-root owner: {error}"
            ))
        })?;
        write_atomic(&path, &bytes).map_err(|error| {
            AppError::Internal(format!(
                "rebind work-root owner {}: {error}",
                path.display()
            ))
        })?;
    }

    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset_roots::AGENT_PROCESS_REGISTRY_FILE;

    fn touch(path: &Path) {
        fs::write(path, b"x").unwrap();
    }

    fn seed_managed_root(data_dir: &Path, relative_path: &str, kind: ManagedRootKind) {
        let path = data_dir.join(relative_path);
        match kind {
            ManagedRootKind::File => {
                fs::create_dir_all(path.parent().expect("managed file parent")).unwrap();
                touch(&path);
            }
            ManagedRootKind::Directory => {
                fs::create_dir_all(&path).unwrap();
                touch(&path.join("sentinel"));
            }
        }
    }

    fn snapshot_active_reset_control(
        data_dir: &Path,
    ) -> Vec<(String, Vec<u8>)> {
        let mut files = fs::read_dir(reset_dir(data_dir))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }

    fn restore_active_reset_control(
        data_dir: &Path,
        files: &[(String, Vec<u8>)],
    ) {
        let directory = reset_dir(data_dir);
        fs::create_dir(&directory).unwrap();
        for (name, bytes) in files {
            fs::write(directory.join(name), bytes).unwrap();
        }
    }

    fn rewrite_as_legacy_v1_plan(
        data_dir: &Path,
        mut plan: DatasetResetPlan,
    ) -> DatasetResetPlan {
        plan.version = LEGACY_PLAN_VERSION;
        plan.persist_work_dir = false;
        plan.automatic_legacy_retirement = false;
        plan.roots.retain(|root| {
            root.relative_path != WORK_ROOT_BINDING_FILE
                && root.relative_path != AGENT_PROCESS_REGISTRY_FILE
        });
        let dir_config_index = lifecycle_managed_roots().count()
            + managed_dataset_roots()
                .position(|root| root.path == crate::dir_config::DIR_CONFIG_FILE)
                .expect("dir-config is registered");
        plan.roots.insert(
            dir_config_index,
            ManagedRootPlan {
                base: ManagedRootBase::DataDir,
                relative_path: crate::dir_config::DIR_CONFIG_FILE.into(),
                retired_relative_path: format!(
                    "{}/{}",
                    plan.retired_dir,
                    crate::dir_config::DIR_CONFIG_FILE
                ),
                kind: ManagedRootKind::File,
                initially_present: fs::symlink_metadata(
                    data_dir.join(crate::dir_config::DIR_CONFIG_FILE),
                )
                .is_ok(),
            },
        );
        let mut released_shape = serde_json::to_value(&plan).unwrap();
        let object = released_shape.as_object_mut().unwrap();
        object.remove("persist_work_dir");
        object.remove("automatic_legacy_retirement");
        let bytes = serde_json::to_vec_pretty(&released_shape).unwrap();
        write_atomic(&plan_path(data_dir), &bytes).unwrap();
        plan
    }

    fn simulate_legacy_v1_quarantine(
        data_dir: &Path,
        work_dir: &Path,
        plan: &DatasetResetPlan,
        install_new_generation: bool,
    ) {
        let canonical_work = fs::canonicalize(work_dir).unwrap();
        for root in &plan.roots {
            if !root.initially_present {
                continue;
            }
            let base = match root.base {
                ManagedRootBase::DataDir => data_dir,
                ManagedRootBase::WorkDir => canonical_work.as_path(),
            };
            let source = base.join(&root.relative_path);
            let destination = base.join(&root.retired_relative_path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::rename(source, destination).unwrap();
        }
        write_phase(data_dir, "quarantine-started").unwrap();
        write_phase(data_dir, "quarantined").unwrap();
        if install_new_generation {
            install_generation(data_dir, &plan.generation).unwrap();
            write_phase(data_dir, "generation-installed").unwrap();
        }
    }

    #[test]
    fn empty_root_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            prepare_v3_dataset(dir.path(), dir.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
    }

    #[test]
    fn interrupted_fresh_bootstrap_control_writes_are_retryable() {
        let data = tempfile::tempdir().unwrap();
        let generation = Uuid::now_v7().to_string();
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            generation.as_bytes(),
        )
        .unwrap();

        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged,
            "a crash immediately after storage-generation must be retryable"
        );

        ensure_v3_work_root_owner(
            data.path(),
            data.path(),
            &generation,
        )
        .unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged,
            "a crash after the matching owner but before bootstrap binding must be retryable"
        );
        assert!(!reset_dir(data.path()).exists());
    }

    #[test]
    fn malformed_interrupted_bootstrap_generation_fails_closed() {
        let data = tempfile::tempdir().unwrap();
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            b"not-a-generation",
        )
        .unwrap();

        let error =
            prepare_v3_dataset(data.path(), data.path()).unwrap_err();

        assert!(error.to_string().contains("storage generation is invalid"));
        assert!(!reset_dir(data.path()).exists());
    }

    #[test]
    fn explicit_reset_quarantines_managed_roots_and_keeps_logs() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join(DB_FILE));
        touch(&dir.path().join("storage-generation"));
        fs::create_dir_all(dir.path().join("conversations")).unwrap();
        touch(&dir.path().join("conversations").join("old.txt"));
        fs::create_dir_all(dir.path().join("logs")).unwrap();
        touch(&dir.path().join("logs").join("app.log"));
        request_v3_dataset_reset(dir.path(), dir.path()).unwrap();

        assert_eq!(
            prepare_v3_dataset(dir.path(), dir.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        assert!(!dir.path().join(DB_FILE).exists());
        assert!(!dir.path().join("conversations").exists());
        assert!(dir.path().join("logs/app.log").exists());
        assert!(!dir.path().join(V3_DATASET_RESET_REQUEST_FILE).exists());
        assert!(dir.path().join(V3_DATASET_RESET_DIR).exists());
        assert!(dir.path().join(RETIRED_DATASETS_DIR).is_dir());
    }

    #[test]
    fn completed_reset_request_replay_never_resets_the_new_v3_dataset() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        fs::create_dir_all(data.path().join(MANAGED_WORKSPACES_DIR))
            .unwrap();
        touch(
            &data
                .path()
                .join(MANAGED_WORKSPACES_DIR)
                .join("old-history"),
        );
        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        let request_bytes =
            fs::read(request_path(data.path())).unwrap();
        let request: DatasetResetRequest =
            serde_json::from_slice(&request_bytes).unwrap();

        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let plan = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .expect("reset plan remains until fresh v3 finalization");
        assert_eq!(
            plan.operation_id, request.operation_id,
            "the immutable plan must inherit the request identity"
        );
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt(data.path(), &plan.generation).unwrap();
        assert!(finalize_v3_dataset_reset(data.path(), data.path()).unwrap());

        let current_conversations =
            data.path().join(MANAGED_WORKSPACES_DIR);
        fs::create_dir_all(&current_conversations).unwrap();
        let current_sentinel =
            current_conversations.join("new-v3-data");
        touch(&current_sentinel);
        let current_generation =
            fs::read(data.path().join(STORAGE_GENERATION_FILE)).unwrap();

        // Simulate a filesystem/power-loss rollback that resurrects the
        // already-consumed request directory entry.
        write_atomic(&request_path(data.path()), &request_bytes).unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged
        );

        assert!(current_sentinel.is_file());
        assert!(data.path().join(DB_FILE).is_file());
        assert_eq!(
            fs::read(data.path().join(STORAGE_GENERATION_FILE)).unwrap(),
            current_generation
        );
        assert!(!reset_dir(data.path()).exists());
        assert!(!request_path(data.path()).exists());
        assert!(
            data.path()
                .join(RETIRED_DATASETS_DIR)
                .join(REPLAYED_COMPLETED_RESET_REQUESTS_DIR)
                .join(format!("{}.json", request.operation_id))
                .is_file()
        );
    }

    #[test]
    fn completed_request_stays_consumed_after_a_later_generation() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));

        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        let first_request_bytes =
            fs::read(request_path(data.path())).unwrap();
        let first_request: DatasetResetRequest =
            serde_json::from_slice(&first_request_bytes).unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let first_plan =
            read_pending_v3_reset(data.path(), data.path())
                .unwrap()
                .unwrap();
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt(
            data.path(),
            &first_plan.generation,
        )
        .unwrap();
        finalize_v3_dataset_reset(data.path(), data.path()).unwrap();

        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let second_plan =
            read_pending_v3_reset(data.path(), data.path())
                .unwrap()
                .unwrap();
        assert_ne!(second_plan.generation, first_plan.generation);
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt(
            data.path(),
            &second_plan.generation,
        )
        .unwrap();
        finalize_v3_dataset_reset(data.path(), data.path()).unwrap();
        fs::create_dir_all(
            data.path().join(MANAGED_WORKSPACES_DIR),
        )
        .unwrap();
        let second_generation_sentinel = data
            .path()
            .join(MANAGED_WORKSPACES_DIR)
            .join("second-generation-data");
        touch(&second_generation_sentinel);
        let current_generation =
            fs::read(data.path().join(STORAGE_GENERATION_FILE)).unwrap();

        write_atomic(
            &request_path(data.path()),
            &first_request_bytes,
        )
        .unwrap();
        assert!(
            requested_v3_reset_work_dir(data.path())
                .unwrap()
                .is_none(),
            "a consumed request must not redirect startup to an older work root"
        );
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert!(second_generation_sentinel.is_file());
        assert_eq!(
            fs::read(data.path().join(STORAGE_GENERATION_FILE)).unwrap(),
            current_generation
        );
        assert!(!reset_dir(data.path()).exists());
        assert!(
            data.path()
                .join(RETIRED_DATASETS_DIR)
                .join(REPLAYED_COMPLETED_RESET_REQUESTS_DIR)
                .join(format!("{}.json", first_request.operation_id))
                .is_file()
        );
    }

    #[test]
    fn completed_reset_control_replay_never_reapplies_its_plan() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let plan = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .unwrap();
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt(data.path(), &plan.generation).unwrap();
        let replay = snapshot_active_reset_control(data.path());
        finalize_v3_dataset_reset(data.path(), data.path()).unwrap();

        fs::create_dir_all(data.path().join(MANAGED_WORKSPACES_DIR))
            .unwrap();
        let sentinel = data
            .path()
            .join(MANAGED_WORKSPACES_DIR)
            .join("current-v3-data");
        touch(&sentinel);
        let generation =
            fs::read(data.path().join(STORAGE_GENERATION_FILE)).unwrap();
        restore_active_reset_control(data.path(), &replay);

        assert!(
            pending_v3_reset_work_dir(data.path()).unwrap().is_none(),
            "a completed control replay must not redirect work-root resolution"
        );
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert!(sentinel.is_file());
        assert!(data.path().join(DB_FILE).is_file());
        assert_eq!(
            fs::read(data.path().join(STORAGE_GENERATION_FILE)).unwrap(),
            generation
        );
        assert!(!reset_dir(data.path()).exists());
        assert!(
            fs::read_dir(data.path().join(&plan.retired_dir))
                .unwrap()
                .any(|entry| {
                    entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(
                            REPLAYED_COMPLETED_RESET_CONTROL_PREFIX,
                        )
                })
        );
    }

    #[test]
    fn old_completed_control_replay_preserves_a_later_generation() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        prepare_v3_dataset(data.path(), data.path()).unwrap();
        let first_plan =
            read_pending_v3_reset(data.path(), data.path())
                .unwrap()
                .unwrap();
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt(
            data.path(),
            &first_plan.generation,
        )
        .unwrap();
        let first_control = snapshot_active_reset_control(data.path());
        finalize_v3_dataset_reset(data.path(), data.path()).unwrap();

        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        prepare_v3_dataset(data.path(), data.path()).unwrap();
        let second_plan =
            read_pending_v3_reset(data.path(), data.path())
                .unwrap()
                .unwrap();
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt(
            data.path(),
            &second_plan.generation,
        )
        .unwrap();
        finalize_v3_dataset_reset(data.path(), data.path()).unwrap();
        fs::create_dir_all(data.path().join(MANAGED_WORKSPACES_DIR))
            .unwrap();
        let sentinel = data
            .path()
            .join(MANAGED_WORKSPACES_DIR)
            .join("later-generation-data");
        touch(&sentinel);
        let generation =
            fs::read(data.path().join(STORAGE_GENERATION_FILE)).unwrap();

        restore_active_reset_control(data.path(), &first_control);
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert!(sentinel.is_file());
        assert_eq!(
            fs::read(data.path().join(STORAGE_GENERATION_FILE)).unwrap(),
            generation
        );
        assert_eq!(
            inspect_v3_dataset_receipt(data.path(), data.path()).unwrap(),
            DatasetReceiptStatus::Current
        );
    }

    #[test]
    fn automatic_legacy_retirement_is_consumed_once_per_installation() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        assert_eq!(
            retire_non_v3_dataset_after_probe(
                data.path(),
                data.path(),
            )
            .unwrap(),
            DatasetPreparation::ResetApplied
        );
        let plan = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .unwrap();
        assert!(plan.automatic_legacy_retirement);
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt(data.path(), &plan.generation).unwrap();
        finalize_v3_dataset_reset(data.path(), data.path()).unwrap();
        assert!(
            automatic_legacy_retirement_path(data.path()).is_file()
        );

        for relative in [
            DB_FILE,
            STORAGE_GENERATION_FILE,
            V3_DATASET_RECEIPT_FILE,
            WORK_ROOT_OWNER_FILE,
            WORK_ROOT_BINDING_FILE,
        ] {
            match fs::remove_file(data.path().join(relative)) {
                Ok(()) => {}
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("remove simulated rolled-back root: {error}"),
            }
        }
        fs::write(
            data.path().join(DB_FILE),
            b"exact-legacy-database-sentinel",
        )
        .unwrap();

        let error = retire_non_v3_dataset_after_probe(
            data.path(),
            data.path(),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("already consumed its one automatic")
        );
        assert_eq!(
            fs::read(data.path().join(DB_FILE)).unwrap(),
            b"exact-legacy-database-sentinel"
        );
        assert!(!reset_dir(data.path()).exists());

        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied,
            "a new explicit user request remains available after the one automatic retirement"
        );
        assert!(!data.path().join(DB_FILE).exists());
    }

    #[test]
    fn completed_request_replay_does_not_block_a_newer_pending_plan() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        let completed_request =
            fs::read(request_path(data.path())).unwrap();
        prepare_v3_dataset(data.path(), data.path()).unwrap();
        let first_plan =
            read_pending_v3_reset(data.path(), data.path())
                .unwrap()
                .unwrap();
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt(
            data.path(),
            &first_plan.generation,
        )
        .unwrap();
        finalize_v3_dataset_reset(data.path(), data.path()).unwrap();

        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        let second_plan = arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::ExplicitFactoryReset,
        )
        .unwrap();
        fs::write(request_path(data.path()), &completed_request).unwrap();

        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let resumed = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .unwrap();
        assert_eq!(resumed.operation_id, second_plan.operation_id);
        assert_eq!(resumed.generation, second_plan.generation);
        assert!(!data.path().join(DB_FILE).exists());
    }

    #[test]
    fn v2_reset_preserves_work_dir_control_config() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        crate::dir_config::set_work_dir(data.path(), work.path()).unwrap();
        touch(&data.path().join(DB_FILE));
        request_v3_dataset_reset(data.path(), work.path()).unwrap();

        assert_eq!(
            prepare_v3_dataset(data.path(), work.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let plan = read_pending_v3_reset(data.path(), work.path())
            .unwrap()
            .unwrap();
        assert_eq!(plan.version, PLAN_VERSION);
        assert!(
            plan.roots.iter().all(|root| {
                root.relative_path != crate::dir_config::DIR_CONFIG_FILE
            })
        );
        assert_eq!(
            fs::canonicalize(
                crate::dir_config::persisted_work_dir(data.path())
                    .unwrap(),
            )
            .unwrap(),
            fs::canonicalize(work.path()).unwrap()
        );
    }

    #[test]
    fn finalized_v1_reset_repairs_only_the_retired_work_dir_config() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let canonical_work =
            crate::paths::canonicalize_simplified(work.path()).unwrap();
        crate::dir_config::set_work_dir(data.path(), work.path()).unwrap();
        touch(&data.path().join(DB_FILE));
        let plan = arm_v3_dataset_reset(
            data.path(),
            work.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        let plan = rewrite_as_legacy_v1_plan(data.path(), plan);

        assert_eq!(
            pending_v3_reset_work_dir(data.path())
                .unwrap()
                .as_deref(),
            Some(canonical_work.as_path())
        );
        simulate_legacy_v1_quarantine(
            data.path(),
            work.path(),
            &plan,
            true,
        );
        apply_pending_v3_dataset_reset(data.path(), work.path()).unwrap();
        assert!(
            crate::dir_config::persisted_work_dir(data.path()).is_none()
        );
        assert!(
            retired_dir_config_path(data.path(), &plan.generation).is_file()
        );

        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            work.path(),
            &plan.generation,
        )
        .unwrap();
        finalize_v3_dataset_reset(data.path(), work.path()).unwrap();

        assert_eq!(
            repair_finalized_legacy_v1_work_dir(data.path())
                .unwrap()
                .as_deref(),
            Some(canonical_work.as_path())
        );
        assert_eq!(
            crate::dir_config::persisted_work_dir(data.path()).as_deref(),
            Some(canonical_work.as_path())
        );
        fs::remove_file(retired_dir_config_path(
            data.path(),
            &plan.generation,
        ))
        .unwrap();
        assert_eq!(
            repair_finalized_legacy_v1_work_dir(data.path()).unwrap(),
            None,
            "the repaired active config makes recovery one-shot"
        );
        assert_eq!(
            inspect_v3_dataset_receipt(data.path(), work.path()).unwrap(),
            DatasetReceiptStatus::Current
        );
        assert!(!reset_dir(data.path()).exists());
    }

    #[test]
    fn pending_v1_plan_rejects_a_recreated_config_for_another_work_root() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let other_work = tempfile::tempdir().unwrap();
        crate::dir_config::set_work_dir(data.path(), work.path()).unwrap();
        touch(&data.path().join(DB_FILE));
        let plan = arm_v3_dataset_reset(
            data.path(),
            work.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        let plan = rewrite_as_legacy_v1_plan(data.path(), plan);
        simulate_legacy_v1_quarantine(
            data.path(),
            work.path(),
            &plan,
            true,
        );
        apply_pending_v3_dataset_reset(data.path(), work.path()).unwrap();

        crate::dir_config::set_work_dir(data.path(), work.path()).unwrap();
        assert!(
            apply_pending_v3_dataset_reset(data.path(), work.path()).unwrap()
        );

        crate::dir_config::set_work_dir(data.path(), other_work.path()).unwrap();
        let error =
            apply_pending_v3_dataset_reset(data.path(), work.path())
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("do not both match the reset plan")
        );
        assert!(reset_dir(data.path()).is_dir());
    }

    #[test]
    fn work_dir_change_creates_one_fresh_generation_and_preserves_old_root() {
        let data = tempfile::tempdir().unwrap();
        let old_work = tempfile::tempdir().unwrap();
        let new_work = tempfile::tempdir().unwrap();
        let canonical_new_work =
            crate::paths::canonicalize_simplified(new_work.path()).unwrap();
        crate::dir_config::set_work_dir(data.path(), old_work.path()).unwrap();
        touch(&data.path().join(DB_FILE));
        let old_generation = Uuid::now_v7().to_string();
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            &old_generation,
        )
        .unwrap();
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            old_work.path(),
            &old_generation,
        )
        .unwrap();
        fs::create_dir_all(old_work.path().join(MANAGED_WORKSPACES_DIR))
            .unwrap();
        touch(
            &old_work
                .path()
                .join(MANAGED_WORKSPACES_DIR)
                .join("old-root.txt"),
        );
        request_v3_dataset_reset_for_work_dir(data.path(), new_work.path())
            .unwrap();
        assert_eq!(
            requested_v3_reset_work_dir(data.path())
                .unwrap()
                .as_deref(),
            Some(canonical_new_work.as_path())
        );
        assert_eq!(
            prepare_v3_dataset(data.path(), new_work.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        assert!(
            old_work
                .path()
                .join(MANAGED_WORKSPACES_DIR)
                .join("old-root.txt")
                .is_file(),
            "the detached historical work root is preserved, not migrated"
        );
        assert!(
            !new_work.path().join(MANAGED_WORKSPACES_DIR).exists(),
            "the target starts without a conversation tree"
        );
        assert_eq!(
            crate::dir_config::persisted_work_dir(data.path()).as_deref(),
            Some(canonical_new_work.as_path())
        );
        let plan = read_pending_v3_reset(data.path(), new_work.path())
            .unwrap()
            .unwrap();
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            new_work.path(),
            &plan.generation,
        )
        .unwrap();
        finalize_v3_dataset_reset(data.path(), new_work.path()).unwrap();
        let retired_count =
            fs::read_dir(data.path().join(RETIRED_DATASETS_DIR))
                .unwrap()
                .count();

        assert_eq!(
            prepare_v3_dataset(data.path(), new_work.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert_eq!(
            fs::read_dir(data.path().join(RETIRED_DATASETS_DIR))
                .unwrap()
                .count(),
            retired_count
        );
    }

    #[test]
    fn work_dir_change_finalization_requires_matching_durable_config() {
        let data = tempfile::tempdir().unwrap();
        let old_work = tempfile::tempdir().unwrap();
        let new_work = tempfile::tempdir().unwrap();
        crate::dir_config::set_work_dir(data.path(), old_work.path()).unwrap();
        touch(&data.path().join(DB_FILE));
        let plan = arm_v3_dataset_reset(
            data.path(),
            new_work.path(),
            DatasetResetReason::WorkDirChange,
        )
        .unwrap();
        assert!(apply_pending_v3_dataset_reset(
            data.path(),
            new_work.path()
        )
        .unwrap());
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            new_work.path(),
            &plan.generation,
        )
        .unwrap();

        let error =
            finalize_v3_dataset_reset(data.path(), new_work.path())
                .unwrap_err();
        assert!(error.to_string().contains("dir-config"));
        assert!(reset_dir(data.path()).is_dir());

        crate::dir_config::set_work_dir(
            data.path(),
            &fs::canonicalize(new_work.path()).unwrap(),
        )
        .unwrap();
        assert!(
            finalize_v3_dataset_reset(data.path(), new_work.path()).unwrap()
        );
        assert!(!reset_dir(data.path()).exists());
        assert!(
            data.path()
                .join(&plan.retired_dir)
                .join(COMPLETED_RESET_CONTROL_DIR)
                .join(V3_DATASET_RESET_PLAN_FILE)
                .is_file()
        );
    }

    #[test]
    fn work_dir_change_never_moves_an_unowned_target_conversations_tree() {
        let data = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let conversations = target.path().join(MANAGED_WORKSPACES_DIR);
        fs::create_dir_all(&conversations).unwrap();
        touch(&conversations.join("personal-file.txt"));

        let error =
            request_v3_dataset_reset_for_work_dir(data.path(), target.path())
                .unwrap_err();
        assert!(matches!(error, AppError::BadRequest(_)));
        assert!(conversations.join("personal-file.txt").is_file());
        assert!(!request_path(data.path()).exists());
        assert!(!reset_dir(data.path()).exists());
    }

    #[test]
    fn historical_nested_work_root_is_reset_once_without_migrating_old_conversations() {
        let data = tempfile::tempdir().unwrap();
        let work = data.path().join("chosen-workspace");
        fs::create_dir_all(work.join(MANAGED_WORKSPACES_DIR)).unwrap();
        touch(&work.join(MANAGED_WORKSPACES_DIR).join("legacy.txt"));
        crate::dir_config::set_work_dir(data.path(), &work).unwrap();
        touch(&data.path().join(DB_FILE));

        assert_eq!(
            retire_non_v3_dataset_after_probe(data.path(), &work).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let plan = read_pending_v3_reset(data.path(), &work)
            .unwrap()
            .expect("legacy retirement must leave a durable plan");
        let retired_conversation = work
            .join(&plan.work_retired_dir)
            .join(MANAGED_WORKSPACES_DIR)
            .join("legacy.txt");
        assert!(retired_conversation.is_file());
        assert!(!work.join(MANAGED_WORKSPACES_DIR).exists());
        let persisted_work =
            crate::dir_config::checked_persisted_work_dir(data.path())
                .unwrap()
                .expect("v2 reset must preserve dir-config");
        assert_eq!(
            fs::canonicalize(persisted_work).unwrap(),
            fs::canonicalize(&work).unwrap()
        );

        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            &work,
            &plan.generation,
        )
        .unwrap();
        assert!(finalize_v3_dataset_reset(data.path(), &work).unwrap());
        let data_retired_count =
            fs::read_dir(data.path().join(RETIRED_DATASETS_DIR))
                .unwrap()
                .count();
        let work_retired_count =
            fs::read_dir(work.join(WORK_RETIRED_DATASETS_DIR))
                .unwrap()
                .count();

        assert_eq!(
            prepare_v3_dataset(data.path(), &work).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert_eq!(
            fs::read_dir(data.path().join(RETIRED_DATASETS_DIR))
                .unwrap()
                .count(),
            data_retired_count
        );
        assert_eq!(
            fs::read_dir(work.join(WORK_RETIRED_DATASETS_DIR))
                .unwrap()
                .count(),
            work_retired_count
        );
        assert!(retired_conversation.is_file());
    }

    #[test]
    fn nested_work_root_inside_managed_data_root_fails_without_mutation() {
        let data = tempfile::tempdir().unwrap();
        let work = data.path().join("projects").join("chosen-workspace");
        fs::create_dir_all(&work).unwrap();
        touch(&data.path().join(DB_FILE));

        let error =
            retire_non_v3_dataset_after_probe(data.path(), &work).unwrap_err();
        assert!(
            error.to_string().contains("product-managed data root")
                || error.to_string().contains("must not contain one another")
                || error.to_string().contains("must be disjoint")
        );
        assert!(data.path().join(DB_FILE).is_file());
        assert!(!reset_dir(data.path()).exists());
        assert!(!data.path().join(RETIRED_DATASETS_DIR).exists());
    }

    #[test]
    fn explicit_reset_request_cannot_be_redirected_to_another_work_root() {
        let data = tempfile::tempdir().unwrap();
        let bound_work = tempfile::tempdir().unwrap();
        let other_work = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        fs::create_dir_all(other_work.path().join(MANAGED_WORKSPACES_DIR))
            .unwrap();
        touch(
            &other_work
                .path()
                .join(MANAGED_WORKSPACES_DIR)
                .join("unrelated.txt"),
        );
        request_v3_dataset_reset(data.path(), bound_work.path()).unwrap();

        let error =
            prepare_v3_dataset(data.path(), other_work.path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match the pending v3 reset request")
        );
        assert!(data.path().join(DB_FILE).is_file());
        assert!(
            other_work
                .path()
                .join(MANAGED_WORKSPACES_DIR)
                .join("unrelated.txt")
                .is_file()
        );
        assert!(request_path(data.path()).is_file());
        assert!(!reset_dir(data.path()).exists());
    }

    #[test]
    fn work_dir_change_is_cancelled_if_target_changes_before_boot() {
        let data = tempfile::tempdir().unwrap();
        let old_work = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let old_generation = Uuid::now_v7().to_string();
        touch(&data.path().join(DB_FILE));
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            &old_generation,
        )
        .unwrap();
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            old_work.path(),
            &old_generation,
        )
        .unwrap();
        crate::dir_config::set_work_dir(data.path(), old_work.path()).unwrap();
        request_v3_dataset_reset_for_work_dir(data.path(), target.path())
            .unwrap();
        fs::create_dir_all(target.path().join(MANAGED_WORKSPACES_DIR))
            .unwrap();
        touch(
            &target
                .path()
                .join(MANAGED_WORKSPACES_DIR)
                .join("appeared-later.txt"),
        );

        let error =
            prepare_v3_dataset(data.path(), target.path()).unwrap_err();
        assert!(error.to_string().contains("was cancelled"));
        assert!(data.path().join(DB_FILE).is_file());
        assert!(
            target
                .path()
                .join(MANAGED_WORKSPACES_DIR)
                .join("appeared-later.txt")
                .is_file()
        );
        let persisted_work =
            crate::dir_config::checked_persisted_work_dir(data.path())
                .unwrap()
                .expect("cancelled change must keep old dir-config");
        assert_eq!(
            fs::canonicalize(persisted_work).unwrap(),
            fs::canonicalize(old_work.path()).unwrap()
        );
        assert!(!request_path(data.path()).exists());
        assert!(!reset_dir(data.path()).exists());
    }

    #[test]
    fn cancelled_work_dir_request_replay_never_resets_current_v3() {
        let data = tempfile::tempdir().unwrap();
        let current_work = tempfile::tempdir().unwrap();
        let cancelled_target = tempfile::tempdir().unwrap();
        let generation = Uuid::now_v7().to_string();
        touch(&data.path().join(DB_FILE));
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            &generation,
        )
        .unwrap();
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            current_work.path(),
            &generation,
        )
        .unwrap();
        crate::dir_config::set_work_dir(
            data.path(),
            current_work.path(),
        )
        .unwrap();
        request_v3_dataset_reset_for_work_dir(
            data.path(),
            cancelled_target.path(),
        )
        .unwrap();
        let replay = fs::read(request_path(data.path())).unwrap();
        fs::create_dir_all(
            cancelled_target.path().join(MANAGED_WORKSPACES_DIR),
        )
        .unwrap();
        touch(
            &cancelled_target
                .path()
                .join(MANAGED_WORKSPACES_DIR)
                .join("foreign-data"),
        );
        assert!(
            prepare_v3_dataset(
                data.path(),
                cancelled_target.path(),
            )
            .unwrap_err()
            .to_string()
            .contains("was cancelled")
        );
        fs::remove_dir_all(
            cancelled_target.path().join(MANAGED_WORKSPACES_DIR),
        )
        .unwrap();
        fs::write(request_path(data.path()), &replay).unwrap();

        assert!(
            requested_v3_reset_work_dir(data.path())
                .unwrap()
                .is_none(),
            "a cancelled operation must not redirect a later boot"
        );
        assert_eq!(
            prepare_v3_dataset(data.path(), current_work.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert!(data.path().join(DB_FILE).is_file());
        assert_eq!(
            fs::read_to_string(
                data.path().join(STORAGE_GENERATION_FILE)
            )
            .unwrap(),
            generation
        );
        assert_eq!(
            inspect_v3_dataset_receipt(
                data.path(),
                current_work.path(),
            )
            .unwrap(),
            DatasetReceiptStatus::Current
        );
    }

    #[test]
    fn cancelled_request_replay_does_not_block_a_new_pending_plan() {
        let data = tempfile::tempdir().unwrap();
        let current_work = tempfile::tempdir().unwrap();
        let cancelled_target = tempfile::tempdir().unwrap();
        let new_target = tempfile::tempdir().unwrap();
        let generation = Uuid::now_v7().to_string();
        touch(&data.path().join(DB_FILE));
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            &generation,
        )
        .unwrap();
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            current_work.path(),
            &generation,
        )
        .unwrap();
        request_v3_dataset_reset_for_work_dir(
            data.path(),
            cancelled_target.path(),
        )
        .unwrap();
        let cancelled_replay =
            fs::read(request_path(data.path())).unwrap();
        fs::create_dir_all(
            cancelled_target.path().join(MANAGED_WORKSPACES_DIR),
        )
        .unwrap();
        assert!(
            prepare_v3_dataset(
                data.path(),
                cancelled_target.path(),
            )
            .is_err()
        );
        fs::remove_dir_all(
            cancelled_target.path().join(MANAGED_WORKSPACES_DIR),
        )
        .unwrap();

        request_v3_dataset_reset_for_work_dir(
            data.path(),
            new_target.path(),
        )
        .unwrap();
        let new_plan = arm_v3_dataset_reset(
            data.path(),
            new_target.path(),
            DatasetResetReason::WorkDirChange,
        )
        .unwrap();
        fs::write(request_path(data.path()), cancelled_replay).unwrap();

        assert_eq!(
            prepare_v3_dataset(data.path(), new_target.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let resumed =
            read_pending_v3_reset(data.path(), new_target.path())
                .unwrap()
                .unwrap();
        assert_eq!(resumed.operation_id, new_plan.operation_id);
        assert_eq!(resumed.generation, new_plan.generation);
        assert!(!data.path().join(DB_FILE).exists());
    }

    /// Managed dataset roots that did not exist when the v1 planner froze its
    /// registry shape. Every later *addition* must be listed here.
    const POST_V1_MANAGED_DATASET_ROOTS: &[&str] = &[
        WORK_ROOT_OWNER_FILE,
        WORK_ROOT_BINDING_FILE,
        AGENT_PROCESS_REGISTRY_FILE,
    ];

    /// The persisted plan shape is a compatibility surface, not an
    /// implementation detail: `RELEASED_V1_MANAGED_ROOTS` must stay
    /// reproducible from the live registry, and the v2 registry that
    /// v0.3.1..=v0.3.7 wrote into user data dirs is the live registry filtered
    /// by [`ResetPolicy::Retire`].
    ///
    /// So dropping a root from `MANAGED_DATASET_ROOTS` — even one whose
    /// subsystem was deleted — both stops factory reset from sweeping data left
    /// by older installations and makes previously written plan bytes fail
    /// `validate_plan` / `completed_plan_replay_mismatch`, which turns
    /// an interrupted reset on upgrade into a hard startup failure. Removals are
    /// therefore never compatible; keep the root and mark it cleanup-only.
    #[test]
    fn released_v1_managed_roots_stay_reproducible_from_the_live_registry() {
        let derived = lifecycle_managed_roots()
            .chain(managed_dataset_roots().filter_map(|root| {
                if POST_V1_MANAGED_DATASET_ROOTS.contains(&root.path) {
                    return None;
                }
                Some((
                    root.path,
                    match root.kind {
                        DatasetRootKind::File => ManagedRootKind::File,
                        DatasetRootKind::Directory => ManagedRootKind::Directory,
                    },
                ))
            }))
            .collect::<Vec<_>>();

        let derived_paths =
            derived.iter().map(|(path, _)| *path).collect::<Vec<_>>();
        let frozen_paths = RELEASED_V1_MANAGED_ROOTS
            .iter()
            .map(|(path, _)| *path)
            .collect::<Vec<_>>();
        let removed = frozen_paths
            .iter()
            .filter(|path| !derived_paths.contains(path))
            .collect::<Vec<_>>();
        let added = derived_paths
            .iter()
            .filter(|path| !frozen_paths.contains(path))
            .collect::<Vec<_>>();
        assert!(
            removed.is_empty(),
            "managed dataset roots disappeared from the live registry: {removed:?}. \
             Persisted v1/v2 reset plans still list them, so removal breaks plan \
             validation on upgrade and abandons data from older installations. \
             Keep the root and mark it cleanup-only instead."
        );
        assert!(
            added.is_empty(),
            "new managed dataset roots are not declared as post-v1 additions: \
             {added:?}. Adding a root changes the v2 plan shape that shipped in \
             v0.3.1..=v0.3.7, so record it in POST_V1_MANAGED_DATASET_ROOTS and \
             confirm the persisted-plan compatibility story."
        );
        assert_eq!(
            derived,
            RELEASED_V1_MANAGED_ROOTS.to_vec(),
            "the live managed-root registry drifted from the released v1 order \
             or kinds; persisted plans are compared element-by-element"
        );
    }

    /// The v2 plan shape must be a contract, not a snapshot of whatever this
    /// process computes. `RELEASED_V2_MANAGED_ROOTS` is what readers validate
    /// persisted plans against, and `current_writer_managed_roots` is what the
    /// planner writes; if they drift, every plan already on disk stops
    /// validating and an interrupted reset carried across the upgrade hard-fails
    /// at startup. That is the `browser-secrets` regression, generalized.
    ///
    /// If this fails, do not edit the frozen list. Either restore the live
    /// registry, or mint a plan version: add `RELEASED_V3_MANAGED_ROOTS`, add an
    /// arm to `released_plan_shape`, bump `PLAN_VERSION`, and give the new
    /// version its own `persists_work_dir` answer.
    #[test]
    fn released_v2_managed_roots_match_the_current_writer() {
        let written = current_writer_managed_roots();
        let frozen = RELEASED_V2_MANAGED_ROOTS.to_vec();
        let dropped = frozen
            .iter()
            .filter(|(path, _)| {
                !written.iter().any(|(other, _)| other == path)
            })
            .collect::<Vec<_>>();
        let gained = written
            .iter()
            .filter(|(path, _)| {
                !frozen.iter().any(|(other, _)| other == path)
            })
            .collect::<Vec<_>>();
        assert!(
            dropped.is_empty(),
            "the current writer no longer plans roots that the frozen v2 shape \
             lists: {dropped:?}. Plans written by released builds still list \
             them, and a removed root is also data the reset stops sweeping."
        );
        assert!(
            gained.is_empty(),
            "the current writer plans roots the frozen v2 shape does not list: \
             {gained:?}. Adding a root changes the persisted plan shape, so it \
             needs a new plan version rather than an edit to v2."
        );
        assert_eq!(
            written, frozen,
            "the current writer's managed-root order or kinds drifted from the \
             frozen v2 shape; persisted plans are compared element-by-element"
        );
        assert_eq!(
            released_plan_shape(PLAN_VERSION).unwrap().managed_roots,
            RELEASED_V2_MANAGED_ROOTS,
            "the current plan version must resolve to the frozen v2 shape"
        );
        assert!(
            released_plan_shape(PLAN_VERSION).unwrap().persists_work_dir,
            "v2 plans always persist their work root"
        );
        assert!(
            !released_plan_shape(LEGACY_PLAN_VERSION)
                .unwrap()
                .persists_work_dir,
            "v1 predates work-root persistence"
        );
        assert!(
            released_plan_shape(PLAN_VERSION + 1).is_err(),
            "an unknown plan version must be refused, never treated as current"
        );
    }

    /// Rewrite the persisted plan so it looks like one written by a build whose
    /// managed-root registry did not contain `dropped_root` — exactly the shape
    /// v0.3.8 wrote after `browser-secrets` was deleted from the registry.
    fn rewrite_plan_with_older_registry_shape(
        data_dir: &Path,
        plan: &DatasetResetPlan,
        dropped_root: &str,
    ) -> DatasetResetPlan {
        let mut older = plan.clone();
        let before = older.roots.len();
        older
            .roots
            .retain(|root| root.relative_path != dropped_root);
        assert_eq!(
            older.roots.len() + 1,
            before,
            "{dropped_root} must be a planned root for this fixture to mean \
             anything"
        );
        let bytes = serde_json::to_vec_pretty(&older).unwrap();
        write_atomic(&plan_path(data_dir), &bytes).unwrap();
        older
    }

    #[test]
    fn interrupted_reset_from_an_older_registry_shape_recovers_at_startup() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        seed_managed_root(data.path(), "knowledge", ManagedRootKind::Directory);
        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        let plan = arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::ExplicitFactoryReset,
        )
        .unwrap();
        let older = rewrite_plan_with_older_registry_shape(
            data.path(),
            &plan,
            "browser-secrets",
        );

        // Nothing here may raise: the plan is unexecutable, which is a fact
        // about the bytes, not a reason to abort a boot on a user's data dir.
        assert!(
            read_pending_v3_reset(data.path(), data.path())
                .unwrap()
                .is_none(),
            "a plan from another registry shape is not an actionable plan"
        );
        assert!(
            !apply_pending_v3_dataset_reset(data.path(), data.path())
                .unwrap(),
            "an unexecutable plan applies no reset instead of failing startup"
        );
        assert!(
            pending_v3_reset_work_dir(data.path()).unwrap().is_none(),
            "an unexecutable plan must not redirect work-root resolution"
        );
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged
        );

        // The safe path is "touch nothing": no root was quarantined, so the
        // dataset the user still has is the dataset they had before.
        assert!(data.path().join(DB_FILE).is_file());
        assert!(data.path().join("knowledge/sentinel").is_file());

        // The control directory itself is moved aside, because its phase
        // markers would otherwise be inherited by the next plan armed here.
        assert!(!reset_dir(data.path()).exists());
        let archived = fs::read_dir(
            data.path()
                .join(RETIRED_DATASETS_DIR)
                .join(UNUSABLE_RESET_PLANS_DIR),
        )
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
        assert_eq!(archived.len(), 1);
        assert!(
            archived[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&older.operation_id),
            "the quarantined control keeps its operation ID: {:?}",
            archived[0]
        );
        let archived_plan: DatasetResetPlan = serde_json::from_slice(
            &fs::read(archived[0].join(V3_DATASET_RESET_PLAN_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            archived_plan.roots.len(),
            older.roots.len(),
            "the unusable plan is preserved for support, not deleted"
        );

        // And the installation is not wedged: an explicit reset still works.
        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        assert!(!data.path().join("knowledge").exists());
        assert!(!data.path().join(DB_FILE).exists());
    }

    #[test]
    fn committed_reset_from_an_older_registry_shape_recovers_at_startup() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        seed_managed_root(data.path(), "knowledge", ManagedRootKind::Directory);
        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        let plan = arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::ExplicitFactoryReset,
        )
        .unwrap();
        apply_pending_v3_dataset_reset(data.path(), data.path()).unwrap();
        assert!(has_phase(data.path(), "generation-installed"));
        let older = rewrite_plan_with_older_registry_shape(
            data.path(),
            &plan,
            "browser-secrets",
        );

        // The destructive half already committed under the older build, so the
        // recovery to prove is that startup completes and the retired data
        // stays retired rather than being swept a second time.
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert!(
            data.path()
                .join(&older.retired_dir)
                .join("knowledge/sentinel")
                .is_file(),
            "the interrupted reset's retired data must survive recovery"
        );
        assert!(!reset_dir(data.path()).exists());
        assert!(
            !has_phase(data.path(), "generation-installed"),
            "the stale phase markers left the active control directory"
        );

        // Without that quarantine, the stale `generation-installed` marker
        // would be inherited by the next plan armed in the same directory and
        // fail its source/destination proof.
        touch(&data.path().join(DB_FILE));
        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
    }

    #[test]
    fn completed_control_replay_from_an_older_registry_shape_does_not_fail_startup(
    ) {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let plan = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .unwrap();
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt(data.path(), &plan.generation).unwrap();
        let replay = snapshot_active_reset_control(data.path());
        finalize_v3_dataset_reset(data.path(), data.path()).unwrap();

        let sentinel = data.path().join("knowledge/current-v3-data");
        fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
        touch(&sentinel);
        let generation =
            fs::read(data.path().join(STORAGE_GENERATION_FILE)).unwrap();

        // The completed control directory reappears, and its plan predates the
        // current managed-root registry.
        restore_active_reset_control(data.path(), &replay);
        rewrite_plan_with_older_registry_shape(
            data.path(),
            &plan,
            "browser-secrets",
        );

        assert!(
            pending_v3_reset_work_dir(data.path()).unwrap().is_none(),
            "a replayed old-shape control must not redirect work-root resolution"
        );
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert!(sentinel.is_file(), "the live v3 dataset was not re-reset");
        assert!(data.path().join(DB_FILE).is_file());
        assert_eq!(
            fs::read(data.path().join(STORAGE_GENERATION_FILE)).unwrap(),
            generation
        );
        assert!(!reset_dir(data.path()).exists());
    }

    #[test]
    fn explicit_reset_quarantines_every_registered_side_store_and_db_family_member() {
        let data = tempfile::tempdir().unwrap();
        for (relative_path, kind) in
            current_writer_managed_roots()
        {
            seed_managed_root(data.path(), relative_path, kind);
        }
        fs::create_dir_all(data.path().join("logs")).unwrap();
        touch(&data.path().join("logs/app.log"));

        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );

        let plan = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .expect("forced reset must leave a pending plan until bootstrap finalizes");
        let planned: std::collections::BTreeMap<_, _> = plan
            .roots
            .iter()
            .map(|root| (root.relative_path.as_str(), root))
            .collect();
        assert_eq!(
            planned.len(),
            current_writer_managed_roots().len(),
            "the reset plan must cover the full managed-root registry exactly once"
        );

        for (relative_path, kind) in
            current_writer_managed_roots()
        {
            let root = planned
                .get(relative_path)
                .unwrap_or_else(|| panic!("missing reset plan root {relative_path}"));
            assert!(root.initially_present, "{relative_path} was seeded");
            if relative_path == STORAGE_GENERATION_FILE {
                assert_eq!(
                    fs::read_to_string(data.path().join(relative_path)).unwrap(),
                    plan.generation,
                    "reset must replace the active storage generation"
                );
            } else {
                assert!(
                    !data.path().join(relative_path).exists(),
                    "active managed root survived forced reset: {relative_path}"
                );
            }
            let retired = data.path().join(&root.retired_relative_path);
            match kind {
                ManagedRootKind::File => {
                    assert!(retired.is_file(), "missing retired file {relative_path}")
                }
                ManagedRootKind::Directory => assert!(
                    retired.join("sentinel").is_file(),
                    "missing retired side-store payload {relative_path}"
                ),
            }
        }
        assert!(data.path().join("logs/app.log").is_file());
        assert_eq!(
            fs::read_to_string(data.path().join(STORAGE_GENERATION_FILE)).unwrap(),
            plan.generation
        );
        assert!(
            !data.path().join(V3_DATASET_RECEIPT_FILE).exists(),
            "forced reset must not publish a receipt before full bootstrap"
        );
    }

    #[test]
    fn reset_retires_process_registry_without_moving_runtime_cache() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        let runtime = data.path().join("runtime");
        fs::create_dir_all(runtime.join("bun-current")).unwrap();
        touch(&runtime.join("runtime.lock"));
        touch(&runtime.join("bun-current/bun"));
        touch(&data.path().join(AGENT_PROCESS_REGISTRY_FILE));

        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let plan = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .unwrap();
        let registry = plan
            .roots
            .iter()
            .find(|root| {
                root.relative_path == AGENT_PROCESS_REGISTRY_FILE
            })
            .expect("process registry must be reset-managed");

        assert!(!data.path().join(AGENT_PROCESS_REGISTRY_FILE).exists());
        assert!(
            data.path()
                .join(&registry.retired_relative_path)
                .is_file()
        );
        assert!(runtime.join("runtime.lock").is_file());
        assert!(runtime.join("bun-current/bun").is_file());
    }

    #[test]
    fn malformed_v3_reset_request_fails_closed_without_touching_data() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(V3_DATASET_RESET_REQUEST_FILE),
            b"not json",
        )
        .unwrap();
        touch(&dir.path().join(DB_FILE));
        assert!(prepare_v3_dataset(dir.path(), dir.path()).is_err());
        assert!(dir.path().join(DB_FILE).exists());
        assert!(!dir.path().join(V3_DATASET_RESET_DIR).exists());
    }

    #[test]
    fn atomic_new_publication_never_replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.json");
        fs::write(&path, b"winner").unwrap();

        let error = write_atomic_new(&path, b"loser").unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).unwrap(), b"winner");
        assert!(
            fs::read_dir(dir.path()).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".control.json.tmp-")
            }),
            "failed no-clobber publication must clean up its temporary file"
        );
    }

    #[test]
    fn reset_directory_rename_never_replaces_an_existing_destination() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        touch(&source.join("source-sentinel"));
        touch(&destination.join("destination-sentinel"));

        let error =
            rename_with_retry(&source, &destination).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(source.join("source-sentinel").is_file());
        assert!(destination.join("destination-sentinel").is_file());
        assert!(!destination.join("source-sentinel").exists());
    }

    #[test]
    fn unproven_legacy_reset_request_is_archived_without_touching_v3_data() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        let operation_id = Uuid::now_v7().to_string();
        let request = serde_json::json!({
            "version": LEGACY_RESET_REQUEST_VERSION,
            "operation_id": operation_id,
            "requested_at": now_ms(),
        });
        fs::write(
            request_path(data.path()),
            serde_json::to_vec_pretty(&request).unwrap(),
        )
        .unwrap();

        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert!(data.path().join(DB_FILE).is_file());
        assert!(!request_path(data.path()).exists());
        assert!(
            data.path()
                .join(RETIRED_DATASETS_DIR)
                .join(IGNORED_LEGACY_RESET_REQUESTS_DIR)
                .join(format!("{operation_id}.json"))
                .is_file()
        );
        assert!(!reset_dir(data.path()).exists());
    }

    #[test]
    fn competing_reset_requests_never_overwrite_each_other() {
        let data = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let data_path = data.path().to_path_buf();
        let target_path = target.path().to_path_buf();

        let generic_barrier = barrier.clone();
        let generic_data = data_path.clone();
        let generic = std::thread::spawn(move || {
            generic_barrier.wait();
            request_v3_dataset_reset(&generic_data, &generic_data)
        });
        let target_barrier = barrier.clone();
        let target_data = data_path.clone();
        let targeted = std::thread::spawn(move || {
            target_barrier.wait();
            request_v3_dataset_reset_for_work_dir(
                &target_data,
                &target_path,
            )
        });
        barrier.wait();

        let results = [generic.join().unwrap(), targeted.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(AppError::Conflict(_))))
                .count(),
            1
        );
        read_v3_dataset_reset_request(&data_path)
            .unwrap()
            .expect("one complete request must win atomically");
    }

    #[test]
    fn external_managed_work_root_is_quarantined_but_arbitrary_workspace_survives() {
        let data = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let workspace = external.path().join("conversations");
        let arbitrary = external.path().join("user-project");
        fs::create_dir_all(&workspace).unwrap();
        touch(&workspace.join("keep.txt"));
        fs::create_dir_all(&arbitrary).unwrap();
        touch(&arbitrary.join("keep.txt"));
        touch(&data.path().join(DB_FILE));
        request_v3_dataset_reset(data.path(), external.path()).unwrap();

        prepare_v3_dataset(data.path(), external.path()).unwrap();
        assert!(!workspace.exists());
        assert!(arbitrary.join("keep.txt").exists());

        let plan = read_pending_v3_reset(data.path(), external.path())
            .unwrap()
            .unwrap();
        assert!(
            external
                .path()
                .join(&plan.work_retired_dir)
                .join(MANAGED_WORKSPACES_DIR)
                .join("keep.txt")
                .exists()
        );
    }

    #[test]
    fn database_with_retired_factory_reset_marker_waits_for_probe() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        fs::write(
            data.path().join(RETIRED_FACTORY_RESET_MARKER),
            b"arbitrary pre-v3 bytes",
        )
        .unwrap();

        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged,
            "a present database must be classified by the app probe before retirement"
        );
        assert!(data.path().join(DB_FILE).is_file());
        assert!(data.path().join(RETIRED_FACTORY_RESET_MARKER).is_file());
        assert!(!data.path().join(V3_DATASET_RESET_DIR).exists());

        assert_eq!(
            retire_non_v3_dataset_after_probe(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let plan = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .unwrap();
        assert_eq!(plan.reason, DatasetResetReason::NonV3Dataset);
        assert!(
            data.path()
                .join(&plan.retired_dir)
                .join(RETIRED_FACTORY_RESET_MARKER)
                .is_file()
        );
    }

    #[test]
    fn pending_plan_resumes_after_source_was_already_moved() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        let plan = arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        let source = data.path().join(DB_FILE);
        let destination = data.path().join(
            &plan
                .roots
                .iter()
                .find(|root| root.relative_path == DB_FILE)
                .expect("database root is present in reset plan")
                .retired_relative_path,
        );
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::rename(&source, &destination).unwrap();

        assert!(apply_pending_v3_dataset_reset(data.path(), data.path()).unwrap());
        assert!(data.path().join("storage-generation").exists());
        assert!(has_phase(data.path(), "generation-installed"));
    }

    #[test]
    fn validated_plan_recovers_if_armed_phase_publish_was_interrupted() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        fs::remove_file(phase_path(data.path(), "armed")).unwrap();

        assert!(apply_pending_v3_dataset_reset(data.path(), data.path()).unwrap());
        assert!(has_phase(data.path(), "armed"));
        assert!(has_phase(data.path(), "generation-installed"));
    }

    #[test]
    fn pending_plan_repairs_work_config_after_apply_publish_gap() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        let plan = arm_v3_dataset_reset(
            data.path(),
            work.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        assert!(plan.persist_work_dir);
        assert!(
            apply_pending_v3_dataset_reset(data.path(), work.path())
                .unwrap()
        );
        assert!(
            crate::dir_config::checked_persisted_work_dir(data.path())
                .unwrap()
                .is_none(),
            "simulate a crash after generation install but before config publication"
        );

        assert_eq!(
            prepare_v3_dataset(data.path(), work.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        assert_eq!(
            fs::canonicalize(
                crate::dir_config::checked_persisted_work_dir(
                    data.path(),
                )
                .unwrap()
                .unwrap(),
            )
            .unwrap(),
            fs::canonicalize(work.path()).unwrap()
        );
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            work.path(),
            &plan.generation,
        )
        .unwrap();
        assert!(
            finalize_v3_dataset_reset(data.path(), work.path())
                .unwrap()
        );
    }

    #[test]
    fn generation_install_phase_gap_recovers_with_or_without_old_marker() {
        for initially_present in [false, true] {
            let data = tempfile::tempdir().unwrap();
            touch(&data.path().join(DB_FILE));
            if initially_present {
                fs::write(
                    data.path().join(STORAGE_GENERATION_FILE),
                    Uuid::now_v7().to_string(),
                )
                .unwrap();
            }
            arm_v3_dataset_reset(
                data.path(),
                data.path(),
                DatasetResetReason::NonV3Dataset,
            )
            .unwrap();
            apply_pending_v3_dataset_reset(data.path(), data.path()).unwrap();
            fs::remove_file(phase_path(
                data.path(),
                "generation-installed",
            ))
            .unwrap();

            assert!(
                apply_pending_v3_dataset_reset(data.path(), data.path())
                    .unwrap()
            );
            assert!(has_phase(data.path(), "generation-installed"));
        }
    }

    #[test]
    fn unstarted_legacy_plan_is_archived_before_any_data_move() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        let plan = arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        let plan = rewrite_as_legacy_v1_plan(data.path(), plan);

        assert!(
            !apply_pending_v3_dataset_reset(data.path(), data.path())
                .unwrap()
        );
        assert!(data.path().join(DB_FILE).is_file());
        assert!(!reset_dir(data.path()).exists());
        assert!(
            data.path()
                .join(RETIRED_DATASETS_DIR)
                .join(IGNORED_LEGACY_RESET_PLANS_DIR)
                .join(plan.operation_id)
                .is_dir()
        );
    }

    #[test]
    fn released_v1_plan_and_distinct_v1_request_recover_together() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        let plan = arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        let plan = rewrite_as_legacy_v1_plan(data.path(), plan);
        assert!(
            plan.roots.iter().all(|root| {
                root.relative_path != AGENT_PROCESS_REGISTRY_FILE
            }),
            "released v1 plans predate the process registry root"
        );
        let request_operation = Uuid::now_v7().to_string();
        assert_ne!(request_operation, plan.operation_id);
        let request = serde_json::json!({
            "version": LEGACY_RESET_REQUEST_VERSION,
            "operation_id": request_operation,
            "requested_at": now_ms(),
        });
        fs::write(
            request_path(data.path()),
            serde_json::to_vec_pretty(&request).unwrap(),
        )
        .unwrap();

        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert!(data.path().join(DB_FILE).is_file());
        assert!(!reset_dir(data.path()).exists());
        assert!(!request_path(data.path()).exists());
        assert!(
            data.path()
                .join(RETIRED_DATASETS_DIR)
                .join(IGNORED_LEGACY_RESET_PLANS_DIR)
                .join(&plan.operation_id)
                .is_dir()
        );
        assert!(
            data.path()
                .join(RETIRED_DATASETS_DIR)
                .join(IGNORED_LEGACY_RESET_REQUESTS_DIR)
                .join(format!("{request_operation}.json"))
                .is_file()
        );
    }

    #[test]
    fn archived_v1_control_replay_with_stale_installed_phase_never_redirects_or_retires_later_v3()
    {
        let data = tempfile::tempdir().unwrap();
        let first_work = tempfile::tempdir().unwrap();
        let second_work = tempfile::tempdir().unwrap();
        let canonical_second_work =
            fs::canonicalize(second_work.path()).unwrap();
        crate::dir_config::set_work_dir(data.path(), first_work.path())
            .unwrap();
        fs::write(data.path().join(DB_FILE), b"preserve-current").unwrap();
        let plan = arm_v3_dataset_reset(
            data.path(),
            first_work.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        let plan = rewrite_as_legacy_v1_plan(data.path(), plan);
        assert_eq!(
            prepare_v3_dataset(data.path(), first_work.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        let archive_root = data
            .path()
            .join(RETIRED_DATASETS_DIR)
            .join(IGNORED_LEGACY_RESET_PLANS_DIR);
        let archived = archive_root.join(&plan.operation_id);
        let replay = fs::read_dir(&archived)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect::<Vec<_>>();

        crate::dir_config::set_work_dir(data.path(), &canonical_second_work)
            .unwrap();
        let current_generation = Uuid::now_v7().to_string();
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            current_generation.as_bytes(),
        )
        .unwrap();
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            &canonical_second_work,
            &current_generation,
        )
        .unwrap();
        fs::create_dir_all(
            canonical_second_work.join(MANAGED_WORKSPACES_DIR),
        )
        .unwrap();
        let current_sentinel = canonical_second_work
            .join(MANAGED_WORKSPACES_DIR)
            .join("current-v3");
        fs::write(&current_sentinel, b"current").unwrap();
        let current_receipt = fs::read(receipt_path(data.path())).unwrap();
        let current_config = fs::read(
            data.path().join(crate::dir_config::DIR_CONFIG_FILE),
        )
        .unwrap();
        first_work.close().unwrap();

        restore_active_reset_control(data.path(), &replay);
        write_phase(data.path(), "generation-installed").unwrap();
        assert!(
            pending_v3_reset_work_dir(data.path()).unwrap().is_none(),
            "a permanently ignored v1 replay must not redirect resolution to its missing old work root"
        );
        assert_eq!(
            prepare_v3_dataset(data.path(), &canonical_second_work).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert_eq!(
            fs::read(data.path().join(DB_FILE)).unwrap(),
            b"preserve-current"
        );
        assert!(current_sentinel.is_file());
        assert_eq!(
            fs::read(data.path().join(STORAGE_GENERATION_FILE)).unwrap(),
            current_generation.as_bytes()
        );
        assert_eq!(
            fs::read(receipt_path(data.path())).unwrap(),
            current_receipt
        );
        assert_eq!(
            fs::read(
                data.path().join(crate::dir_config::DIR_CONFIG_FILE)
            )
            .unwrap(),
            current_config
        );
        assert!(!reset_dir(data.path()).exists());
        assert!(
            fs::read_dir(&archive_root).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{}-replay-", plan.operation_id))
            })
        );
    }

    #[test]
    fn replayed_v1_request_does_not_block_a_v2_pending_plan() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        let plan = arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        let request_operation = Uuid::now_v7().to_string();
        let request = serde_json::json!({
            "version": LEGACY_RESET_REQUEST_VERSION,
            "operation_id": request_operation,
            "requested_at": now_ms(),
        });
        fs::write(
            request_path(data.path()),
            serde_json::to_vec_pretty(&request).unwrap(),
        )
        .unwrap();

        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let resumed = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .unwrap();
        assert_eq!(resumed.operation_id, plan.operation_id);
        assert_eq!(resumed.generation, plan.generation);
        assert!(!request_path(data.path()).exists());
        assert!(
            data.path()
                .join(RETIRED_DATASETS_DIR)
                .join(IGNORED_LEGACY_RESET_REQUESTS_DIR)
                .join(format!("{request_operation}.json"))
                .is_file()
        );
    }

    #[test]
    fn partially_applied_legacy_plan_rolls_back_without_moving_database() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join("nomifun-backend.db-wal"));
        touch(&data.path().join(DB_FILE));
        let plan = arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        let plan = rewrite_as_legacy_v1_plan(data.path(), plan);
        let wal = plan
            .roots
            .iter()
            .find(|root| {
                root.relative_path == "nomifun-backend.db-wal"
            })
            .unwrap();
        let destination = data.path().join(&wal.retired_relative_path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::rename(
            data.path().join("nomifun-backend.db-wal"),
            &destination,
        )
        .unwrap();

        assert!(
            !apply_pending_v3_dataset_reset(data.path(), data.path())
                .unwrap()
        );
        assert!(data.path().join(DB_FILE).is_file());
        assert!(data.path().join("nomifun-backend.db-wal").is_file());
        assert!(!destination.exists());
        assert!(!reset_dir(data.path()).exists());
    }

    #[test]
    fn fully_quarantined_legacy_plan_rolls_back_before_strict_reprobe() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        let plan = arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        let plan = rewrite_as_legacy_v1_plan(data.path(), plan);
        simulate_legacy_v1_quarantine(
            data.path(),
            data.path(),
            &plan,
            false,
        );

        assert!(
            !apply_pending_v3_dataset_reset(data.path(), data.path())
                .unwrap()
        );
        assert!(data.path().join(DB_FILE).is_file());
        assert!(!reset_dir(data.path()).exists());
        assert!(
            data.path()
                .join(RETIRED_DATASETS_DIR)
                .join(IGNORED_LEGACY_RESET_PLANS_DIR)
                .join(plan.operation_id)
                .is_dir()
        );
    }

    #[test]
    fn legacy_generation_marker_phase_gap_rolls_back_to_old_generation() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        let old_generation = Uuid::now_v7().to_string();
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            &old_generation,
        )
        .unwrap();
        let plan = arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::NonV3Dataset,
        )
        .unwrap();
        let plan = rewrite_as_legacy_v1_plan(data.path(), plan);
        simulate_legacy_v1_quarantine(
            data.path(),
            data.path(),
            &plan,
            true,
        );
        fs::remove_file(phase_path(
            data.path(),
            "generation-installed",
        ))
        .unwrap();

        assert!(
            !apply_pending_v3_dataset_reset(data.path(), data.path())
                .unwrap()
        );
        assert!(data.path().join(DB_FILE).is_file());
        assert_eq!(
            fs::read_to_string(
                data.path().join(STORAGE_GENERATION_FILE)
            )
            .unwrap(),
            old_generation
        );
        assert!(!reset_dir(data.path()).exists());
    }

    #[test]
    fn fresh_empty_root_does_not_arm_reset() {
        let data = tempfile::tempdir().unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert!(!data.path().join(V3_DATASET_RESET_DIR).exists());
    }

    #[test]
    fn database_without_receipt_waits_for_application_probe() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        fs::create_dir_all(data.path().join("conversations")).unwrap();
        touch(&data.path().join("conversations").join("v3.txt"));

        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert!(data.path().join(DB_FILE).is_file());
        assert!(data.path().join("conversations/v3.txt").is_file());
        assert!(!data.path().join(V3_DATASET_RESET_DIR).exists());
    }

    #[test]
    fn receipt_is_bound_to_canonical_resolved_work_root() {
        let data = tempfile::tempdir().unwrap();
        let first_work = tempfile::tempdir().unwrap();
        let second_work = tempfile::tempdir().unwrap();
        let generation = Uuid::now_v7().to_string();
        touch(&data.path().join(DB_FILE));
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            generation.as_bytes(),
        )
        .unwrap();
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            first_work.path(),
            &generation,
        )
        .unwrap();

        assert_eq!(
            inspect_v3_dataset_receipt(data.path(), first_work.path()).unwrap(),
            DatasetReceiptStatus::Current
        );
        assert_eq!(
            inspect_v3_dataset_receipt(data.path(), second_work.path()).unwrap(),
            DatasetReceiptStatus::WorkRootMismatch
        );
        assert!(
            require_current_v3_dataset_for_work_dir(data.path(), second_work.path()).is_err()
        );
        require_current_v3_dataset(data.path()).unwrap();
    }

    #[test]
    fn finalized_pre_owner_dataset_installs_owner_exactly_once() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let generation = Uuid::now_v7().to_string();
        touch(&data.path().join(DB_FILE));
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            generation.as_bytes(),
        )
        .unwrap();
        let receipt = DatasetReceipt {
            contract_version: V3_DATASET_CONTRACT_VERSION,
            generation: generation.clone(),
            work_root: fs::canonicalize(work.path())
                .unwrap()
                .display()
                .to_string(),
            work_root_binding_required: false,
            installed_at: now_ms(),
        };
        write_atomic(
            &receipt_path(data.path()),
            &serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
        let owner_path = work.path().join(WORK_ROOT_OWNER_FILE);
        assert!(!owner_path.exists());

        ensure_current_v3_work_root_owner(data.path(), work.path()).unwrap();
        let first_owner = fs::read(&owner_path).unwrap();
        let binding_path = data.path().join(WORK_ROOT_BINDING_FILE);
        let first_binding = fs::read(&binding_path).unwrap();
        let upgraded_receipt: DatasetReceipt = serde_json::from_slice(
            &fs::read(receipt_path(data.path())).unwrap(),
        )
        .unwrap();
        assert!(
            upgraded_receipt.work_root_binding_required,
            "the durable receipt must close the one-time compatibility window"
        );
        require_v3_work_root_owner(
            data.path(),
            work.path(),
            &generation,
        )
        .unwrap();

        ensure_current_v3_work_root_owner(data.path(), work.path()).unwrap();
        assert_eq!(
            fs::read(&owner_path).unwrap(),
            first_owner,
            "the compatibility backfill must be idempotent"
        );
        assert_eq!(
            fs::read(&binding_path).unwrap(),
            first_binding,
            "the data-side compatibility proof must be idempotent"
        );

        fs::remove_file(&owner_path).unwrap();
        let missing_owner =
            ensure_current_v3_work_root_owner(data.path(), work.path())
                .unwrap_err();
        assert!(matches!(
            missing_owner,
            AppError::Internal(_) | AppError::Conflict(_)
        ));
        assert!(
            !owner_path.exists(),
            "a finalized receipt must never recreate a missing owner"
        );

        write_atomic(&owner_path, &first_owner).unwrap();
        fs::remove_file(&binding_path).unwrap();
        let missing_binding =
            ensure_current_v3_work_root_owner(data.path(), work.path())
                .unwrap_err();
        assert!(matches!(missing_binding, AppError::Conflict(_)));
        assert!(
            !binding_path.exists(),
            "a finalized receipt must never recreate a missing data-side binding"
        );
    }

    #[test]
    fn finalized_owner_generation_mismatch_fails_without_rotation() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let owner_generation = Uuid::now_v7().to_string();
        let replaced_generation = Uuid::now_v7().to_string();
        ensure_v3_work_root_owner(
            data.path(),
            work.path(),
            &owner_generation,
        )
        .unwrap();
        touch(&data.path().join(DB_FILE));
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            replaced_generation.as_bytes(),
        )
        .unwrap();
        let receipt = DatasetReceipt {
            contract_version: V3_DATASET_CONTRACT_VERSION,
            generation: replaced_generation.clone(),
            work_root: fs::canonicalize(work.path())
                .unwrap()
                .display()
                .to_string(),
            work_root_binding_required: false,
            installed_at: now_ms(),
        };
        write_atomic(
            &receipt_path(data.path()),
            &serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();

        let error =
            ensure_current_v3_work_root_owner(data.path(), work.path())
                .unwrap_err();

        assert!(matches!(error, AppError::Conflict(_)));
        require_v3_work_root_owner(
            data.path(),
            work.path(),
            &owner_generation,
        )
        .unwrap();
        assert!(
            require_v3_work_root_owner(
                data.path(),
                work.path(),
                &replaced_generation,
            )
            .is_err(),
            "the finalized compatibility bridge must not rewrite an existing owner"
        );
    }

    #[test]
    fn validated_reset_plan_can_rotate_owner_generation() {
        let data = tempfile::tempdir().unwrap();
        let old_generation = Uuid::now_v7().to_string();
        ensure_v3_work_root_owner(
            data.path(),
            data.path(),
            &old_generation,
        )
        .unwrap();

        let plan = arm_v3_dataset_reset(
            data.path(),
            data.path(),
            DatasetResetReason::ExplicitFactoryReset,
        )
        .unwrap();

        require_v3_work_root_owner(
            data.path(),
            data.path(),
            &plan.generation,
        )
        .unwrap();
        assert_ne!(plan.generation, old_generation);
    }

    #[test]
    fn work_root_owner_cannot_be_claimed_by_another_data_root() {
        let first_data = tempfile::tempdir().unwrap();
        let second_data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let first_generation = Uuid::now_v7().to_string();
        let second_generation = Uuid::now_v7().to_string();

        ensure_v3_work_root_owner(
            first_data.path(),
            work.path(),
            &first_generation,
        )
        .unwrap();
        let error = ensure_v3_work_root_owner(
            second_data.path(),
            work.path(),
            &second_generation,
        )
        .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
        require_v3_work_root_owner(
            first_data.path(),
            work.path(),
            &first_generation,
        )
        .unwrap();

        let request_error = request_v3_dataset_reset_for_work_dir(
            second_data.path(),
            work.path(),
        )
        .unwrap_err();
        assert!(matches!(request_error, AppError::Conflict(_)));
        assert!(!request_path(second_data.path()).exists());
        assert!(!reset_dir(second_data.path()).exists());
    }

    #[test]
    fn data_root_owned_as_another_dataset_work_root_fails_closed() {
        let first_data = tempfile::tempdir().unwrap();
        let second_data = tempfile::tempdir().unwrap();
        let second_work = tempfile::tempdir().unwrap();
        let generation = Uuid::now_v7().to_string();
        ensure_v3_work_root_owner(
            first_data.path(),
            second_data.path(),
            &generation,
        )
        .unwrap();
        fs::create_dir_all(
            second_data.path().join(MANAGED_WORKSPACES_DIR),
        )
        .unwrap();
        let sentinel = second_data
            .path()
            .join(MANAGED_WORKSPACES_DIR)
            .join("first-dataset-live.txt");
        touch(&sentinel);

        let error = prepare_v3_dataset(
            second_data.path(),
            second_work.path(),
        )
        .unwrap_err();

        assert!(matches!(error, AppError::Conflict(_)));
        assert!(sentinel.is_file());
        assert!(!reset_dir(second_data.path()).exists());
    }

    #[test]
    fn work_dir_change_target_cannot_be_another_data_root() {
        let data = tempfile::tempdir().unwrap();
        let target_data_root = tempfile::tempdir().unwrap();
        touch(&target_data_root.path().join("server.lock"));

        let error = request_v3_dataset_reset_for_work_dir(
            data.path(),
            target_data_root.path(),
        )
        .unwrap_err();

        assert!(matches!(error, AppError::Conflict(_)));
        assert!(!request_path(data.path()).exists());
        assert!(!reset_dir(data.path()).exists());
    }

    #[test]
    fn owned_work_dir_change_target_can_resume_after_generation_install() {
        let data = tempfile::tempdir().unwrap();
        let old_work = tempfile::tempdir().unwrap();
        let new_work = tempfile::tempdir().unwrap();
        let old_generation = Uuid::now_v7().to_string();
        touch(&data.path().join(DB_FILE));
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            old_generation.as_bytes(),
        )
        .unwrap();
        write_v3_dataset_receipt_for_work_dir(
            data.path(),
            old_work.path(),
            &old_generation,
        )
        .unwrap();
        let plan = arm_v3_dataset_reset(
            data.path(),
            new_work.path(),
            DatasetResetReason::WorkDirChange,
        )
        .unwrap();

        assert!(
            apply_pending_v3_dataset_reset(
                data.path(),
                new_work.path()
            )
            .unwrap()
        );
        let conversations = new_work.path().join(MANAGED_WORKSPACES_DIR);
        fs::create_dir_all(&conversations).unwrap();
        touch(&conversations.join("new-generation.txt"));

        assert!(
            apply_pending_v3_dataset_reset(
                data.path(),
                new_work.path()
            )
            .unwrap(),
            "a matching persistent owner makes the post-install retry unambiguous"
        );
        assert!(conversations.join("new-generation.txt").is_file());
        require_v3_work_root_owner(
            data.path(),
            new_work.path(),
            &plan.generation,
        )
        .unwrap();
    }

    #[test]
    fn unfinished_bootstrap_binding_recovers_only_with_the_same_work_root() {
        let data = tempfile::tempdir().unwrap();
        let first_work = tempfile::tempdir().unwrap();
        let second_work = tempfile::tempdir().unwrap();
        let generation = Uuid::now_v7().to_string();
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            generation.as_bytes(),
        )
        .unwrap();
        write_v3_dataset_bootstrap_binding(data.path(), first_work.path(), &generation)
            .unwrap();

        assert_eq!(
            inspect_v3_dataset_bootstrap_binding(data.path(), first_work.path()).unwrap(),
            DatasetReceiptStatus::Current
        );
        assert_eq!(
            prepare_v3_dataset(data.path(), first_work.path()).unwrap(),
            DatasetPreparation::Unchanged
        );
        assert!(
            prepare_v3_dataset(data.path(), second_work.path())
                .unwrap_err()
                .to_string()
                .contains("different resolved work root")
        );
        assert!(!data.path().join(V3_DATASET_RESET_DIR).exists());
    }

    #[test]
    fn final_receipt_replaces_unfinished_bootstrap_binding() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let generation = Uuid::now_v7().to_string();
        touch(&data.path().join(DB_FILE));
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            generation.as_bytes(),
        )
        .unwrap();
        write_v3_dataset_bootstrap_binding(data.path(), work.path(), &generation).unwrap();
        assert!(data.path().join(V3_DATASET_BOOTSTRAP_FILE).is_file());

        write_v3_dataset_receipt_for_work_dir(data.path(), work.path(), &generation).unwrap();

        assert!(!data.path().join(V3_DATASET_BOOTSTRAP_FILE).exists());
        assert_eq!(
            inspect_v3_dataset_receipt(data.path(), work.path()).unwrap(),
            DatasetReceiptStatus::Current
        );
    }

    #[test]
    fn finalization_requires_matching_receipt_and_fresh_database() {
        let data = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        let plan = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .expect("pending reset plan");

        let missing_receipt = finalize_v3_dataset_reset(data.path(), data.path())
            .expect_err("receipt is mandatory");
        assert!(missing_receipt.to_string().contains("receipt"));

        touch(&data.path().join(DB_FILE));
        let wrong_generation = Uuid::now_v7().to_string();
        let forged_receipt = DatasetReceipt {
            contract_version: V3_DATASET_CONTRACT_VERSION,
            generation: wrong_generation,
            work_root: fs::canonicalize(data.path())
                .unwrap()
                .display()
                .to_string(),
            work_root_binding_required: true,
            installed_at: now_ms(),
        };
        write_atomic(
            &receipt_path(data.path()),
            &serde_json::to_vec_pretty(&forged_receipt).unwrap(),
        )
        .unwrap();
        let mismatched = finalize_v3_dataset_reset(data.path(), data.path())
            .expect_err("receipt generation must match the reset plan");
        assert!(mismatched.to_string().contains("does not match"));
        assert!(data.path().join(V3_DATASET_RESET_DIR).is_dir());

        write_v3_dataset_receipt(data.path(), &plan.generation).unwrap();
        assert!(finalize_v3_dataset_reset(data.path(), data.path()).unwrap());
        assert!(!data.path().join(V3_DATASET_RESET_DIR).exists());
        assert_eq!(
            inspect_v3_dataset_receipt(data.path(), data.path()).unwrap(),
            DatasetReceiptStatus::Current
        );
    }

    #[cfg(unix)]
    #[test]
    fn completed_request_tombstone_parent_symlink_fails_closed() {
        let data = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        touch(&data.path().join(DB_FILE));
        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        prepare_v3_dataset(data.path(), data.path()).unwrap();
        let plan = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .unwrap();
        touch(&data.path().join(DB_FILE));
        write_v3_dataset_receipt(data.path(), &plan.generation).unwrap();
        let completed_parent =
            completed_requests_dir(data.path());
        std::os::unix::fs::symlink(
            outside.path(),
            &completed_parent,
        )
        .unwrap();

        let error =
            finalize_v3_dataset_reset(data.path(), data.path())
                .unwrap_err();
        assert!(error.to_string().contains("real directory"));
        assert!(
            fs::read_dir(outside.path()).unwrap().next().is_none(),
            "a tombstone must never be written through a symlink parent"
        );
        assert!(reset_dir(data.path()).is_dir());
        assert!(data.path().join(DB_FILE).is_file());
    }

    #[test]
    fn empty_data_dir_with_external_managed_workspace_is_retired() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let conversations = work.path().join(MANAGED_WORKSPACES_DIR);
        fs::create_dir_all(&conversations).unwrap();
        touch(&conversations.join("legacy.txt"));

        assert_eq!(
            prepare_v3_dataset(data.path(), work.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        assert!(!conversations.exists());

        let plan = read_pending_v3_reset(data.path(), work.path())
            .unwrap()
            .expect("external workspace retirement must leave a pending reset plan");
        assert_eq!(plan.reason, DatasetResetReason::NonV3Dataset);
        assert!(
            work.path()
                .join(plan.work_retired_dir)
                .join(MANAGED_WORKSPACES_DIR)
                .join("legacy.txt")
                .is_file()
        );
    }

    #[test]
    fn database_probe_can_override_a_matching_but_forged_receipt() {
        let data = tempfile::tempdir().unwrap();
        let generation = Uuid::now_v7().to_string();
        touch(&data.path().join(DB_FILE));
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            generation.as_bytes(),
        )
        .unwrap();
        write_v3_dataset_receipt(data.path(), &generation).unwrap();

        assert_eq!(
            prepare_v3_dataset(data.path(), data.path()).unwrap(),
            DatasetPreparation::Unchanged,
            "the filesystem hand-off alone cannot inspect SQLite identity"
        );
        assert_eq!(
            retire_non_v3_dataset_after_probe(data.path(), data.path()).unwrap(),
            DatasetPreparation::ResetApplied
        );
        assert!(!data.path().join(DB_FILE).exists());

        let plan = read_pending_v3_reset(data.path(), data.path())
            .unwrap()
            .expect("probe-triggered retirement must leave a pending reset plan");
        assert_eq!(plan.reason, DatasetResetReason::NonV3Dataset);
        assert!(
            data.path()
                .join(plan.retired_dir)
                .join(DB_FILE)
                .is_file()
        );
    }

    #[test]
    fn offline_dataset_gate_rejects_explicit_reset_request() {
        let data = tempfile::tempdir().unwrap();
        let generation = Uuid::now_v7().to_string();
        touch(&data.path().join(DB_FILE));
        fs::write(
            data.path().join(STORAGE_GENERATION_FILE),
            generation.as_bytes(),
        )
        .unwrap();
        write_v3_dataset_receipt(data.path(), &generation).unwrap();
        require_current_v3_dataset(data.path()).unwrap();

        request_v3_dataset_reset(data.path(), data.path()).unwrap();
        let error = require_current_v3_dataset(data.path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("explicit v3 dataset reset has been requested")
        );
    }
}
