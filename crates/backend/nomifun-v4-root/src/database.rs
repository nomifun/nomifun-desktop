use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use nomifun_agent_contracts::{
    AgentPresetSource, CapabilityKind, CapabilityManifest, DigestHex,
    FreshV4SchemaMetadata, LocalizedMetadata, OfficialPresetKey, OfficialPresetSeed,
    PackageManifest, PrincipalRef, SkillDefinition, TargetPackageContribution,
    TargetPackageInventoryPayload, canonical_json_bytes, digest_payload,
    fresh_v4_schema_manifest_payload, FRESH_V4_BASELINE_SQL,
};
use serde::Serialize;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::{SqlitePool, Transaction};

use crate::inputs::FrozenRootInputs;
use crate::{FreshV4AccessAudit, FreshV4AccessKind, FreshV4RootError};

const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const BASELINE_MIGRATION_NAME: &str = "0001_fresh_v4";
const OFFICIAL_SEED_ACTOR: &str = "system:official";
const OFFICIAL_SEED_REASON: &str = "fresh-v4-official-seed";

pub(crate) struct FreshV4Database {
    pool: SqlitePool,
}

impl FreshV4Database {
    pub(crate) async fn open_for_initialization(
        path: &Path,
        audit: &dyn FreshV4AccessAudit,
    ) -> Result<Self, FreshV4RootError> {
        audit.record(FreshV4AccessKind::Database, path)?;
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Delete)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(DATABASE_BUSY_TIMEOUT);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    pub(crate) async fn open_read_only(
        path: &Path,
        audit: &dyn FreshV4AccessAudit,
    ) -> Result<Self, FreshV4RootError> {
        audit.record(FreshV4AccessKind::Database, path)?;
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .read_only(true)
            .foreign_keys(true)
            .busy_timeout(DATABASE_BUSY_TIMEOUT);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    pub(crate) async fn inspect_schema_metadata(
        &self,
    ) -> Result<Option<FreshV4SchemaMetadata>, FreshV4RootError> {
        read_schema_metadata(&self.pool).await
    }

    pub(crate) async fn ensure_schema_metadata(
        &self,
        root_instance_id: &str,
        inputs: &FrozenRootInputs,
    ) -> Result<FreshV4SchemaMetadata, FreshV4RootError> {
        let expected = expected_schema_metadata(root_instance_id, inputs)?;
        if !table_exists(&self.pool, "schema_metadata").await? {
            let existing_tables: Vec<String> = sqlx::query_scalar(
                "SELECT name FROM sqlite_schema \
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )
            .fetch_all(&self.pool)
            .await?;
            if !existing_tables.is_empty() {
                return Err(FreshV4RootError::State(format!(
                    "database has tables but no canonical schema_metadata: {}",
                    existing_tables.join(", ")
                )));
            }

            let mut transaction = self.pool.begin().await?;
            sqlx::raw_sql(FRESH_V4_BASELINE_SQL)
                .execute(&mut *transaction)
                .await?;
            insert_schema_metadata(&mut transaction, &expected).await?;
            sqlx::query(
                "INSERT INTO schema_migrations \
                 (version, name, checksum, applied_at) VALUES (?, ?, ?, ?)",
            )
            .bind(i64::from(expected.migration_head))
            .bind(BASELINE_MIGRATION_NAME)
            .bind(
                inputs
                    .canonical_manifest
                    .payload
                    .database_schema_digest
                    .as_ref(),
            )
            .bind(0_i64)
            .execute(&mut *transaction)
            .await?;
            transaction.commit().await?;
        }

        self.validate_schema(&expected, inputs).await?;
        Ok(expected)
    }

    pub(crate) async fn ensure_materialization(
        &self,
        inputs: &FrozenRootInputs,
    ) -> Result<(), FreshV4RootError> {
        let expected = expected_materialization(&inputs.target_inventory)?;
        let mut transaction = self.pool.begin().await?;

        for row in &expected.packages {
            sqlx::query(
                "INSERT INTO plugin_packages \
                 (package_id, package_version, manifest_json, manifest_digest, display_json) \
                 VALUES (?, ?, ?, ?, ?) \
                 ON CONFLICT (package_id, package_version) DO NOTHING",
            )
            .bind(&row.package_id)
            .bind(&row.package_version)
            .bind(&row.manifest_json)
            .bind(&row.manifest_digest)
            .bind(&row.display_json)
            .execute(&mut *transaction)
            .await?;
        }
        for row in &expected.mounts {
            sqlx::query(
                "INSERT INTO plugin_mounts \
                 (mount_id, package_id, package_version, source_json, desired_state, \
                  effective_state, criticality) \
                 VALUES (?, ?, ?, ?, 'enabled', 'active', 'required') \
                 ON CONFLICT (mount_id) DO NOTHING",
            )
            .bind(&row.mount_id)
            .bind(&row.package_id)
            .bind(&row.package_version)
            .bind(&row.source_json)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT INTO plugin_configs \
                 (package_id, mount_id, config_json, revision) \
                 VALUES (?, ?, '{}', 1) \
                 ON CONFLICT (package_id, mount_id) DO NOTHING",
            )
            .bind(&row.package_id)
            .bind(&row.mount_id)
            .execute(&mut *transaction)
            .await?;
        }
        for row in &expected.capabilities {
            sqlx::query(
                "INSERT INTO capability_definitions \
                 (capability_id, capability_version, package_id, package_version, \
                  manifest_json, manifest_digest) \
                 VALUES (?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (capability_id, capability_version) DO NOTHING",
            )
            .bind(&row.capability_id)
            .bind(&row.capability_version)
            .bind(&row.package_id)
            .bind(&row.package_version)
            .bind(&row.manifest_json)
            .bind(&row.manifest_digest)
            .execute(&mut *transaction)
            .await?;
        }
        for row in &expected.skills {
            sqlx::query(
                "INSERT INTO skill_instructions \
                 (skill_id, skill_version, package_id, package_version, \
                  definition_json, definition_digest) \
                 VALUES (?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (skill_id, skill_version) DO NOTHING",
            )
            .bind(&row.skill_id)
            .bind(&row.skill_version)
            .bind(&row.package_id)
            .bind(&row.package_version)
            .bind(&row.definition_json)
            .bind(&row.definition_digest)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        validate_materialization(&self.pool, &expected).await
    }

    pub(crate) async fn ensure_official_seed(
        &self,
        inputs: &FrozenRootInputs,
    ) -> Result<(), FreshV4RootError> {
        let expected = expected_official_seed(inputs)?;
        let mut transaction = self.pool.begin().await?;

        for row in &expected.templates {
            sqlx::query(
                "INSERT INTO agent_preset_templates \
                 (template_key, source_package_id, source_package_version, source_kind, \
                  template_json, template_digest) \
                 VALUES (?, ?, ?, 'official', ?, ?) \
                 ON CONFLICT (template_key) DO NOTHING",
            )
            .bind(&row.template_key)
            .bind(&row.source_package_id)
            .bind(&row.source_package_version)
            .bind(&row.template_json)
            .bind(&row.template_digest)
            .execute(&mut *transaction)
            .await?;
        }
        for row in &expected.presets {
            sqlx::query(
                "INSERT INTO agent_presets \
                 (preset_id, owner_ref_json, source_json, display_json, \
                  current_stable_revision, created_at) \
                 VALUES (?, ?, ?, ?, 1, 0) \
                 ON CONFLICT (preset_id) DO NOTHING",
            )
            .bind(&row.preset_id)
            .bind(&row.owner_ref_json)
            .bind(&row.source_json)
            .bind(&row.display_json)
            .execute(&mut *transaction)
            .await?;
        }
        for row in &expected.revisions {
            sqlx::query(
                "INSERT INTO agent_preset_revisions \
                 (revision_id, preset_id, revision_no, schema_version, editor_document_json, \
                  revision_digest, created_by, created_at, reason) \
                 VALUES (?, ?, 1, ?, ?, ?, ?, 0, ?) \
                 ON CONFLICT (revision_id) DO NOTHING",
            )
            .bind(&row.revision_id)
            .bind(&row.preset_id)
            .bind(&row.schema_version)
            .bind(&row.editor_document_json)
            .bind(&row.revision_digest)
            .bind(OFFICIAL_SEED_ACTOR)
            .bind(OFFICIAL_SEED_REASON)
            .execute(&mut *transaction)
            .await?;
        }
        for row in &expected.initial_capabilities {
            sqlx::query(
                "INSERT INTO preset_initial_capabilities \
                 (revision_id, capability_id, capability_version, selection_json) \
                 VALUES (?, ?, ?, ?) \
                 ON CONFLICT (revision_id, capability_id) DO NOTHING",
            )
            .bind(&row.revision_id)
            .bind(&row.capability_id)
            .bind(&row.capability_version)
            .bind(&row.selection_json)
            .execute(&mut *transaction)
            .await?;
        }
        for row in &expected.on_demand_capabilities {
            sqlx::query(
                "INSERT INTO preset_on_demand_capabilities \
                 (revision_id, capability_id, capability_version, selection_json) \
                 VALUES (?, ?, ?, ?) \
                 ON CONFLICT (revision_id, capability_id) DO NOTHING",
            )
            .bind(&row.revision_id)
            .bind(&row.capability_id)
            .bind(&row.capability_version)
            .bind(&row.selection_json)
            .execute(&mut *transaction)
            .await?;
        }
        for row in &expected.skills {
            sqlx::query(
                "INSERT INTO preset_skill_bindings \
                 (revision_id, skill_id, skill_version) VALUES (?, ?, ?) \
                 ON CONFLICT (revision_id, skill_id) DO NOTHING",
            )
            .bind(&row.revision_id)
            .bind(&row.skill_id)
            .bind(&row.skill_version)
            .execute(&mut *transaction)
            .await?;
        }
        for row in &expected.resources {
            sqlx::query(
                "INSERT INTO preset_resource_bindings \
                 (revision_id, resource_binding_id, binding_json) VALUES (?, ?, ?) \
                 ON CONFLICT (revision_id, resource_binding_id) DO NOTHING",
            )
            .bind(&row.revision_id)
            .bind(&row.resource_binding_id)
            .bind(&row.binding_json)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        validate_official_seed(&self.pool, &expected).await
    }

    pub(crate) async fn validate_complete(
        &self,
        expected_metadata: &FreshV4SchemaMetadata,
        inputs: &FrozenRootInputs,
    ) -> Result<(), FreshV4RootError> {
        self.validate_schema(expected_metadata, inputs).await?;
        validate_materialization(
            &self.pool,
            &expected_materialization(&inputs.target_inventory)?,
        )
        .await?;
        validate_official_seed(&self.pool, &expected_official_seed(inputs)?).await?;

        let quick_check: Vec<String> =
            sqlx::query_scalar("PRAGMA quick_check")
                .fetch_all(&self.pool)
                .await?;
        if quick_check.as_slice() != ["ok"] {
            return Err(FreshV4RootError::State(format!(
                "SQLite quick_check failed: {}",
                quick_check.join("; ")
            )));
        }
        let foreign_key_failures: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&self.pool)
                .await?;
        if !foreign_key_failures.is_empty() {
            return Err(FreshV4RootError::State(format!(
                "SQLite foreign_key_check found {} violations",
                foreign_key_failures.len()
            )));
        }
        Ok(())
    }

    async fn validate_schema(
        &self,
        expected: &FreshV4SchemaMetadata,
        inputs: &FrozenRootInputs,
    ) -> Result<(), FreshV4RootError> {
        let actual = read_schema_metadata(&self.pool).await?.ok_or_else(|| {
            FreshV4RootError::State("canonical schema_metadata row is missing".into())
        })?;
        actual
            .validate()
            .map_err(FreshV4RootError::Contract)?;
        if &actual != expected {
            return Err(FreshV4RootError::State(format!(
                "schema_metadata mismatch: expected {expected:?}, found {actual:?}"
            )));
        }

        let schema = fresh_v4_schema_manifest_payload();
        let expected_tables = schema
            .tables
            .into_iter()
            .map(|table| table.table_name)
            .collect::<BTreeSet<_>>();
        let actual_tables = sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_schema \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .collect::<BTreeSet<_>>();
        if actual_tables != expected_tables {
            return Err(FreshV4RootError::State(format!(
                "fresh-v4 table exact-set mismatch: expected {expected_tables:?}, found {actual_tables:?}"
            )));
        }
        let actual_objects = schema_objects(&self.pool).await?;
        let expected_objects = baseline_schema_objects().await?;
        if actual_objects != expected_objects {
            return Err(FreshV4RootError::State(
                "fresh-v4 table/index/trigger definitions do not match the embedded baseline"
                    .into(),
            ));
        }
        let user_version: i64 =
            sqlx::query_scalar("PRAGMA user_version")
                .fetch_one(&self.pool)
                .await?;
        if user_version != 0 {
            return Err(FreshV4RootError::State(format!(
                "PRAGMA user_version must remain 0; schema_metadata owns generation and migration head, found {user_version}"
            )));
        }

        let migrations: Vec<(i64, String, String, i64)> = sqlx::query_as(
            "SELECT version, name, checksum, applied_at \
             FROM schema_migrations ORDER BY version",
        )
        .fetch_all(&self.pool)
        .await?;
        let expected_migrations = vec![(
            i64::from(expected.migration_head),
            BASELINE_MIGRATION_NAME.to_owned(),
            inputs
                .canonical_manifest
                .payload
                .database_schema_digest
                .as_ref()
                .to_owned(),
            0_i64,
        )];
        if migrations != expected_migrations {
            return Err(FreshV4RootError::State(format!(
                "fresh-v4 migration lineage mismatch: {migrations:?}"
            )));
        }
        Ok(())
    }

    pub(crate) async fn close(self) {
        self.pool.close().await;
    }
}

pub(crate) fn expected_schema_metadata(
    root_instance_id: &str,
    inputs: &FrozenRootInputs,
) -> Result<FreshV4SchemaMetadata, FreshV4RootError> {
    let schema = fresh_v4_schema_manifest_payload();
    let metadata = FreshV4SchemaMetadata {
        singleton_key: "canonical".into(),
        data_generation: schema.data_generation,
        root_instance_id: root_instance_id.to_owned(),
        migration_head: schema.migration_head,
        seed_manifest_digest: inputs.seed_manifest_digest.clone(),
        canonical_schema_manifest_digest: inputs.canonical_manifest.payload_digest.clone(),
        projection_schema_version: schema.projection_schema_version,
    };
    metadata
        .validate()
        .map_err(FreshV4RootError::Contract)?;
    Ok(metadata)
}

async fn insert_schema_metadata(
    transaction: &mut Transaction<'_, sqlx::Sqlite>,
    metadata: &FreshV4SchemaMetadata,
) -> Result<(), FreshV4RootError> {
    sqlx::query(
        "INSERT INTO schema_metadata \
         (singleton_key, data_generation, root_instance_id, migration_head, \
          seed_manifest_digest, canonical_schema_manifest_digest, projection_schema_version) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&metadata.singleton_key)
    .bind(i64::from(metadata.data_generation))
    .bind(&metadata.root_instance_id)
    .bind(i64::from(metadata.migration_head))
    .bind(metadata.seed_manifest_digest.as_ref())
    .bind(metadata.canonical_schema_manifest_digest.as_ref())
    .bind(i64::from(metadata.projection_schema_version))
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn read_schema_metadata(
    pool: &SqlitePool,
) -> Result<Option<FreshV4SchemaMetadata>, FreshV4RootError> {
    if !table_exists(pool, "schema_metadata").await? {
        return Ok(None);
    }
    let row: Option<(String, i64, String, i64, String, String, i64)> = sqlx::query_as(
        "SELECT singleton_key, data_generation, root_instance_id, migration_head, \
         seed_manifest_digest, canonical_schema_manifest_digest, projection_schema_version \
         FROM schema_metadata",
    )
    .fetch_optional(pool)
    .await?;
    let Some((
        singleton_key,
        data_generation,
        root_instance_id,
        migration_head,
        seed_manifest_digest,
        canonical_schema_manifest_digest,
        projection_schema_version,
    )) = row
    else {
        return Ok(None);
    };
    let metadata = FreshV4SchemaMetadata {
        singleton_key,
        data_generation: u32_field("data_generation", data_generation)?,
        root_instance_id,
        migration_head: u32_field("migration_head", migration_head)?,
        seed_manifest_digest: DigestHex::from(seed_manifest_digest),
        canonical_schema_manifest_digest: DigestHex::from(canonical_schema_manifest_digest),
        projection_schema_version: u32_field(
            "projection_schema_version",
            projection_schema_version,
        )?,
    };
    Ok(Some(metadata))
}

async fn table_exists(pool: &SqlitePool, table: &str) -> Result<bool, FreshV4RootError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?",
    )
    .bind(table)
    .fetch_one(pool)
    .await?;
    Ok(count == 1)
}

type SchemaObject = (String, String, String, Option<String>);

async fn schema_objects(pool: &SqlitePool) -> Result<Vec<SchemaObject>, FreshV4RootError> {
    Ok(sqlx::query_as(
        "SELECT type, name, tbl_name, sql FROM sqlite_schema \
         WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )
    .fetch_all(pool)
    .await?)
}

async fn baseline_schema_objects() -> Result<Vec<SchemaObject>, FreshV4RootError> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(true),
        )
        .await?;
    sqlx::raw_sql(FRESH_V4_BASELINE_SQL)
        .execute(&pool)
        .await?;
    let objects = schema_objects(&pool).await;
    pool.close().await;
    objects
}

fn u32_field(field: &str, value: i64) -> Result<u32, FreshV4RootError> {
    u32::try_from(value).map_err(|_| {
        FreshV4RootError::State(format!(
            "schema_metadata.{field} is outside the u32 range: {value}"
        ))
    })
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedPackage {
    package_id: String,
    package_version: String,
    manifest_json: String,
    manifest_digest: String,
    display_json: String,
    capabilities: BTreeMap<(String, String), CapabilityKind>,
    skills: BTreeSet<(String, String)>,
    mcp_tools: Vec<nomifun_agent_contracts::McpToolCapabilityMapping>,
    role_contracts: Vec<nomifun_agent_contracts::RoleContractManifest>,
    role_providers: Vec<nomifun_agent_contracts::RoleProviderContribution>,
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedMount {
    mount_id: String,
    package_id: String,
    package_version: String,
    source_json: String,
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedCapability {
    capability_id: String,
    capability_version: String,
    kind: CapabilityKind,
    package_id: String,
    package_version: String,
    manifest_json: String,
    manifest_digest: String,
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedSkill {
    skill_id: String,
    skill_version: String,
    package_id: String,
    package_version: String,
    definition_json: String,
    definition_digest: String,
}

struct ExpectedMaterialization {
    packages: Vec<ExpectedPackage>,
    mounts: Vec<ExpectedMount>,
    capabilities: Vec<ExpectedCapability>,
    skills: Vec<ExpectedSkill>,
}

fn expected_materialization(
    inventory: &TargetPackageInventoryPayload,
) -> Result<ExpectedMaterialization, FreshV4RootError> {
    if inventory
        .packages
        .iter()
        .any(|package| !package.mcp_tools.is_empty())
    {
        return Err(FreshV4RootError::Contract(
            "the frozen target inventory contains MCP mappings without canonical server ownership seed inputs"
                .into(),
        ));
    }

    let mut packages = Vec::new();
    let mut mounts = Vec::new();
    let mut capabilities = Vec::new();
    let mut skills = Vec::new();
    for package in &inventory.packages {
        let package_id = package.package.id.as_ref().to_owned();
        let package_version = package.package.version.as_ref().to_owned();
        let source_json = canonical_json_string(&package.source)?;
        packages.push(ExpectedPackage {
            package_id: package_id.clone(),
            package_version: package_version.clone(),
            manifest_json: canonical_json_string(package)?,
            manifest_digest: digest_string(package)?,
            display_json: canonical_json_string(&LocalizedMetadata {
                name: package_id.clone(),
                description: format!("Bundled package {package_id}"),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            })?,
            capabilities: package
                .capabilities
                .iter()
                .map(|capability| {
                    (
                        (
                            capability.capability.id.as_ref().to_owned(),
                            capability.capability.version.as_ref().to_owned(),
                        ),
                        capability.kind,
                    )
                })
                .collect(),
            skills: package
                .skills
                .iter()
                .map(|skill| {
                    (
                        skill.id.as_ref().to_owned(),
                        skill.version.as_ref().to_owned(),
                    )
                })
                .collect(),
            mcp_tools: package.mcp_tools.clone(),
            role_contracts: package.role_contracts.clone(),
            role_providers: package.role_providers.clone(),
        });
        mounts.push(ExpectedMount {
            mount_id: package_id.clone(),
            package_id: package_id.clone(),
            package_version: package_version.clone(),
            source_json,
        });
        for capability in &package.capabilities {
            capabilities.push(ExpectedCapability {
                capability_id: capability.capability.id.as_ref().to_owned(),
                capability_version: capability.capability.version.as_ref().to_owned(),
                kind: capability.kind,
                package_id: package_id.clone(),
                package_version: package_version.clone(),
                manifest_json: canonical_json_string(capability)?,
                manifest_digest: digest_string(capability)?,
            });
        }
        for skill in &package.skills {
            skills.push(ExpectedSkill {
                skill_id: skill.id.as_ref().to_owned(),
                skill_version: skill.version.as_ref().to_owned(),
                package_id: package_id.clone(),
                package_version: package_version.clone(),
                definition_json: canonical_json_string(skill)?,
                definition_digest: digest_string(skill)?,
            });
        }
    }
    packages.sort_by(|left, right| {
        (&left.package_id, &left.package_version)
            .cmp(&(&right.package_id, &right.package_version))
    });
    mounts.sort_by(|left, right| left.mount_id.cmp(&right.mount_id));
    capabilities.sort_by(|left, right| {
        (&left.capability_id, &left.capability_version)
            .cmp(&(&right.capability_id, &right.capability_version))
    });
    skills.sort_by(|left, right| {
        (&left.skill_id, &left.skill_version)
            .cmp(&(&right.skill_id, &right.skill_version))
    });
    Ok(ExpectedMaterialization {
        packages,
        mounts,
        capabilities,
        skills,
    })
}

async fn validate_materialization(
    pool: &SqlitePool,
    expected: &ExpectedMaterialization,
) -> Result<(), FreshV4RootError> {
    for row in &expected.packages {
        let actual: Option<(String, String, String)> = sqlx::query_as(
            "SELECT manifest_json, manifest_digest, display_json \
             FROM plugin_packages WHERE package_id = ? AND package_version = ?",
        )
        .bind(&row.package_id)
        .bind(&row.package_version)
        .fetch_optional(pool)
        .await?;
        let wanted = Some((
            row.manifest_json.clone(),
            row.manifest_digest.clone(),
            row.display_json.clone(),
        ));
        if actual != wanted
            && !actual
                .as_ref()
                .is_some_and(|value| runtime_package_matches(value, row))
        {
            return Err(FreshV4RootError::State(format!(
                "bundled package materialization mismatch for {}@{}",
                row.package_id, row.package_version
            )));
        }
    }
    for row in &expected.mounts {
        let actual: Option<(String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT package_id, package_version, source_json, desired_state, \
                    effective_state, criticality \
             FROM plugin_mounts WHERE mount_id = ?",
        )
        .bind(&row.mount_id)
        .fetch_optional(pool)
        .await?;
        let wanted = Some((
            row.package_id.clone(),
            row.package_version.clone(),
            row.source_json.clone(),
            "enabled".to_owned(),
            "active".to_owned(),
            "required".to_owned(),
        ));
        if actual != wanted {
            return Err(FreshV4RootError::State(format!(
                "bundled mount materialization mismatch for {}",
                row.mount_id
            )));
        }
        let config: Option<(String, i64)> = sqlx::query_as(
            "SELECT config_json, revision FROM plugin_configs \
             WHERE package_id = ? AND mount_id = ?",
        )
        .bind(&row.package_id)
        .bind(&row.mount_id)
        .fetch_optional(pool)
        .await?;
        if config != Some(("{}".to_owned(), 1)) {
            return Err(FreshV4RootError::State(format!(
                "bundled config materialization mismatch for {}",
                row.mount_id
            )));
        }
    }
    for row in &expected.capabilities {
        let actual: Option<(String, String, String, String)> = sqlx::query_as(
            "SELECT package_id, package_version, manifest_json, manifest_digest \
             FROM capability_definitions \
             WHERE capability_id = ? AND capability_version = ?",
        )
        .bind(&row.capability_id)
        .bind(&row.capability_version)
        .fetch_optional(pool)
        .await?;
        let wanted = Some((
            row.package_id.clone(),
            row.package_version.clone(),
            row.manifest_json.clone(),
            row.manifest_digest.clone(),
        ));
        if actual != wanted
            && !actual
                .as_ref()
                .is_some_and(|value| runtime_capability_matches(value, row))
        {
            return Err(FreshV4RootError::State(format!(
                "capability materialization mismatch for {}@{}",
                row.capability_id, row.capability_version
            )));
        }
    }
    for row in &expected.skills {
        let actual: Option<(String, String, String, String)> = sqlx::query_as(
            "SELECT package_id, package_version, definition_json, definition_digest \
             FROM skill_instructions WHERE skill_id = ? AND skill_version = ?",
        )
        .bind(&row.skill_id)
        .bind(&row.skill_version)
        .fetch_optional(pool)
        .await?;
        let wanted = Some((
            row.package_id.clone(),
            row.package_version.clone(),
            row.definition_json.clone(),
            row.definition_digest.clone(),
        ));
        if actual != wanted
            && !actual
                .as_ref()
                .is_some_and(|value| runtime_skill_matches(value, row))
        {
            return Err(FreshV4RootError::State(format!(
                "skill materialization mismatch for {}@{}",
                row.skill_id, row.skill_version
            )));
        }
    }
    Ok(())
}

/// Bootstrap initially writes the compact target-inventory projection. The
/// composed AgentPlatform then replaces those rows with the full canonical
/// runtime manifests. Both representations must validate at this boundary;
/// the runtime representation still proves its own digest and exact inventory
/// identity instead of being accepted as arbitrary JSON.
fn runtime_package_matches(
    actual: &(String, String, String),
    expected: &ExpectedPackage,
) -> bool {
    let Ok(manifest) = serde_json::from_str::<PackageManifest>(&actual.0) else {
        return false;
    };
    if manifest.package_id.as_ref() != expected.package_id
        || manifest.package_version.as_ref() != expected.package_version
    {
        return false;
    }
    let Ok(digest) = digest_payload(&manifest) else {
        return false;
    };
    if digest.as_ref() != actual.1 {
        return false;
    }
    let Ok(display) = canonical_json_string(&manifest.display) else {
        return false;
    };
    if display != actual.2 {
        return false;
    }
    let capabilities = manifest
        .contributions
        .capabilities
        .iter()
        .map(|capability| {
            (
                (
                    capability.id.as_ref().to_owned(),
                    capability.version.as_ref().to_owned(),
                ),
                capability.kind,
            )
        })
        .collect::<BTreeMap<_, _>>();
    if capabilities != expected.capabilities {
        return false;
    }
    let skills = manifest
        .contributions
        .skills
        .iter()
        .map(|skill| {
            (
                skill.id.as_ref().to_owned(),
                skill.version.as_ref().to_owned(),
            )
        })
        .collect::<BTreeSet<_>>();
    skills == expected.skills
        && manifest.contributions.mcp_tools == expected.mcp_tools
        && manifest.contributions.role_contracts == expected.role_contracts
        && manifest.contributions.role_providers == expected.role_providers
}

fn runtime_capability_matches(
    actual: &(String, String, String, String),
    expected: &ExpectedCapability,
) -> bool {
    let Ok(manifest) = serde_json::from_str::<CapabilityManifest>(&actual.2) else {
        return false;
    };
    if manifest.id.as_ref() != expected.capability_id
        || manifest.version.as_ref() != expected.capability_version
        || manifest.kind != expected.kind
        || manifest.package.id.as_ref() != expected.package_id
        || manifest.package.version.as_ref() != expected.package_version
    {
        return false;
    }
    digest_payload(&manifest)
        .map(|digest| digest.as_ref() == actual.3)
        .unwrap_or(false)
}

fn runtime_skill_matches(
    actual: &(String, String, String, String),
    expected: &ExpectedSkill,
) -> bool {
    let Ok(skill) = serde_json::from_str::<SkillDefinition>(&actual.2) else {
        return false;
    };
    if skill.id.as_ref() != expected.skill_id
        || skill.version.as_ref() != expected.skill_version
        || skill.package.id.as_ref() != expected.package_id
        || skill.package.version.as_ref() != expected.package_version
    {
        return false;
    }
    digest_payload(&skill)
        .map(|digest| digest.as_ref() == actual.3)
        .unwrap_or(false)
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedTemplate {
    template_key: String,
    source_package_id: String,
    source_package_version: String,
    template_json: String,
    template_digest: String,
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedPreset {
    preset_id: String,
    owner_ref_json: String,
    source_json: String,
    display_json: String,
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedRevision {
    revision_id: String,
    preset_id: String,
    schema_version: String,
    editor_document_json: String,
    revision_digest: String,
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedCapabilitySelection {
    revision_id: String,
    capability_id: String,
    capability_version: String,
    selection_json: String,
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedSkillBinding {
    revision_id: String,
    skill_id: String,
    skill_version: String,
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedResourceBinding {
    revision_id: String,
    resource_binding_id: String,
    binding_json: String,
}

struct ExpectedOfficialSeed {
    templates: Vec<ExpectedTemplate>,
    presets: Vec<ExpectedPreset>,
    revisions: Vec<ExpectedRevision>,
    initial_capabilities: Vec<ExpectedCapabilitySelection>,
    on_demand_capabilities: Vec<ExpectedCapabilitySelection>,
    skills: Vec<ExpectedSkillBinding>,
    resources: Vec<ExpectedResourceBinding>,
}

fn expected_official_seed(
    inputs: &FrozenRootInputs,
) -> Result<ExpectedOfficialSeed, FreshV4RootError> {
    let owner_ref_json = canonical_json_string(&PrincipalRef {
        principal_kind: "system".into(),
        principal_id: "official".into(),
    })?;
    let source_json = canonical_json_string(&AgentPresetSource::Official)?;
    let mut templates = Vec::new();
    let mut presets = Vec::new();
    let mut revisions = Vec::new();
    let mut initial_capabilities = Vec::new();
    let mut on_demand_capabilities = Vec::new();
    let mut skills = Vec::new();
    let mut resources = Vec::new();

    for key in OfficialPresetKey::ALL {
        let seed = inputs.seed_manifest.templates.get(&key).ok_or_else(|| {
            FreshV4RootError::Contract(format!(
                "official seed manifest is missing {}",
                key.as_str()
            ))
        })?;
        let source = source_package_for_seed(seed, &inputs.target_inventory)?;
        let template_key = key.as_str().to_owned();
        let revision_id = format!("{template_key}@1");
        let template_json = canonical_json_string(seed)?;
        let template_digest = digest_string(seed)?;

        templates.push(ExpectedTemplate {
            template_key: template_key.clone(),
            source_package_id: source.package.id.as_ref().to_owned(),
            source_package_version: source.package.version.as_ref().to_owned(),
            template_json: template_json.clone(),
            template_digest: template_digest.clone(),
        });
        presets.push(ExpectedPreset {
            preset_id: template_key.clone(),
            owner_ref_json: owner_ref_json.clone(),
            source_json: source_json.clone(),
            display_json: canonical_json_string(&LocalizedMetadata {
                name: key.as_str().to_owned(),
                description: format!("Official {} authoring seed", key.as_str()),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            })?,
        });
        revisions.push(ExpectedRevision {
            revision_id: revision_id.clone(),
            preset_id: key.as_str().to_owned(),
            schema_version: inputs.seed_manifest.manifest_version.as_ref().to_owned(),
            editor_document_json: template_json,
            revision_digest: template_digest,
        });
        for capability in &seed.initial_capabilities {
            initial_capabilities.push(ExpectedCapabilitySelection {
                revision_id: revision_id.clone(),
                capability_id: capability.id.as_ref().to_owned(),
                capability_version: capability.version.as_ref().to_owned(),
                selection_json: canonical_json_string(capability)?,
            });
        }
        for capability in &seed.on_demand_capabilities {
            on_demand_capabilities.push(ExpectedCapabilitySelection {
                revision_id: revision_id.clone(),
                capability_id: capability.id.as_ref().to_owned(),
                capability_version: capability.version.as_ref().to_owned(),
                selection_json: canonical_json_string(capability)?,
            });
        }
        for skill in &seed.skill_bindings {
            skills.push(ExpectedSkillBinding {
                revision_id: revision_id.clone(),
                skill_id: skill.id.as_ref().to_owned(),
                skill_version: skill.version.as_ref().to_owned(),
            });
        }
        for resource in &seed.typed_resource_defaults {
            resources.push(ExpectedResourceBinding {
                revision_id: revision_id.clone(),
                resource_binding_id: resource.slot_key.clone(),
                binding_json: canonical_json_string(resource)?,
            });
        }
    }

    Ok(ExpectedOfficialSeed {
        templates,
        presets,
        revisions,
        initial_capabilities,
        on_demand_capabilities,
        skills,
        resources,
    })
}

fn source_package_for_seed<'a>(
    seed: &OfficialPresetSeed,
    inventory: &'a TargetPackageInventoryPayload,
) -> Result<&'a TargetPackageContribution, FreshV4RootError> {
    for selected in seed
        .initial_capabilities
        .iter()
        .chain(&seed.on_demand_capabilities)
    {
        if let Some(package) = inventory.packages.iter().find(|package| {
            package
                .capabilities
                .iter()
                .any(|capability| &capability.capability == selected)
        }) {
            return Ok(package);
        }
    }
    inventory.packages.iter().min_by(|left, right| {
        (&left.package.id, &left.package.version)
            .cmp(&(&right.package.id, &right.package.version))
    })
    .ok_or_else(|| FreshV4RootError::Contract("target package inventory is empty".into()))
}

async fn validate_official_seed(
    pool: &SqlitePool,
    expected: &ExpectedOfficialSeed,
) -> Result<(), FreshV4RootError> {
    let official_keys = sqlx::query_scalar::<_, String>(
        "SELECT template_key FROM agent_preset_templates \
         WHERE source_kind = 'official' ORDER BY template_key",
    )
    .fetch_all(pool)
    .await?;
    let expected_keys = expected
        .templates
        .iter()
        .map(|row| row.template_key.clone())
        .collect::<BTreeSet<_>>();
    if official_keys.into_iter().collect::<BTreeSet<_>>() != expected_keys {
        return Err(FreshV4RootError::State(
            "official template key exact-set does not contain exactly the frozen seven".into(),
        ));
    }

    for row in &expected.templates {
        let actual: Option<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT source_package_id, source_package_version, source_kind, \
                    template_json, template_digest \
             FROM agent_preset_templates WHERE template_key = ?",
        )
        .bind(&row.template_key)
        .fetch_optional(pool)
        .await?;
        let wanted = Some((
            row.source_package_id.clone(),
            row.source_package_version.clone(),
            "official".to_owned(),
            row.template_json.clone(),
            row.template_digest.clone(),
        ));
        if actual != wanted {
            return Err(FreshV4RootError::State(format!(
                "official template seed mismatch for {}",
                row.template_key
            )));
        }
    }
    for row in &expected.presets {
        let actual: Option<(String, String, String, Option<i64>, i64)> = sqlx::query_as(
            "SELECT owner_ref_json, source_json, display_json, current_stable_revision, \
                    created_at \
             FROM agent_presets WHERE preset_id = ?",
        )
        .bind(&row.preset_id)
        .fetch_optional(pool)
        .await?;
        let wanted = Some((
            row.owner_ref_json.clone(),
            row.source_json.clone(),
            row.display_json.clone(),
            Some(1_i64),
            0_i64,
        ));
        if actual != wanted {
            return Err(FreshV4RootError::State(format!(
                "official authoring preset mismatch for {}",
                row.preset_id
            )));
        }
    }
    for row in &expected.revisions {
        let actual: Option<(String, i64, String, String, String, String, i64, String)> =
            sqlx::query_as(
                "SELECT preset_id, revision_no, schema_version, editor_document_json, \
                        revision_digest, created_by, created_at, reason \
                 FROM agent_preset_revisions WHERE revision_id = ?",
            )
            .bind(&row.revision_id)
            .fetch_optional(pool)
            .await?;
        let wanted = Some((
            row.preset_id.clone(),
            1_i64,
            row.schema_version.clone(),
            row.editor_document_json.clone(),
            row.revision_digest.clone(),
            OFFICIAL_SEED_ACTOR.to_owned(),
            0_i64,
            OFFICIAL_SEED_REASON.to_owned(),
        ));
        if actual != wanted {
            return Err(FreshV4RootError::State(format!(
                "official authoring revision mismatch for {}",
                row.revision_id
            )));
        }
    }

    validate_capability_selection_set(
        pool,
        "preset_initial_capabilities",
        &expected.initial_capabilities,
        &expected.revisions,
    )
    .await?;
    validate_capability_selection_set(
        pool,
        "preset_on_demand_capabilities",
        &expected.on_demand_capabilities,
        &expected.revisions,
    )
    .await?;
    for revision in &expected.revisions {
        let actual_skills: Vec<(String, String)> = sqlx::query_as(
            "SELECT skill_id, skill_version FROM preset_skill_bindings \
             WHERE revision_id = ? ORDER BY skill_id",
        )
        .bind(&revision.revision_id)
        .fetch_all(pool)
        .await?;
        let mut wanted_skills = expected
            .skills
            .iter()
            .filter(|row| row.revision_id == revision.revision_id)
            .map(|row| (row.skill_id.clone(), row.skill_version.clone()))
            .collect::<Vec<_>>();
        wanted_skills.sort();
        if actual_skills != wanted_skills {
            return Err(FreshV4RootError::State(format!(
                "official skill seed mismatch for {}",
                revision.revision_id
            )));
        }

        let actual_resources: Vec<(String, String)> = sqlx::query_as(
            "SELECT resource_binding_id, binding_json FROM preset_resource_bindings \
             WHERE revision_id = ? ORDER BY resource_binding_id",
        )
        .bind(&revision.revision_id)
        .fetch_all(pool)
        .await?;
        let mut wanted_resources = expected
            .resources
            .iter()
            .filter(|row| row.revision_id == revision.revision_id)
            .map(|row| (row.resource_binding_id.clone(), row.binding_json.clone()))
            .collect::<Vec<_>>();
        wanted_resources.sort();
        if actual_resources != wanted_resources {
            return Err(FreshV4RootError::State(format!(
                "official resource seed mismatch for {}",
                revision.revision_id
            )));
        }
    }
    Ok(())
}

async fn validate_capability_selection_set(
    pool: &SqlitePool,
    table: &'static str,
    expected: &[ExpectedCapabilitySelection],
    revisions: &[ExpectedRevision],
) -> Result<(), FreshV4RootError> {
    for revision in revisions {
        let sql = match table {
            "preset_initial_capabilities" => {
                "SELECT capability_id, capability_version, selection_json \
                 FROM preset_initial_capabilities \
                 WHERE revision_id = ? ORDER BY capability_id"
            }
            "preset_on_demand_capabilities" => {
                "SELECT capability_id, capability_version, selection_json \
                 FROM preset_on_demand_capabilities \
                 WHERE revision_id = ? ORDER BY capability_id"
            }
            _ => {
                return Err(FreshV4RootError::Contract(format!(
                    "unsupported official seed table {table}"
                )));
            }
        };
        let actual: Vec<(String, String, String)> = sqlx::query_as(sql)
            .bind(&revision.revision_id)
            .fetch_all(pool)
            .await?;
        let mut wanted = expected
            .iter()
            .filter(|row| row.revision_id == revision.revision_id)
            .map(|row| {
                (
                    row.capability_id.clone(),
                    row.capability_version.clone(),
                    row.selection_json.clone(),
                )
            })
            .collect::<Vec<_>>();
        wanted.sort();
        if actual != wanted {
            return Err(FreshV4RootError::State(format!(
                "official capability seed mismatch in {table} for {}",
                revision.revision_id
            )));
        }
    }
    Ok(())
}

fn canonical_json_string<T: Serialize>(value: &T) -> Result<String, FreshV4RootError> {
    String::from_utf8(canonical_json_bytes(value)?).map_err(|error| {
        FreshV4RootError::Contract(format!(
            "canonical JSON unexpectedly contained invalid UTF-8: {error}"
        ))
    })
}

fn digest_string<T: Serialize>(value: &T) -> Result<String, FreshV4RootError> {
    Ok(digest_payload(value)?.as_ref().to_owned())
}
