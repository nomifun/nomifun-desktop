//! Production Fresh-v4 Agent platform composition.
//!
//! The canonical Agent platform owns a validated Fresh-v4 pool and its
//! registration inventory. It never accepts a legacy application service graph
//! or a v3 database pool.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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
use nomifun_agent_domain_wave1::Wave1HostPort;
use nomifun_agent_domain_wave2::Wave2HostPort;
use nomifun_agent_platform::{
    AgentPlatform, AgentPlatformConfig, ChatExecutionAuthority,
    BrokerBackedRuntimePort, ProductionChatCausalityGate,
    RuntimeStartTurnBrokerBridge, SupervisedCodexRuntimePort,
};
use nomifun_agent_session::AgentSessionStore;
use nomifun_codex_runtime::CodexRuntimeSupervisor;
use nomifun_v4_root::application_build_digest;
#[cfg(test)]
use nomifun_v4_root::{
    FRESH_V4_DATABASE_FILE, FRESH_V4_READY_MARKER_FILE,
    canonical_schema_manifest_digest,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

use super::agent_wave2_host::Wave2ApplicationHost;
use super::agent_wave4_host::Wave4ApplicationHost;
use super::chat_broker_host::{
    ChatBrokerHostComposition, ConnectionCredentialLeaseRegistry,
    SqliteChatOperationClaimStore,
};
use crate::bootstrap::APPLICATION_BUILD_IDENTITY;

const CONTRACT_VERSION: &str = "1.0.0";
const C7_AVAILABILITY_REVISION: &str = "c7-windows-continuous-2026-08-30";
const BASELINE_MIGRATION_NAME: &str = "0001_fresh_v4";
const MAX_READY_MARKER_BYTES: u64 = 64 * 1024;
const MOUNT_INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(30);
const MODEL_MEDIA_PACKAGE_ID: &str = "nomifun.model-media";
const RUNTIME_FEATURE_INVENTORY_JSON: &str = include_str!(
    "../../../nomifun-agent-contracts/contracts/runtime/coding-runtime-feature-inventory.payload.json"
);

/// Build the canonical Agent platform from an already-open Fresh-v4 pool and
/// an optional provider pool.  A Fresh-v4 host passes its own pool here so
/// provider/model/connection facts remain in the same canonical database; the
/// explicit `None` path is retained for test fixtures that exercise the
/// fail-closed unconfigured broker shape.
pub(crate) async fn build_from_open_pool(
    pool: SqlitePool,
    ready_path: PathBuf,
    marker: FreshV4ReadyMarker,
    expected_schema_digest: DigestHex,
    provider_pool: Option<SqlitePool>,
    encryption_key: [u8; 32],
    workspace_root: PathBuf,
) -> anyhow::Result<Arc<AgentPlatform>> {
    initialize_platform_with_cleanup(
        pool,
        ready_path,
        marker,
        expected_schema_digest,
        provider_pool,
        encryption_key,
        nomifun_agent_domain_wave1::unconfigured_host_port(),
        Arc::new(Wave2ApplicationHost::for_workspace_root(workspace_root)),
    )
    .await
}

pub(crate) async fn open_validated_pool(path: &Path) -> anyhow::Result<SqlitePool> {
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

async fn initialize_platform_with_cleanup(
    pool: SqlitePool,
    ready_path: PathBuf,
    marker: FreshV4ReadyMarker,
    expected_schema_digest: DigestHex,
    provider_pool: Option<SqlitePool>,
    encryption_key: [u8; 32],
    wave1_host: Arc<dyn Wave1HostPort>,
    wave2_host: Arc<dyn Wave2HostPort>,
) -> anyhow::Result<Arc<AgentPlatform>> {
    // AgentPlatform::from_pool takes ownership of the pool while it builds its
    // persistent adapters and publishes the initial generation. Keep one
    // cleanup handle outside that future so every ordinary initialization
    // failure closes the pool after all internal clones have been dropped.
    let cleanup_pool = pool.clone();
    let result = match tokio::time::timeout(
        MOUNT_INITIALIZATION_TIMEOUT,
        initialize_platform(
            pool,
            ready_path,
            marker,
            expected_schema_digest,
            provider_pool,
            encryption_key,
            wave1_host,
            wave2_host,
        ),
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
    provider_pool: Option<SqlitePool>,
    encryption_key: [u8; 32],
    wave1_host: Arc<dyn Wave1HostPort>,
    wave2_host: Arc<dyn Wave2HostPort>,
) -> anyhow::Result<Arc<AgentPlatform>> {
    validate_ready_marker(&marker, &expected_schema_digest)?;
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

    let registrations = bundled_registrations(wave1_host, wave2_host)?;

    // Runtime process supervision is real and shared by all v4 Sessions. Its
    // constructor is inert: no Tokio task or child process exists until a
    // Session is launched. Compose the broker from exact v4 route records,
    // provider/connection repositories, a durable Session facts gate, and a
    // single-attempt provider transport.
    let sessions = Arc::new(AgentSessionStore::from_pool(pool.clone()).await?);
    let operation_claims = Arc::new(SqliteChatOperationClaimStore::new(sessions.clone()));
    let causality_gate = Arc::new(ProductionChatCausalityGate::new(
        sessions.clone(),
        operation_claims,
        ChatExecutionAuthority::Primary,
    ));
    let broker = match provider_pool {
        Some(provider_pool) => {
            let composition = ChatBrokerHostComposition::new(
                pool.clone(),
                provider_pool,
                encryption_key,
                ConnectionCredentialLeaseRegistry::new(),
            );
            let model_invoke = composition.build_model_invoke(
                nomifun_net::http_client_no_redirect()
                    .map_err(|error| anyhow::anyhow!("build provider HTTP client: {error}"))?,
            );
            composition.build_broker(
                causality_gate,
                model_invoke,
                nomifun_chat_model_broker::BrokerRetryPolicy::default(),
            )?
        }
        None => super::chat_broker_host::build_unconfigured_broker(
            causality_gate,
            encryption_key,
            nomifun_chat_model_broker::BrokerRetryPolicy::default(),
        )?,
    };
    let supervisor = Arc::new(CodexRuntimeSupervisor::new());
    let runtime_delegate = Arc::new(SupervisedCodexRuntimePort::new(supervisor));
    let runtime_bridge = Arc::new(RuntimeStartTurnBrokerBridge::new(
        Arc::clone(&sessions),
        broker.clone(),
    ));
    let runtime = Arc::new(BrokerBackedRuntimePort::new(
        runtime_delegate,
        runtime_bridge,
    ));
    let mut config = AgentPlatformConfig::with_runtime(
        pool,
        policy,
        release,
        kernel_environment,
        runtime,
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
    wave1_host: Arc<dyn Wave1HostPort>,
    wave2_host: Arc<dyn Wave2HostPort>,
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
        nomifun_agent_domain_wave1::registrations_with_host_port(wave1_host),
    )?;
    append_wave_registrations(
        &mut registrations,
        "Wave 2",
        &nomifun_agent_domain_wave2::PACKAGE_IDS,
        nomifun_agent_domain_wave2::registrations_with_host_port(wave2_host),
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
        nomifun_agent_domain_wave4::registrations_with_host_port(Arc::new(
            Wave4ApplicationHost,
        )),
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
    use async_trait::async_trait;
    use futures_util::StreamExt;
    use nomifun_agent_contracts::{
        ChatRouteCandidate, ChatRouteFeature, ChatRouteIdentity, ChatRouteProtocol,
        ChatRouteRecord, ChatRouteRecordSchema, ChatRouteTask,
    };
    use nomifun_chat_model_broker::{
        BrokerRetryPolicy, ChatCausality, ChatCausalityGate, ChatModelError,
        ChatModelErrorCode, ChatProtocol, ProviderIdRef,
        ProductionProviderRepository as ProductionProviderRepositoryPort,
    };
    use nomifun_common::encrypt_string;
    use nomifun_db::{
        CreateProviderParams, IProviderRepository, NewProviderModel,
        NewProviderModelCapability, SqliteProviderRepository,
    };
    use super::super::chat_broker_host::{
        ChatBrokerHostComposition, ConnectionCredentialLeaseRegistry,
    };
    use std::collections::BTreeSet;

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
        let registrations =
            bundled_registrations(
                nomifun_agent_domain_wave1::unconfigured_host_port(),
                Arc::new(Wave2ApplicationHost::new()),
            )
                .unwrap();
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
            pool.clone(),
            PathBuf::from("missing-ready-marker"),
            valid_ready_marker(),
            canonical_schema_manifest_digest().unwrap(),
            Some(pool),
            [0; 32],
            nomifun_agent_domain_wave1::unconfigured_host_port(),
            Arc::new(Wave2ApplicationHost::new()),
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
        let pool = open_validated_pool(&data_dir.join(FRESH_V4_DATABASE_FILE))
            .await
            .unwrap();
        let platform = initialize_platform_with_cleanup(
            pool.clone(),
            data_dir.join(FRESH_V4_READY_MARKER_FILE),
            outcome.ready_marker,
            canonical_schema_manifest_digest().unwrap(),
            Some(pool),
            [0; 32],
            nomifun_agent_domain_wave1::unconfigured_host_port(),
            Arc::new(Wave2ApplicationHost::new()),
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

    struct AllowChatGate;

    #[async_trait]
    impl ChatCausalityGate for AllowChatGate {
        async fn authorize(&self, _causality: &ChatCausality) -> Result<(), ChatModelError> {
            Ok(())
        }
    }

    async fn production_chat_fixture(
        status: u16,
    ) -> (
        nomifun_chat_model_broker::ChatModelStream,
        wiremock::MockServer,
    ) {
        let server = wiremock::MockServer::start().await;
        let body = if status == 200 {
            "event: response.created\ndata: {\"id\":\"host-response\"}\n\n\
             event: text.delta\ndata: {\"text\":\"host success\"}\n\n\
             event: usage\ndata: {\"input_tokens\":1,\"output_tokens\":2}\n\n\
             event: response.completed\ndata: {\"finish_reason\":\"stop\"}\n\n"
        } else {
            "{\"error\":{\"message\":\"provider unavailable\"}}"
        };
        let mut response = wiremock::ResponseTemplate::new(status);
        response = if status == 200 {
            response
                .set_body_raw(body, "text/event-stream")
                .insert_header("cache-control", "no-cache")
        } else {
            response.set_body_raw(body, "application/json")
        };
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat"))
            .respond_with(response)
            .mount(&server)
            .await;

        let v4_dir = tempfile::tempdir().expect("v4 temp root");
        nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(v4_dir.path(), APPLICATION_BUILD_IDENTITY, &[])
            .await
            .expect("v4 root");
        let v4_pool = super::open_validated_pool(
            &v4_dir.path().join(FRESH_V4_DATABASE_FILE),
        )
        .await
        .expect("v4 pool");
        let encrypted =
            encrypt_string(r#"{"api_keys":["host-test-key"]}"#, &[0x41; 32])
                .expect("encrypted credentials");
        let capabilities = [NewProviderModelCapability {
            task: "chat",
            traits: "[]",
            protocol: "openai.chat_text",
            connection_role: "default",
            endpoint: Some("/chat"),
            provider_params: r#"{"temperature":0.25}"#,
            output_limit: Some(64),
            ..Default::default()
        }];
        let (provider, _) = SqliteProviderRepository::new(v4_pool.clone())
            .create(
                CreateProviderParams {
                    provider_id: None,
                    platform: "openai",
                    name: "host test provider",
                    base_url: &server.uri(),
                    auth_scheme: "bearer",
                    credentials_encrypted: &encrypted,
                    enabled: true,
                    bedrock_config: None,
                    sort_order: Some(0),
                },
                &NewProviderModel {
                    model: "host-test-model",
                    enabled: true,
                    sort_order: 0,
                    description: None,
                    capabilities: &capabilities,
                },
                &[],
            )
            .await
            .expect("provider graph");
        let provider_id = ProviderIdRef::from(provider.provider_id.clone());
        let provider_repository =
            super::super::chat_broker_host::ProductionProviderRepository::new(
                v4_pool.clone(),
            );
        let provider_record = provider_repository
            .find_provider(&provider_id)
            .await
            .expect("provider digest")
            .expect("provider row");

        sqlx::query(
            "INSERT INTO agent_presets \
             (preset_id, owner_ref_json, source_json, display_json, \
              current_stable_revision, created_at) \
             VALUES (?, '{}', '{}', '{}', 1, 0)",
        )
        .bind("host-preset")
        .execute(&v4_pool)
        .await
        .expect("preset row");
        sqlx::query(
            "INSERT INTO agent_preset_revisions \
             (revision_id, preset_id, revision_no, schema_version, \
              editor_document_json, revision_digest, created_by, created_at, reason) \
             VALUES (?, ?, 1, '1.0.0', '{}', ?, 'host-test-owner', 0, '')",
        )
        .bind("host-preset@1")
        .bind("host-preset")
        .bind("a".repeat(64))
        .execute(&v4_pool)
        .await
        .expect("revision row");
        let route_record = ChatRouteRecord {
            schema: ChatRouteRecordSchema::V1,
            task: ChatRouteTask::AgentChat,
            primary: ChatRouteCandidate {
                model_route_id: "host-route".into(),
                model_route_revision: 1,
                provider_id: provider.provider_id,
                model: "host-test-model".to_owned(),
                protocol: ChatRouteProtocol::OpenaiChat,
                connection_config_ref: "default".into(),
                config_revision_digest: provider_record.config_revision_digest,
                credential_ref: "host-credential".to_owned(),
                features: BTreeSet::from([
                    ChatRouteFeature::TextInput,
                    ChatRouteFeature::ImageInput,
                    ChatRouteFeature::AudioInput,
                    ChatRouteFeature::TextOutput,
                    ChatRouteFeature::ToolCalls,
                    ChatRouteFeature::Reasoning,
                    ChatRouteFeature::StructuredOutput,
                ]),
            },
            failovers: Vec::new(),
        };
        sqlx::query(
            "INSERT INTO agent_preset_model_routes \
             (revision_id, model_task, route_json) VALUES (?, ?, ?)",
        )
        .bind("host-preset@1")
        .bind("agent_chat")
        .bind(route_record.to_canonical_json().expect("route JSON"))
        .execute(&v4_pool)
        .await
        .expect("route row");

        let composition = ChatBrokerHostComposition::new(
            v4_pool.clone(),
            v4_pool.clone(),
            [0x41; 32],
            ConnectionCredentialLeaseRegistry::new(),
        );
        let broker = composition
            .build_broker(
                Arc::new(AllowChatGate),
                composition.build_model_invoke(
                    reqwest::Client::builder()
                        .no_proxy()
                        .build()
                        .expect("HTTP client"),
                ),
                BrokerRetryPolicy {
                    max_total_attempts: 1,
                    max_attempts_per_route: 1,
                },
            )
            .expect("production broker");
        let fixture = nomifun_chat_model_broker::recorded_conformance_fixtures()
            .into_iter()
            .find(|fixture| fixture.protocol == ChatProtocol::OpenaiChat)
            .expect("OpenAI Chat fixture");
        let mut request = fixture.request;
        let identity = ChatRouteIdentity::new(
            "host-preset@1",
            nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
            "host-route".into(),
            1,
        );
        request.route = identity.clone();
        request.causality.route_identity = identity;
        let stream = broker
            .open_chat_stream(request)
            .await
            .expect("broker stream");
        (stream, server)
    }

    #[tokio::test]
    async fn production_host_chat_broker_streams_a_real_provider_response() {
        let (stream, server) = production_chat_fixture(200).await;
        let events = stream.collect::<Vec<_>>().await;
        assert!(events.iter().all(Result::is_ok));
        assert!(events.iter().any(|event| {
            event.as_ref().is_ok_and(|event| {
                matches!(
                    event.event,
                    nomifun_chat_model_broker::ChatModelEvent::OutputTextDelta { .. }
                )
            })
        }));
        assert!(events.last().is_some_and(|event| {
            event
                .as_ref()
                .is_ok_and(|event| event.event.is_terminal())
        }));
        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1);
        let body: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("request JSON");
        assert_eq!(body["temperature"], 0.25);
        assert_eq!(body["max_tokens"], 64);
    }

    #[tokio::test]
    async fn production_host_chat_broker_reports_provider_unavailable_without_fake_output() {
        let (stream, server) = production_chat_fixture(503).await;
        let events = stream.collect::<Vec<_>>().await;
        let error = events
            .last()
            .expect("terminal broker error")
            .as_ref()
            .expect_err("provider failure");
        assert_eq!(error.code, ChatModelErrorCode::ProviderUnavailable);
        assert!(events.iter().all(|event| event.is_err()));
        assert_eq!(
            server.received_requests().await.expect("requests").len(),
            1
        );
    }
}
