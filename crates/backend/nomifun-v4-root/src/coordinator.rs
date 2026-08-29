use std::fs::File;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use nomifun_agent_contracts::{
    FreshV4OperationKind, FreshV4ParentOperationMarker, FreshV4ReadyMarker,
    FreshV4SchemaMetadata, OperationId, canonical_json_bytes,
};
use uuid::Uuid;

use crate::database::{FreshV4Database, expected_schema_metadata};
use crate::filesystem::{
    EntryKind, RootPaths, atomic_staging_path, create_directory, entry_kind,
    normalize_root, open_regular_read_write, read_bounded, read_bounded_file,
    remove_empty_directory, remove_file_durable, rename_directory,
    require_real_directory, same_filesystem, sync_regular_file, write_atomic_durable,
    write_create_new_durable,
};
use crate::inputs::{FrozenRootInputs, application_build_digest};
use crate::{
    FreshV4AccessAudit, FreshV4Clock, FreshV4FaultInjector, FreshV4FaultPoint,
    FreshV4QuiescePort, FreshV4RootError, NoAccessAudit, NoFaults,
    PreServiceQuiesced, SystemUtcClock,
};

pub const FRESH_V4_DATABASE_FILE: &str = "nomifun-v4.db";
pub const FRESH_V4_INITIALIZING_MARKER_FILE: &str = ".nomifun-v4-initializing.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreshV4RecoveryPhase {
    Ready,
    IntentDurable,
    CutoverRenamed,
    CanonicalRootCreated,
    SchemaMetadataCommitted,
    ReadyMarkerPublished,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreshV4BootstrapOutcome {
    pub canonical_root: PathBuf,
    pub operation_kind: Option<FreshV4OperationKind>,
    pub recovered_from: FreshV4RecoveryPhase,
    pub schema_metadata: FreshV4SchemaMetadata,
    pub ready_marker: FreshV4ReadyMarker,
}

pub struct FreshV4Coordinator<
    Q = PreServiceQuiesced,
    F = NoFaults,
    A = NoAccessAudit,
    C = SystemUtcClock,
> {
    quiesce: Q,
    faults: F,
    audit: A,
    clock: C,
}

impl Default
    for FreshV4Coordinator<PreServiceQuiesced, NoFaults, NoAccessAudit, SystemUtcClock>
{
    fn default() -> Self {
        Self {
            quiesce: PreServiceQuiesced,
            faults: NoFaults,
            audit: NoAccessAudit,
            clock: SystemUtcClock,
        }
    }
}

impl<Q, F, A, C> FreshV4Coordinator<Q, F, A, C>
where
    Q: FreshV4QuiescePort,
    F: FreshV4FaultInjector,
    A: FreshV4AccessAudit,
    C: FreshV4Clock,
{
    pub fn with_ports(quiesce: Q, faults: F, audit: A, clock: C) -> Self {
        Self {
            quiesce,
            faults,
            audit,
            clock,
        }
    }

    pub async fn bootstrap(
        &self,
        canonical_root: &Path,
        application_build_identity: &str,
        protected_roots: &[PathBuf],
    ) -> Result<FreshV4BootstrapOutcome, FreshV4RootError> {
        self.quiesce.quiesce_before_root_change()?;
        self.faults.check(FreshV4FaultPoint::AfterQuiesce)?;

        let inputs = FrozenRootInputs::load()?;
        let build_digest = application_build_digest(application_build_identity)?;
        let paths = normalize_root(canonical_root, protected_roots, &self.audit)?;

        match entry_kind(&paths.parent_marker, &self.audit)? {
            EntryKind::Missing => {
                self.start_without_parent_marker(&paths, &inputs, &build_digest)
                    .await
            }
            EntryKind::File => {
                let guard = self.open_parent_marker(&paths)?;
                self.run_with_parent_marker(
                    &paths,
                    guard,
                    &inputs,
                    &build_digest,
                )
                .await
            }
            kind => Err(FreshV4RootError::State(format!(
                "Fresh-v4 parent marker must be absent or a regular file, found {kind:?}: {}",
                paths.parent_marker.display()
            ))),
        }
    }

    async fn start_without_parent_marker(
        &self,
        paths: &RootPaths,
        inputs: &FrozenRootInputs,
        build_digest: &nomifun_agent_contracts::DigestHex,
    ) -> Result<FreshV4BootstrapOutcome, FreshV4RootError> {
        match entry_kind(&paths.canonical_root, &self.audit)? {
            EntryKind::Missing => {
                let marker = self.new_parent_marker(
                    paths,
                    FreshV4OperationKind::Fresh,
                    None,
                    inputs,
                )?;
                let guard = self.create_parent_marker(paths, marker)?;
                self.run_with_parent_marker(paths, guard, inputs, build_digest)
                    .await
            }
            EntryKind::Directory => {
                let ready_kind = entry_kind(&paths.ready_marker, &self.audit)?;
                let initializing_kind =
                    entry_kind(&paths.initializing_marker, &self.audit)?;
                match (ready_kind, initializing_kind) {
                    (EntryKind::File, EntryKind::Missing) => {
                        let (metadata, ready) = self
                            .validate_ready_root(paths, inputs, build_digest, None)
                            .await?;
                        Ok(FreshV4BootstrapOutcome {
                            canonical_root: paths.canonical_root.clone(),
                            operation_kind: None,
                            recovered_from: FreshV4RecoveryPhase::Ready,
                            schema_metadata: metadata,
                            ready_marker: ready,
                        })
                    }
                    (EntryKind::Missing, EntryKind::Missing) => {
                        self.faults
                            .check(FreshV4FaultPoint::BeforeSameFilesystemPreflight)?;
                        if !same_filesystem(
                            &paths.parent,
                            &paths.canonical_root,
                            &self.audit,
                        )? {
                            return Err(FreshV4RootError::InvalidRoot(
                                "cutover source and archive sibling are not on the same filesystem"
                                    .into(),
                            ));
                        }
                        let archive_basename = format!(
                            "{}.pre-v4-archive-{}",
                            paths.canonical_basename,
                            self.clock.utc_timestamp()
                        );
                        let marker = self.new_parent_marker(
                            paths,
                            FreshV4OperationKind::Cutover,
                            Some(archive_basename),
                            inputs,
                        )?;
                        let archive = archive_path(paths, &marker)?;
                        if entry_kind(&archive, &self.audit)? != EntryKind::Missing {
                            return Err(FreshV4RootError::State(format!(
                                "cutover archive target already exists: {}",
                                archive.display()
                            )));
                        }
                        let guard = self.create_parent_marker(paths, marker)?;
                        self.run_with_parent_marker(
                            paths,
                            guard,
                            inputs,
                            build_digest,
                        )
                        .await
                    }
                    (EntryKind::File, _) => Err(FreshV4RootError::State(format!(
                        "ready Fresh-v4 root has a residual or invalid initializing marker: {}",
                        paths.initializing_marker.display()
                    ))),
                    (ready, initializing) => Err(FreshV4RootError::State(format!(
                        "canonical root has invalid Fresh-v4 marker entries: ready={ready:?}, initializing={initializing:?}"
                    ))),
                }
            }
            kind => Err(FreshV4RootError::InvalidRoot(format!(
                "canonical data root must be absent or a real directory, found {kind:?}: {}",
                paths.canonical_root.display()
            ))),
        }
    }

    fn new_parent_marker(
        &self,
        paths: &RootPaths,
        operation_kind: FreshV4OperationKind,
        archive_basename: Option<String>,
        inputs: &FrozenRootInputs,
    ) -> Result<FreshV4ParentOperationMarker, FreshV4RootError> {
        let marker = FreshV4ParentOperationMarker {
            operation_id: OperationId::from(Uuid::now_v7().to_string()),
            operation_kind,
            canonical_normalized_relative_basename: paths.canonical_basename.clone(),
            cutover_archive_sibling_relative_basename: archive_basename,
            target_data_generation:
                nomifun_agent_contracts::FRESH_V4_DATA_GENERATION,
            canonical_schema_manifest_digest: inputs
                .canonical_manifest
                .payload_digest
                .clone(),
        };
        marker
            .validate()
            .map_err(FreshV4RootError::Contract)?;
        Ok(marker)
    }

    fn create_parent_marker(
        &self,
        paths: &RootPaths,
        marker: FreshV4ParentOperationMarker,
    ) -> Result<ParentMarkerGuard, FreshV4RootError> {
        if entry_kind(&paths.parent_marker, &self.audit)? != EntryKind::Missing {
            return Err(FreshV4RootError::State(format!(
                "Fresh-v4 parent marker target is no longer absent: {}",
                paths.parent_marker.display()
            )));
        }
        let bytes = canonical_json_bytes(&marker)?;
        let file =
            write_create_new_durable(&paths.parent_marker, &bytes, &self.audit)?;
        lock_parent_marker(&file, &paths.parent_marker)?;
        self.faults
            .check(FreshV4FaultPoint::AfterParentMarkerDurable)?;
        Ok(ParentMarkerGuard {
            _file: file,
            marker,
            bytes,
        })
    }

    fn open_parent_marker(
        &self,
        paths: &RootPaths,
    ) -> Result<ParentMarkerGuard, FreshV4RootError> {
        let mut file = open_regular_read_write(&paths.parent_marker, &self.audit)?;
        lock_parent_marker(&file, &paths.parent_marker)?;
        let bytes =
            read_bounded_file(&mut file, &paths.parent_marker, &self.audit)?;
        let marker: FreshV4ParentOperationMarker = serde_json::from_slice(&bytes)?;
        marker
            .validate()
            .map_err(FreshV4RootError::Contract)?;
        if bytes != canonical_json_bytes(&marker)? {
            return Err(FreshV4RootError::State(format!(
                "Fresh-v4 parent marker is not canonical JSON: {}",
                paths.parent_marker.display()
            )));
        }
        Ok(ParentMarkerGuard {
            _file: file,
            marker,
            bytes,
        })
    }

    async fn run_with_parent_marker(
        &self,
        paths: &RootPaths,
        guard: ParentMarkerGuard,
        inputs: &FrozenRootInputs,
        build_digest: &nomifun_agent_contracts::DigestHex,
    ) -> Result<FreshV4BootstrapOutcome, FreshV4RootError> {
        self.validate_parent_marker_binding(paths, &guard.marker, inputs)?;
        let operation_kind = guard.marker.operation_kind;
        let recovered_from =
            self.recovery_phase(paths, &guard.marker).await?;

        match operation_kind {
            FreshV4OperationKind::Fresh => {
                if guard
                    .marker
                    .cutover_archive_sibling_relative_basename
                    .is_some()
                {
                    return Err(FreshV4RootError::Contract(
                        "fresh operation unexpectedly carries an archive basename".into(),
                    ));
                }
            }
            FreshV4OperationKind::Cutover => {
                self.prepare_cutover_root(paths, &guard.marker).await?;
            }
        }
        self.ensure_initializing_root(paths, &guard).await?;

        let (metadata, ready) = match entry_kind(&paths.ready_marker, &self.audit)? {
            EntryKind::File => {
                self.validate_ready_root(
                    paths,
                    inputs,
                    build_digest,
                    Some(guard.marker.operation_id.as_ref()),
                )
                .await?
            }
            EntryKind::Missing => {
                match entry_kind(&paths.database, &self.audit)? {
                    EntryKind::Missing | EntryKind::File => {}
                    kind => {
                        return Err(FreshV4RootError::State(format!(
                            "initializing database has invalid kind {kind:?}: {}",
                            paths.database.display()
                        )));
                    }
                }
                let database =
                    FreshV4Database::open_for_initialization(
                        &paths.database,
                        &self.audit,
                    )
                    .await?;
                let metadata = database
                    .ensure_schema_metadata(
                        guard.marker.operation_id.as_ref(),
                        inputs,
                    )
                    .await?;
                self.faults
                    .check(FreshV4FaultPoint::AfterSchemaMetadataCommitted)?;
                database.ensure_materialization(inputs).await?;
                self.faults
                    .check(FreshV4FaultPoint::AfterMaterializationCommitted)?;
                database.ensure_official_seed(inputs).await?;
                self.faults
                    .check(FreshV4FaultPoint::AfterSeedCommitted)?;
                database.validate_complete(&metadata, inputs).await?;
                database.close().await;
                sync_regular_file(&paths.database, &self.audit)?;

                let ready = ready_from_metadata(&metadata, build_digest.clone())?;
                let bytes = canonical_json_bytes(&ready)?;
                write_atomic_durable(&paths.ready_marker, &bytes, &self.audit)?;
                self.faults
                    .check(FreshV4FaultPoint::AfterReadyMarkerDurable)?;
                self.validate_ready_root(
                    paths,
                    inputs,
                    build_digest,
                    Some(guard.marker.operation_id.as_ref()),
                )
                .await?
            }
            kind => {
                return Err(FreshV4RootError::State(format!(
                    "Fresh-v4 ready marker must be absent or a regular file, found {kind:?}: {}",
                    paths.ready_marker.display()
                )));
            }
        };

        self.remove_initializing_marker(paths, &guard)?;
        self.faults
            .check(FreshV4FaultPoint::AfterInitializingMarkerRemoved)?;
        remove_file_durable(&paths.parent_marker, &self.audit)?;
        self.faults
            .check(FreshV4FaultPoint::AfterParentMarkerRemoved)?;
        drop(guard);

        Ok(FreshV4BootstrapOutcome {
            canonical_root: paths.canonical_root.clone(),
            operation_kind: Some(operation_kind),
            recovered_from,
            schema_metadata: metadata,
            ready_marker: ready,
        })
    }

    fn validate_parent_marker_binding(
        &self,
        paths: &RootPaths,
        marker: &FreshV4ParentOperationMarker,
        inputs: &FrozenRootInputs,
    ) -> Result<(), FreshV4RootError> {
        if marker.canonical_normalized_relative_basename
            != paths.canonical_basename
        {
            return Err(FreshV4RootError::State(format!(
                "parent marker canonical basename {:?} does not match requested root {:?}",
                marker.canonical_normalized_relative_basename,
                paths.canonical_basename
            )));
        }
        if marker.canonical_schema_manifest_digest
            != inputs.canonical_manifest.payload_digest
        {
            return Err(FreshV4RootError::State(
                "parent marker canonical schema manifest digest does not match this build"
                    .into(),
            ));
        }
        Ok(())
    }

    async fn prepare_cutover_root(
        &self,
        paths: &RootPaths,
        marker: &FreshV4ParentOperationMarker,
    ) -> Result<(), FreshV4RootError> {
        let archive = archive_path(paths, marker)?;
        let root_kind = entry_kind(&paths.canonical_root, &self.audit)?;
        let archive_kind = entry_kind(&archive, &self.audit)?;
        match (root_kind, archive_kind) {
            (EntryKind::Directory, EntryKind::Missing) => {
                let ready_kind =
                    entry_kind(&paths.ready_marker, &self.audit)?;
                let initializing_kind =
                    entry_kind(&paths.initializing_marker, &self.audit)?;
                if ready_kind != EntryKind::Missing
                    || initializing_kind != EntryKind::Missing
                {
                    return Err(FreshV4RootError::State(format!(
                        "cutover archive is missing while the canonical root carries Fresh-v4 markers: ready={ready_kind:?}, initializing={initializing_kind:?}"
                    )));
                }
                self.faults
                    .check(FreshV4FaultPoint::BeforeSameFilesystemPreflight)?;
                require_real_directory(
                    &paths.canonical_root,
                    "cutover source",
                    &self.audit,
                )?;
                if !same_filesystem(
                    &paths.parent,
                    &paths.canonical_root,
                    &self.audit,
                )? {
                    return Err(FreshV4RootError::InvalidRoot(
                        "cutover source and archive sibling are not on the same filesystem"
                            .into(),
                    ));
                }
                self.faults
                    .check(FreshV4FaultPoint::BeforeCutoverRename)?;
                rename_directory(
                    &paths.canonical_root,
                    &archive,
                    &self.audit,
                )?;
                self.faults
                    .check(FreshV4FaultPoint::AfterCutoverRename)?;
            }
            (EntryKind::Missing, EntryKind::Directory)
            | (EntryKind::Directory, EntryKind::Directory) => {}
            (EntryKind::Directory, archive_kind) => {
                return Err(FreshV4RootError::State(format!(
                    "cutover archive target has invalid kind {archive_kind:?}: {}",
                    archive.display()
                )));
            }
            (root_kind, EntryKind::Directory) => {
                return Err(FreshV4RootError::State(format!(
                    "cutover canonical root has invalid kind {root_kind:?}: {}",
                    paths.canonical_root.display()
                )));
            }
            (root_kind, archive_kind) => {
                return Err(FreshV4RootError::State(format!(
                    "cutover state is inconsistent: canonical={root_kind:?}, archive={archive_kind:?}"
                )));
            }
        }
        Ok(())
    }

    async fn ensure_initializing_root(
        &self,
        paths: &RootPaths,
        guard: &ParentMarkerGuard,
    ) -> Result<(), FreshV4RootError> {
        let mut created_now = false;
        match entry_kind(&paths.canonical_root, &self.audit)? {
            EntryKind::Missing => {
                create_directory(&paths.canonical_root, &self.audit)?;
                created_now = true;
                self.faults
                    .check(FreshV4FaultPoint::AfterCanonicalRootCreated)?;
            }
            EntryKind::Directory => {}
            kind => {
                return Err(FreshV4RootError::State(format!(
                    "initializing canonical root must be absent or a real directory, found {kind:?}: {}",
                    paths.canonical_root.display()
                )));
            }
        }

        match entry_kind(&paths.ready_marker, &self.audit)? {
            EntryKind::File => {
                match entry_kind(&paths.initializing_marker, &self.audit)? {
                    EntryKind::Missing => return Ok(()),
                    EntryKind::File => {
                        self.require_initializing_marker(paths, guard)?;
                        return Ok(());
                    }
                    kind => {
                        return Err(FreshV4RootError::State(format!(
                            "ready root has invalid initializing marker kind {kind:?}: {}",
                            paths.initializing_marker.display()
                        )));
                    }
                }
            }
            EntryKind::Missing => {}
            kind => {
                return Err(FreshV4RootError::State(format!(
                    "ready marker has invalid kind {kind:?}: {}",
                    paths.ready_marker.display()
                )));
            }
        }

        match entry_kind(&paths.initializing_marker, &self.audit)? {
            EntryKind::File => self.require_initializing_marker(paths, guard)?,
            EntryKind::Missing => {
                if !created_now {
                    if entry_kind(&paths.database, &self.audit)?
                        != EntryKind::Missing
                    {
                        return Err(FreshV4RootError::State(format!(
                            "unbound incomplete root already contains a database: {}",
                            paths.database.display()
                        )));
                    }
                    match remove_empty_directory(
                        &paths.canonical_root,
                        &self.audit,
                    ) {
                        Ok(()) => {
                            create_directory(
                                &paths.canonical_root,
                                &self.audit,
                            )?;
                            self.faults.check(
                                FreshV4FaultPoint::AfterCanonicalRootCreated,
                            )?;
                        }
                        Err(error) => {
                            return Err(FreshV4RootError::State(format!(
                                "incomplete canonical root is not safely removable before initialization: {} ({error})",
                                paths.canonical_root.display()
                            )));
                        }
                    }
                }
                let file = write_create_new_durable(
                    &paths.initializing_marker,
                    &guard.bytes,
                    &self.audit,
                )?;
                drop(file);
                self.faults
                    .check(FreshV4FaultPoint::AfterInitializingMarkerDurable)?;
            }
            kind => {
                return Err(FreshV4RootError::State(format!(
                    "initializing marker must be absent or a regular file, found {kind:?}: {}",
                    paths.initializing_marker.display()
                )));
            }
        }
        Ok(())
    }

    fn require_initializing_marker(
        &self,
        paths: &RootPaths,
        guard: &ParentMarkerGuard,
    ) -> Result<(), FreshV4RootError> {
        let bytes = read_bounded(&paths.initializing_marker, &self.audit)?;
        if bytes != guard.bytes {
            return Err(FreshV4RootError::State(format!(
                "initializing marker does not exactly bind the immutable parent operation: {}",
                paths.initializing_marker.display()
            )));
        }
        Ok(())
    }

    fn remove_initializing_marker(
        &self,
        paths: &RootPaths,
        guard: &ParentMarkerGuard,
    ) -> Result<(), FreshV4RootError> {
        match entry_kind(&paths.initializing_marker, &self.audit)? {
            EntryKind::Missing => Ok(()),
            EntryKind::File => {
                self.require_initializing_marker(paths, guard)?;
                remove_file_durable(&paths.initializing_marker, &self.audit)
            }
            kind => Err(FreshV4RootError::State(format!(
                "initializing marker has invalid kind {kind:?}: {}",
                paths.initializing_marker.display()
            ))),
        }
    }

    async fn validate_ready_root(
        &self,
        paths: &RootPaths,
        inputs: &FrozenRootInputs,
        build_digest: &nomifun_agent_contracts::DigestHex,
        expected_root_instance_id: Option<&str>,
    ) -> Result<(FreshV4SchemaMetadata, FreshV4ReadyMarker), FreshV4RootError> {
        require_real_directory(
            &paths.canonical_root,
            "ready canonical root",
            &self.audit,
        )?;
        if entry_kind(&paths.ready_marker, &self.audit)? != EntryKind::File {
            return Err(FreshV4RootError::State(format!(
                "Fresh-v4 ready marker is missing: {}",
                paths.ready_marker.display()
            )));
        }
        let ready_staging = atomic_staging_path(&paths.ready_marker)?;
        if entry_kind(&ready_staging, &self.audit)? != EntryKind::Missing {
            return Err(FreshV4RootError::State(format!(
                "Fresh-v4 ready staging path must be absent: {}",
                ready_staging.display()
            )));
        }
        if entry_kind(&paths.database, &self.audit)? != EntryKind::File {
            return Err(FreshV4RootError::State(format!(
                "Fresh-v4 database is missing or not a regular file: {}",
                paths.database.display()
            )));
        }

        let ready_bytes = read_bounded(&paths.ready_marker, &self.audit)?;
        let ready: FreshV4ReadyMarker = serde_json::from_slice(&ready_bytes)?;
        ready.validate().map_err(FreshV4RootError::Contract)?;
        if ready_bytes != canonical_json_bytes(&ready)? {
            return Err(FreshV4RootError::State(format!(
                "Fresh-v4 ready marker is not canonical JSON: {}",
                paths.ready_marker.display()
            )));
        }
        if &ready.application_build_digest != build_digest {
            return Err(FreshV4RootError::State(
                "Fresh-v4 ready marker application build digest does not match this build"
                    .into(),
            ));
        }

        let database =
            FreshV4Database::open_read_only(&paths.database, &self.audit).await?;
        let metadata = database
            .inspect_schema_metadata()
            .await?
            .ok_or_else(|| {
                FreshV4RootError::State(
                    "Fresh-v4 ready root has no schema_metadata row".into(),
                )
            })?;
        let expected =
            expected_schema_metadata(&metadata.root_instance_id, inputs)?;
        if metadata != expected {
            database.close().await;
            return Err(FreshV4RootError::State(format!(
                "ready root schema_metadata does not match the embedded Fresh-v4 contract: {metadata:?}"
            )));
        }
        if expected_root_instance_id
            .is_some_and(|expected_id| metadata.root_instance_id != expected_id)
        {
            database.close().await;
            return Err(FreshV4RootError::State(
                "ready root_instance_id does not match the immutable parent operation"
                    .into(),
            ));
        }
        database.validate_complete(&metadata, inputs).await?;
        database.close().await;

        if !ready.matches_schema_metadata(&metadata) {
            return Err(FreshV4RootError::State(
                "Fresh-v4 ready marker does not match schema_metadata".into(),
            ));
        }
        Ok((metadata, ready))
    }

    async fn recovery_phase(
        &self,
        paths: &RootPaths,
        marker: &FreshV4ParentOperationMarker,
    ) -> Result<FreshV4RecoveryPhase, FreshV4RootError> {
        if entry_kind(&paths.ready_marker, &self.audit)? == EntryKind::File {
            return Ok(FreshV4RecoveryPhase::ReadyMarkerPublished);
        }
        if entry_kind(&paths.initializing_marker, &self.audit)? == EntryKind::File {
            match entry_kind(&paths.database, &self.audit)? {
                EntryKind::Missing => {
                    return Ok(FreshV4RecoveryPhase::CanonicalRootCreated);
                }
                EntryKind::File => {
                    let database =
                        FreshV4Database::open_read_only(
                            &paths.database,
                            &self.audit,
                        )
                        .await?;
                    let metadata = database.inspect_schema_metadata().await?;
                    database.close().await;
                    return Ok(if metadata.is_some() {
                        FreshV4RecoveryPhase::SchemaMetadataCommitted
                    } else {
                        FreshV4RecoveryPhase::CanonicalRootCreated
                    });
                }
                kind => {
                    return Err(FreshV4RootError::State(format!(
                        "incomplete Fresh-v4 database has invalid kind {kind:?}: {}",
                        paths.database.display()
                    )));
                }
            }
        }
        if marker.operation_kind == FreshV4OperationKind::Cutover {
            let archive = archive_path(paths, marker)?;
            let root_kind = entry_kind(&paths.canonical_root, &self.audit)?;
            let archive_kind = entry_kind(&archive, &self.audit)?;
            match (root_kind, archive_kind) {
                (EntryKind::Directory, EntryKind::Missing) => {
                    return Ok(FreshV4RecoveryPhase::IntentDurable);
                }
                (EntryKind::Missing, EntryKind::Directory) => {
                    return Ok(FreshV4RecoveryPhase::CutoverRenamed);
                }
                _ => {}
            }
        }
        if entry_kind(&paths.canonical_root, &self.audit)? == EntryKind::Directory {
            return Ok(FreshV4RecoveryPhase::CanonicalRootCreated);
        }
        Ok(FreshV4RecoveryPhase::IntentDurable)
    }
}

struct ParentMarkerGuard {
    _file: File,
    marker: FreshV4ParentOperationMarker,
    bytes: Vec<u8>,
}

fn lock_parent_marker(file: &File, path: &Path) -> Result<(), FreshV4RootError> {
    file.try_lock_exclusive().map_err(|error| {
        FreshV4RootError::Quiesce(format!(
            "another Fresh-v4 coordinator owns {}: {error}",
            path.display()
        ))
    })
}

fn archive_path(
    paths: &RootPaths,
    marker: &FreshV4ParentOperationMarker,
) -> Result<PathBuf, FreshV4RootError> {
    let basename = marker
        .cutover_archive_sibling_relative_basename
        .as_deref()
        .ok_or_else(|| {
            FreshV4RootError::Contract(
                "cutover marker is missing its archive sibling basename".into(),
            )
        })?;
    Ok(paths.parent.join(basename))
}

fn ready_from_metadata(
    metadata: &FreshV4SchemaMetadata,
    application_build_digest: nomifun_agent_contracts::DigestHex,
) -> Result<FreshV4ReadyMarker, FreshV4RootError> {
    let ready = FreshV4ReadyMarker {
        data_generation: metadata.data_generation,
        root_instance_id: metadata.root_instance_id.clone(),
        migration_head: metadata.migration_head,
        seed_manifest_digest: metadata.seed_manifest_digest.clone(),
        canonical_schema_manifest_digest: metadata
            .canonical_schema_manifest_digest
            .clone(),
        projection_schema_version: metadata.projection_schema_version,
        application_build_digest,
    };
    ready.validate().map_err(FreshV4RootError::Contract)?;
    Ok(ready)
}
