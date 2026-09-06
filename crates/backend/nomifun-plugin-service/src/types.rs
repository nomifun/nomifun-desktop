use std::path::PathBuf;

use nomifun_agent_contracts::{
    CandidateTestReceipt, PluginAutoApplyEligibility, PluginContractDiff, PluginPackageArtifactV1,
};
use nomifun_api_types::{
    BuildPluginProjectRequest, ConfigurePluginRequest, CreatePluginProjectRequest,
    DeletePluginDataRequest, ImportPluginRequest, RestorePluginPreviousRequest,
    RetryPluginRequest, SetPluginEnabledRequest, TestPluginCandidateRequest,
    UninstallPluginRequest,
};
use nomifun_db::{
    PluginArtifactRow, PluginCandidateTestReceiptRow, PluginMountRow, PluginProjectRow,
    PluginReadyCandidateRow, ProductOperationRow,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginInventory {
    pub library_revision: u64,
    pub artifacts: Vec<PluginArtifactRow>,
    pub projects: Vec<PluginProjectRow>,
    pub mounts: Vec<PluginMountRow>,
    pub candidates: Vec<PluginReadyCandidateRow>,
    pub receipts: Vec<PluginCandidateTestReceiptRow>,
    pub operations: Vec<ProductOperationRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedPluginArtifact {
    pub artifact: nomifun_agent_contracts::PluginPackageArtifactV1,
    pub managed_relative_path: String,
    pub package_root: PathBuf,
    pub already_present: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginServicePaths {
    pub project_relative_root: String,
    pub mount_data_relative_root: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateProjectInput {
    pub owner_user_id: String,
    pub request: CreatePluginProjectRequest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkPluginProjectParams {
    pub owner_user_id: String,
    pub project_id: String,
    pub expected_project_revision: u64,
    pub mount_id: String,
    pub expected_mount_revision: u64,
    pub expected_target_digest: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfigureInput {
    pub owner_user_id: String,
    pub request: ConfigurePluginRequest,
}

pub type BuildRequest = BuildPluginProjectRequest;
pub type ImportRequest = ImportPluginRequest;
pub type TestRequest = TestPluginCandidateRequest;
pub type EnableRequest = SetPluginEnabledRequest;
pub type RetryRequest = RetryPluginRequest;
pub type RestoreRequest = RestorePluginPreviousRequest;
pub type UninstallRequest = UninstallPluginRequest;
pub type DeleteDataRequest = DeletePluginDataRequest;

#[derive(Clone, Debug, PartialEq)]
pub struct BuildOutput {
    pub artifact: PluginPackageArtifactV1,
    pub managed_relative_path: String,
    pub source_snapshot_digest: String,
    pub dependency_lock_digest: String,
    pub base_target_digest: Option<String>,
    pub contract_diff: PluginContractDiff,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateTestOutput {
    pub receipt: CandidateTestReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyAuthorization {
    Manual,
    StandingAuto {
        authorization_revision: u64,
        eligibility: PluginAutoApplyEligibility,
    },
}
