use std::path::Path;

use chrono::Utc;

use crate::FreshV4RootError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreshV4FaultPoint {
    AfterQuiesce,
    BeforeSameFilesystemPreflight,
    AfterParentMarkerDurable,
    BeforeCutoverRename,
    AfterCutoverRename,
    AfterCanonicalRootCreated,
    AfterInitializingMarkerDurable,
    AfterSchemaMetadataCommitted,
    AfterMaterializationCommitted,
    AfterSeedCommitted,
    AfterReadyMarkerDurable,
    AfterInitializingMarkerRemoved,
    AfterParentMarkerRemoved,
}

pub trait FreshV4FaultInjector: Send + Sync {
    fn check(&self, point: FreshV4FaultPoint) -> Result<(), FreshV4RootError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoFaults;

impl FreshV4FaultInjector for NoFaults {
    fn check(&self, _point: FreshV4FaultPoint) -> Result<(), FreshV4RootError> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreshV4AccessKind {
    Metadata,
    Read,
    Write,
    Remove,
    RenameSource,
    RenameTarget,
    Database,
}

pub trait FreshV4AccessAudit: Send + Sync {
    fn record(&self, kind: FreshV4AccessKind, path: &Path) -> Result<(), FreshV4RootError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoAccessAudit;

impl FreshV4AccessAudit for NoAccessAudit {
    fn record(
        &self,
        _kind: FreshV4AccessKind,
        _path: &Path,
    ) -> Result<(), FreshV4RootError> {
        Ok(())
    }
}

pub trait FreshV4QuiescePort: Send + Sync {
    fn quiesce_before_root_change(&self) -> Result<(), FreshV4RootError>;
}

/// Cold-start proof used by the application before any service, sidecar, or
/// worker has been constructed in the current process.
#[derive(Clone, Copy, Debug, Default)]
pub struct PreServiceQuiesced;

impl FreshV4QuiescePort for PreServiceQuiesced {
    fn quiesce_before_root_change(&self) -> Result<(), FreshV4RootError> {
        Ok(())
    }
}

pub trait FreshV4Clock: Send + Sync {
    fn utc_timestamp(&self) -> String;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemUtcClock;

impl FreshV4Clock for SystemUtcClock {
    fn utc_timestamp(&self) -> String {
        Utc::now().format("%Y%m%d%H%M%S").to_string()
    }
}
