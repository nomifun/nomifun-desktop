//! Production Fresh-v4 Agent platform mount.
//!
//! The legacy application service graph remains alive for the slices that have
//! not yet crossed their C7 boundary.  This module deliberately gives the
//! canonical Agent platform its own validated Fresh-v4 pool and its own
//! registration inventory; it never passes the v3 application pool into
//! `AgentPlatform`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    CodingRuntimeFeatureInventoryPayload, DigestHex, FreshV4ReadyMarker,
    FreshV4SchemaMetadata, RuntimeProfileKind, RuntimeTarget, VersionString,
    canonical_json_bytes, digest_bytes, digest_payload,
    fresh_v4_schema_manifest_payload, official_preset_seed_manifest_payload,
    FRESH_V4_BASELINE_SQL, FRESH_V4_DATA_GENERATION, FRESH_V4_MIGRATION_HEAD,
    FRESH_V4_PROJECTION_SCHEMA_VERSION,
};
use nomifun_agent_control_plane::CompilerReleaseInputs;
use nomifun_agent_kernel::{CompilerEnvironment, MaterializationPolicy};
use nomifun_agent_platform::{AgentPlatform, AgentPlatformConfig};
use nomifun_chat_model_broker::{
    ChatBrokerPort, ChatModelError, ChatModelErrorCode, ChatModelRequest, ChatModelStream,
    ChatRetryDirective,
};
use nomifun_codex_runtime::CodexRuntimeSupervisor;
use nomifun_v4_root::{
    FRESH_V4_DATABASE_FILE, FRESH_V4_INITIALIZING_MARKER_FILE,
    FRESH_V4_PARENT_MARKER_FILE, FRESH_V4_READY_MARKER_FILE,
    application_build_digest, canonical_schema_manifest_digest,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::services::AppServices;

const CONTRACT_VERSION: &str = "1.0.0";
const C7_AVAILABILITY_REVISION: &str = "c7-windows-continuous-2026-08-30";
const APPLICATION_BUILD_IDENTITY: &str =
    concat!("nomifun-app@", env!("CARGO_PKG_VERSION"));
const BASELINE_MIGRATION_NAME: &str = "0001_fresh_v4";
const MAX_READY_MARKER_BYTES: u64 = 64 * 1024;
const MOUNT_INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(30);
const MODEL_MEDIA_PACKAGE_ID: &str = "nomifun.model-media";
const RUNTIME_FEATURE_INVENTORY_JSON: &str = include_str!(
    "../../../nomifun-agent-contracts/contracts/runtime/coding-runtime-feature-inventory.payload.json"
);

/// Mount the canonical platform if the pre-service Fresh-v4 root is ready.
///
/// A legacy-only test or an older embedding that has not run the Fresh-v4
/// bootstrap simply keeps its existing router.  It is not allowed to open a
/// second v4 database or silently fall back to the v3 pool.
pub(crate) async fn try_build(
    services: &AppServices,
) -> anyhow::Result<Option<Arc<AgentPlatform>>> {
    let Some(paths) = probe_fresh_v4_mount(&services.data_dir)? else {
        return Ok(None);
    };

    let marker = read_ready_marker(&paths.ready)?;
    let expected_schema_digest = canonical_schema_manifest_digest()?;
    validate_ready_marker(&marker, &expected_schema_digest)?;
    let pool = open_validated_pool(&paths.database).await?;
    initialize_platform_with_cleanup(
        pool,
        paths.ready,
        marker,
        expected_schema_digest,
    )
    .await
    .map(Some)
}

async fn initialize_platform_with_cleanup(
    pool: SqlitePool,
    ready_path: PathBuf,
    marker: FreshV4ReadyMarker,
    expected_schema_digest: DigestHex,
) -> anyhow::Result<Arc<AgentPlatform>> {
    // AgentPlatform::from_pool takes ownership of the pool while it builds its
    // persistent adapters and publishes the initial generation. Keep one
    // cleanup handle outside that future so every ordinary initialization
    // failure closes the pool after all internal clones have been dropped.
    let cleanup_pool = pool.clone();
    let result = match tokio::time::timeout(
        MOUNT_INITIALIZATION_TIMEOUT,
        initialize_platform(pool, ready_path, marker, expected_schema_digest),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => Err(anyhow::anyhow!(
            "Fresh-v4 Agent platform initialization timed out after {} seconds: {error}",
            MOUNT_INITIALIZATION_TIMEOUT.as_secs()
        )),
    };
    match result {
        Ok(platform) => Ok(platform),
        Err(error) => {
            cleanup_pool.close().await;
            Err(error)
        }
    }
}

async fn initialize_platform(
    pool: SqlitePool,
    ready_path: PathBuf,
    marker: FreshV4ReadyMarker,
    expected_schema_digest: DigestHex,
) -> anyhow::Result<Arc<AgentPlatform>> {
    validate_schema_metadata(&pool, &marker, &expected_schema_digest).await?;
    // Bind the opened pool to the same immutable marker that was inspected
    // before the connection was established. The application lock normally
    // prevents this race; retaining the check makes a replacement fail closed.
    if read_ready_marker(&ready_path)? != marker {
        anyhow::bail!("Fresh-v4 ready marker changed while the mount was opening");
    }

    let feature_inventory: CodingRuntimeFeatureInventoryPayload =
        serde_json::from_str(RUNTIME_FEATURE_INVENTORY_JSON)?;
    feature_inventory
        .validate()
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let feature_digest = digest_payload(&feature_inventory)?;
    let seed = official_preset_seed_manifest_payload();
    if feature_digest != seed.target_runtime_feature_inventory_digest {
        anyhow::bail!("runtime feature inventory digest differs from the frozen seed");
    }

    let mut policy = MaterializationPolicy::stable(CONTRACT_VERSION);
    policy.available_runtime_features = feature_inventory.runtime_features.clone();
    let kernel_environment = CompilerEnvironment {
        resolver_version: VersionString::from(CONTRACT_VERSION),
        required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
        required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest: feature_digest.clone(),
        available_runtime_features: feature_inventory.runtime_features,
        canonical_schema_manifest_digest: expected_schema_digest.clone(),
        target_contribution_manifest_digest: seed.target_first_party_contribution_digest.clone(),
        host_target: current_runtime_target(),
        host_surface: current_host_surface(),
        availability_evidence_revision: C7_AVAILABILITY_REVISION.to_owned(),
    };
    let release = CompilerReleaseInputs {
        resolver_version: VersionString::from(CONTRACT_VERSION),
        runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
        runtime_feature_inventory_digest: feature_digest,
        canonical_schema_manifest_digest: expected_schema_digest,
        target_contribution_manifest_digest: seed.target_first_party_contribution_digest,
        availability_evidence_revision: C7_AVAILABILITY_REVISION.to_owned(),
    };

    let registrations = bundled_registrations()?;

    // Runtime process supervision is real and shared by all v4 Sessions. Its
    // constructor is inert: no Tokio task or child process exists until a
    // Session is launched.
    //
    // AppServices currently exposes ModelInvokeService, but that public API is
    // not a ChatBrokerPort and cannot supply the broker's exact route resolver,
    // credential lease, causality gate, six-adapter set, and sole-retry
    // boundary. Adapting it here would bypass those contracts. Keep this
    // bounded, task-free broker fail-closed until a composed public
    // ChatModelBroker factory is available; it never routes through legacy
    // Nomi or invents a provider fallback.
    let supervisor = Arc::new(CodexRuntimeSupervisor::new());
    let broker: Arc<dyn ChatBrokerPort> = Arc::new(ProviderRouteRequiredBroker);
    let mut config = AgentPlatformConfig::with_supervisor(
        pool,
        policy,
        release,
        kernel_environment,
        supervisor,
        broker,
    );
    config.initial_plugins = registrations;
    // `initial_plugins` is the sole publication input for this host
    // generation. AgentPlatform publishes it once transactionally while it
    // constructs the platform; do not publish the same inventory a second
    // time from router assembly.
    Ok(AgentPlatform::from_pool(config).await?)
}

fn bundled_registrations(
) -> anyhow::Result<Vec<nomifun_agent_kernel::PluginRegistration>> {
    let target_specs = nomifun_agent_domain_support::c7_package_specs();
    let model_media_specs = target_specs
        .iter()
        .copied()
        .filter(|spec| spec.id == MODEL_MEDIA_PACKAGE_ID)
        .collect::<Vec<_>>();
    if model_media_specs.len() != 1 {
        anyhow::bail!(
            "C7 target inventory must contain exactly one {MODEL_MEDIA_PACKAGE_ID} package spec"
        );
    }
    let model_media_spec = model_media_specs[0];
    let mut registrations =
        Vec::with_capacity(target_specs.len());
    append_wave_registrations(
        &mut registrations,
        "Wave 1",
        &nomifun_agent_domain_wave1::PACKAGE_IDS,
        nomifun_agent_domain_wave1::registrations(),
    )?;
    append_wave_registrations(
        &mut registrations,
        "Wave 2",
        &nomifun_agent_domain_wave2::PACKAGE_IDS,
        nomifun_agent_domain_wave2::registrations(),
    )?;
    append_wave_registrations(
        &mut registrations,
        "Wave 3",
        &nomifun_agent_domain_wave3::PACKAGE_IDS,
        nomifun_agent_domain_wave3::registrations(),
    )?;
    append_wave_registrations(
        &mut registrations,
        "Wave 4",
        &nomifun_agent_domain_wave4::PACKAGE_IDS,
        nomifun_agent_domain_wave4::registrations(),
    )?;
    append_wave_registrations(
        &mut registrations,
        "Wave 5",
        &nomifun_agent_domain_wave5::PACKAGE_IDS,
        nomifun_agent_domain_wave5::registrations(),
    )?;
    registrations.push(
        nomifun_agent_domain_support::registration(model_media_spec)
            .map_err(|error| anyhow::anyhow!(error))?,
    );
    validate_bundled_registrations(&registrations, &target_specs)?;
    Ok(registrations)
}

fn append_wave_registrations(
    destination: &mut Vec<nomifun_agent_kernel::PluginRegistration>,
    wave_name: &str,
    expected_package_ids: &[&str],
    registrations: Result<
        Vec<nomifun_agent_kernel::PluginRegistration>,
        String,
    >,
) -> anyhow::Result<()> {
    let registrations =
        registrations.map_err(|error| anyhow::anyhow!("{wave_name} registration failed: {error}"))?;
    validate_registration_package_set(
        wave_name,
        &registrations,
        expected_package_ids,
    )?;
    destination.extend(registrations);
    Ok(())
}

fn validate_registration_package_set(
    owner: &str,
    registrations: &[nomifun_agent_kernel::PluginRegistration],
    expected_package_ids: &[&str],
) -> anyhow::Result<()> {
    let actual = registrations
        .iter()
        .map(|registration| {
            registration
                .metadata
                .manifest
                .payload
                .package_id
                .as_ref()
                .to_owned()
        })
        .collect::<BTreeSet<_>>();
    let expected = expected_package_ids
        .iter()
        .map(|package_id| (*package_id).to_owned())
        .collect::<BTreeSet<_>>();
    if actual != expected || registrations.len() != expected.len() {
        anyhow::bail!(
            "{owner} registration package set mismatch: expected {expected:?}, found {actual:?}"
        );
    }
    Ok(())
}

fn validate_bundled_registrations(
    registrations: &[nomifun_agent_kernel::PluginRegistration],
    target_specs: &[nomifun_agent_domain_support::PackageSpec],
) -> anyhow::Result<()> {
    nomifun_agent_domain_support::validate_inventory(registrations)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let mut expected_packages = BTreeSet::new();
    let mut expected_capabilities = BTreeMap::<String, BTreeSet<String>>::new();
    for spec in target_specs {
        if !expected_packages.insert(spec.id.to_owned()) {
            anyhow::bail!(
                "C7 target inventory contains duplicate package {}",
                spec.id
            );
        }
        let capabilities = expected_capabilities
            .entry(spec.id.to_owned())
            .or_default();
        for capability in spec.capabilities {
            if !capabilities.insert(capability.id.to_owned()) {
                anyhow::bail!(
                    "C7 target inventory contains duplicate capability {} in {}",
                    capability.id,
                    spec.id
                );
            }
        }
    }

    let mut actual_packages = BTreeSet::new();
    let mut actual_capabilities = BTreeMap::<String, BTreeSet<String>>::new();
    for registration in registrations {
        let manifest = &registration.metadata.manifest.payload;
        let package_id = manifest.package_id.as_ref().to_owned();
        if !actual_packages.insert(package_id.clone()) {
            anyhow::bail!(
                "bundled registration inventory publishes package {} more than once",
                package_id
            );
        }
        if manifest.package_version.as_ref() != CONTRACT_VERSION {
            anyhow::bail!(
                "bundled package {} has unexpected version {}",
                package_id,
                manifest.package_version.as_ref()
            );
        }
        let capabilities = actual_capabilities
            .entry(package_id)
            .or_default();
        for capability in &manifest.contributions.capabilities {
            if !capabilities.insert(capability.id.as_ref().to_owned()) {
                anyhow::bail!(
                    "bundled registration inventory publishes capability {} more than once",
                    capability.id.as_ref()
                );
            }
        }
    }

    if actual_packages != expected_packages
        || actual_capabilities != expected_capabilities
    {
        anyhow::bail!(
            "bundled C7 registration inventory differs from the frozen target inventory: \
             expected packages={expected_packages:?}, capabilities={expected_capabilities:?}; \
             found packages={actual_packages:?}, capabilities={actual_capabilities:?}"
        );
    }
    Ok(())
}

struct ProviderRouteRequiredBroker;

#[async_trait]
impl ChatBrokerPort for ProviderRouteRequiredBroker {
    async fn open_chat_stream(
        &self,
        _request: ChatModelRequest,
    ) -> Result<ChatModelStream, ChatModelError> {
        Err(ChatModelError::new(
            ChatModelErrorCode::AdapterUnavailable,
            "no canonical provider route is configured for this AgentSession",
            ChatRetryDirective::Never,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MountEntryKind {
    Missing,
    RealFile,
    Other,
}

struct FreshV4MountPaths {
    database: PathBuf,
    ready: PathBuf,
}

fn probe_fresh_v4_mount(
    data_dir: &Path,
) -> anyhow::Result<Option<FreshV4MountPaths>> {
    let database = data_dir.join(FRESH_V4_DATABASE_FILE);
    let ready = data_dir.join(FRESH_V4_READY_MARKER_FILE);
    let initializing = data_dir.join(FRESH_V4_INITIALIZING_MARKER_FILE);
    let ready_staging = data_dir.join(format!(
        "{FRESH_V4_READY_MARKER_FILE}.staging"
    ));
    let parent_marker = data_dir
        .parent()
        .map(|parent| parent.join(FRESH_V4_PARENT_MARKER_FILE));

    let ready_kind = classify_mount_entry(&ready)?;
    let database_kind = classify_mount_entry(&database)?;
    if classify_mount_entry(&initializing)? != MountEntryKind::Missing {
        anyhow::bail!(
            "Fresh-v4 initializing marker must be absent before Agent platform mount: {}",
            initializing.display()
        );
    }
    if classify_mount_entry(&ready_staging)? != MountEntryKind::Missing {
        anyhow::bail!(
            "Fresh-v4 ready marker staging path must be absent before Agent platform mount: {}",
            ready_staging.display()
        );
    }
    if let Some(parent_marker) = parent_marker
        && classify_mount_entry(&parent_marker)? != MountEntryKind::Missing
    {
        anyhow::bail!(
            "Fresh-v4 parent operation marker must be absent before Agent platform mount: {}",
            parent_marker.display()
        );
    }

    match (ready_kind, database_kind) {
        (MountEntryKind::Missing, MountEntryKind::Missing) => Ok(None),
        (MountEntryKind::RealFile, MountEntryKind::RealFile) => {
            require_real_directory(data_dir)?;
            Ok(Some(FreshV4MountPaths { database, ready }))
        }
        (MountEntryKind::Missing, MountEntryKind::RealFile) => {
            anyhow::bail!(
                "Fresh-v4 database exists without its ready marker: {}",
                database.display()
            );
        }
        (MountEntryKind::RealFile, MountEntryKind::Missing) => {
            anyhow::bail!(
                "Fresh-v4 ready marker exists without its database: {}",
                ready.display()
            );
        }
        _ => anyhow::bail!(
            "Fresh-v4 database and ready marker must both be real regular files: database={}, ready={}",
            database.display(),
            ready.display()
        ),
    }
}

fn classify_mount_entry(path: &Path) -> anyhow::Result<MountEntryKind> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MountEntryKind::Missing);
        }
        Err(error) => {
            return Err(anyhow::Error::new(error).context(format!(
                "inspect Fresh-v4 mount entry {}",
                path.display()
            )));
        }
    };
    if metadata_is_link_or_reparse(&metadata) {
        return Ok(MountEntryKind::Other);
    }
    Ok(if metadata.is_file() {
        MountEntryKind::RealFile
    } else {
        MountEntryKind::Other
    })
}

fn require_real_directory(path: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
        anyhow::bail!(
            "Fresh-v4 canonical data root must be a real directory: {}",
            path.display()
        );
    }
    Ok(())
}

fn read_ready_marker(path: &Path) -> anyhow::Result<FreshV4ReadyMarker> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
        anyhow::bail!(
            "Fresh-v4 ready marker must be a real regular file: {}",
            path.display()
        );
    }
    if metadata.len() > MAX_READY_MARKER_BYTES {
        anyhow::bail!(
            "Fresh-v4 ready marker exceeds the {} byte limit: {}",
            MAX_READY_MARKER_BYTES,
            path.display()
        );
    }

    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_READY_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_READY_MARKER_BYTES {
        anyhow::bail!(
            "Fresh-v4 ready marker exceeds the {} byte limit: {}",
            MAX_READY_MARKER_BYTES,
            path.display()
        );
    }
    let marker: FreshV4ReadyMarker = serde_json::from_slice(&bytes)?;
    marker
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid Fresh-v4 ready marker: {error}"))?;
    if bytes != canonical_json_bytes(&marker)? {
        anyhow::bail!(
            "Fresh-v4 ready marker is not canonical JSON: {}",
            path.display()
        );
    }
    Ok(marker)
}

fn validate_ready_marker(
    marker: &FreshV4ReadyMarker,
    expected_schema_digest: &DigestHex,
) -> anyhow::Result<()> {
    marker
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid Fresh-v4 ready marker: {error}"))?;
    if marker.canonical_schema_manifest_digest != *expected_schema_digest {
        anyhow::bail!(
            "Fresh-v4 ready marker schema digest differs from the canonical contract"
        );
    }
    let expected_build_digest =
        application_build_digest(APPLICATION_BUILD_IDENTITY)?;
    if marker.application_build_digest != expected_build_digest {
        anyhow::bail!(
            "Fresh-v4 ready marker application build digest does not match this build"
        );
    }
    let expected_seed_digest =
        digest_payload(&official_preset_seed_manifest_payload())?;
    if marker.seed_manifest_digest != expected_seed_digest {
        anyhow::bail!(
            "Fresh-v4 ready marker seed manifest digest does not match the frozen seed"
        );
    }
    Ok(())
}

async fn open_validated_pool(path: &Path) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    Ok(SqlitePoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await?)
}

type SchemaObject = (String, String, String, Option<String>);

async fn validate_schema_metadata(
    pool: &SqlitePool,
    marker: &FreshV4ReadyMarker,
    schema_digest: &DigestHex,
) -> anyhow::Result<()> {
    let rows: Vec<(
        String,
        i64,
        String,
        i64,
        String,
        String,
        i64,
    )> = sqlx::query_as(
        "SELECT singleton_key, data_generation, root_instance_id, migration_head, \
                seed_manifest_digest, canonical_schema_manifest_digest, \
                projection_schema_version \
         FROM schema_metadata ORDER BY singleton_key",
    )
    .fetch_all(pool)
    .await?;
    if rows.len() != 1 {
        anyhow::bail!(
            "Fresh-v4 schema_metadata must contain exactly one canonical row, found {}",
            rows.len()
        );
    }
    let (
        singleton_key,
        data_generation,
        root_instance_id,
        migration_head,
        seed_manifest_digest,
        canonical_schema_manifest_digest,
        projection_schema_version,
    ) = rows.into_iter().next().expect("row count checked");
    let metadata = FreshV4SchemaMetadata {
        singleton_key,
        data_generation: u32_from_sqlite("data_generation", data_generation)?,
        root_instance_id,
        migration_head: u32_from_sqlite("migration_head", migration_head)?,
        seed_manifest_digest: DigestHex::from(seed_manifest_digest),
        canonical_schema_manifest_digest: DigestHex::from(
            canonical_schema_manifest_digest,
        ),
        projection_schema_version: u32_from_sqlite(
            "projection_schema_version",
            projection_schema_version,
        )?,
    };
    metadata
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid Fresh-v4 schema metadata: {error}"))?;
    let expected_seed_digest =
        digest_payload(&official_preset_seed_manifest_payload())?;
    if metadata.data_generation != FRESH_V4_DATA_GENERATION
        || metadata.migration_head != FRESH_V4_MIGRATION_HEAD
        || metadata.projection_schema_version
            != FRESH_V4_PROJECTION_SCHEMA_VERSION
        || metadata.seed_manifest_digest != expected_seed_digest
        || metadata.canonical_schema_manifest_digest != *schema_digest
        || marker.canonical_schema_manifest_digest != *schema_digest
        || !marker.matches_schema_metadata(&metadata)
    {
        anyhow::bail!("Fresh-v4 schema_metadata does not match the ready marker");
    }

    let expected_tables = fresh_v4_schema_manifest_payload()
        .tables
        .into_iter()
        .map(|table| table.table_name)
        .collect::<BTreeSet<_>>();
    let actual_tables = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_schema \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect::<BTreeSet<_>>();
    if actual_tables != expected_tables {
        anyhow::bail!(
            "Fresh-v4 table exact-set mismatch: expected {expected_tables:?}, found {actual_tables:?}"
        );
    }

    let actual_objects = schema_objects(pool).await?;
    let expected_objects = baseline_schema_objects().await?;
    if actual_objects != expected_objects {
        anyhow::bail!(
            "Fresh-v4 table/index/trigger definitions do not match the embedded baseline"
        );
    }

    let foreign_keys: i64 =
        sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(pool)
            .await?;
    if foreign_keys != 1 {
        anyhow::bail!("Fresh-v4 pool did not enable SQLite foreign-key enforcement");
    }
    let user_version: i64 =
        sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(pool)
            .await?;
    if user_version != 0 {
        anyhow::bail!(
            "Fresh-v4 PRAGMA user_version must remain 0, found {user_version}"
        );
    }

    let expected_migrations = vec![(
        i64::from(FRESH_V4_MIGRATION_HEAD),
        BASELINE_MIGRATION_NAME.to_owned(),
        digest_bytes(FRESH_V4_BASELINE_SQL.as_bytes()).as_ref().to_owned(),
        0_i64,
    )];
    let migrations: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT version, name, checksum, applied_at \
         FROM schema_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await?;
    if migrations != expected_migrations {
        anyhow::bail!(
            "Fresh-v4 migration lineage mismatch: {migrations:?}"
        );
    }

    let quick_check: Vec<String> =
        sqlx::query_scalar("PRAGMA quick_check")
            .fetch_all(pool)
            .await?;
    if quick_check.as_slice() != ["ok"] {
        anyhow::bail!(
            "Fresh-v4 SQLite quick_check failed: {}",
            quick_check.join("; ")
        );
    }
    let foreign_key_failures: Vec<(String, i64, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await?;
    if !foreign_key_failures.is_empty() {
        anyhow::bail!(
            "Fresh-v4 SQLite foreign_key_check found {} violations",
            foreign_key_failures.len()
        );
    }
    Ok(())
}

fn schema_objects(
    pool: &SqlitePool,
) -> impl std::future::Future<Output = anyhow::Result<Vec<SchemaObject>>> + '_ {
    async move {
        Ok(sqlx::query_as(
            "SELECT type, name, tbl_name, sql FROM sqlite_schema \
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .fetch_all(pool)
        .await?)
    }
}

async fn baseline_schema_objects() -> anyhow::Result<Vec<SchemaObject>> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(
            SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(true),
        )
        .await?;
    let result: anyhow::Result<Vec<SchemaObject>> = async {
        sqlx::raw_sql(FRESH_V4_BASELINE_SQL)
            .execute(&pool)
            .await?;
        schema_objects(&pool).await
    }
    .await;
    pool.close().await;
    result
}

fn u32_from_sqlite(field: &str, value: i64) -> anyhow::Result<u32> {
    u32::try_from(value).map_err(|_| {
        anyhow::anyhow!(
            "Fresh-v4 schema_metadata.{field} is outside the u32 range: {value}"
        )
    })
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

fn current_host_surface() -> String {
    if cfg!(feature = "computer-use") {
        "desktop".to_owned()
    } else {
        "headless".to_owned()
    }
}

fn current_runtime_target() -> RuntimeTarget {
    let target = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else {
        "unsupported-local-target"
    };
    RuntimeTarget::from(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_ready_marker() -> FreshV4ReadyMarker {
        FreshV4ReadyMarker {
            data_generation: FRESH_V4_DATA_GENERATION,
            root_instance_id: "test-root".to_owned(),
            migration_head: FRESH_V4_MIGRATION_HEAD,
            seed_manifest_digest: digest_payload(
                &official_preset_seed_manifest_payload(),
            )
            .unwrap(),
            canonical_schema_manifest_digest:
                canonical_schema_manifest_digest().unwrap(),
            projection_schema_version: FRESH_V4_PROJECTION_SCHEMA_VERSION,
            application_build_digest:
                application_build_digest(APPLICATION_BUILD_IDENTITY).unwrap(),
        }
    }

    #[test]
    fn host_target_is_a_concrete_platform_label() {
        assert!(!current_runtime_target().as_ref().is_empty());
        assert!(!current_host_surface().is_empty());
    }

    #[test]
    fn feature_inventory_is_frozen_and_non_empty() {
        let inventory: CodingRuntimeFeatureInventoryPayload =
            serde_json::from_str(RUNTIME_FEATURE_INVENTORY_JSON).unwrap();
        inventory.validate().unwrap();
        assert!(!inventory.runtime_features.is_empty());
        assert_eq!(inventory.supported_profiles.len(), 2);
    }

    #[test]
    fn bundled_registration_inventory_is_complete_and_unique() {
        let registrations = bundled_registrations().unwrap();
        let target_specs = nomifun_agent_domain_support::c7_package_specs();
        validate_bundled_registrations(&registrations, &target_specs)
            .unwrap();
        let expected_packages = target_specs
            .iter()
            .map(|spec| spec.id)
            .collect::<BTreeSet<_>>();
        let actual_packages = registrations
            .iter()
            .map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .package_id
                    .as_ref()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(actual_packages, expected_packages);
        assert_eq!(
            registrations
                .iter()
                .filter(|registration| {
                    registration
                        .metadata
                        .manifest
                        .payload
                        .package_id
                        .as_ref()
                        == MODEL_MEDIA_PACKAGE_ID
                })
                .count(),
            1
        );
        assert_eq!(registrations.len(), target_specs.len());
        assert_eq!(
            registrations
                .iter()
                .map(|registration| {
                    registration
                        .metadata
                        .manifest
                        .payload
                        .contributions
                        .capabilities
                        .len()
                })
                .sum::<usize>(),
            137
        );
    }

    #[test]
    fn ready_marker_requires_canonical_bytes_and_current_build() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FRESH_V4_READY_MARKER_FILE);
        let marker = valid_ready_marker();
        let mut bytes = canonical_json_bytes(&marker).unwrap();
        bytes.push(b'\n');
        std::fs::write(&path, bytes).unwrap();
        assert!(read_ready_marker(&path).is_err());

        std::fs::write(&path, canonical_json_bytes(&marker).unwrap()).unwrap();
        let read = read_ready_marker(&path).unwrap();
        validate_ready_marker(
            &read,
            &read.canonical_schema_manifest_digest,
        )
        .unwrap();

        let mut wrong_build = read;
        wrong_build.application_build_digest = DigestHex::from("0".repeat(64));
        assert!(validate_ready_marker(
            &wrong_build,
            &wrong_build.canonical_schema_manifest_digest
        )
        .is_err());
    }

    #[test]
    fn mount_probe_rejects_partial_and_in_progress_roots() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        assert!(probe_fresh_v4_mount(&data_dir).unwrap().is_none());

        std::fs::write(data_dir.join(FRESH_V4_DATABASE_FILE), []).unwrap();
        assert!(probe_fresh_v4_mount(&data_dir).is_err());

        std::fs::remove_file(data_dir.join(FRESH_V4_DATABASE_FILE)).unwrap();
        std::fs::write(
            data_dir.join(FRESH_V4_INITIALIZING_MARKER_FILE),
            b"{}",
        )
        .unwrap();
        assert!(probe_fresh_v4_mount(&data_dir).is_err());
    }

    #[tokio::test]
    async fn initialization_failure_closes_the_owned_pool() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        let observer = pool.clone();
        let result = initialize_platform_with_cleanup(
            pool,
            PathBuf::from("missing-ready-marker"),
            valid_ready_marker(),
            canonical_schema_manifest_digest().unwrap(),
        )
        .await;
        assert!(result.is_err());
        assert!(observer.is_closed());
    }

    #[tokio::test]
    async fn canonical_fresh_v4_root_passes_mount_validation() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(&data_dir, APPLICATION_BUILD_IDENTITY, &[])
            .await
            .unwrap();
        let pool = open_validated_pool(
            &data_dir.join(FRESH_V4_DATABASE_FILE),
        )
        .await
        .unwrap();
        validate_schema_metadata(
            &pool,
            &outcome.ready_marker,
            &outcome
                .ready_marker
                .canonical_schema_manifest_digest,
        )
        .await
        .unwrap();
        pool.close().await;
    }

    #[tokio::test]
    async fn canonical_mount_publishes_one_complete_registration_generation() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(&data_dir, APPLICATION_BUILD_IDENTITY, &[])
            .await
            .unwrap();
        let platform = initialize_platform_with_cleanup(
            open_validated_pool(
                &data_dir.join(FRESH_V4_DATABASE_FILE),
            )
            .await
            .unwrap(),
            data_dir.join(FRESH_V4_READY_MARKER_FILE),
            outcome.ready_marker,
            canonical_schema_manifest_digest().unwrap(),
        )
        .await
        .unwrap();

        let registry = platform.materialized_registry().unwrap();
        assert_eq!(registry.generation, 1);
        assert_eq!(registry.packages.len(), 26);
        assert_eq!(registry.capabilities.len(), 137);
        let package_rows: Vec<String> = sqlx::query_scalar(
            "SELECT package_id FROM plugin_packages \
             WHERE package_version = ? ORDER BY package_id",
        )
        .bind(CONTRACT_VERSION)
        .fetch_all(platform.pool())
        .await
        .unwrap();
        assert_eq!(package_rows.len(), 26);
        assert_eq!(
            package_rows.iter().collect::<BTreeSet<_>>().len(),
            package_rows.len()
        );
        platform.pool().close().await;
    }
}
